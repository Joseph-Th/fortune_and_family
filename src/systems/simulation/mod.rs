//! Deterministic daily economic pipeline and 30-day/360-day coordinated cadence.
//!
//! Purpose: advance one campaign day through a fixed 18-step causal order
//! (routes/laws/crises → purchases → production → sales → household
//! consumption → maintenance → spoilage/pricing → price controls → lifecycle
//! → clock/expiry → weekly/monthly/annual strategic hooks → phase refresh →
//! chronicle/audit → invariants). Order is a product contract.
//! Owns: `advance_days` (clone-then-replace atomicity), `advance_days_scratch`
//! (in-place variant for disposable harness branches), and the per-day
//! decide/apply phases. `purchases.rs` owns input procurement;
//! `market.rs` owns spoilage, pricing, and break-even floors; this file owns
//! workshop maintenance and lifecycle.
//! Reads: `Registry` (immutable defs) and `AppState` (mutable working copy).
//! Mutates: the working `AppState`; callers observe all-or-nothing
//! replacement — a failed day leaves the original unchanged.
//! Does not own: scheduled weekly/monthly/annual rules (`strategic/*.rs`)
//! or persistence/validation.
//! Canonical operations: `advance_days(registry, &mut state, days)` and
//! `advance_days_scratch` (disposable branch); `run_one_day` orchestrates
//! the 18 steps with `decide_*` → `apply_*` per phase.
//! Relevant invariants: maintenance and procurement respect cash reserves and
//! tool scarcity with daily rotation for fair allocation; pricing enforces
//! production break-even floors so operating income covers input costs;
//! business status transitions respect cash/inventory thresholds with
//! hysteresis; audit days are nondecreasing.
//! Determinism: ordered `BTreeMap` iteration, typed-ID tie-breakers,
//! state-owned RNG; `DailyCapacityScratch` preserves order under parallel
//! planning.
//! Focused tests: `src/systems/simulation/simulation_tests.rs`, soak and
//! deterministic-replay gates.

use super::SimulationError;
use super::transactions::{
    checked_future_day, next_business_finance_version, next_family_charter_version,
};
use crate::core::{
    AppState, AuditKind, AuditRecord, BusinessStatus, Character, CharacterCapabilities,
    CharacterIdentity, CharacterRole, CharacterRuntime, CharacterStatus, ChronicleEntry,
    ChronicleKind, CrisisKind, FamilyLink, FamilyLinkKind, HouseGovernance, OutboxKind,
    SocialClass,
};
// `EmploymentStatus` and `MarketCause` are re-exported via `super::*` for
// `simulation_tests.rs` (`use super::*`); production paths reference them
// through `crate::core::*`/`crate::systems::strategic::*` instead, so keep
// the test-visible re-exports without triggering `unused_imports` in dev builds.
#[allow(unused_imports)]
use crate::core::{EmploymentStatus, MarketCause};
use crate::ids::{BusinessId, CharacterId, DynastyId, GoodId, RecipeId};
use crate::money::{Money, Quantity, affordable_quantity, checked_cost_for, cost_for};
#[allow(unused_imports)]
use crate::registry::{GoodCategory, RecipeDef, Registry};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod market;
mod purchases;
#[allow(unused_imports)]
pub(crate) use market::{
    PRICE_SHOCK_REPEAT_SUPPRESSION_DAYS, PRICE_SHOCK_SUMMARY_SEPARATOR, PRICE_SHOCKS_PER_DAY,
    apply_market_spoilage, business_sustainable_unit_cost, ceil_div_nonnegative_wide,
    price_shock_good_name, price_shock_summary, production_price_floors, recently_shocked_goods,
    update_market_prices,
};
#[allow(unused_imports)]
pub(crate) use purchases::{apply_business_purchases, decide_business_purchases};

#[derive(Clone, Debug)]
struct ProductionLine {
    business_id: BusinessId,
    inputs: Vec<(GoodId, Quantity)>,
    output_good_id: GoodId,
    output_quantity: Quantity,
    operating_cost: Money,
    tool_quantity: Quantity,
    tool_cost: Money,
}

#[derive(Clone, Debug)]
struct ProductionPlan {
    tools_id: GoodId,
    lines: Vec<ProductionLine>,
}

#[derive(Clone, Debug)]
struct BusinessSaleLine {
    business_id: BusinessId,
    good_id: GoodId,
    quantity: Quantity,
    revenue: Money,
}

#[derive(Clone, Debug)]
struct BusinessSalePlan {
    lines: Vec<BusinessSaleLine>,
}

#[derive(Clone, Debug)]
struct HouseholdPurchaseLine {
    household_id: crate::ids::HouseholdId,
    good_id: GoodId,
    quantity: Quantity,
    cost: Money,
}

#[derive(Clone, Debug)]
struct HouseholdConsumptionPlan {
    lines: Vec<HouseholdPurchaseLine>,
    food_satisfaction: BTreeMap<crate::ids::HouseholdId, u16>,
}

#[derive(Clone, Debug)]
struct MaintenanceLine {
    business_id: BusinessId,
    cost: Money,
    tool_quantity: Quantity,
    tool_cost: Money,
    condition_delta: i16,
    quality_delta: i16,
}

#[derive(Clone, Debug)]
struct MaintenancePlan {
    tools_id: GoodId,
    lines: Vec<MaintenanceLine>,
}

#[derive(Clone, Debug)]
struct SuccessionLine {
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
    formally_prepared: bool,
    family_unity_loss: u16,
    family_loyalty_loss: u16,
    legitimacy_loss: u16,
    new_heir_name: String,
    new_heir_birth_day: i64,
    new_heir_link_kind: FamilyLinkKind,
    next_generation: u16,
    next_charter_version: u64,
    new_heir_capabilities: CharacterCapabilities,
}

#[derive(Clone, Copy, Debug)]
struct SuccessionShock {
    formally_prepared: bool,
    family_unity_loss: u16,
    family_loyalty_loss: u16,
    legitimacy_loss: u16,
}

/// Advances the canonical simulation pipeline by a positive number of days.
///
/// # Errors
///
#[allow(clippy::missing_errors_doc)]
fn validate_advance_preconditions(
    registry: &Registry,
    state: &AppState,
    days: u32,
) -> Result<(), SimulationError> {
    if days == 0 {
        return Err(SimulationError::InvalidDayCount { days });
    }
    if state
        .clock
        .day()
        .checked_add(i64::from(days))
        .is_none_or(|final_day| final_day == i64::MAX)
    {
        return Err(SimulationError::DayRangeExhausted {
            current_day: state.clock.day(),
            requested_days: days,
        });
    }
    if state.scenario_key != registry.scenario().key() {
        return Err(SimulationError::RegistryMismatch {
            state_scenario: state.scenario_key.clone(),
            registry_scenario: registry.scenario().key().to_owned(),
        });
    }
    validate_market_quotes(registry, state)
}

/// Advances the canonical simulation pipeline by a positive number of days.
///
/// # Errors
///
/// Returns an error for a zero day count, an exhausted day or schedule range, a registry mismatch,
/// missing market definitions, identifier-allocation exhaustion, or a business finance ledger that
/// cannot represent a required mutation. The campaign is unchanged when any requested day fails.
pub fn advance_days(
    registry: &Registry,
    state: &mut AppState,
    days: u32,
) -> Result<(), SimulationError> {
    validate_advance_preconditions(registry, state, days)?;

    // Invariant IDs are per-registry constants; prepare once per call so the debug sweep
    // scales with state size. Invariants run only in debug builds.
    let invariant_ids = super::invariants::prepare_invariant_ids(registry);
    let mut next_state = state.clone();
    run_day_loop(registry, &mut next_state, days, invariant_ids.as_ref())?;
    *state = next_state;

    Ok(())
}

/// Advances an exclusively owned scratch state in place.
///
/// Identical to [`advance_days`] on success, including validation and ordering.
/// Skips the defensive whole-campaign copy: the caller must hold `state` as
/// a disposable branch and discard it when a day fails (state may be partially advanced).
/// Used by harness counterfactual branches to avoid a second deep copy per branch.
pub(crate) fn advance_days_scratch(
    registry: &Registry,
    state: &mut AppState,
    days: u32,
) -> Result<(), SimulationError> {
    validate_advance_preconditions(registry, state, days)?;
    let invariant_ids = super::invariants::prepare_invariant_ids(registry);
    run_day_loop(registry, state, days, invariant_ids.as_ref())
}

/// Shared day loop of both advance entries; `state` is mutated in place.
fn run_day_loop(
    registry: &Registry,
    state: &mut AppState,
    days: u32,
    invariant_ids: Option<&super::invariants::RegistryIds>,
) -> Result<(), SimulationError> {
    for _ in 0..days {
        run_one_day(registry, state)?;
        if let Some(ids) = invariant_ids {
            super::invariants::validate_invariants_with_ids(registry, state, ids);
        }
    }
    Ok(())
}

fn validate_market_quotes(registry: &Registry, state: &AppState) -> Result<(), SimulationError> {
    for good in registry.goods() {
        if state.market.get_quote(good.id()).is_none() {
            return Err(SimulationError::MarketQuoteMissing { good_id: good.id() });
        }
    }
    Ok(())
}

/// Executes one deterministic simulation day in the canonical 18-step order.
///
/// The order is the product contract (see `ARCHITECTURE.md`): 1 reset
/// flows, 2 routes/laws/crisis/AI-recovery/route-supply, 3-4 business
/// purchases, 5-6 production, 7-8 sales, 9-10 household consumption,
/// 11-12 maintenance, 13-14 spoilage + pricing + price controls, 15
/// lifecycle, 16 clock + expiry, 17 weekly/monthly/annual hooks +
/// phase refresh, 18 audit + invariant validation (outer loop).
fn run_one_day(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
    // 1. Reset per-good daily flow counters so the day's demand/supply is
    // isolated.
    reset_market_flows(state);
    // 2. Daily strategic pre-phase: routes, laws, crisis effects, AI
    // recovery, external route supply.
    super::strategic::run_daily_strategic_systems(registry, state)?;
    // Business status is fixed until lifecycle evaluation after sales; one snapshot serves
    // purchases, production, and sales without per-phase rescans.
    let capacity_scratch = super::DailyCapacityScratch::collect(state);

    let purchase_plan = purchases::decide_business_purchases(registry, state, &capacity_scratch)?;
    purchases::apply_business_purchases(state, purchase_plan)?;

    let production_plan = decide_production(registry, state, &capacity_scratch);
    apply_production(state, production_plan)?;

    let sale_plan = decide_business_sales(registry, state, &capacity_scratch)?;
    apply_business_sales(state, sale_plan)?;

    let household_plan = decide_household_consumption(registry, state);
    apply_household_consumption(state, household_plan)?;

    let maintenance_plan = decide_maintenance(registry, state);
    apply_maintenance(state, maintenance_plan)?;

    apply_market_spoilage(registry, state);
    update_market_prices(registry, state)?;
    super::strategic::apply_law_price_controls(registry, state);
    update_business_lifecycle(registry, state)?;

    state.clock.advance_one_day();
    super::strategic::expire_time_limited_state(state);
    if state.clock.is_week_boundary() {
        settle_weekly_external_income(state)?;
        super::strategic::run_weekly_strategic_systems(registry, state)?;
    }
    if state.clock.day() > 0 && state.clock.day() % 30 == 0 {
        super::strategic::run_monthly_strategic_systems(registry, state)?;
    }
    if state.clock.is_year_boundary() {
        process_year_boundary(registry, state)?;
        super::strategic::run_annual_strategic_systems(state)?;
    }
    super::refresh_campaign_phases(state);

    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::DayAdvanced,
        subject: "simulation".into(),
        detail: format!("day={}", state.clock.day()).into(),
    });
    Ok(())
}

fn reset_market_flows(state: &mut AppState) {
    for quote in state.market.quotes.values_mut() {
        quote.demand_today = Quantity::ZERO;
        quote.supply_today = Quantity::ZERO;
    }
}

