//! Deterministic daily simulation pipeline; each phase decides before it applies.

use super::SimulationError;
use super::transactions::{
    checked_future_day, debit_market_clearing_account, next_business_finance_version,
    next_family_charter_version,
};
use crate::core::{
    AppState, AuditKind, AuditRecord, BusinessStatus, Character, CharacterCapabilities,
    CharacterIdentity, CharacterRole, CharacterRuntime, CharacterStatus, ChronicleEntry,
    ChronicleKind, CrisisKind, EmploymentStatus, FamilyLink, FamilyLinkKind, HouseGovernance,
    MarketCause, OutboxKind, SocialClass,
};
use crate::ids::{BusinessId, CharacterId, DynastyId, GoodId, RecipeId};
use crate::money::{Money, Quantity, affordable_quantity, checked_cost_for, cost_for};
use crate::registry::{GoodCategory, RecipeDef, Registry};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
struct BusinessPurchaseLine {
    business_id: BusinessId,
    good_id: GoodId,
    quantity: Quantity,
    cost: Money,
}

#[derive(Clone, Debug)]
struct BusinessPurchasePlan {
    lines: Vec<BusinessPurchaseLine>,
}

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
/// Returns an error for a zero day count, an exhausted day or schedule range, a registry mismatch,
/// missing market definitions, identifier-allocation exhaustion, or a business finance ledger that
/// cannot represent a required mutation. The campaign is unchanged when any requested day fails.
pub fn advance_days(
    registry: &Registry,
    state: &mut AppState,
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
    validate_market_quotes(registry, state)?;

    let mut next_state = state.clone();
    for _ in 0..days {
        run_one_day(registry, &mut next_state)?;
        super::validate_invariants(registry, &next_state);
    }
    *state = next_state;

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

fn run_one_day(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
    reset_market_flows(state);
    super::strategic::run_daily_strategic_systems(registry, state)?;

    let purchase_plan = decide_business_purchases(registry, state)?;
    apply_business_purchases(state, purchase_plan)?;

    let production_plan = decide_production(registry, state);
    apply_production(state, production_plan)?;

    let sale_plan = decide_business_sales(registry, state)?;
    apply_business_sales(state, sale_plan)?;

    let household_plan = decide_household_consumption(registry, state)?;
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
        detail: format!("day={}", state.clock.day()),
    });
    Ok(())
}

fn reset_market_flows(state: &mut AppState) {
    for quote in state.market.quotes.values_mut() {
        quote.demand_today = Quantity::ZERO;
        quote.supply_today = Quantity::ZERO;
    }
}

fn decide_business_purchases(
    registry: &Registry,
    state: &AppState,
) -> Result<BusinessPurchasePlan, SimulationError> {
    let mut remaining_stock: BTreeMap<GoodId, Quantity> = state
        .market
        .quotes
        .iter()
        .map(|(good_id, quote)| (*good_id, quote.stock))
        .collect();
    let mut available_cash: BTreeMap<BusinessId, Money> = state
        .businesses
        .iter()
        .map(|business| (business.id(), business.cash()))
        .collect();
    let mut lines = Vec::new();

    for business in state.businesses.iter() {
        if matches!(
            business.status(),
            BusinessStatus::Closed | BusinessStatus::Insolvent
        ) {
            continue;
        }
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("business recipe reference must be valid");
        for input in recipe.inputs() {
            let target_batches = i64::from(business.operations.capacity_batches_per_day)
                .saturating_mul(i64::from(business.policy.target_input_days));
            let desired = input.quantity().saturating_mul_ratio(target_batches, 1);
            let current = business.inventory_quantity(input.good_id());
            if current >= desired {
                continue;
            }

            let quote = state.market.quotes.get(&input.good_id()).ok_or(
                SimulationError::MarketQuoteMissing {
                    good_id: input.good_id(),
                },
            )?;
            let stock = remaining_stock
                .get(&input.good_id())
                .copied()
                .unwrap_or(Quantity::ZERO);
            let cash = available_cash
                .get(&business.id())
                .copied()
                .unwrap_or(Money::ZERO);
            let spendable = cash.saturating_sub(business.policy.minimum_cash_reserve);
            let shortfall = desired.saturating_sub(current);
            let quantity = shortfall
                .min(stock)
                .min(affordable_quantity(spendable, quote.price));
            if quantity.is_zero() {
                continue;
            }
            let cost = cost_for(quantity, quote.price);
            remaining_stock.insert(
                input.good_id(),
                stock
                    .checked_sub(quantity)
                    .expect("planned business purchase must not exceed market stock"),
            );
            available_cash.insert(
                business.id(),
                cash.checked_sub(cost)
                    .expect("affordable business purchase must not exceed available cash"),
            );
            lines.push(BusinessPurchaseLine {
                business_id: business.id(),
                good_id: input.good_id(),
                quantity,
                cost,
            });
        }
    }

    Ok(BusinessPurchasePlan { lines })
}