// ── Production ────────────────────────────────────────────────────────────
fn decide_production(
    registry: &Registry,
    state: &AppState,
    capacity_scratch: &super::DailyCapacityScratch,
) -> ProductionPlan {
    let tools_id = registry
        .get_good_id("tools")
        .expect("Rivergate registry must define tools");
    let tools_quote = state
        .market
        .quotes
        .get(&tools_id)
        .expect("Rivergate market must define tools");
    let tools_price = tools_quote.price;
    let mut remaining_tools_stock = tools_quote.stock;
    let mut lines = Vec::new();
    // Rotate tool-allocation priority by campaign day so low IDs do not always win the scarce
    // tool race. Wrapping on `clock.day()` is intentional.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let day_hash = state.clock.day() as u32;
    let mut businesses: Vec<_> = state.businesses.iter().collect();
    businesses.sort_by_key(|business| business.id().value().wrapping_add(day_hash));
    for business in businesses {
        let Some(line) = decide_business_production(
            registry,
            state,
            business,
            capacity_scratch,
            tools_id,
            remaining_tools_stock,
            tools_price,
        ) else {
            continue;
        };
        remaining_tools_stock = remaining_tools_stock.saturating_sub(line.tool_quantity);
        lines.push(line);
    }
    ProductionPlan { tools_id, lines }
}

/// Share of daily operating cost attributable to tool wear and replacement.
/// At 25% tools are a meaningful but not dominant industrial input, so a
/// tool shortage constrains production without making every workshop's daily
/// viability depend on 80% of its operating budget being tools.
const PRODUCTION_TOOL_SHARE_BASIS_POINTS: i64 = crate::systems::TOOL_SHARE_BASIS_POINTS;

/// Heads become eligible for succession at this age. Combined with the
/// annual chance ramp below, this keeps the first transition within a
/// playable session rather than pushing the dynasty fantasy past the
/// horizon most campaigns reach. At 52 the 50-52 year old founders have
/// two quiet years to establish institutional standing before succession
/// pressure begins, so office reliably precedes succession.
const SUCCESSION_ELIGIBILITY_AGE_YEARS: i64 = 52;

/// Health an heir resumes natural aging from when they accede to the headship.
/// The annual health pass pins a designated heir's collapsed health at a
/// survivable floor so a sick heir can neither collapse into incapacity nor
/// die before inheriting; accession lifts that artificial floor so the new
/// head does not carry a guaranteed next-year collapse into office.
const SUCCESSION_ACCESSION_HEALTH_FLOOR: u16 = 1_000;

/// Survivable health floor for characters the lifecycle cannot retire: a
/// designated heir awaiting succession and a head with no possible successor.
/// Active records must keep positive health (lifecycle invariant), and these
/// roles are exempt from or required by succession machinery that runs in the
/// same annual pass.
const COLLAPSED_HEALTH_SURVIVABLE_FLOOR: u16 = 1;

/// Falling into distress needs three days of operating cover — two proved
/// too eager to tip a workshop on a single price swing, stranding players
/// in thrash without adding drama.
const ACTIVE_CASH_DAYS_OF_OPERATING_COST: i64 = 3;
/// Climbing out needs four, so a business near the threshold cannot flap
/// between `Distressed` and `Active` on daily price noise, but recovery
/// stays reachable for cash-positive workshops rather than stranding them.
const RECOVERY_CASH_DAYS_OF_OPERATING_COST: i64 = 4;

/// Canonical business operating status after its cash or inventory changes:
/// the same thresholds and rehabilitation clamp the daily lifecycle pass
/// applies, evaluated immediately so a capital injection or acquisition can
/// never leap past the documented `Insolvent -> Distressed -> Active`
/// recovery path or install an `Active` status its cash cannot sustain.
pub(crate) fn business_status_after_capitalization(
    prior_status: BusinessStatus,
    cash: Money,
    has_inventory: bool,
    minimum_cash_reserve: Money,
    daily_operating_cost: Money,
) -> BusinessStatus {
    // Recovery carries a higher cash bar than distress onset so a business
    // sitting near the threshold cannot flap between `Distressed` and
    // `Active` on daily price noise: falling into distress needs two days
    // of operating cover, but climbing out needs six.
    let active_status_cash_days = if matches!(
        prior_status,
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) {
        RECOVERY_CASH_DAYS_OF_OPERATING_COST
    } else {
        ACTIVE_CASH_DAYS_OF_OPERATING_COST
    };
    let candidate_status =
        if prior_status == BusinessStatus::Insolvent && cash == Money::ZERO && !has_inventory {
            BusinessStatus::Closed
        } else if cash == Money::ZERO && !has_inventory {
            BusinessStatus::Insolvent
        } else if cash
            < minimum_cash_reserve
                .saturating_add(daily_operating_cost.saturating_mul(active_status_cash_days))
        {
            BusinessStatus::Distressed
        } else {
            BusinessStatus::Active
        };
    match (prior_status, candidate_status) {
        // An insolvent business that receives capital must pass back through
        // `Distressed` rehabilitation before regaining full operation; it may
        // not leap directly from insolvency to `Active`.
        (BusinessStatus::Insolvent, BusinessStatus::Active) => BusinessStatus::Distressed,
        (_, status) => status,
    }
}

/// Annual succession-chance pressure per year of head age past the eligibility
/// threshold. The rate places the median first transition near 850-1050 days
/// (late second to early third year): late enough that a founder pursuing
/// institutional standing reliably reaches office and established memberships
/// before succession tests continuity, early enough that dynastic continuity
/// remains ordinary play rather than only generation-length simulations.
const AGE_PRESSURE_PER_YEAR_OVER_ELIGIBILITY: i64 = 280;

fn decide_business_production(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
    capacity_scratch: &super::DailyCapacityScratch,
    tools_id: GoodId,
    available_tools: Quantity,
    tools_price: Money,
) -> Option<ProductionLine> {
    if matches!(
        business.status(),
        BusinessStatus::Closed | BusinessStatus::Insolvent
    ) {
        return None;
    }
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe reference must be valid");
    let manager = state
        .characters
        .get(business.manager_id())
        .expect("business manager reference must be valid");
    let mut batches = effective_capacity_batches(
        state,
        business,
        capacity_scratch.office_administrative_load(business.owner_dynasty_id()),
    );
    batches = batches.min(output_limited_batches(
        state,
        business,
        recipe,
        capacity_scratch.business_contract_reserve(business.id(), recipe.output_good_id()),
    ));
    batches = batches.min(capacity_scratch.worker_limited_batches(business.id()));
    batches = batches.min(input_limited_batches(business, recipe));
    batches = batches.min(cash_limited_batches(business, recipe));
    // An input-less recipe is an import trade: it converts regional access
    // into goods rather than processing local inputs. A disrupted road must
    // throttle the city's import houses exactly as it throttles direct route
    // supply, so sustained disruption reaches the staple chains downstream
    // instead of stopping at the gatehouses.
    if recipe.inputs().is_empty() {
        let availability = i64::from(import_trade_availability_basis_points(state));
        batches = u16::try_from((i64::from(batches) * availability + 5_000) / 10_000)
            .unwrap_or(batches)
            .min(batches);
    }
    if recipe.output_good_id() != tools_id {
        batches = batches.min(tool_limited_batches(
            batches,
            available_tools,
            recipe.daily_operating_cost(),
            tools_price,
        ));
    }
    if batches == 0 {
        return None;
    }
    let inputs = recipe
        .inputs()
        .iter()
        .map(|input| {
            (
                input.good_id(),
                input.quantity().saturating_mul_ratio(i64::from(batches), 1),
            )
        })
        .collect();
    let quality_efficiency = 9_000_u32
        .saturating_add(u32::from(business.operations.quality_basis_points) / 10)
        .min(10_000);
    let craft_efficiency = 9_000_u32
        .saturating_add(u32::from(manager.capabilities.craft).saturating_mul(10))
        .min(10_000);
    let operating_cost = recipe
        .daily_operating_cost()
        .saturating_mul(i64::from(batches));
    let tool_quantity = if recipe.output_good_id() == tools_id {
        Quantity::ZERO
    } else {
        production_tool_quantity(operating_cost, tools_price)
    };
    Some(ProductionLine {
        business_id: business.id(),
        inputs,
        output_good_id: recipe.output_good_id(),
        output_quantity: recipe
            .output_quantity()
            .saturating_mul_ratio(i64::from(batches), 1)
            .saturating_mul_ratio(i64::from(quality_efficiency), 10_000)
            .saturating_mul_ratio(i64::from(craft_efficiency), 10_000),
        operating_cost,
        tool_quantity,
        tool_cost: cost_for(tool_quantity, tools_price),
    })
}

fn production_tool_quantity(operating_cost: Money, tools_price: Money) -> Quantity {
    if operating_cost <= Money::ZERO || tools_price <= Money::ZERO {
        return Quantity::ZERO;
    }
    let tool_budget =
        operating_cost.saturating_mul_ratio(PRODUCTION_TOOL_SHARE_BASIS_POINTS, 10_000);
    affordable_quantity(tool_budget, tools_price)
}

fn tool_limited_batches(
    maximum_batches: u16,
    available_tools: Quantity,
    daily_operating_cost: Money,
    tools_price: Money,
) -> u16 {
    if maximum_batches == 0
        || daily_operating_cost <= Money::ZERO
        || tools_price <= Money::ZERO
        || available_tools == Quantity::ZERO
    {
        return if production_tool_quantity(daily_operating_cost, tools_price).is_zero() {
            maximum_batches
        } else {
            0
        };
    }
    let mut low = 0_u16;
    let mut high = maximum_batches;
    while low < high {
        let midpoint = low.saturating_add(high.saturating_sub(low).div_ceil(2));
        let required = production_tool_quantity(
            daily_operating_cost.saturating_mul(i64::from(midpoint)),
            tools_price,
        );
        if required <= available_tools {
            low = midpoint;
        } else {
            high = midpoint.saturating_sub(1);
        }
    }
    low
}

pub(crate) fn effective_capacity_batches(
    state: &AppState,
    business: &crate::core::Business,
    office_administrative_load: u16,
) -> u16 {
    let dynasty = state
        .dynasties
        .get(&business.owner_dynasty_id())
        .expect("business owner reference must be valid");
    let governance = state
        .family_councils
        .get(&dynasty.id())
        .expect("business owner dynasty must have family governance")
        .governance;
    // The governance multiplier stays un-truncated so small houses keep their
    // full bonus: the ratio divides once instead of compounding truncations.
    let weighted_administrative_capacity = u32::from(dynasty.administrative_capacity())
        .saturating_mul(governance_administrative_multiplier(governance));
    let effective_administrative_load = dynasty
        .administrative_load()
        .saturating_add(office_administrative_load);
    let administrative_efficiency = if effective_administrative_load == 0 {
        // Nothing to administer: no load can throttle capacity.
        10_000_u16
    } else {
        // `weighted_administrative_capacity` is capacity scaled by the
        // governance multiplier in basis points, so one division by the load
        // yields basis points directly.
        u16::try_from(
            (u64::from(weighted_administrative_capacity)
                / u64::from(effective_administrative_load))
            .min(10_000),
        )
        .unwrap_or(10_000)
    };
    let status_efficiency = match business.status() {
        BusinessStatus::Active => 10_000_u16,
        BusinessStatus::Distressed => 8_000_u16,
        BusinessStatus::Insolvent | BusinessStatus::Closed => 0,
    };
    let effective_basis_points = administrative_efficiency
        .min(status_efficiency)
        .min(business.operations.condition_basis_points.max(2_500));
    let weighted_batches = u32::from(business.operations.capacity_batches_per_day)
        .saturating_mul(u32::from(effective_basis_points));
    let batches = u16::try_from(weighted_batches.saturating_add(5_000) / 10_000)
        .expect("effective batches must fit u16");
    // An operating business keeps a minimum viable batch so degraded
    // administration and condition throttle rather than halt it. A business
    // with zero status efficiency reports zero capacity, never phantom work.
    if status_efficiency == 0 {
        batches
    } else {
        batches.max(1)
    }
}

fn output_limited_batches(
    state: &AppState,
    business: &crate::core::Business,
    recipe: &RecipeDef,
    contract_reserve: Quantity,
) -> u16 {
    let output_good_id = recipe.output_good_id();
    let policy_reserve = super::business_policy_reserve(business, recipe.output_quantity());
    let market_capacity = super::market_absorption_capacity(state, output_good_id);
    let output_headroom = policy_reserve
        .saturating_add(contract_reserve)
        .saturating_add(market_capacity)
        .saturating_sub(business.inventory_quantity(output_good_id))
        .max(Quantity::ZERO);
    let output_per_batch = recipe.output_quantity().milliunits();
    if output_per_batch <= 0 {
        return 0;
    }
    u16::try_from((output_headroom.milliunits() / output_per_batch).max(0)).unwrap_or(u16::MAX)
}

fn input_limited_batches(business: &crate::core::Business, recipe: &RecipeDef) -> u16 {
    if recipe.inputs().is_empty() {
        return u16::MAX;
    }
    recipe.inputs().iter().fold(u16::MAX, |limit, input| {
        let available = business.inventory_quantity(input.good_id()).milliunits();
        let per_batch = input.quantity().milliunits();
        let input_limited = if per_batch == 0 {
            i64::from(u16::MAX)
        } else {
            available / per_batch
        };
        limit.min(u16::try_from(input_limited.max(0)).unwrap_or(u16::MAX))
    })
}

fn cash_limited_batches(business: &crate::core::Business, recipe: &RecipeDef) -> u16 {
    if recipe.daily_operating_cost().copper() <= 0 {
        return u16::MAX;
    }
    let cash_reserve = if business.status() == BusinessStatus::Distressed {
        Money::ZERO
    } else {
        business.policy.minimum_cash_reserve
    };
    let spendable = business.cash().saturating_sub(cash_reserve);
    let affordable = spendable.copper() / recipe.daily_operating_cost().copper();
    u16::try_from(affordable.max(0)).unwrap_or(u16::MAX)
}

const fn governance_administrative_multiplier(governance: HouseGovernance) -> u32 {
    match governance {
        HouseGovernance::HeadCommand => 11_000,
        HouseGovernance::Primogeniture => 10_000,
        HouseGovernance::FamilyPartnership => 10_500,
        HouseGovernance::BranchFederation => 11_500,
        HouseGovernance::ElectedHead => 9_500,
    }
}

fn apply_production(state: &mut AppState, plan: ProductionPlan) -> Result<(), SimulationError> {
    let ProductionPlan { tools_id, lines } = plan;
    let mut total_output_milliunits = 0_i128;
    let mut total_operating_cost_copper = 0_i128;
    let mut total_tool_quantity_milliunits = 0_i128;
    let mut total_tool_spending_copper = 0_i128;

    for line in lines {
        let ProductionLine {
            business_id,
            inputs,
            output_good_id,
            output_quantity,
            operating_cost,
            tool_quantity,
            tool_cost,
        } = line;
        let market_update = planned_tool_market_update(state, tools_id, tool_quantity, tool_cost)?;
        // Tool spending reaches the clearing account through the planned
        // market update. The remaining operating cost pays unmodeled services
        // and labor, which flow into the same pool so every business debit
        // has a credited counterparty instead of vanishing from the economy.
        let tool_backed_clearing = match market_update {
            Some((_, _, clearing)) => clearing,
            None => state.market.clearing_account,
        };
        let service_residual = operating_cost.saturating_sub(tool_cost);
        let resulting_clearing = tool_backed_clearing.checked_add(service_residual).ok_or(
            SimulationError::MarketClearingAccountOverflow {
                current: tool_backed_clearing,
                change: service_residual,
            },
        )?;
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("planned production business must exist");
        let output_inventory_after_inputs = inputs
            .iter()
            .filter(|(good_id, _)| *good_id == output_good_id)
            .fold(
                business.inventory_quantity(output_good_id),
                |current, (_, quantity)| {
                    current
                        .checked_sub(*quantity)
                        .expect("planned production inputs must fit business inventory")
                },
            );
        output_inventory_after_inputs
            .checked_add(output_quantity)
            .ok_or(SimulationError::BusinessInventoryOverflow {
                business_id,
                good_id: output_good_id,
                current: output_inventory_after_inputs,
                incoming: output_quantity,
            })?;
        let resulting_cash = business
            .finance
            .cash
            .checked_sub(operating_cost)
            .expect("planned production must fit available cash");
        let resulting_lifetime_costs = business
            .finance
            .lifetime_costs
            .checked_add(operating_cost)
            .ok_or(SimulationError::BusinessLifetimeCostsOverflow {
                business_id,
                current: business.finance.lifetime_costs,
                incoming: operating_cost,
            })?;
        let next_finance_version = next_business_finance_version(business)?;
        for (good_id, quantity) in inputs {
            business.remove_inventory(good_id, quantity);
        }
        business.add_inventory(output_good_id, output_quantity);
        business.finance.cash = resulting_cash;
        business.finance.lifetime_costs = resulting_lifetime_costs;
        business.finance.version = next_finance_version;
        if let Some((resulting_stock, resulting_demand, _)) = market_update {
            let quote = state
                .market
                .quotes
                .get_mut(&tools_id)
                .expect("planned production tools quote must exist");
            quote.stock = resulting_stock;
            quote.demand_today = resulting_demand;
        }
        state.market.clearing_account = resulting_clearing;
        total_tool_quantity_milliunits += i128::from(tool_quantity.milliunits());
        total_tool_spending_copper += i128::from(tool_cost.copper());
        total_output_milliunits += i128::from(output_quantity.milliunits());
        total_operating_cost_copper += i128::from(operating_cost.copper());
    }

    if total_output_milliunits != 0 {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::Production,
            subject: "businesses".into(),
            detail: format!(
                "output={total_output_milliunits}; operating_cost={total_operating_cost_copper}; tools={total_tool_quantity_milliunits}; tool_spending={total_tool_spending_copper}"
            ).into(),
        });
    }
    Ok(())
}

fn planned_tool_market_update(
    state: &AppState,
    tools_id: GoodId,
    quantity: Quantity,
    cost: Money,
) -> Result<Option<(Quantity, Quantity, Money)>, SimulationError> {
    if quantity <= Quantity::ZERO {
        return Ok(None);
    }
    let quote = state
        .market
        .quotes
        .get(&tools_id)
        .expect("planned tools quote must exist");
    let resulting_stock = quote
        .stock
        .checked_sub(quantity)
        .expect("planned tool use must fit market stock");
    let resulting_demand =
        quote
            .demand_today
            .checked_add(quantity)
            .ok_or(SimulationError::MarketDemandOverflow {
                good_id: tools_id,
                current: quote.demand_today,
                incoming: quantity,
            })?;
    let clearing_before = state.market.clearing_account;
    let resulting_clearing = clearing_before.checked_add(cost).ok_or(
        SimulationError::MarketClearingAccountOverflow {
            current: clearing_before,
            change: cost,
        },
    )?;
    Ok(Some((
        resulting_stock,
        resulting_demand,
        resulting_clearing,
    )))
}

/// A business's sellable stock after policy and contract reserves, plus the
/// market context that bounds how much of it can be placed today.
// ── Business sales ────────────────────────────────────────────────────────
struct BusinessSaleCandidate {
    good_id: GoodId,
    surplus: Quantity,
    capacity: Quantity,
    commerce_efficiency: i64,
    owner_reputation: i64,
    /// Share of the claimed market capacity the house may place today. An
    /// active `GuildEntryRestriction` reserves craft-market access for the
    /// chartered guild's members and scales outsiders down with its value.
    guild_access_basis_points: i64,
    price: Money,
}

fn decide_business_sales(
    registry: &Registry,
    state: &AppState,
    capacity_scratch: &super::DailyCapacityScratch,
) -> Result<BusinessSalePlan, SimulationError> {
    // Shared per-good absorption headroom in a flat vector indexed by the
    // registry's dense good identifiers (see `decide_business_purchases`).
    let mut market_capacity = vec![Quantity::ZERO; registry.goods().len()];
    for (good_id, quote) in &state.market.quotes {
        let maximum_stock = quote.target_stock.saturating_mul_ratio(3, 2);
        market_capacity[good_id.value() as usize] = maximum_stock
            .saturating_sub(quote.stock)
            .max(Quantity::ZERO);
    }
    let mut lines = Vec::new();

    for business in state.businesses.iter() {
        if matches!(
            business.status(),
            BusinessStatus::Closed | BusinessStatus::Insolvent
        ) {
            continue;
        }
        let Some(mut candidate) = plan_sale_candidate(registry, state, business, capacity_scratch)?
        else {
            continue;
        };
        // Sellers share one absorption ceiling per good: each placement
        // consumes the headroom later sellers plan against, mirroring the
        // shared stock accounting in `decide_business_purchases`.
        let good_slot = candidate.good_id.value() as usize;
        let shared_capacity = market_capacity[good_slot];
        candidate.capacity = candidate.capacity.min(shared_capacity);
        let quantity = sale_quantity(&candidate);
        if quantity.is_zero() {
            continue;
        }
        let revenue = validate_business_sale_revenue(
            business.id(),
            business.cash(),
            candidate.good_id,
            quantity,
            candidate.price,
        )?;
        // Remaining shared capacity must stay nonnegative; the quantity is
        // already clamped to the shared remainder in `sale_quantity`.
        market_capacity[good_slot] = candidate
            .capacity
            .saturating_sub(quantity)
            .max(Quantity::ZERO);
        lines.push(BusinessSaleLine {
            business_id: business.id(),
            good_id: candidate.good_id,
            quantity,
            revenue,
        });
    }

    Ok(BusinessSalePlan { lines })
}

/// Reserves, skill, and renown for one business's sales decision, or `None`
/// when the business has no sellable output in its current state.
fn plan_sale_candidate(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
    capacity_scratch: &super::DailyCapacityScratch,
) -> Result<Option<BusinessSaleCandidate>, SimulationError> {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe reference must be valid");
    let manager = state
        .characters
        .get(business.manager_id())
        .expect("business manager reference must be valid");
    let good_id = recipe.output_good_id();
    let inventory = business.inventory_quantity(good_id);
    let policy_reserve = super::business_policy_reserve(business, recipe.output_quantity());
    let contract_reserve = capacity_scratch.business_contract_reserve(business.id(), good_id);
    let policy_reserve_basis_points = match business.status() {
        BusinessStatus::Active
            if business.cash() < recipe.daily_operating_cost().saturating_mul(2) =>
        {
            5_000
        }
        BusinessStatus::Active => 10_000,
        // Distressed firms liquidate freely; Closed and Insolvent are
        // unreachable because the caller filters them out first.
        BusinessStatus::Distressed | BusinessStatus::Insolvent | BusinessStatus::Closed => 0,
    };
    let adjusted_policy_reserve =
        policy_reserve.saturating_mul_ratio(policy_reserve_basis_points, 10_000);
    let reserve = adjusted_policy_reserve.saturating_add(contract_reserve);
    let surplus = inventory.saturating_sub(reserve).max(Quantity::ZERO);
    if surplus.is_zero() {
        return Ok(None);
    }
    let capacity = super::market_absorption_capacity(state, good_id);
    let quote = state
        .market
        .quotes
        .get(&good_id)
        .ok_or(SimulationError::MarketQuoteMissing { good_id })?;
    let guild_access_basis_points = match super::strategic::active_law_value(
        state,
        crate::core::LawKind::GuildEntryRestriction,
    ) {
        Some(value) if value > 0 => {
            let chartered = super::manager_holds_chartered_guild_membership(
                registry,
                state,
                business.recipe_id(),
                manager.id(),
            );
            if chartered {
                10_000
            } else {
                (10_000 - value.clamp(0, 10_000) / super::GUILD_RESTRICTION_OUTSIDER_DIVISOR).max(1)
            }
        }
        _ => 10_000,
    };
    Ok(Some(BusinessSaleCandidate {
        good_id,
        surplus,
        capacity,
        commerce_efficiency: 9_000_i64
            .saturating_add(i64::from(manager.capabilities.commerce).saturating_mul(10))
            .min(10_000),
        owner_reputation: state
            .dynasties
            .get(&business.owner_dynasty_id())
            .map_or(5_000, |dynasty| {
                i64::from(dynasty.resources.reputation_quality_basis_points)
            }),
        guild_access_basis_points,
        price: quote.price,
    }))
}

/// Skill converts stocked surplus into placed sales: a struggling sales
/// operation moves less of what it has even when the market has room. Renown
/// then governs access to genuinely scarce capacity: households and merchants
/// seek out reputable houses first, so an established quality reputation
/// claims the shared remainder ahead of an obscure or disgraced house. A guild
/// entry restriction narrows that access again for outsiders to the trade's
/// chartered guild. No factor can place more than the business actually
/// stocked or more than the good's remaining absorption headroom.
fn sale_quantity(candidate: &BusinessSaleCandidate) -> Quantity {
    let renown_basis_points = (10_000 + (candidate.owner_reputation - 5_000) / 3).max(1);
    let skilled_claim = candidate
        .surplus
        .saturating_mul_ratio(candidate.commerce_efficiency, 10_000);
    let claimed_capacity = candidate
        .capacity
        .saturating_mul_ratio(renown_basis_points, 10_000)
        .saturating_mul_ratio(candidate.guild_access_basis_points, 10_000);
    // Renown prioritizes a claim on the shared remainder; it never places past
    // it, which would breach the absorption ceiling the remainder enforces.
    skilled_claim.min(claimed_capacity).min(candidate.capacity)
}