fn apply_business_purchases(
    state: &mut AppState,
    plan: BusinessPurchasePlan,
) -> Result<(), SimulationError> {
    let mut total_cost_copper = 0_i128;
    let mut total_quantity_milliunits = 0_i128;
    for line in plan.lines {
        let BusinessPurchaseLine {
            business_id,
            good_id,
            quantity,
            cost,
        } = line;
        let (resulting_market_stock, resulting_market_demand) = {
            let quote = state
                .market
                .quotes
                .get(&good_id)
                .expect("planned market purchase quote must exist");
            (
                quote
                    .stock
                    .checked_sub(quantity)
                    .expect("planned business purchase must not exceed market stock"),
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
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("planned business purchase target must exist");
            let current_inventory = business.inventory_quantity(good_id);
            current_inventory.checked_add(quantity).ok_or(
                SimulationError::BusinessInventoryOverflow {
                    business_id,
                    good_id,
                    current: current_inventory,
                    incoming: quantity,
                },
            )?;
            let resulting_cash = business
                .finance
                .cash
                .checked_sub(cost)
                .expect("planned business purchase must fit available cash");
            let resulting_lifetime_costs =
                business.finance.lifetime_costs.checked_add(cost).ok_or(
                    SimulationError::BusinessLifetimeCostsOverflow {
                        business_id,
                        current: business.finance.lifetime_costs,
                        incoming: cost,
                    },
                )?;
            let next_finance_version = next_business_finance_version(business)?;
            business.finance.cash = resulting_cash;
            business.finance.lifetime_costs = resulting_lifetime_costs;
            business.finance.version = next_finance_version;
            business.add_inventory(good_id, quantity);
        }
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("planned market purchase quote must exist");
            quote.stock = resulting_market_stock;
            quote.demand_today = resulting_market_demand;
        }
        state.market.clearing_account = resulting_clearing;
        total_cost_copper += i128::from(cost.copper());
        total_quantity_milliunits += i128::from(quantity.milliunits());
    }
    if total_quantity_milliunits != 0 {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::MarketPurchase,
            subject: "businesses".into(),
            detail: format!("quantity={total_quantity_milliunits}; cost={total_cost_copper}"),
        });
    }
    Ok(())
}