fn validate_business_sale_revenue(
    business_id: BusinessId,
    current_cash: Money,
    good_id: GoodId,
    quantity: Quantity,
    unit_price: Money,
) -> Result<Money, SimulationError> {
    let revenue = checked_cost_for(quantity, unit_price).ok_or(
        SimulationError::MarketTradeValueOverflow {
            good_id,
            quantity,
            unit_price,
        },
    )?;
    current_cash
        .checked_add(revenue)
        .ok_or(SimulationError::BusinessCashOverflow {
            business_id,
            current: current_cash,
            incoming: revenue,
        })?;
    Ok(revenue)
}

fn apply_business_sales(
    state: &mut AppState,
    plan: BusinessSalePlan,
) -> Result<(), SimulationError> {
    let mut total_revenue_copper = 0_i128;
    let mut total_quantity_milliunits = 0_i128;
    for line in plan.lines {
        let BusinessSaleLine {
            business_id,
            good_id,
            quantity,
            revenue,
        } = line;
        let (resulting_market_stock, resulting_market_supply) = {
            let quote = state
                .market
                .quotes
                .get(&good_id)
                .ok_or(SimulationError::MarketQuoteMissing { good_id })?;
            (
                quote
                    .stock
                    .checked_add(quantity)
                    .ok_or(SimulationError::MarketStockOverflow {
                        good_id,
                        current: quote.stock,
                        incoming: quantity,
                    })?,
                quote.supply_today.checked_add(quantity).ok_or(
                    SimulationError::MarketSupplyOverflow {
                        good_id,
                        current: quote.supply_today,
                        incoming: quantity,
                    },
                )?,
            )
        };
        // Sales revenue is paid out of the pooled market sector and may drive
        // it into a deliberate deficit: households replenish the pool with
        // their own purchases over the following days, so a negative balance
        // is short-term consumer credit, not an accounting failure. Only
        // type-range overflow is an error here.
        let clearing_before = state.market.clearing_account;
        let clearing_change = Money::from_copper(-revenue.copper());
        let resulting_clearing = clearing_before.checked_sub(revenue).ok_or(
            SimulationError::MarketClearingAccountOverflow {
                current: clearing_before,
                change: clearing_change,
            },
        )?;
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("planned business sale source must exist");
            let resulting_cash = business
                .finance
                .cash
                .checked_add(revenue)
                .expect("planned sale revenue must fit business cash");
            let resulting_lifetime_revenue = business
                .finance
                .lifetime_revenue
                .checked_add(revenue)
                .ok_or(SimulationError::BusinessLifetimeRevenueOverflow {
                    business_id,
                    current: business.finance.lifetime_revenue,
                    incoming: revenue,
                })?;
            let next_finance_version = next_business_finance_version(business)?;
            business.remove_inventory(good_id, quantity);
            business.finance.cash = resulting_cash;
            business.finance.lifetime_revenue = resulting_lifetime_revenue;
            business.finance.version = next_finance_version;
        }
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("prevalidated business sale quote must exist");
            quote.stock = resulting_market_stock;
            quote.supply_today = resulting_market_supply;
        }
        state.market.clearing_account = resulting_clearing;
        total_revenue_copper += i128::from(revenue.copper());
        total_quantity_milliunits += i128::from(quantity.milliunits());
    }
    if total_quantity_milliunits != 0 {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::MarketSale,
            subject: "businesses".into(),
            detail: format!("quantity={total_quantity_milliunits}; revenue={total_revenue_copper}")
                .into(),
        });
    }
    Ok(())
}

// ── Household consumption ─────────────────────────────────────────────────
fn decide_household_consumption(registry: &Registry, state: &AppState) -> HouseholdConsumptionPlan {
    let bread_id = registry
        .get_good_id("bread")
        .expect("Rivergate registry must define bread");
    let flour_id = registry
        .get_good_id("flour")
        .expect("Rivergate registry must define flour");
    let grain_id = registry
        .get_good_id("grain")
        .expect("Rivergate registry must define grain");
    let ale_id = registry
        .get_good_id("ale")
        .expect("Rivergate registry must define ale");
    let charcoal_id = registry
        .get_good_id("charcoal")
        .expect("Rivergate registry must define charcoal");
    let cloth_id = registry
        .get_good_id("cloth")
        .expect("Rivergate registry must define cloth");
    // Shared market stock in a flat vector indexed by the registry's dense
    // good identifiers (see `decide_business_purchases`).
    let mut stock = vec![Quantity::ZERO; registry.goods().len()];
    // Quote prices are constant across the whole planning pass (the apply
    // phase performs every write), so they are prefetched next to the stock
    // scratch pad and the per-household loop reads them without repeated
    // map probes.
    let mut prices = vec![Money::ZERO; registry.goods().len()];
    for (good_id, quote) in &state.market.quotes {
        let slot = good_id.value() as usize;
        stock[slot] = quote.stock;
        prices[slot] = quote.price;
    }
    // Cloth demand discipline scales with the market's current reference-price
    // ratio, which is identical for every household in the same planning pass.
    // Resolving the good and its ratio once per day keeps the per-household
    // loop free of repeated registry string lookups without changing any
    // computed value.
    let cloth_ratio_basis_points = cloth_price_ratio_basis_points(registry, state);
    let mut lines = Vec::new();
    let mut food_satisfaction = BTreeMap::new();

    for household in state.households.iter() {
        let mut cash = household.cash;
        let mut food_acquired = Quantity::ZERO;
        // Households prefer finished bread and fall back to cheaper upstream
        // staples only when bread is unavailable or unaffordable, so processed
        // food keeps its market while poverty still has a substitution path.
        for good_id in [bread_id, flour_id, grain_id] {
            let remaining_need = household.bread_need_daily.saturating_sub(food_acquired);
            if remaining_need.is_zero() {
                break;
            }
            let quantity = plan_household_purchase(
                household.id(),
                good_id,
                remaining_need,
                prices[good_id.value() as usize],
                &mut cash,
                &mut stock,
                &mut lines,
            );
            food_acquired = food_acquired.saturating_add(quantity);
        }
        let (charcoal_need, cloth_need) = household_secondary_needs(household.social_class());
        // Cloth is the one secondary staple whose market must stay balanced
        // against the city's weaving capacity. Households do not pay any
        // price for it: dear cloth means mending and waiting, so demand
        // scales down with the going price instead of ratcheting a shortage
        // ever upward. Tools are an industrial input consumed only by
        // workshops and civic construction; households do not consume tools
        // as a daily staple, so tool demand is driven by production and
        // maintenance rather than household shopping.
        let cloth_need = match cloth_ratio_basis_points {
            Some(ratio_basis_points) => cloth_need.saturating_mul_ratio(ratio_basis_points, 10_000),
            None => cloth_need,
        };
        for (good_id, need) in [
            (ale_id, household.ale_need_daily),
            (charcoal_id, charcoal_need),
            (cloth_id, cloth_need),
        ] {
            plan_household_purchase(
                household.id(),
                good_id,
                need,
                prices[good_id.value() as usize],
                &mut cash,
                &mut stock,
                &mut lines,
            );
        }

        let daily_satisfaction = if household.bread_need_daily.is_zero() {
            10_000
        } else {
            u16::try_from(
                food_acquired
                    .saturating_mul_ratio(10_000, household.bread_need_daily.milliunits())
                    .milliunits(),
            )
            .unwrap_or(10_000)
            .min(10_000)
        };
        let smoothed = u16::try_from(
            (u32::from(household.food_satisfaction_basis_points) * 9
                + u32::from(daily_satisfaction))
                / 10,
        )
        .expect("smoothed satisfaction must fit u16");
        food_satisfaction.insert(household.id(), smoothed);
    }

    HouseholdConsumptionPlan {
        lines,
        food_satisfaction,
    }
}
fn plan_household_purchase(
    household_id: crate::ids::HouseholdId,
    good_id: GoodId,
    need: Quantity,
    price: Money,
    cash: &mut Money,
    stock: &mut [Quantity],
    lines: &mut Vec<HouseholdPurchaseLine>,
) -> Quantity {
    let good_slot = good_id.value() as usize;
    debug_assert!(good_slot < stock.len());
    let available = stock.get(good_slot).copied().unwrap_or(Quantity::ZERO);
    let quantity = need.min(available).min(affordable_quantity(*cash, price));
    if quantity.is_zero() {
        return Quantity::ZERO;
    }
    let cost = cost_for(quantity, price);
    stock[good_slot] = available
        .checked_sub(quantity)
        .expect("planned purchase quantity must not exceed available stock");
    *cash = cash
        .checked_sub(cost)
        .expect("affordable planned purchase must not exceed household cash");
    lines.push(HouseholdPurchaseLine {
        household_id,
        good_id,
        quantity,
        cost,
    });
    quantity
}

fn household_secondary_needs(social_class: SocialClass) -> (Quantity, Quantity) {
    // Clothing is a recurring household staple, not a luxury. Rivergate's
    // nominal cloth needs sit just under the city's weaving capacity (the
    // player's loomhouse plus the Veyra workshop), so both weavers sell at
    // viable margins instead of glutting the market into structural losses,
    // while [`cloth_price_ratio_basis_points`] scales need back when prices
    // climb so a shortage cannot ratchet. The household income in bootstrap is
    // calibrated to carry this budget alongside food. Tools are industrial
    // inputs for workshops and civic works, not household staples.
    let (charcoal, cloth) = match social_class {
        SocialClass::Laboring => (180, 400),
        SocialClass::Artisan => (240, 800),
        SocialClass::Merchant => (300, 1_200),
    };
    (
        Quantity::from_milliunits(charcoal),
        Quantity::from_milliunits(cloth),
    )
}

/// Cloth demand's price-discipline ratio in basis points, resolved once per
/// planning pass: at or below the good's registry reference price households
/// buy their full clothing need; above it they economize proportionally,
/// never falling below a quarter of the need. Without this response, a
/// crisis- or shortage-driven cloth price spike ratchets unchecked because
/// fixed demand cannot answer a rising price, and households burn their food
/// buffer on expensive cloth.
fn cloth_price_ratio_basis_points(registry: &Registry, state: &AppState) -> Option<i64> {
    let cloth_id = registry.get_good_id("cloth")?;
    let reference = registry
        .get_good(cloth_id)
        .map(crate::registry::GoodDef::base_price)?;
    let quote = state.market.quotes.get(&cloth_id)?;
    let reference_copper = reference.copper().max(1);
    let current_copper = quote.price.copper().max(1);
    Some((reference_copper * 10_000 / current_copper).clamp(2_500, 10_000))
}

fn apply_household_consumption(
    state: &mut AppState,
    plan: HouseholdConsumptionPlan,
) -> Result<(), SimulationError> {
    let HouseholdConsumptionPlan {
        lines,
        food_satisfaction,
    } = plan;
    let mut total_cost_copper = 0_i128;
    let mut total_quantity_milliunits = 0_i128;
    for line in lines {
        let HouseholdPurchaseLine {
            household_id,
            good_id,
            quantity,
            cost,
        } = line;
        // The quote stays a pure read here: its write-back below must wait
        // until the clearing-account and household checks have also passed,
        // so a rejected line leaves market stock untouched.
        let (resulting_market_stock, resulting_market_demand) = {
            let quote = state
                .market
                .quotes
                .get(&good_id)
                .expect("planned household purchase quote must exist");
            (
                quote
                    .stock
                    .checked_sub(quantity)
                    .expect("planned household purchase must not exceed market stock"),
                quote.demand_today.checked_add(quantity).ok_or(
                    SimulationError::MarketDemandOverflow {
                        good_id,
                        current: quote.demand_today,
                        incoming: quantity,
                    },
                )?,
            )
        };
        let clearing_before = state.market.clearing_account;
        let resulting_clearing = clearing_before.checked_add(cost).ok_or(
            SimulationError::MarketClearingAccountOverflow {
                current: clearing_before,
                change: cost,
            },
        )?;
        {
            // One traversal computes and commits the household deduction;
            // both preceding checks have already passed, so this write is
            // reached exactly when the previous form wrote it.
            let household = state
                .households
                .get_mut(household_id)
                .expect("planned household purchase target must exist");
            let resulting_household_cash = household
                .cash
                .checked_sub(cost)
                .expect("planned household purchase must not exceed household cash");
            household.cash = resulting_household_cash;
        }
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("planned household purchase quote must exist");
            quote.stock = resulting_market_stock;
            quote.demand_today = resulting_market_demand;
        }
        state.market.clearing_account = resulting_clearing;
        total_cost_copper += i128::from(cost.copper());
        total_quantity_milliunits += i128::from(quantity.milliunits());
    }
    for (household_id, satisfaction) in food_satisfaction {
        state
            .households
            .get_mut(household_id)
            .expect("planned household satisfaction target must exist")
            .food_satisfaction_basis_points = satisfaction;
    }
    if total_quantity_milliunits > 0 {
        // Like every other economic phase, days with no activity leave the
        // audit trail untouched instead of recording zero-traffic entries.
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::HouseholdConsumption,
            subject: "households".into(),
            detail: format!("quantity={total_quantity_milliunits}; spending={total_cost_copper}")
                .into(),
        });
    }
    Ok(())
}