fn decide_production(registry: &Registry, state: &AppState) -> ProductionPlan {
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
    for business in state.businesses.iter() {
        let Some(line) = decide_business_production(
            registry,
            state,
            business,
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

const PRODUCTION_TOOL_SHARE_BASIS_POINTS: i64 = 8_000;

/// Heads become eligible for succession at this age. Combined with the
/// annual chance ramp below, this keeps the first transition within a
/// playable session rather than pushing the dynasty fantasy past the
/// horizon most campaigns reach.
const SUCCESSION_ELIGIBILITY_AGE_YEARS: i64 = 50;

fn decide_business_production(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
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
    let mut batches = effective_capacity_batches(state, business);
    batches = batches.min(output_limited_batches(state, business, recipe));
    batches = batches.min(worker_limited_batches(state, business.id()));
    batches = batches.min(input_limited_batches(business, recipe));
    batches = batches.min(cash_limited_batches(business, recipe));
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

fn effective_capacity_batches(state: &AppState, business: &crate::core::Business) -> u16 {
    let dynasty = state
        .dynasties
        .get(&business.owner_dynasty_id())
        .expect("business owner reference must be valid");
    let governance = state
        .family_councils
        .get(&dynasty.id())
        .expect("business owner dynasty must have family governance")
        .governance;
    let effective_administrative_capacity = u32::from(dynasty.administrative_capacity())
        .saturating_mul(governance_administrative_multiplier(governance))
        / 10_000;
    let effective_administrative_load = dynasty.administrative_load().saturating_add(
        super::strategic::dynasty_office_administrative_load(state, dynasty.id()),
    );
    let administrative_efficiency = if effective_administrative_load == 0
        || u32::from(effective_administrative_load) <= effective_administrative_capacity
    {
        10_000_u16
    } else {
        u16::try_from(
            effective_administrative_capacity * 10_000 / u32::from(effective_administrative_load),
        )
        .expect("administrative efficiency must fit u16")
    };
    let status_efficiency = match business.status() {
        BusinessStatus::Active => 10_000_u16,
        BusinessStatus::Distressed => 6_000_u16,
        BusinessStatus::Insolvent | BusinessStatus::Closed => 0,
    };
    let effective_basis_points = administrative_efficiency
        .min(status_efficiency)
        .min(business.operations.condition_basis_points.max(2_500));
    let weighted_batches = u32::from(business.operations.capacity_batches_per_day)
        .saturating_mul(u32::from(effective_basis_points));
    u16::try_from(weighted_batches.saturating_add(5_000) / 10_000)
        .expect("effective batches must fit u16")
        .max(1)
}

fn output_limited_batches(
    state: &AppState,
    business: &crate::core::Business,
    recipe: &RecipeDef,
) -> u16 {
    let output_good_id = recipe.output_good_id();
    let reserve_batches = i64::from(business.operations.capacity_batches_per_day)
        .saturating_mul(i64::from(business.policy.target_output_days));
    let policy_reserve = recipe
        .output_quantity()
        .saturating_mul_ratio(reserve_batches, 1);
    let contract_reserve = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == crate::core::ContractStatus::Active
                && contract.seller_business_id == business.id()
                && contract.good_id == output_good_id
        })
        .fold(Quantity::ZERO, |total, contract| {
            total.saturating_add(contract.quantity_per_week)
        });
    let market_capacity =
        state
            .market
            .quotes
            .get(&output_good_id)
            .map_or(Quantity::ZERO, |quote| {
                quote
                    .target_stock
                    .saturating_mul_ratio(3, 2)
                    .saturating_sub(quote.stock)
                    .max(Quantity::ZERO)
            });
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

fn worker_limited_batches(state: &AppState, business_id: BusinessId) -> u16 {
    let active_workers = super::saturating_worker_count(
        state
            .employment
            .values()
            .filter(|agreement| agreement.business_id == business_id)
            .map(|agreement| match agreement.status {
                EmploymentStatus::Active => u32::from(agreement.workers),
                EmploymentStatus::Disputed => u32::from(agreement.workers).div_ceil(2),
                EmploymentStatus::Suspended | EmploymentStatus::Ended => 0,
            }),
    );
    u16::try_from(active_workers / u32::from(super::WORKERS_PER_BATCH)).unwrap_or(u16::MAX)
}

fn input_limited_batches(business: &crate::core::Business, recipe: &RecipeDef) -> u16 {
    recipe.inputs().iter().fold(u16::MAX, |limit, input| {
        let available = business.inventory_quantity(input.good_id()).milliunits();
        let per_batch = input.quantity().milliunits();
        let input_limited = if per_batch == 0 {
            0
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
        if let Some((resulting_stock, resulting_demand, resulting_clearing)) = market_update {
            let quote = state
                .market
                .quotes
                .get_mut(&tools_id)
                .expect("planned production tools quote must exist");
            quote.stock = resulting_stock;
            quote.demand_today = resulting_demand;
            state.market.clearing_account = resulting_clearing;
        }
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
            ),
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

fn decide_business_sales(
    registry: &Registry,
    state: &AppState,
) -> Result<BusinessSalePlan, SimulationError> {
    let mut market_capacity: BTreeMap<GoodId, Quantity> = state
        .market
        .quotes
        .iter()
        .map(|(good_id, quote)| {
            let maximum_stock = quote.target_stock.saturating_mul_ratio(3, 2);
            (
                *good_id,
                maximum_stock
                    .saturating_sub(quote.stock)
                    .max(Quantity::ZERO),
            )
        })
        .collect();
    let mut lines = Vec::new();

    for business in state.businesses.iter() {
        if matches!(
            business.status(),
            BusinessStatus::Closed | BusinessStatus::Insolvent
        ) {
            continue;
        }
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("business recipe reference must be valid");
        let manager = state
            .characters
            .get(business.manager_id())
            .expect("business manager reference must be valid");
        let good_id = recipe.output_good_id();
        let inventory = business.inventory_quantity(good_id);
        let reserve_batches = i64::from(business.operations.capacity_batches_per_day)
            .saturating_mul(i64::from(business.policy.target_output_days));
        let policy_reserve = recipe
            .output_quantity()
            .saturating_mul_ratio(reserve_batches, 1);
        let contract_reserve = state
            .contracts
            .values()
            .filter(|contract| {
                contract.status == crate::core::ContractStatus::Active
                    && contract.seller_business_id == business.id()
                    && contract.good_id == good_id
            })
            .fold(Quantity::ZERO, |total, contract| {
                total.saturating_add(contract.quantity_per_week)
            });
        let policy_reserve_basis_points = match business.status() {
            BusinessStatus::Distressed | BusinessStatus::Insolvent | BusinessStatus::Closed => 0,
            BusinessStatus::Active
                if business.cash() < recipe.daily_operating_cost().saturating_mul(2) =>
            {
                5_000
            }
            BusinessStatus::Active => 10_000,
        };
        let adjusted_policy_reserve =
            policy_reserve.saturating_mul_ratio(policy_reserve_basis_points, 10_000);
        let reserve = adjusted_policy_reserve.saturating_add(contract_reserve);
        let surplus = inventory.saturating_sub(reserve).max(Quantity::ZERO);
        let capacity = market_capacity
            .get(&good_id)
            .copied()
            .unwrap_or(Quantity::ZERO);
        let quote = state
            .market
            .quotes
            .get(&good_id)
            .ok_or(SimulationError::MarketQuoteMissing { good_id })?;
        let commerce_efficiency = 9_000_i64
            .saturating_add(i64::from(manager.capabilities.commerce).saturating_mul(10))
            .min(10_000);
        let quantity = surplus
            .min(capacity)
            .saturating_mul_ratio(commerce_efficiency, 10_000);
        if quantity.is_zero() {
            continue;
        }
        let revenue = validate_business_sale_revenue(
            business.id(),
            business.cash(),
            good_id,
            quantity,
            quote.price,
        )?;
        market_capacity.insert(good_id, capacity.saturating_sub(quantity));
        lines.push(BusinessSaleLine {
            business_id: business.id(),
            good_id,
            quantity,
            revenue,
        });
    }

    Ok(BusinessSalePlan { lines })
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
            detail: format!("quantity={total_quantity_milliunits}; revenue={total_revenue_copper}"),
        });
    }
    Ok(())
}

fn decide_household_consumption(
    registry: &Registry,
    state: &AppState,
) -> Result<HouseholdConsumptionPlan, SimulationError> {
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
    let tools_id = registry
        .get_good_id("tools")
        .expect("Rivergate registry must define tools");
    let mut stock: BTreeMap<GoodId, Quantity> = state
        .market
        .quotes
        .iter()
        .map(|(good_id, quote)| (*good_id, quote.stock))
        .collect();
    let mut lines = Vec::new();
    let mut food_satisfaction = BTreeMap::new();

    for household in state.households.iter() {
        let mut cash = household.cash;
        let mut food_acquired = Quantity::ZERO;
        for good_id in [bread_id, flour_id, grain_id] {
            let remaining_need = household.bread_need_daily.saturating_sub(food_acquired);
            if remaining_need.is_zero() {
                break;
            }
            let quantity = plan_household_purchase(
                state,
                household.id(),
                good_id,
                remaining_need,
                &mut cash,
                &mut stock,
                &mut lines,
            )?;
            food_acquired = food_acquired.saturating_add(quantity);
        }
        let (charcoal_need, cloth_need, tools_need) =
            household_secondary_needs(household.social_class());
        for (good_id, need) in [
            (ale_id, household.ale_need_daily),
            (charcoal_id, charcoal_need),
            (cloth_id, cloth_need),
            (tools_id, tools_need),
        ] {
            plan_household_purchase(
                state,
                household.id(),
                good_id,
                need,
                &mut cash,
                &mut stock,
                &mut lines,
            )?;
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

    Ok(HouseholdConsumptionPlan {
        lines,
        food_satisfaction,
    })
}

fn plan_household_purchase(
    state: &AppState,
    household_id: crate::ids::HouseholdId,
    good_id: GoodId,
    need: Quantity,
    cash: &mut Money,
    stock: &mut BTreeMap<GoodId, Quantity>,
    lines: &mut Vec<HouseholdPurchaseLine>,
) -> Result<Quantity, SimulationError> {
    let quote = state
        .market
        .quotes
        .get(&good_id)
        .ok_or(SimulationError::MarketQuoteMissing { good_id })?;
    let available = stock.get(&good_id).copied().unwrap_or(Quantity::ZERO);
    let quantity = need
        .min(available)
        .min(affordable_quantity(*cash, quote.price));
    if quantity.is_zero() {
        return Ok(Quantity::ZERO);
    }
    let cost = cost_for(quantity, quote.price);
    stock.insert(
        good_id,
        available
            .checked_sub(quantity)
            .expect("planned purchase quantity must not exceed available stock"),
    );
    *cash = cash
        .checked_sub(cost)
        .expect("affordable planned purchase must not exceed household cash");
    lines.push(HouseholdPurchaseLine {
        household_id,
        good_id,
        quantity,
        cost,
    });
    Ok(quantity)
}

fn household_secondary_needs(social_class: SocialClass) -> (Quantity, Quantity, Quantity) {
    let (charcoal, cloth, tools) = match social_class {
        SocialClass::Laboring => (180, 200, 30),
        SocialClass::Artisan => (240, 400, 120),
        SocialClass::Merchant => (300, 600, 180),
        SocialClass::Elite => (360, 800, 240),
    };
    (
        Quantity::from_milliunits(charcoal),
        Quantity::from_milliunits(cloth),
        Quantity::from_milliunits(tools),
    )
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
        let resulting_household_cash = state
            .households
            .get(household_id)
            .expect("planned household purchase target must exist")
            .cash
            .checked_sub(cost)
            .expect("planned household purchase must not exceed household cash");
        {
            let household = state
                .households
                .get_mut(household_id)
                .expect("planned household purchase target must exist");
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
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::HouseholdConsumption,
        subject: "households".into(),
        detail: format!("quantity={total_quantity_milliunits}; spending={total_cost_copper}"),
    });
    Ok(())
}

const MAINTENANCE_TOOL_SHARE_BASIS_POINTS: i64 = 10_000;

#[derive(Clone, Copy)]
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
    let snapshots: Vec<_> = state
        .businesses
        .iter()
        .filter(|business| {
            !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
        })
        .map(|business| MaintenanceSnapshot {
            business_id: business.id(),
            recipe_id: business.recipe_id(),
            cash: business.cash(),
            minimum_cash_reserve: business.policy.minimum_cash_reserve,
            maintenance_basis_points: business.policy.maintenance_basis_points,
            quality_target_basis_points: business.policy.quality_target_basis_points,
            condition_basis_points: business.operations.condition_basis_points,
            quality_basis_points: business.operations.quality_basis_points,
        })
        .collect();
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
        desired_cost.saturating_mul_ratio(MAINTENANCE_TOOL_SHARE_BASIS_POINTS, 10_000)
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
    let random_wear = i16::try_from(state.rng.range_u32(4)).expect("wear fits i16");
    let neglect_penalty = if maintenance_succeeds { 0 } else { 5 };
    let accident_penalty = if condition_basis_points < 4_000 && state.rng.is_chance_success(40) {
        120
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
    let quality_decline = if maintenance_succeeds { 0 } else { 3 };
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

fn maintenance_cost(daily_operating_cost: Money, maintenance_basis_points: u16) -> Money {
    if maintenance_basis_points == 0 || daily_operating_cost <= Money::ZERO {
        return Money::ZERO;
    }
    daily_operating_cost
        .saturating_mul_ratio_ceil_nonnegative(i64::from(maintenance_basis_points), 20_000)
}

fn maintenance_effect(maintenance_basis_points: u16, condition_basis_points: u16) -> i16 {
    let scaled = u32::from(maintenance_basis_points)
        .saturating_mul(32)
        .div_ceil(10_000);
    let catch_up = u32::from(9_500_u16.saturating_sub(condition_basis_points)).div_ceil(400);
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
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("planned maintenance business must exist");
        if cost > Money::ZERO {
            debug_assert!(tool_cost <= cost);
            let resulting_cash = business
                .finance
                .cash
                .checked_sub(cost)
                .expect("planned maintenance must fit available cash");
            let resulting_lifetime_costs =
                business.finance.lifetime_costs.checked_add(cost).ok_or(
                    SimulationError::BusinessLifetimeCostsOverflow {
                        business_id,
                        current: business.finance.lifetime_costs,
                        incoming: cost,
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
        if let Some((resulting_stock, resulting_demand, resulting_clearing)) = market_update {
            let quote = state
                .market
                .quotes
                .get_mut(&tools_id)
                .expect("planned maintenance tools quote must exist");
            quote.stock = resulting_stock;
            quote.demand_today = resulting_demand;
            state.market.clearing_account = resulting_clearing;
        }
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
            ),
        });
    }
    Ok(())
}

fn apply_market_spoilage(registry: &Registry, state: &mut AppState) {
    for good in registry.goods() {
        let quote = state
            .market
            .quotes
            .get_mut(&good.id())
            .expect("every registry good must have a market quote");
        let spoiled = quote
            .stock
            .saturating_mul_ratio(i64::from(good.daily_spoilage_basis_points()), 10_000);
        quote.stock = quote.stock.saturating_sub(spoiled);
    }
}

fn update_market_prices(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
    let day_of_year = state.clock.day_of_year();
    let production_floors = production_price_floors(registry, state);
    let mut price_shocks = Vec::new();

    for good in registry.goods() {
        let quote = state
            .market
            .quotes
            .get_mut(&good.id())
            .expect("every registry good must have a market quote");
        let target = quote.target_stock.milliunits().max(1);
        let stock_pressure =
            Quantity::from_milliunits(target.saturating_sub(quote.stock.milliunits()))
                .saturating_mul_ratio(1_000, target)
                .milliunits();
        let total_flow = quote
            .demand_today
            .milliunits()
            .saturating_add(quote.supply_today.milliunits())
            .max(1);
        let flow_pressure = Quantity::from_milliunits(
            quote
                .demand_today
                .milliunits()
                .saturating_sub(quote.supply_today.milliunits()),
        )
        .saturating_mul_ratio(500, total_flow)
        .milliunits();
        let seasonal_pressure = seasonal_pressure_basis_points(good.category(), day_of_year);
        let total_pressure = i128::from(stock_pressure)
            .saturating_add(i128::from(flow_pressure))
            .saturating_add(i128::from(seasonal_pressure))
            .clamp(-800, 800);
        let previous_price = quote.price;
        let raw_price = previous_price
            .saturating_mul_ratio(
                10_000
                    + i64::try_from(total_pressure)
                        .expect("clamped market pressure must fit the fixed-point ratio"),
                10_000,
            )
            .copper();
        let minimum_price = production_floors
            .get(&good.id())
            .copied()
            .unwrap_or_else(|| Money::from_copper((good.base_price().copper() / 2).max(1)))
            .copper()
            .max((good.base_price().copper() / 2).max(1));
        let maximum_price = good.base_price().copper().saturating_mul(4);
        quote.previous_price = previous_price;
        quote.price = Money::from_copper(raw_price.clamp(minimum_price, maximum_price));
        quote.causes = decide_market_causes(quote, seasonal_pressure);

        let change_basis_points = if previous_price.copper() == 0 {
            0
        } else {
            Money::from_copper(quote.price.copper().saturating_sub(previous_price.copper()))
                .saturating_mul_ratio(10_000, previous_price.copper())
                .copper()
        };
        if change_basis_points.unsigned_abs() >= 700 {
            price_shocks.push((good.name().to_owned(), quote.price, change_basis_points));
        }
    }

    price_shocks.sort_by(|left, right| {
        right
            .2
            .unsigned_abs()
            .cmp(&left.2.unsigned_abs())
            .then_with(|| left.0.cmp(&right.0))
    });
    for (good_name, price, change_basis_points) in price_shocks.into_iter().take(3) {
        let id = state.next_ids.try_chronicle()?;
        state.chronicle.push(ChronicleEntry {
            id,
            day: state.clock.day(),
            kind: ChronicleKind::PriceShock,
            summary: format!("{good_name} moved by {change_basis_points} basis points to {price}."),
        });
    }
    Ok(())
}

fn production_price_floors(registry: &Registry, state: &AppState) -> BTreeMap<GoodId, Money> {
    let mut floors = BTreeMap::new();
    for business in state.businesses.iter().filter(|business| {
        !matches!(
            business.status(),
            BusinessStatus::Closed | BusinessStatus::Insolvent
        )
    }) {
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("business recipe must exist");
        let output_milliunits = recipe.output_quantity().milliunits();
        if output_milliunits <= 0 {
            continue;
        }
        let weekly_labor_copper = state
            .employment
            .values()
            .filter(|agreement| {
                agreement.business_id == business.id()
                    && agreement.status != EmploymentStatus::Ended
            })
            .fold(0_i128, |total, agreement| {
                total + i128::from(agreement.weekly_wage.copper())
            });
        let daily_labor_copper =
            ceil_div_nonnegative_wide(weekly_labor_copper, 7).min(i128::from(i64::MAX));
        let daily_labor = Money::from_copper(
            i64::try_from(daily_labor_copper).expect("clamped daily labor cost must fit i64"),
        );
        let daily_maintenance = maintenance_cost(
            recipe.daily_operating_cost(),
            business.policy.maintenance_basis_points,
        );
        let overhead_per_batch = daily_labor.saturating_add(daily_maintenance);
        let batch_cost = recipe.inputs().iter().fold(
            recipe
                .daily_operating_cost()
                .saturating_add(overhead_per_batch),
            |total, input| {
                let price = state
                    .market
                    .quotes
                    .get(&input.good_id())
                    .expect("recipe input good must have a market quote")
                    .price;
                total.saturating_add(cost_for(input.quantity(), price))
            },
        );
        let break_even = batch_cost.saturating_mul_ratio_ceil_nonnegative(1_000, output_milliunits);
        let sustainable = break_even.saturating_mul_ratio_ceil_nonnegative(11, 10);
        floors
            .entry(recipe.output_good_id())
            .and_modify(|floor: &mut Money| *floor = (*floor).min(sustainable))
            .or_insert(sustainable);
    }
    floors
}

fn ceil_div_nonnegative_wide(numerator: i128, denominator: i128) -> i128 {
    debug_assert!(numerator >= 0 && denominator > 0);
    let quotient = numerator / denominator;
    quotient + i128::from(numerator % denominator != 0)
}

fn seasonal_pressure_basis_points(category: GoodCategory, day_of_year: u16) -> i64 {
    match category {
        GoodCategory::Staple => {
            if day_of_year >= 300 || day_of_year <= 60 {
                140
            } else if (210..=280).contains(&day_of_year) {
                -100
            } else {
                0
            }
        }
        GoodCategory::Fuel => {
            if day_of_year >= 300 || day_of_year <= 75 {
                110
            } else {
                0
            }
        }
        GoodCategory::Drink
        | GoodCategory::Textile
        | GoodCategory::Material
        | GoodCategory::Tool => 0,
    }
}

fn decide_market_causes(
    quote: &crate::core::MarketQuote,
    seasonal_pressure: i64,
) -> Vec<MarketCause> {
    let mut causes = Vec::new();
    if quote.stock < quote.target_stock {
        causes.push(MarketCause::StockBelowTarget);
    } else if quote.stock > quote.target_stock {
        causes.push(MarketCause::StockAboveTarget);
    }
    if quote.demand_today > quote.supply_today {
        causes.push(MarketCause::DemandExceededSupply);
    } else if quote.supply_today > quote.demand_today {
        causes.push(MarketCause::SupplyExceededDemand);
    }
    if seasonal_pressure != 0 {
        causes.push(MarketCause::SeasonalPressure);
    }
    if causes.is_empty() {
        causes.push(MarketCause::StableConditions);
    }
    causes
}

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
            super::synchronize_employment_for_business_status(
                state,
                business_id,
                BusinessStatus::Closed,
            );
            continue;
        }
        let recipe = registry
            .get_recipe(recipe_id)
            .expect("business recipe reference must be valid");
        let new_status =
            if prior_status == BusinessStatus::Insolvent && cash == Money::ZERO && !has_inventory {
                BusinessStatus::Closed
            } else if cash == Money::ZERO && !has_inventory {
                BusinessStatus::Insolvent
            } else if cash
                < minimum_cash_reserve
                    .saturating_add(recipe.daily_operating_cost().saturating_mul(2))
            {
                BusinessStatus::Distressed
            } else {
                BusinessStatus::Active
            };
        if new_status != prior_status {
            state
                .businesses
                .get_mut(business_id)
                .expect("lifecycle business must exist")
                .operations
                .status = new_status;
            events.push((business_id, prior_status, new_status));
        }
        super::synchronize_employment_for_business_status(state, business_id, new_status);
    }

    for (business_id, prior_status, new_status) in events {
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

fn settle_weekly_external_income(state: &mut AppState) -> Result<(), SimulationError> {
    let payments: Vec<_> = state
        .households
        .iter()
        .map(|household| {
            household.cash.checked_add(household.weekly_income).ok_or(
                SimulationError::HouseholdCashOverflow {
                    household_id: household.id(),
                    current: household.cash,
                    incoming: household.weekly_income,
                },
            )?;
            Ok((household.id(), household.weekly_income))
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
    debit_market_clearing_account(state, total)?;

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
        detail: format!("weekly_income={}", total.copper()),
    });
    Ok(())
}

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
        character.runtime.health_basis_points =
            if resolved_health == 0 && heir_ids.contains(&character.id()) {
                1
            } else {
                resolved_health
            };
        if character.runtime.health_basis_points == 0 && !head_ids.contains(&character.id()) {
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
    Ok(())
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
            && link.kind == FamilyLinkKind::Ward
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
    for institution in state.institutions.values_mut() {
        institution.members.remove(&character_id);
        if institution.office_holder_id == Some(character_id) {
            institution.office_holder_id = None;
            institution.next_selection_day = institution
                .next_selection_day
                .min(replacement_selection_day.expect("office replacement day was preflighted"));
        }
    }
    let replacement_manager_id = state
        .dynasties
        .get(&dynasty_id)
        .expect("character dynasty must exist")
        .head_id();
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
    let heir_marker = format!("heir={incoming_head_id}");
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::HeirDesignation && record.subject() == subject)
        .is_some_and(|record| record.detail().split(';').any(|part| part == heir_marker))
}

fn succession_shock(
    state: &AppState,
    dynasty_id: DynastyId,
    incoming_head_id: CharacterId,
    succession_risk_basis_points: u16,
) -> SuccessionShock {
    let formally_prepared = heir_was_formally_prepared(state, dynasty_id, incoming_head_id);
    if formally_prepared {
        SuccessionShock {
            formally_prepared,
            family_unity_loss: 1_000_u16
                .saturating_add(succession_risk_basis_points / 5)
                .min(2_500),
            family_loyalty_loss: 350_u16
                .saturating_add(succession_risk_basis_points / 12)
                .min(1_200),
            legitimacy_loss: succession_risk_basis_points / 8,
        }
    } else {
        SuccessionShock {
            formally_prepared,
            family_unity_loss: 2_500_u16
                .saturating_add(succession_risk_basis_points / 3)
                .min(5_000),
            family_loyalty_loss: 1_000_u16
                .saturating_add(succession_risk_basis_points / 8)
                .min(2_500),
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
    let (birth_day, link_kind) = if incoming_age_days >= parent_child_age_requirement {
        (
            state.clock.day().saturating_sub(20 * 360),
            FamilyLinkKind::ParentChild,
        )
    } else {
        (
            state.clock.day().saturating_sub(18 * 360),
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
    let age_pressure = (age_years - SUCCESSION_ELIGIBILITY_AGE_YEARS).saturating_mul(300);
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
    outgoing_head_id: CharacterId,
    replacement_selection_day: Option<i64>,
) {
    for institution in state.institutions.values_mut() {
        institution.members.remove(&outgoing_head_id);
        if institution.office_holder_id == Some(outgoing_head_id) {
            institution.office_holder_id = None;
            institution.next_selection_day = institution
                .next_selection_day
                .min(replacement_selection_day.expect("office replacement day was preflighted"));
        }
    }
}

fn transfer_succession_business_management(
    state: &mut AppState,
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
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
                .is_some_and(|business| business.manager_id() == outgoing_head_id)
        })
        .collect();
    for business_id in managed_business_ids {
        state
            .businesses
            .get_mut(business_id)
            .expect("owner business index must resolve")
            .operations
            .manager_id = incoming_head_id;
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
            property_claim_basis_points: 8_000,
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
        }

        update_institutions_for_succession(state, outgoing_head_id, replacement_selection_day);
        transfer_succession_business_management(
            state,
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
        );
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
        dynasty.resources.legitimacy_basis_points = dynasty
            .resources
            .legitimacy_basis_points
            .saturating_sub(legitimacy_loss);
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