#[derive(Clone, Copy)]
// ── Maintenance ───────────────────────────────────────────────────────────
struct MaintenanceSnapshot {
    business_id: BusinessId,
    recipe_id: RecipeId,
    cash: Money,
    minimum_cash_reserve: Money,
    maintenance_basis_points: u16,
    quality_target_basis_points: u16,
    condition_basis_points: u16,
    quality_basis_points: u16,
}

fn decide_maintenance(registry: &Registry, state: &mut AppState) -> MaintenancePlan {
    let tools_id = registry
        .get_good_id("tools")
        .expect("Rivergate registry must define tools");
    let tools_quote = state
        .market
        .quotes
        .get(&tools_id)
        .expect("Rivergate market must define tools");
    let tools_price = tools_quote.price;
    let mut remaining_tools_stock = tools_quote.stock;
    // Low-word hash of campaign day — wrapping is intentional for the rotation below.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let day_hash = state.clock.day() as u32;
    let mut snapshots: Vec<_> = state
        .businesses
        .iter()
        .filter(|business| {
            !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
        })
        .map(|business| {
            // A manager who belongs to the trade's chartered guild sustains a
            // higher quality ceiling: guild training shows in the work, so the
            // same maintenance budget pushes quality further before it stalls.
            let guild_quality_bonus = if super::manager_holds_chartered_guild_membership(
                registry,
                state,
                business.recipe_id(),
                business.manager_id(),
            ) {
                super::GUILD_CRAFT_QUALITY_TARGET_BONUS
            } else {
                0
            };
            MaintenanceSnapshot {
                business_id: business.id(),
                recipe_id: business.recipe_id(),
                cash: business.cash(),
                // A distressed firm may spend through its minimum cash reserve
                // to keep operating, matching the daily cost limiter; purchase
                // and maintenance planning must fence off no more, or a
                // distressed firm would keep buying inputs while systematically
                // failing every maintenance check.
                minimum_cash_reserve: if business.status() == BusinessStatus::Distressed {
                    Money::ZERO
                } else {
                    business.policy.minimum_cash_reserve
                },
                maintenance_basis_points: business.policy.maintenance_basis_points,
                quality_target_basis_points: business
                    .policy
                    .quality_target_basis_points
                    .saturating_add(guild_quality_bonus),
                condition_basis_points: business.operations.condition_basis_points,
                quality_basis_points: business.operations.quality_basis_points,
            }
        })
        .collect();
    // Rotate maintenance tool priority same as production: deterministic daily rotation avoids
    // systematic starvation of high-ID workshops when tools are scarce.
    snapshots.sort_by_key(|snapshot| snapshot.business_id.value().wrapping_add(day_hash));
    let lines = snapshots
        .into_iter()
        .map(|snapshot| {
            maintenance_line(
                registry,
                state,
                snapshot,
                tools_id,
                tools_price,
                &mut remaining_tools_stock,
            )
        })
        .collect();

    MaintenancePlan { tools_id, lines }
}

fn maintenance_line(
    registry: &Registry,
    state: &mut AppState,
    snapshot: MaintenanceSnapshot,
    tools_id: GoodId,
    tools_price: Money,
    remaining_tools_stock: &mut Quantity,
) -> MaintenanceLine {
    let MaintenanceSnapshot {
        business_id,
        recipe_id,
        cash,
        minimum_cash_reserve,
        maintenance_basis_points,
        quality_target_basis_points,
        condition_basis_points,
        quality_basis_points,
    } = snapshot;
    let recipe = registry
        .get_recipe(recipe_id)
        .expect("business recipe reference must be valid");
    let desired_cost = maintenance_cost(recipe.daily_operating_cost(), maintenance_basis_points);
    let effect_points = maintenance_effect(maintenance_basis_points, condition_basis_points);
    let can_maintain =
        desired_cost > Money::ZERO && cash.saturating_sub(minimum_cash_reserve) >= desired_cost;
    let tool_budget = if can_maintain && recipe.output_good_id() != tools_id {
        // Maintenance buys its tools with the full budget; the tool share of
        // production and public works is deliberately smaller.
        desired_cost
    } else {
        Money::ZERO
    };
    let required_tool_quantity = affordable_quantity(tool_budget, tools_price);
    let tool_quantity = (*remaining_tools_stock).min(required_tool_quantity);
    let tool_cost = cost_for(tool_quantity, tools_price);
    *remaining_tools_stock = (*remaining_tools_stock).saturating_sub(tool_quantity);
    let tools_available = tool_quantity >= required_tool_quantity;
    let maintenance_succeeds =
        can_maintain && (recipe.output_good_id() == tools_id || tools_available);
    let random_wear = i16::try_from(state.rng.range_u32(3)).expect("wear fits i16");
    let neglect_penalty = if maintenance_succeeds { 0 } else { 2 };
    // Low-condition accidents are frequent small setbacks, not rare catastrophes:
    // the expected daily erosion stays similar but variance is bounded so
    // routine upkeep can gradually recover a neglected workshop instead of
    // requiring luck to avoid a single -120 collapse.
    let accident_penalty = if condition_basis_points < 3_500 && state.rng.is_chance_success(200) {
        15
    } else {
        0
    };
    let improvement = if maintenance_succeeds && condition_basis_points < 9_500 {
        effect_points
    } else {
        0
    };
    let quality_improvement =
        if maintenance_succeeds && quality_basis_points < quality_target_basis_points {
            i16::try_from(
                (quality_target_basis_points - quality_basis_points)
                    .min(u16::try_from(effect_points).expect("maintenance effect is nonnegative")),
            )
            .expect("bounded quality improvement must fit i16")
        } else {
            0
        };
    // Quality above its target mean-reverts under successful maintenance:
    // losing a guild-trained manager or lowering the policy target must let
    // the old excellence decay instead of ratcheting upward forever.
    let quality_target_excess_decline =
        if maintenance_succeeds && quality_basis_points > quality_target_basis_points {
            i16::try_from(
                (quality_basis_points - quality_target_basis_points)
                    .min(u16::try_from(effect_points).expect("maintenance effect is nonnegative"))
                    .div_ceil(4),
            )
            .expect("bounded quality decline must fit i16")
        } else {
            0
        };
    let quality_decline = if maintenance_succeeds {
        quality_target_excess_decline
    } else {
        3
    };
    MaintenanceLine {
        business_id,
        cost: if maintenance_succeeds {
            desired_cost
        } else {
            Money::ZERO
        },
        tool_quantity,
        tool_cost,
        condition_delta: improvement - 2 - random_wear - neglect_penalty - accident_penalty,
        quality_delta: quality_improvement - quality_decline,
    }
}

pub(crate) fn maintenance_cost(
    daily_operating_cost: Money,
    maintenance_basis_points: u16,
) -> Money {
    if maintenance_basis_points == 0 || daily_operating_cost <= Money::ZERO {
        return Money::ZERO;
    }
    daily_operating_cost
        .saturating_mul_ratio_ceil_nonnegative(i64::from(maintenance_basis_points), 20_000)
}

fn maintenance_effect(maintenance_basis_points: u16, condition_basis_points: u16) -> i16 {
    let scaled = u32::from(maintenance_basis_points)
        .saturating_mul(36)
        .div_ceil(10_000);
    let catch_up = u32::from(9_500_u16.saturating_sub(condition_basis_points)).div_ceil(300);
    i16::try_from(scaled.saturating_add(catch_up)).expect("bounded maintenance effect must fit i16")
}

fn apply_maintenance(state: &mut AppState, plan: MaintenancePlan) -> Result<(), SimulationError> {
    let MaintenancePlan { tools_id, lines } = plan;
    let mut total_cost_copper = 0_i128;
    let mut total_tool_cost_copper = 0_i128;
    let mut total_tool_quantity_milliunits = 0_i128;
    for line in lines {
        let MaintenanceLine {
            business_id,
            cost,
            tool_quantity,
            tool_cost,
            condition_delta,
            quality_delta,
        } = line;
        let market_update = planned_tool_market_update(state, tools_id, tool_quantity, tool_cost)?;
        // A successful maintenance pays its full desired cost, which already
        // includes the consumed tools. A failed maintenance that still consumed
        // partially available tools must pay for exactly those tools;
        // otherwise the market clearing account would receive unbacked money.
        let charge = cost.max(tool_cost);
        // Whatever part of the charge is not tool spending buys unmodeled
        // services and materials, so it flows into the clearing pool like
        // every other business payment instead of vanishing.
        let tool_backed_clearing = match market_update {
            Some((_, _, clearing)) => clearing,
            None => state.market.clearing_account,
        };
        let service_residual = charge.saturating_sub(tool_cost);
        let resulting_clearing = tool_backed_clearing.checked_add(service_residual).ok_or(
            SimulationError::MarketClearingAccountOverflow {
                current: tool_backed_clearing,
                change: service_residual,
            },
        )?;
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("planned maintenance business must exist");
        if charge > Money::ZERO {
            let resulting_cash = business
                .finance
                .cash
                .checked_sub(charge)
                .expect("planned maintenance must fit available cash");
            let resulting_lifetime_costs =
                business.finance.lifetime_costs.checked_add(charge).ok_or(
                    SimulationError::BusinessLifetimeCostsOverflow {
                        business_id,
                        current: business.finance.lifetime_costs,
                        incoming: charge,
                    },
                )?;
            let next_finance_version = next_business_finance_version(business)?;
            business.finance.cash = resulting_cash;
            business.finance.lifetime_costs = resulting_lifetime_costs;
            business.finance.version = next_finance_version;
        }
        let condition = i32::from(business.operations.condition_basis_points)
            .saturating_add(i32::from(condition_delta))
            .clamp(0, 10_000);
        business.operations.condition_basis_points =
            u16::try_from(condition).expect("clamped condition must fit u16");
        let quality = i32::from(business.operations.quality_basis_points)
            .saturating_add(i32::from(quality_delta))
            .clamp(0, 10_000);
        business.operations.quality_basis_points =
            u16::try_from(quality).expect("clamped quality must fit u16");
        if let Some((resulting_stock, resulting_demand, _)) = market_update {
            let quote = state
                .market
                .quotes
                .get_mut(&tools_id)
                .expect("planned maintenance tools quote must exist");
            quote.stock = resulting_stock;
            quote.demand_today = resulting_demand;
        }
        state.market.clearing_account = resulting_clearing;
        total_cost_copper += i128::from(cost.copper());
        total_tool_cost_copper += i128::from(tool_cost.copper());
        total_tool_quantity_milliunits += i128::from(tool_quantity.milliunits());
    }
    if total_cost_copper != 0 {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::Maintenance,
            subject: "businesses".into(),
            detail: format!(
                "cost={total_cost_copper}; tools={total_tool_quantity_milliunits}; tool_spending={total_tool_cost_copper}"
            ).into(),
        });
    }
    Ok(())
}

// ── Business lifecycle ──────────────────────────────────────────────────
fn update_business_lifecycle(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let snapshots: Vec<_> = state
        .businesses
        .iter()
        .map(|business| {
            (
                business.id(),
                business.status(),
                business.cash(),
                business.recipe_id(),
                business.policy.minimum_cash_reserve,
                business
                    .inventory
                    .values()
                    .any(|quantity| !quantity.is_zero()),
            )
        })
        .collect();
    let mut events = Vec::new();

    for (business_id, prior_status, cash, recipe_id, minimum_cash_reserve, has_inventory) in
        snapshots
    {
        if prior_status == BusinessStatus::Closed {
            continue;
        }
        let recipe = registry
            .get_recipe(recipe_id)
            .expect("business recipe reference must be valid");
        let new_status = business_status_after_capitalization(
            prior_status,
            cash,
            has_inventory,
            minimum_cash_reserve,
            recipe.daily_operating_cost(),
        );
        if new_status != prior_status {
            state
                .businesses
                .get_mut(business_id)
                .expect("lifecycle business must exist")
                .operations
                .status = new_status;
            super::synchronize_employment_for_business_status(state, business_id, new_status);
            events.push((business_id, prior_status, new_status));
        }
    }

    for (business_id, prior_status, new_status) in events {
        // A business that loses active standing cannot stay bound to scheduled
        // supply: terminate its active contracts immediately so the
        // no-inactive-contract-party lifecycle invariant holds every day,
        // not only at week boundaries.
        if matches!(
            new_status,
            BusinessStatus::Insolvent | BusinessStatus::Closed
        ) {
            super::strategic::terminate_active_contracts_for_business(state, business_id)?;
        }
        let (kind, summary) = match new_status {
            BusinessStatus::Distressed | BusinessStatus::Insolvent => (
                ChronicleKind::BusinessDistress,
                format!("Business {business_id} entered {new_status:?} status."),
            ),
            BusinessStatus::Active => match prior_status {
                BusinessStatus::Distressed | BusinessStatus::Insolvent => (
                    ChronicleKind::BusinessRecovered,
                    format!("Business {business_id} recovered to active operation."),
                ),
                BusinessStatus::Active | BusinessStatus::Closed => continue,
            },
            BusinessStatus::Closed => (
                ChronicleKind::BusinessDistress,
                format!("Business {business_id} closed after unresolved insolvency."),
            ),
        };
        let id = state.next_ids.try_chronicle()?;
        state.chronicle.push(ChronicleEntry {
            id,
            day: state.clock.day(),
            kind,
            summary,
        });
    }
    Ok(())
}

// ── External income ─────────────────────────────────────────────────────
fn settle_weekly_external_income(state: &mut AppState) -> Result<(), SimulationError> {
    // Regional households earn part of their living beyond the modeled market:
    // hauling freight, provisioning caravans, selling crafts and labor to the
    // outside world. That earning power is outside silver flowing into the
    // city, not a draw on the pooled market sector. Only available
    // (unemployed) members can do outside work: employed members' labor is
    // already captured in weekly wage settlement, so double-counting would
    // make every employed household richer than a subsistence one and hide
    // the real trade-off between workshop employment and caravan work. The
    // available share is floored at 20% so a fully employed household still
    // retains minimal outside opportunity instead of zeroing out.
    let availability = regional_demand_availability_basis_points(state);
    let payments: Vec<_> = state
        .households
        .iter()
        .map(|household| {
            let available = i64::from(crate::systems::available_household_workers(
                state,
                household.id(),
            ));
            let members = i64::from(household.members());
            let available_ratio = if members > 0 {
                (available * 10_000 / members).clamp(2_000, 10_000)
            } else {
                10_000
            };
            let scaled_income = household
                .weekly_income
                .saturating_mul_ratio(available_ratio, 10_000);
            let paid = scaled_income.saturating_mul_ratio(i64::from(availability), 10_000);
            household
                .cash
                .checked_add(paid)
                .ok_or(SimulationError::HouseholdCashOverflow {
                    household_id: household.id(),
                    current: household.cash,
                    incoming: paid,
                })?;
            Ok((household.id(), paid))
        })
        .collect::<Result<_, SimulationError>>()?;
    let mut total = Money::ZERO;
    for (_, paid) in &payments {
        total = total
            .checked_add(*paid)
            .ok_or(SimulationError::WeeklyExternalIncomeOverflow {
                accumulated: total,
                incoming: *paid,
            })?;
    }
    for (household_id, paid) in payments {
        let household = state
            .households
            .get_mut(household_id)
            .expect("weekly income household must exist");
        household.cash = household
            .cash
            .checked_add(paid)
            .expect("bounded weekly income must fit household cash");
    }
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::LaborSettlement,
        subject: "external-economy".into(),
        detail: format!(
            "weekly_income={}; regional_availability={availability}",
            total.copper()
        )
        .into(),
    });
    Ok(())
}

/// Regional demand for the city's household labor and crafts, in basis points
/// of normal earning power: the average availability of the active external
/// routes, floored so a total blockade still leaves subsistence work.
/// Campaigns whose regional economy is not modeled through routes keep full
/// availability.
const REGIONAL_DEMAND_MIN_AVAILABILITY_BASIS_POINTS: u16 = 2_500;

/// Import trades keep a smaller blockade floor than households: a closed road
/// strands most of the regional trade but never stops every cart, river barge,
/// and smuggler, so a total blockade leaves a trickle rather than zero.
const IMPORT_TRADE_MIN_AVAILABILITY_BASIS_POINTS: u16 = 1_000;

/// Average availability across active external routes, floored so a total
/// blockade degrades to the caller's minimum instead of perfect health.
/// Campaigns whose regional economy is not modeled through routes keep full
/// availability.
fn average_route_availability_basis_points(state: &AppState, minimum: u16) -> u16 {
    if state.external_routes.is_empty() {
        return 10_000;
    }
    if state.external_routes.values().all(|route| !route.active) {
        return minimum;
    }
    let weighted_disruption = crate::systems::capacity_weighted_route_disruption(state);
    let weighted_availability = 10_000_u16.saturating_sub(weighted_disruption);
    u16::try_from(
        u32::from(weighted_availability)
            .max(u32::from(minimum))
            .min(10_000),
    )
    .expect("availability clamped into basis-point range must fit u16")
}

fn regional_demand_availability_basis_points(state: &AppState) -> u16 {
    average_route_availability_basis_points(state, REGIONAL_DEMAND_MIN_AVAILABILITY_BASIS_POINTS)
}

/// Availability import trades depend on: the same route health households feel,
/// with a trader's floor instead of a subsistence floor.
pub(crate) fn import_trade_availability_basis_points(state: &AppState) -> u16 {
    average_route_availability_basis_points(state, IMPORT_TRADE_MIN_AVAILABILITY_BASIS_POINTS)
}

// ── Year boundary & succession ──────────────────────────────────────────
fn process_year_boundary(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
    let year = state.clock.year(registry.scenario().start_year());
    let id = state.next_ids.try_chronicle()?;
    state.chronicle.push(ChronicleEntry {
        id,
        day: state.clock.day(),
        kind: ChronicleKind::NewYear,
        summary: format!("Rivergate entered the year {year}."),
    });

    update_succession_risks(state);
    update_character_health(state)?;
    let succession_plan = decide_successions(state)?;
    apply_successions(state, succession_plan)?;
    Ok(())
}

fn update_character_health(state: &mut AppState) -> Result<(), SimulationError> {
    // An incapacitated member has already left every active duty; a bounded
    // window of collapsed health eventually claims them instead of leaving
    // an inert record that can neither recover nor die.
    const INCAPACITATED_DEATH_WINDOW_DAYS: i64 = 3 * 360;
    let epidemic_severity = state
        .crises
        .values()
        .filter(|crisis| crisis.kind == CrisisKind::Epidemic && crisis.status.is_active())
        .map(|crisis| crisis.severity_basis_points)
        .max()
        .unwrap_or(0);
    let day = state.clock.day();
    let head_ids: BTreeSet<_> = state
        .dynasties
        .values()
        .map(crate::core::Dynasty::head_id)
        .collect();
    let heir_ids: BTreeSet<_> = state
        .dynasties
        .values()
        .filter_map(crate::core::Dynasty::heir_id)
        .collect();
    let mut newly_incapacitated = Vec::new();
    for character in state.characters.iter_mut() {
        if character.status() != CharacterStatus::Active {
            continue;
        }
        let age_years = day.saturating_sub(character.birth_day()) / 360;
        let resolved_health = resolve_annual_health(
            character.runtime.health_basis_points,
            age_years,
            epidemic_severity,
        );
        // A designated heir whose resolved health collapses is pinned at a
        // survivable floor for this year instead of becoming incapacitated:
        // succession needs a live designated heir, and the floor is lifted on
        // accession (SUCCESSION_ACCESSION_HEALTH_FLOOR). Heads are not
        // pinned: a sole head with collapsed health proceeds to forced
        // succession via the emergency-heir path, so no dynasty becomes
        // immortal at 1 hp while its house has no other members.
        let pinned_at_survivable_floor = resolved_health == 0 && heir_ids.contains(&character.id());
        character.runtime.health_basis_points = if pinned_at_survivable_floor {
            COLLAPSED_HEALTH_SURVIVABLE_FLOOR
        } else {
            resolved_health
        };
        if character.runtime.health_basis_points == 0 && !head_ids.contains(&character.id()) {
            if character.runtime.incapacitated_day.is_none() {
                character.runtime.incapacitated_day = Some(state.clock.day());
            }
            character.runtime.status = CharacterStatus::Incapacitated;
            newly_incapacitated.push((
                character.id(),
                character.dynasty_id(),
                character.name().to_owned(),
            ));
        }
    }
    for (character_id, dynasty_id, character_name) in newly_incapacitated {
        synchronize_character_incapacitation(state, character_id, dynasty_id, &character_name)?;
    }
    // An incapacitated member has already left every active duty; a bounded
    // window of collapsed health eventually claims them instead of leaving
    // an inert record that can neither recover nor die.
    let day = state.clock.day();
    let dying_ids: Vec<(CharacterId, DynastyId)> = state
        .characters
        .iter()
        .filter(|character| character.status() == CharacterStatus::Incapacitated)
        .filter(|character| {
            character
                .runtime
                .incapacitated_day
                .is_some_and(|collapsed_day| {
                    day.saturating_sub(collapsed_day) >= INCAPACITATED_DEATH_WINDOW_DAYS
                })
        })
        .map(|character| (character.id(), character.dynasty_id()))
        .collect();
    for (character_id, dynasty_id) in dying_ids {
        retire_incapacitated_member(state, character_id);
        if dynasty_id == state.player_dynasty_id {
            super::strategic::try_push_outbox(
                state,
                OutboxKind::Family,
                format!("Character {character_id} passed away"),
                "A family member who had been incapacitated by collapsed health has died."
                    .to_owned(),
            )?;
        }
    }
    reconcile_inactive_business_managers(state);
    designate_emergency_heirs(state);
    Ok(())
}

/// A head whose health has collapsed with no designated heir must not keep
/// running the house indefinitely: designate the most capable adult member
/// as emergency heir so this year's succession pass can execute normally.
/// When a house has no other active members at all, an emergency heir is
/// generated instead of pinning the head forever at 1 hp — the dynasty
/// remains mortal and succession tests the rebuilt house rather than an
/// immortal founder.
fn designate_emergency_heirs(state: &mut AppState) {
    let needing: Vec<DynastyId> = state
        .dynasties
        .values()
        .filter(|dynasty| {
            dynasty.heir_id().is_none()
                && state
                    .characters
                    .get(dynasty.head_id())
                    .is_some_and(|head| head.runtime.health_basis_points == 0)
        })
        .map(crate::core::Dynasty::id)
        .collect();
    for dynasty_id in needing {
        if let Some(successor_id) = emergency_successor(
            state,
            state
                .dynasties
                .get(&dynasty_id)
                .expect("dynasty must exist")
                .head_id(),
        ) {
            state
                .dynasties
                .get_mut(&dynasty_id)
                .expect("emergency succession dynasty must exist")
                .relationships
                .heir_id = Some(successor_id);
            continue;
        }
        // No other active member exists: generate a successor so the
        // collapsed sole head can retire without leaving an immortal
        // Active 0-health head or a headless dynasty. The new heir is a
        // young adult ward-like successor tied to the dynasty by history.
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("dynasty must exist")
            .head_id();
        let (birth_day, link_kind, capabilities) = generate_next_heir(state, head_id);
        let dynasty_name = state
            .dynasties
            .get(&dynasty_id)
            .expect("dynasty must exist")
            .name()
            .to_owned();
        let generation = state
            .dynasties
            .get(&dynasty_id)
            .expect("dynasty must exist")
            .runtime
            .generation;
        let new_heir_name = format!("{dynasty_name} Heir {generation}");
        let new_heir_id = {
            let mut next_ids = state.next_ids.clone();
            let id = next_ids
                .try_character()
                .expect("emergency heir character space must be available");
            let link_id = next_ids
                .try_family_link()
                .expect("emergency heir link space must be available");
            state.next_ids = next_ids;
            state.characters.insert(Character {
                identity: CharacterIdentity {
                    id,
                    dynasty_id,
                    name: new_heir_name,
                    birth_day,
                },
                capabilities,
                runtime: CharacterRuntime {
                    status: CharacterStatus::Active,
                    health_basis_points: 9_500,
                    loyalty_basis_points: 8_000,
                    role: CharacterRole::Heir,
                    incapacitated_day: None,
                },
            });
            state.family_links.insert(
                link_id,
                FamilyLink {
                    id: link_id,
                    first_character_id: head_id,
                    second_character_id: id,
                    kind: link_kind,
                    active: true,
                },
            );
            state
                .family_councils
                .get_mut(&dynasty_id)
                .expect("council must exist")
                .members
                .insert(id);
            id
        };
        state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("dynasty must exist")
            .relationships
            .heir_id = Some(new_heir_id);
    }
}

/// Selects the most capable adult active dynasty member other than the head,
/// breaking ties by stable ID order. When the house has no adult besides the
/// head, the most capable active member of any age succeeds instead, so a
/// household of minors still designates an heir and the annual succession
/// pass can execute rather than leaving a headless house operating.
fn emergency_successor(state: &AppState, head_id: CharacterId) -> Option<CharacterId> {
    let head = state.characters.get(head_id)?;
    let candidate = |character: &&crate::core::Character| {
        character.dynasty_id() == head.dynasty_id()
            && character.id() != head_id
            && character.status() == CharacterStatus::Active
    };
    let mut candidates: Vec<_> = state.characters.iter().filter(candidate).collect();
    if candidates.iter().all(|character| {
        state.clock.day().saturating_sub(character.birth_day())
            < crate::systems::commands::HEIR_MINIMUM_AGE_DAYS
    }) {
        return candidates
            .into_iter()
            .max_by_key(|character| emergency_successor_rank(character))
            .map(crate::core::Character::id);
    }
    candidates.retain(|character| {
        state.clock.day().saturating_sub(character.birth_day())
            >= crate::systems::commands::HEIR_MINIMUM_AGE_DAYS
    });
    candidates
        .into_iter()
        .max_by_key(|character| emergency_successor_rank(character))
        .map(crate::core::Character::id)
}

/// Capability sum with stable typed-ID tie-breaking.
fn emergency_successor_rank(character: &crate::core::Character) -> (u32, u32) {
    (
        u32::from(character.capabilities.administration)
            + u32::from(character.capabilities.commerce)
            + u32::from(character.capabilities.social)
            + u32::from(character.capabilities.craft),
        character.id().value(),
    )
}

/// Vacates any office held by `character_id` (clamping the replacement
/// selection day) and removes them from every institutional membership.
fn vacate_character_institutional_roles(
    state: &mut AppState,
    character_id: CharacterId,
    replacement_selection_day: Option<i64>,
) {
    for institution in state.institutions.values_mut() {
        institution.members.remove(&character_id);
        if institution.office_holder_id == Some(character_id) {
            institution.office_holder_id = None;
            institution.next_selection_day = institution
                .next_selection_day
                .min(replacement_selection_day.expect("office replacement day was preflighted"));
        }
    }
}

/// Hands management of a character's businesses to `replacement_manager_id`.
fn reassign_managed_businesses(
    state: &mut AppState,
    dynasty_id: DynastyId,
    character_id: CharacterId,
    replacement_manager_id: CharacterId,
) {
    let managed_business_ids: Vec<_> = state
        .businesses
        .ids_for_owner(dynasty_id)
        .into_iter()
        .flatten()
        .copied()
        .filter(|business_id| {
            state
                .businesses
                .get(*business_id)
                .is_some_and(|business| business.manager_id() == character_id)
        })
        .collect();
    for business_id in managed_business_ids {
        state
            .businesses
            .get_mut(business_id)
            .expect("owner business index must resolve")
            .operations
            .manager_id = replacement_manager_id;
    }
}

/// Picks the active dynasty member who should take over management duties from
/// an inactive character. The head is preferred while active; otherwise the
/// most capable active member takes over, so an operating business is never
/// left in the hands of an incapacitated or deceased manager.
fn resolve_active_management_successor(
    state: &AppState,
    dynasty_id: DynastyId,
    departing_character_id: CharacterId,
) -> Option<CharacterId> {
    let dynasty = state.dynasties.get(&dynasty_id)?;
    let head_id = dynasty.head_id();
    if head_id != departing_character_id
        && state
            .characters
            .get(head_id)
            .is_some_and(|head| head.status() == CharacterStatus::Active)
    {
        return Some(head_id);
    }
    state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == dynasty_id
                && character.id() != departing_character_id
                && character.status() == CharacterStatus::Active
        })
        .max_by_key(|character| emergency_successor_rank(character))
        .map(crate::core::Character::id)
}

/// Annual reconciliation: no business may keep a manager whose health or
/// succession has taken them out of active standing. The per-character handoff
/// in `synchronize_character_incapacitation` can target a head that a later
/// succession retires in the same pass, so this sweep guarantees the lifecycle
/// invariant instead of trusting once-per-character ordering.
fn reconcile_inactive_business_managers(state: &mut AppState) {
    let stale_managers: Vec<(BusinessId, DynastyId)> = state
        .businesses
        .iter()
        .filter(|business| {
            state
                .characters
                .get(business.manager_id())
                .is_none_or(|manager| manager.status() != CharacterStatus::Active)
        })
        .map(|business| (business.id(), business.owner_dynasty_id()))
        .collect();
    for (business_id, dynasty_id) in stale_managers {
        let departing_manager_id = state
            .businesses
            .get(business_id)
            .map(crate::core::Business::manager_id)
            .expect("stale-manager business must exist");
        if let Some(successor_id) =
            resolve_active_management_successor(state, dynasty_id, departing_manager_id)
        {
            reassign_managed_businesses(state, dynasty_id, departing_manager_id, successor_id);
        }
    }
}

fn synchronize_character_incapacitation(
    state: &mut AppState,
    character_id: CharacterId,
    dynasty_id: DynastyId,
    character_name: &str,
) -> Result<(), SimulationError> {
    let replacement_selection_day = state
        .institutions
        .values()
        .any(|institution| institution.office_holder_id == Some(character_id))
        .then(|| checked_future_day(state.clock.day(), 30))
        .transpose()?;
    for link in state.family_links.values_mut().filter(|link| {
        link.active
            && matches!(link.kind, FamilyLinkKind::Ward | FamilyLinkKind::Marriage)
            && (link.first_character_id == character_id || link.second_character_id == character_id)
    }) {
        link.active = false;
    }
    state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("character dynasty must have a family council")
        .members
        .remove(&character_id);
    vacate_character_institutional_roles(state, character_id, replacement_selection_day);
    if let Some(replacement_manager_id) =
        resolve_active_management_successor(state, dynasty_id, character_id)
    {
        reassign_managed_businesses(state, dynasty_id, character_id, replacement_manager_id);
    }
    if dynasty_id == state.player_dynasty_id {
        super::strategic::try_push_outbox(
            state,
            OutboxKind::Family,
            format!("{character_name} became incapacitated"),
            format!(
                "Character {character_id} left active family, institutional, and business duties because their health reached zero."
            ),
        )?;
    }
    Ok(())
}

/// Marks a long-incapacitated member as deceased. Incapacitation already
/// vacated every council, institutional, and management duty, so only the
/// status and any surviving active family links need closing.
fn retire_incapacitated_member(state: &mut AppState, character_id: CharacterId) {
    state
        .characters
        .get_mut(character_id)
        .expect("incapacitated character must exist")
        .runtime
        .status = CharacterStatus::Deceased;
    for link in state.family_links.values_mut().filter(|link| {
        link.active
            && (link.first_character_id == character_id || link.second_character_id == character_id)
    }) {
        link.active = false;
    }
}

fn resolve_annual_health(current: u16, age_years: i64, epidemic_severity: u16) -> u16 {
    if current == 0 {
        return 0;
    }
    let age_delta = match age_years {
        ..=39 => 100,
        40..=54 => -100,
        55..=69 => -300,
        _ => -700,
    };
    let epidemic_penalty = i32::from(epidemic_severity / 10);
    i32::from(current)
        .saturating_add(age_delta)
        .saturating_sub(epidemic_penalty)
        .clamp(0, 10_000)
        .try_into()
        .expect("clamped health must fit u16")
}

fn update_succession_risks(state: &mut AppState) {
    let governance: BTreeMap<_, _> = state
        .family_councils
        .iter()
        .map(|(dynasty_id, council)| (*dynasty_id, council.governance))
        .collect();
    let office_loads: BTreeMap<_, _> = state
        .dynasties
        .keys()
        .copied()
        .map(|dynasty_id| {
            (
                dynasty_id,
                super::strategic::dynasty_office_administrative_load(state, dynasty_id),
            )
        })
        .collect();
    for dynasty in state.dynasties.values_mut() {
        let office_load = office_loads.get(&dynasty.id()).copied().unwrap_or(0);
        let overextension = dynasty
            .administrative_load()
            .saturating_add(office_load)
            .saturating_sub(dynasty.administrative_capacity());
        let base_risk = i32::from(
            1_000_u16
                .saturating_add(overextension.saturating_mul(25))
                .min(9_500),
        );
        let governance_adjustment = match governance
            .get(&dynasty.id())
            .copied()
            .unwrap_or(HouseGovernance::Primogeniture)
        {
            HouseGovernance::HeadCommand => 500,
            HouseGovernance::Primogeniture => -400,
            HouseGovernance::FamilyPartnership => -250,
            HouseGovernance::BranchFederation => 200,
            HouseGovernance::ElectedHead => 700,
        };
        dynasty.runtime.succession_risk_basis_points = u16::try_from(
            base_risk
                .saturating_add(governance_adjustment)
                .clamp(0, 9_500),
        )
        .expect("clamped succession risk must fit u16");
    }
}

fn decide_successions(state: &mut AppState) -> Result<Vec<SuccessionLine>, SimulationError> {
    let snapshots: Vec<_> = state
        .dynasties
        .values()
        .filter_map(|dynasty| {
            dynasty.heir_id().map(|heir_id| {
                (
                    dynasty.id(),
                    dynasty.name().to_owned(),
                    dynasty.head_id(),
                    heir_id,
                    dynasty.runtime.generation,
                    dynasty.runtime.succession_risk_basis_points,
                )
            })
        })
        .collect();
    let mut lines = Vec::new();

    for (dynasty_id, dynasty_name, head_id, heir_id, generation, succession_risk_basis_points) in
        snapshots
    {
        let head = state
            .characters
            .get(head_id)
            .expect("dynasty head reference must be valid");
        let age_days = state.clock.day().saturating_sub(head.birth_day());
        let age_years = age_days / 360;
        let health_forces_succession = head.runtime.health_basis_points == 0;
        if age_years < SUCCESSION_ELIGIBILITY_AGE_YEARS && !health_forces_succession {
            continue;
        }
        let annual_chance = succession_chance_basis_points(
            age_years,
            succession_risk_basis_points,
            head.runtime.health_basis_points,
        );
        if !health_forces_succession && !state.rng.is_chance_success(annual_chance) {
            continue;
        }
        let next_generation = generation
            .checked_add(1)
            .filter(|next| *next < u16::MAX)
            .ok_or(SimulationError::DynastyGenerationExhausted { dynasty_id })?;
        let current_charter_version = state
            .family_councils
            .get(&dynasty_id)
            .expect("succession dynasty must have a family council")
            .charter_version;
        let next_charter_version =
            next_family_charter_version(dynasty_id, current_charter_version)?;
        let SuccessionShock {
            formally_prepared,
            family_unity_loss,
            family_loyalty_loss,
            legitimacy_loss,
        } = succession_shock(state, dynasty_id, heir_id, succession_risk_basis_points);
        let (new_heir_birth_day, new_heir_link_kind, new_heir_capabilities) =
            generate_next_heir(state, heir_id);
        lines.push(SuccessionLine {
            dynasty_id,
            outgoing_head_id: head_id,
            incoming_head_id: heir_id,
            formally_prepared,
            family_unity_loss,
            family_loyalty_loss,
            legitimacy_loss,
            new_heir_name: format!("{dynasty_name} Heir {next_generation}"),
            new_heir_birth_day,
            new_heir_link_kind,
            next_generation,
            next_charter_version,
            new_heir_capabilities,
        });
    }

    Ok(lines)
}

fn heir_was_formally_prepared(
    state: &AppState,
    dynasty_id: DynastyId,
    incoming_head_id: CharacterId,
) -> bool {
    let subject = format!("dynasty:{dynasty_id}");
    // Any designation naming the incoming head counts, not just the most
    // recent one: a later re-designation of a different heir must not erase
    // an earlier formal preparation of the character who actually succeeds.
    state
        .audit_log
        .iter()
        .filter(|record| record.kind() == AuditKind::HeirDesignation && record.subject() == subject)
        .any(|record| super::heir_audit_detail_matches(record, incoming_head_id))
}

fn succession_shock(
    state: &AppState,
    dynasty_id: DynastyId,
    incoming_head_id: CharacterId,
    succession_risk_basis_points: u16,
) -> SuccessionShock {
    let formally_prepared = heir_was_formally_prepared(state, dynasty_id, incoming_head_id);
    if formally_prepared {
        // Harness showed stranded recoveries after succession: legitimacy at
        // 15 bp and no patronage for 720 days despite building political
        // embedding. Trim losses so a prepared heir can actually rebuild.
        SuccessionShock {
            formally_prepared,
            family_unity_loss: 1_000_u16
                .saturating_add(succession_risk_basis_points / 8)
                .min(2_200),
            family_loyalty_loss: 350_u16
                .saturating_add(succession_risk_basis_points / 12)
                .min(900),
            legitimacy_loss: succession_risk_basis_points / 10,
        }
    } else {
        SuccessionShock {
            formally_prepared,
            family_unity_loss: 2_200_u16
                .saturating_add(succession_risk_basis_points / 3)
                .min(4_500),
            family_loyalty_loss: 900_u16
                .saturating_add(succession_risk_basis_points / 6)
                .min(2_000),
            legitimacy_loss: succession_risk_basis_points / 3,
        }
    }
}

fn generate_next_heir(
    state: &mut AppState,
    incoming_head_id: CharacterId,
) -> (i64, FamilyLinkKind, CharacterCapabilities) {
    let incoming_age_days = state.clock.day().saturating_sub(
        state
            .characters
            .get(incoming_head_id)
            .expect("dynasty heir reference must be valid")
            .birth_day(),
    );
    let parent_child_age_requirement =
        (20 * 360_i64).saturating_add(crate::core::MIN_PARENT_CHILD_AGE_GAP_DAYS);
    let incoming_birth_day = state
        .characters
        .get(incoming_head_id)
        .expect("dynasty heir reference must be valid")
        .birth_day();
    let (birth_day, link_kind) = if incoming_age_days >= parent_child_age_requirement {
        (
            state.clock.day().saturating_sub(20 * 360),
            FamilyLinkKind::ParentChild,
        )
    } else {
        // A generated sibling must always be younger than the incoming head,
        // even when forced succession elevates a child or adolescent heir.
        // Enforce at least a one-year gap so siblings are not 1 day apart,
        // which would be biologically incoherent and could create a minor
        // heir whose sibling is practically the same age.
        (
            state
                .clock
                .day()
                .saturating_sub(18 * 360)
                .max(incoming_birth_day.saturating_add(360)),
            FamilyLinkKind::Sibling,
        )
    };
    let capabilities = CharacterCapabilities {
        administration: 40_u16
            .saturating_add(u16::try_from(state.rng.range_u32(50)).expect("random value fits u16")),
        commerce: 40_u16
            .saturating_add(u16::try_from(state.rng.range_u32(50)).expect("random value fits u16")),
        social: 40_u16
            .saturating_add(u16::try_from(state.rng.range_u32(50)).expect("random value fits u16")),
        craft: 30_u16
            .saturating_add(u16::try_from(state.rng.range_u32(55)).expect("random value fits u16")),
    };
    (birth_day, link_kind, capabilities)
}

fn succession_chance_basis_points(
    age_years: i64,
    succession_risk_basis_points: u16,
    health_basis_points: u16,
) -> u16 {
    if age_years < SUCCESSION_ELIGIBILITY_AGE_YEARS {
        return 0;
    }
    // The ramp must mature succession pressure inside the session that builds
    // the dynasty: founders begin at 56-58 years old, so this rate puts the
    // median first transition in the second or third campaign year while
    // still leaving most of an establishment phase untouched.
    let age_pressure = (age_years - SUCCESSION_ELIGIBILITY_AGE_YEARS)
        .saturating_mul(AGE_PRESSURE_PER_YEAR_OVER_ELIGIBILITY);
    let governance_pressure = i64::from(succession_risk_basis_points / 2);
    let health_pressure = i64::from(10_000_u16.saturating_sub(health_basis_points) / 2);
    u16::try_from(
        age_pressure
            .saturating_add(governance_pressure)
            .saturating_add(health_pressure)
            .clamp(0, 9_500),
    )
    .expect("clamped succession chance must fit u16")
}

fn retire_outgoing_head(state: &mut AppState, outgoing_head_id: CharacterId) {
    state
        .characters
        .get_mut(outgoing_head_id)
        .expect("succession outgoing head must exist")
        .runtime
        .status = CharacterStatus::Deceased;
    for link in state.family_links.values_mut().filter(|link| {
        link.active
            && matches!(link.kind, FamilyLinkKind::Marriage | FamilyLinkKind::Ward)
            && (link.first_character_id == outgoing_head_id
                || link.second_character_id == outgoing_head_id)
    }) {
        link.active = false;
    }
}

fn update_institutions_for_succession(
    state: &mut AppState,
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
    replacement_selection_day: Option<i64>,
) {
    // Non-player heads hold institutional seats by dynasty standing, so the
    // incoming head inherits them. Player dynasties earn membership through
    // patronage instead, so their seats are not transferred.
    let transfer_membership = dynasty_id != state.player_dynasty_id;
    for institution in state.institutions.values_mut() {
        institution.members.remove(&outgoing_head_id);
        if transfer_membership {
            institution.members.insert(incoming_head_id);
        }
        if institution.office_holder_id == Some(outgoing_head_id) {
            institution.office_holder_id = None;
            institution.next_selection_day = institution
                .next_selection_day
                .min(replacement_selection_day.expect("office replacement day was preflighted"));
        }
    }
}

fn insert_succession_heir(
    state: &mut AppState,
    dynasty_id: DynastyId,
    incoming_head_id: CharacterId,
    new_heir_name: String,
    new_heir_birth_day: i64,
    new_heir_link_kind: FamilyLinkKind,
    new_heir_capabilities: CharacterCapabilities,
) -> Result<CharacterId, SimulationError> {
    let mut next_ids = state.next_ids.clone();
    let new_heir_id = next_ids.try_character()?;
    let family_link_id = next_ids.try_family_link()?;
    state.next_ids = next_ids;
    state.characters.insert(Character {
        identity: CharacterIdentity {
            id: new_heir_id,
            dynasty_id,
            name: new_heir_name,
            birth_day: new_heir_birth_day,
        },
        capabilities: new_heir_capabilities,
        runtime: CharacterRuntime {
            status: CharacterStatus::Active,
            health_basis_points: 9_500,
            loyalty_basis_points: 8_000,
            role: CharacterRole::Heir,
            incapacitated_day: None,
        },
    });
    state.family_links.insert(
        family_link_id,
        FamilyLink {
            id: family_link_id,
            first_character_id: incoming_head_id,
            second_character_id: new_heir_id,
            kind: new_heir_link_kind,
            active: true,
        },
    );
    Ok(new_heir_id)
}
fn apply_family_succession_transition(
    state: &mut AppState,
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
    new_heir_id: CharacterId,
    shock: SuccessionShock,
    next_charter_version: u64,
) {
    let affected_family_members = {
        let council = state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("succession dynasty must have a family council");
        council.members.remove(&outgoing_head_id);
        council.members.insert(incoming_head_id);
        council.members.insert(new_heir_id);
        council.unity_basis_points = council
            .unity_basis_points
            .saturating_sub(shock.family_unity_loss);
        council.charter_version = next_charter_version;
        council
            .members
            .iter()
            .copied()
            .filter(|character_id| {
                *character_id != incoming_head_id && *character_id != new_heir_id
            })
            .collect::<Vec<_>>()
    };
    for character_id in affected_family_members {
        if let Some(character) = state.characters.get_mut(character_id)
            && character.status() == CharacterStatus::Active
        {
            character.runtime.loyalty_basis_points = character
                .runtime
                .loyalty_basis_points
                .saturating_sub(shock.family_loyalty_loss);
        }
    }
}

fn record_succession_transition(
    state: &mut AppState,
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
    formally_prepared: bool,
    family_unity_loss: u16,
    legitimacy_loss: u16,
) -> Result<(), SimulationError> {
    let id = state.next_ids.try_chronicle()?;
    state.chronicle.push(ChronicleEntry {
        id,
        day: state.clock.day(),
        kind: ChronicleKind::Succession,
        summary: format!(
            "Dynasty {dynasty_id} passed from character {outgoing_head_id} to {incoming_head_id}; formal preparation was {formally_prepared}."
        ),
    });
    if dynasty_id == state.player_dynasty_id {
        super::strategic::try_push_outbox(
            state,
            OutboxKind::Family,
            "A new generation inherited the house".to_owned(),
            format!(
                "Character {incoming_head_id} succeeded character {outgoing_head_id}. Family unity fell by {family_unity_loss} bp and legitimacy by {legitimacy_loss} bp. Formal heir preparation was {formally_prepared}, so the severity of the transition reflects the dynasty's succession planning."
            ),
        )?;
    }
    Ok(())
}

fn apply_successions(
    state: &mut AppState,
    lines: Vec<SuccessionLine>,
) -> Result<(), SimulationError> {
    if lines.is_empty() {
        return Ok(());
    }
    let mut candidate = state.clone();
    apply_successions_in_place(&mut candidate, lines)?;
    *state = candidate;
    Ok(())
}

fn apply_successions_in_place(
    state: &mut AppState,
    lines: Vec<SuccessionLine>,
) -> Result<(), SimulationError> {
    for line in lines {
        let SuccessionLine {
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            formally_prepared,
            family_unity_loss,
            family_loyalty_loss,
            legitimacy_loss,
            new_heir_name,
            new_heir_birth_day,
            new_heir_link_kind,
            next_generation,
            next_charter_version,
            new_heir_capabilities,
        } = line;
        let replacement_selection_day = state
            .institutions
            .values()
            .any(|institution| institution.office_holder_id == Some(outgoing_head_id))
            .then(|| checked_future_day(state.clock.day(), 30))
            .transpose()?;
        retire_outgoing_head(state, outgoing_head_id);
        {
            let incoming = state
                .characters
                .get_mut(incoming_head_id)
                .expect("succession incoming head must exist");
            incoming.runtime.role = CharacterRole::HeadOfHouse;
            incoming.runtime.loyalty_basis_points = 10_000;
            // Lift the heir health pin: see SUCCESSION_ACCESSION_HEALTH_FLOOR.
            incoming.runtime.health_basis_points = incoming
                .runtime
                .health_basis_points
                .max(SUCCESSION_ACCESSION_HEALTH_FLOOR);
        }

        update_institutions_for_succession(
            state,
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            replacement_selection_day,
        );
        reassign_managed_businesses(state, dynasty_id, outgoing_head_id, incoming_head_id);
        let new_heir_id = insert_succession_heir(
            state,
            dynasty_id,
            incoming_head_id,
            new_heir_name,
            new_heir_birth_day,
            new_heir_link_kind,
            new_heir_capabilities,
        )?;
        apply_family_succession_transition(
            state,
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            new_heir_id,
            SuccessionShock {
                formally_prepared,
                family_unity_loss,
                family_loyalty_loss,
                legitimacy_loss,
            },
            next_charter_version,
        );

        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("succession dynasty must exist");
        dynasty.relationships.head_id = incoming_head_id;
        dynasty.relationships.heir_id = Some(new_heir_id);
        dynasty.runtime.generation = next_generation;
        dynasty.runtime.phase = crate::core::CampaignPhase::Legacy;
        let remaining = dynasty
            .resources
            .legitimacy_basis_points
            .saturating_sub(legitimacy_loss);
        // A dynasty that built political embedding should not emerge from
        // succession with single-digit legitimacy and no credible recovery
        // route — the harness stranded such houses for 720+ days at 15 bp.
        // Preserve a floor so the new head can afford the first patronage
        // step that rebuilds institutional standing.
        dynasty.resources.legitimacy_basis_points = if formally_prepared {
            remaining.max(2_000)
        } else {
            remaining.max(1_500)
        };
        record_succession_transition(
            state,
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            formally_prepared,
            family_unity_loss,
            legitimacy_loss,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "simulation_tests.rs"]
mod tests;
