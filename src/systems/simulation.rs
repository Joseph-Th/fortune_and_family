//! Deterministic daily simulation pipeline; each phase decides before it applies.

use super::SimulationError;
use crate::core::{
    AppState, AuditKind, AuditRecord, BusinessStatus, CampaignPhase, Character,
    CharacterCapabilities, CharacterIdentity, CharacterRole, CharacterRuntime, CharacterStatus,
    ChronicleEntry, ChronicleKind, CrisisKind, EmploymentStatus, FamilyLink, FamilyLinkKind,
    HouseGovernance, MarketCause, SocialClass,
};
use crate::ids::{BusinessId, CharacterId, DynastyId, GoodId};
use crate::money::{Money, Quantity, affordable_quantity, cost_for};
use crate::registry::{GoodCategory, RecipeDef, Registry};
use std::collections::BTreeMap;

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
}

#[derive(Clone, Debug)]
struct ProductionPlan {
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
    condition_delta: i16,
    quality_delta: i16,
}

#[derive(Clone, Debug)]
struct MaintenancePlan {
    lines: Vec<MaintenanceLine>,
}

#[derive(Clone, Debug)]
struct SuccessionLine {
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
    new_heir_name: String,
    new_heir_birth_day: i64,
    new_heir_capabilities: CharacterCapabilities,
}

/// Advances the canonical simulation pipeline by a positive number of days.
///
/// # Errors
///
/// Returns an error for a zero day count, a registry mismatch, or missing market definitions.
pub fn advance_days(
    registry: &Registry,
    state: &mut AppState,
    days: u32,
) -> Result<(), SimulationError> {
    if days == 0 {
        return Err(SimulationError::InvalidDayCount { days });
    }
    if state.scenario_key != registry.scenario().key() {
        return Err(SimulationError::RegistryMismatch {
            state_scenario: state.scenario_key.clone(),
            registry_scenario: registry.scenario().key().to_owned(),
        });
    }
    validate_market_quotes(registry, state)?;

    for _ in 0..days {
        run_one_day(registry, state)?;
        super::validate_invariants(registry, state);
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

fn run_one_day(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
    reset_market_flows(state);
    super::strategic::run_daily_strategic_systems(registry, state);

    let purchase_plan = decide_business_purchases(registry, state)?;
    apply_business_purchases(state, purchase_plan);

    let production_plan = decide_production(registry, state);
    apply_production(state, production_plan);

    let sale_plan = decide_business_sales(registry, state)?;
    apply_business_sales(state, sale_plan);

    let household_plan = decide_household_consumption(registry, state)?;
    apply_household_consumption(state, household_plan);

    let maintenance_plan = decide_maintenance(registry, state);
    apply_maintenance(state, maintenance_plan);

    apply_market_spoilage(registry, state);
    update_market_prices(registry, state);
    super::strategic::apply_law_price_controls(registry, state);
    update_business_lifecycle(registry, state);

    state.clock.advance_one_day();
    if state.clock.is_week_boundary() {
        settle_weekly_external_income(state);
        super::strategic::run_weekly_strategic_systems(registry, state);
    }
    if state.clock.day() > 0 && state.clock.day() % 30 == 0 {
        super::strategic::run_monthly_strategic_systems(registry, state);
    }
    if state.clock.is_year_boundary() {
        process_year_boundary(registry, state);
        super::strategic::run_annual_strategic_systems(state);
    }

    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::DayAdvanced,
        subject: "simulation".to_owned(),
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
            let desired = input.quantity().saturating_mul_ratio(
                i64::from(
                    business
                        .operations
                        .capacity_batches_per_day
                        .saturating_mul(business.policy.target_input_days),
                ),
                1,
            );
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
            remaining_stock.insert(input.good_id(), stock.saturating_sub(quantity));
            available_cash.insert(business.id(), cash.saturating_sub(cost));
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

fn apply_business_purchases(state: &mut AppState, plan: BusinessPurchasePlan) {
    let mut total_cost = Money::ZERO;
    let mut total_quantity = Quantity::ZERO;
    for line in plan.lines {
        let BusinessPurchaseLine {
            business_id,
            good_id,
            quantity,
            cost,
        } = line;
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("planned business purchase target must exist");
            business.finance.cash = business.finance.cash.saturating_sub(cost);
            business.finance.lifetime_costs = business.finance.lifetime_costs.saturating_add(cost);
            business.finance.version = business.finance.version.saturating_add(1);
            business.add_inventory(good_id, quantity);
        }
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("planned market purchase quote must exist");
            quote.stock = quote.stock.saturating_sub(quantity);
            quote.demand_today = quote.demand_today.saturating_add(quantity);
        }
        state.market.clearing_account = state.market.clearing_account.saturating_add(cost);
        total_cost = total_cost.saturating_add(cost);
        total_quantity = total_quantity.saturating_add(quantity);
    }
    if !total_quantity.is_zero() {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::MarketPurchase,
            subject: "businesses".to_owned(),
            detail: format!(
                "quantity={}; cost={}",
                total_quantity.milliunits(),
                total_cost.copper()
            ),
        });
    }
}

fn decide_production(registry: &Registry, state: &AppState) -> ProductionPlan {
    ProductionPlan {
        lines: state
            .businesses
            .iter()
            .filter_map(|business| decide_business_production(registry, state, business))
            .collect(),
    }
}

fn decide_business_production(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
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
    Some(ProductionLine {
        business_id: business.id(),
        inputs,
        output_good_id: recipe.output_good_id(),
        output_quantity: recipe
            .output_quantity()
            .saturating_mul_ratio(i64::from(batches), 1)
            .saturating_mul_ratio(i64::from(quality_efficiency), 10_000)
            .saturating_mul_ratio(i64::from(craft_efficiency), 10_000),
        operating_cost: recipe
            .daily_operating_cost()
            .saturating_mul(i64::from(batches)),
    })
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
    let administrative_efficiency = if dynasty.administrative_load() == 0
        || u32::from(dynasty.administrative_load()) <= effective_administrative_capacity
    {
        10_000_u16
    } else {
        u16::try_from(
            effective_administrative_capacity * 10_000 / u32::from(dynasty.administrative_load()),
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
    let active_workers: u32 = state
        .employment
        .values()
        .filter(|agreement| agreement.business_id == business_id)
        .map(|agreement| match agreement.status {
            EmploymentStatus::Active => u32::from(agreement.workers),
            EmploymentStatus::Disputed => u32::from(agreement.workers).div_ceil(2),
            EmploymentStatus::Suspended | EmploymentStatus::Ended => 0,
        })
        .sum();
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

fn apply_production(state: &mut AppState, plan: ProductionPlan) {
    let mut total_output = Quantity::ZERO;
    let mut total_operating_cost = Money::ZERO;

    for line in plan.lines {
        let ProductionLine {
            business_id,
            inputs,
            output_good_id,
            output_quantity,
            operating_cost,
        } = line;
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("planned production business must exist");
        for (good_id, quantity) in inputs {
            business.remove_inventory(good_id, quantity);
        }
        business.add_inventory(output_good_id, output_quantity);
        business.finance.cash = business.finance.cash.saturating_sub(operating_cost);
        business.finance.lifetime_costs = business
            .finance
            .lifetime_costs
            .saturating_add(operating_cost);
        business.finance.version = business.finance.version.saturating_add(1);
        total_output = total_output.saturating_add(output_quantity);
        total_operating_cost = total_operating_cost.saturating_add(operating_cost);
    }

    if !total_output.is_zero() {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::Production,
            subject: "businesses".to_owned(),
            detail: format!(
                "output={}; operating_cost={}",
                total_output.milliunits(),
                total_operating_cost.copper()
            ),
        });
    }
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
        let commerce_efficiency = 9_000_i64
            .saturating_add(i64::from(manager.capabilities.commerce).saturating_mul(10))
            .min(10_000);
        let quantity = surplus
            .min(capacity)
            .saturating_mul_ratio(commerce_efficiency, 10_000);
        if quantity.is_zero() {
            continue;
        }
        let quote = state
            .market
            .quotes
            .get(&good_id)
            .ok_or(SimulationError::MarketQuoteMissing { good_id })?;
        let revenue = cost_for(quantity, quote.price);
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

fn apply_business_sales(state: &mut AppState, plan: BusinessSalePlan) {
    let mut total_revenue = Money::ZERO;
    let mut total_quantity = Quantity::ZERO;
    for line in plan.lines {
        let BusinessSaleLine {
            business_id,
            good_id,
            quantity,
            revenue,
        } = line;
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("planned business sale source must exist");
            business.remove_inventory(good_id, quantity);
            business.finance.cash = business.finance.cash.saturating_add(revenue);
            business.finance.lifetime_revenue =
                business.finance.lifetime_revenue.saturating_add(revenue);
            business.finance.version = business.finance.version.saturating_add(1);
        }
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("planned market sale quote must exist");
            quote.stock = quote.stock.saturating_add(quantity);
            quote.supply_today = quote.supply_today.saturating_add(quantity);
        }
        state.market.clearing_account = state.market.clearing_account.saturating_sub(revenue);
        total_revenue = total_revenue.saturating_add(revenue);
        total_quantity = total_quantity.saturating_add(quantity);
    }
    if !total_quantity.is_zero() {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::MarketSale,
            subject: "businesses".to_owned(),
            detail: format!(
                "quantity={}; revenue={}",
                total_quantity.milliunits(),
                total_revenue.copper()
            ),
        });
    }
}

fn decide_household_consumption(
    registry: &Registry,
    state: &AppState,
) -> Result<HouseholdConsumptionPlan, SimulationError> {
    let bread_id = registry
        .get_good_id("bread")
        .expect("Rivergate registry must define bread");
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
        let mut bread_acquired = Quantity::ZERO;
        let (charcoal_need, cloth_need, tools_need) =
            household_secondary_needs(household.social_class());
        for (good_id, need) in [
            (bread_id, household.bread_need_daily),
            (ale_id, household.ale_need_daily),
            (charcoal_id, charcoal_need),
            (cloth_id, cloth_need),
            (tools_id, tools_need),
        ] {
            let quote = state
                .market
                .quotes
                .get(&good_id)
                .ok_or(SimulationError::MarketQuoteMissing { good_id })?;
            let available = stock.get(&good_id).copied().unwrap_or(Quantity::ZERO);
            let quantity = need
                .min(available)
                .min(affordable_quantity(cash, quote.price));
            if quantity.is_zero() {
                continue;
            }
            let cost = cost_for(quantity, quote.price);
            stock.insert(good_id, available.saturating_sub(quantity));
            cash = cash.saturating_sub(cost);
            if good_id == bread_id {
                bread_acquired = quantity;
            }
            lines.push(HouseholdPurchaseLine {
                household_id: household.id(),
                good_id,
                quantity,
                cost,
            });
        }

        let daily_satisfaction = if household.bread_need_daily.is_zero() {
            10_000
        } else {
            u16::try_from(
                bread_acquired.milliunits().saturating_mul(10_000)
                    / household.bread_need_daily.milliunits(),
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

fn household_secondary_needs(social_class: SocialClass) -> (Quantity, Quantity, Quantity) {
    let (charcoal, cloth, tools) = match social_class {
        SocialClass::Laboring => (180, 100, 10),
        SocialClass::Artisan => (240, 200, 60),
        SocialClass::Merchant => (300, 300, 80),
        SocialClass::Elite => (360, 400, 100),
    };
    (
        Quantity::from_milliunits(charcoal),
        Quantity::from_milliunits(cloth),
        Quantity::from_milliunits(tools),
    )
}

fn apply_household_consumption(state: &mut AppState, plan: HouseholdConsumptionPlan) {
    let HouseholdConsumptionPlan {
        lines,
        food_satisfaction,
    } = plan;
    let mut total_cost = Money::ZERO;
    let mut total_quantity = Quantity::ZERO;
    for line in lines {
        let HouseholdPurchaseLine {
            household_id,
            good_id,
            quantity,
            cost,
        } = line;
        {
            let household = state
                .households
                .get_mut(household_id)
                .expect("planned household purchase target must exist");
            household.cash = household.cash.saturating_sub(cost);
        }
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("planned household purchase quote must exist");
            quote.stock = quote.stock.saturating_sub(quantity);
            quote.demand_today = quote.demand_today.saturating_add(quantity);
        }
        state.market.clearing_account = state.market.clearing_account.saturating_add(cost);
        total_cost = total_cost.saturating_add(cost);
        total_quantity = total_quantity.saturating_add(quantity);
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
        subject: "households".to_owned(),
        detail: format!(
            "quantity={}; spending={}",
            total_quantity.milliunits(),
            total_cost.copper()
        ),
    });
}

fn decide_maintenance(registry: &Registry, state: &mut AppState) -> MaintenancePlan {
    let snapshots: Vec<_> = state
        .businesses
        .iter()
        .map(|business| {
            (
                business.id(),
                business.recipe_id(),
                business.cash(),
                business.policy.minimum_cash_reserve,
                business.policy.maintenance_basis_points,
                business.policy.quality_target_basis_points,
                business.operations.condition_basis_points,
                business.operations.quality_basis_points,
                business.status(),
            )
        })
        .collect();
    let mut lines = Vec::new();

    for (
        business_id,
        recipe_id,
        cash,
        minimum_cash_reserve,
        maintenance_basis_points,
        quality_target_basis_points,
        condition_basis_points,
        quality_basis_points,
        status,
    ) in snapshots
    {
        if matches!(status, BusinessStatus::Closed | BusinessStatus::Insolvent) {
            continue;
        }
        let recipe = registry
            .get_recipe(recipe_id)
            .expect("business recipe reference must be valid");
        let desired_cost = Money::from_copper(
            recipe
                .daily_operating_cost()
                .copper()
                .saturating_mul(i64::from(maintenance_basis_points))
                / 20_000,
        );
        let can_maintain = cash.saturating_sub(minimum_cash_reserve) >= desired_cost;
        let random_wear = i16::try_from(state.rng.range_u32(4)).expect("wear fits i16");
        let neglect_penalty = if can_maintain { 0 } else { 5 };
        let accident_penalty = if condition_basis_points < 4_000 && state.rng.is_chance_success(40)
        {
            120
        } else {
            0
        };
        let improvement = if can_maintain && condition_basis_points < 9_500 {
            8
        } else {
            0
        };
        let quality_improvement =
            if can_maintain && quality_basis_points < quality_target_basis_points {
                i16::try_from((quality_target_basis_points - quality_basis_points).min(8))
                    .expect("bounded quality improvement must fit i16")
            } else {
                0
            };
        let quality_decline = if can_maintain { 0 } else { 3 };
        lines.push(MaintenanceLine {
            business_id,
            cost: if can_maintain {
                desired_cost
            } else {
                Money::ZERO
            },
            condition_delta: improvement - 2 - random_wear - neglect_penalty - accident_penalty,
            quality_delta: quality_improvement - quality_decline,
        });
    }

    MaintenancePlan { lines }
}

fn apply_maintenance(state: &mut AppState, plan: MaintenancePlan) {
    let mut total_cost = Money::ZERO;
    for line in plan.lines {
        let MaintenanceLine {
            business_id,
            cost,
            condition_delta,
            quality_delta,
        } = line;
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("planned maintenance business must exist");
        business.finance.cash = business.finance.cash.saturating_sub(cost);
        business.finance.lifetime_costs = business.finance.lifetime_costs.saturating_add(cost);
        business.finance.version = business.finance.version.saturating_add(1);
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
        total_cost = total_cost.saturating_add(cost);
    }
    if total_cost != Money::ZERO {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::Maintenance,
            subject: "businesses".to_owned(),
            detail: format!("cost={}", total_cost.copper()),
        });
    }
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

fn update_market_prices(registry: &Registry, state: &mut AppState) {
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
        let stock_pressure = (target - quote.stock.milliunits()).saturating_mul(1_000) / target;
        let total_flow = quote
            .demand_today
            .milliunits()
            .saturating_add(quote.supply_today.milliunits())
            .max(1);
        let flow_pressure = (quote.demand_today.milliunits() - quote.supply_today.milliunits())
            .saturating_mul(500)
            / total_flow;
        let seasonal_pressure = seasonal_pressure_basis_points(good.category(), day_of_year);
        let total_pressure = (stock_pressure + flow_pressure + seasonal_pressure).clamp(-800, 800);
        let previous_price = quote.price;
        let raw_price = previous_price
            .copper()
            .saturating_mul(10_000 + total_pressure)
            / 10_000;
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
            (quote.price.copper() - previous_price.copper()).saturating_mul(10_000)
                / previous_price.copper()
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
        let id = state.next_ids.chronicle();
        state.chronicle.push(ChronicleEntry {
            id,
            day: state.clock.day(),
            kind: ChronicleKind::PriceShock,
            summary: format!("{good_name} moved by {change_basis_points} basis points to {price}."),
        });
    }
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
        let weekly_labor = state
            .employment
            .values()
            .filter(|agreement| {
                agreement.business_id == business.id()
                    && agreement.status != EmploymentStatus::Ended
            })
            .fold(Money::ZERO, |total, agreement| {
                total.saturating_add(agreement.weekly_wage)
            });
        let daily_labor = Money::from_copper(ceil_div_nonnegative(weekly_labor.copper(), 7));
        let daily_maintenance = Money::from_copper(
            recipe
                .daily_operating_cost()
                .copper()
                .saturating_mul(i64::from(business.policy.maintenance_basis_points))
                / 20_000,
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
        let numerator = batch_cost.copper().saturating_mul(1_000);
        let break_even = ceil_div_nonnegative(numerator, output_milliunits);
        let sustainable =
            Money::from_copper(ceil_div_nonnegative(break_even.saturating_mul(11), 10));
        floors
            .entry(recipe.output_good_id())
            .and_modify(|floor: &mut Money| *floor = (*floor).min(sustainable))
            .or_insert(sustainable);
    }
    floors
}

fn ceil_div_nonnegative(numerator: i64, denominator: i64) -> i64 {
    debug_assert!(numerator >= 0 && denominator > 0);
    let quotient = numerator / denominator;
    quotient.saturating_add(i64::from(numerator % denominator != 0))
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

fn update_business_lifecycle(registry: &Registry, state: &mut AppState) {
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
        let new_status = if cash == Money::ZERO && !has_inventory {
            BusinessStatus::Insolvent
        } else if cash
            < minimum_cash_reserve.saturating_add(recipe.daily_operating_cost().saturating_mul(2))
        {
            BusinessStatus::Distressed
        } else {
            BusinessStatus::Active
        };
        if new_status == prior_status {
            continue;
        }
        state
            .businesses
            .get_mut(business_id)
            .expect("lifecycle business must exist")
            .operations
            .status = new_status;
        events.push((business_id, prior_status, new_status));
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
            BusinessStatus::Closed => match prior_status {
                BusinessStatus::Active
                | BusinessStatus::Distressed
                | BusinessStatus::Insolvent
                | BusinessStatus::Closed => continue,
            },
        };
        let id = state.next_ids.chronicle();
        state.chronicle.push(ChronicleEntry {
            id,
            day: state.clock.day(),
            kind,
            summary,
        });
    }
}

fn settle_weekly_external_income(state: &mut AppState) {
    let household_ids: Vec<_> = state
        .households
        .iter()
        .map(crate::core::Household::id)
        .collect();
    let mut total = Money::ZERO;
    for household_id in household_ids {
        let household = state
            .households
            .get_mut(household_id)
            .expect("weekly income household must exist");
        household.cash = household.cash.saturating_add(household.weekly_income);
        total = total.saturating_add(household.weekly_income);
    }
    state.market.clearing_account = state.market.clearing_account.saturating_sub(total);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::LaborSettlement,
        subject: "external-economy".to_owned(),
        detail: format!("weekly_income={}", total.copper()),
    });
}

fn process_year_boundary(registry: &Registry, state: &mut AppState) {
    let year = state.clock.year(registry.scenario().start_year());
    let id = state.next_ids.chronicle();
    state.chronicle.push(ChronicleEntry {
        id,
        day: state.clock.day(),
        kind: ChronicleKind::NewYear,
        summary: format!("Rivergate entered the year {year}."),
    });

    update_campaign_phases(state);
    update_character_health(state);
    let succession_plan = decide_successions(state);
    apply_successions(state, succession_plan);
}

fn update_character_health(state: &mut AppState) {
    let epidemic_severity = state
        .crises
        .values()
        .filter(|crisis| crisis.kind == CrisisKind::Epidemic && crisis.status.is_active())
        .map(|crisis| crisis.severity_basis_points)
        .max()
        .unwrap_or(0);
    let day = state.clock.day();
    for character in state.characters.iter_mut() {
        if character.status() != CharacterStatus::Active {
            continue;
        }
        let age_years = day.saturating_sub(character.birth_day()) / 360;
        character.runtime.health_basis_points = resolve_annual_health(
            character.runtime.health_basis_points,
            age_years,
            epidemic_severity,
        );
    }
}

fn resolve_annual_health(current: u16, age_years: i64, epidemic_severity: u16) -> u16 {
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

fn update_campaign_phases(state: &mut AppState) {
    let elapsed_years = state.clock.day() / 360;
    let phase = match elapsed_years {
        0..=4 => CampaignPhase::Foundation,
        5..=14 => CampaignPhase::Establishment,
        15..=29 => CampaignPhase::Ascendancy,
        30..=49 => CampaignPhase::Dominion,
        _ => CampaignPhase::Legacy,
    };
    let governance: BTreeMap<_, _> = state
        .family_councils
        .iter()
        .map(|(dynasty_id, council)| (*dynasty_id, council.governance))
        .collect();
    for dynasty in state.dynasties.values_mut() {
        dynasty.runtime.phase = phase;
        let overextension = dynasty
            .administrative_load()
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

fn decide_successions(state: &mut AppState) -> Vec<SuccessionLine> {
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
        if age_years < 55 {
            continue;
        }
        let annual_chance = succession_chance_basis_points(
            age_years,
            succession_risk_basis_points,
            head.runtime.health_basis_points,
        );
        if !state.rng.is_chance_success(annual_chance) {
            continue;
        }
        let next_generation = generation.saturating_add(1);
        lines.push(SuccessionLine {
            dynasty_id,
            outgoing_head_id: head_id,
            incoming_head_id: heir_id,
            new_heir_name: format!("{dynasty_name} Heir {next_generation}"),
            new_heir_birth_day: state.clock.day().saturating_sub(20 * 360),
            new_heir_capabilities: CharacterCapabilities {
                administration: 40_u16.saturating_add(
                    u16::try_from(state.rng.range_u32(50)).expect("random value fits u16"),
                ),
                commerce: 40_u16.saturating_add(
                    u16::try_from(state.rng.range_u32(50)).expect("random value fits u16"),
                ),
                social: 40_u16.saturating_add(
                    u16::try_from(state.rng.range_u32(50)).expect("random value fits u16"),
                ),
                craft: 30_u16.saturating_add(
                    u16::try_from(state.rng.range_u32(55)).expect("random value fits u16"),
                ),
            },
        });
    }

    lines
}

fn succession_chance_basis_points(
    age_years: i64,
    succession_risk_basis_points: u16,
    health_basis_points: u16,
) -> u16 {
    if age_years < 55 {
        return 0;
    }
    let age_pressure = (age_years - 50).saturating_mul(120);
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
            && link.kind == FamilyLinkKind::Marriage
            && (link.first_character_id == outgoing_head_id
                || link.second_character_id == outgoing_head_id)
    }) {
        link.active = false;
    }
}

fn update_institutions_for_succession(
    state: &mut AppState,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
) {
    for institution in state.institutions.values_mut() {
        if institution.members.remove(&outgoing_head_id) {
            institution.members.insert(incoming_head_id);
        }
        if institution.office_holder_id == Some(outgoing_head_id) {
            institution.office_holder_id = None;
            institution.next_selection_day = institution
                .next_selection_day
                .min(state.clock.day().saturating_add(30));
        }
    }
}

fn apply_successions(state: &mut AppState, lines: Vec<SuccessionLine>) {
    for line in lines {
        let SuccessionLine {
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            new_heir_name,
            new_heir_birth_day,
            new_heir_capabilities,
        } = line;
        retire_outgoing_head(state, outgoing_head_id);
        {
            let incoming = state
                .characters
                .get_mut(incoming_head_id)
                .expect("succession incoming head must exist");
            incoming.runtime.role = CharacterRole::HeadOfHouse;
            incoming.runtime.loyalty_basis_points = 10_000;
        }

        update_institutions_for_succession(state, outgoing_head_id, incoming_head_id);

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

        let new_heir_id = state.next_ids.character();
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

        let family_link_id = state.next_ids.family_link();
        state.family_links.insert(
            family_link_id,
            FamilyLink {
                id: family_link_id,
                first_character_id: incoming_head_id,
                second_character_id: new_heir_id,
                kind: FamilyLinkKind::ParentChild,
                active: true,
                property_claim_basis_points: 8_000,
            },
        );
        let council = state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("succession dynasty must have a family council");
        council.members.remove(&outgoing_head_id);
        council.members.insert(incoming_head_id);
        council.members.insert(new_heir_id);

        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("succession dynasty must exist");
        dynasty.relationships.head_id = incoming_head_id;
        dynasty.relationships.heir_id = Some(new_heir_id);
        dynasty.runtime.generation = dynasty.runtime.generation.saturating_add(1);
        dynasty.resources.legitimacy_basis_points = dynasty
            .resources
            .legitimacy_basis_points
            .saturating_sub(dynasty.runtime.succession_risk_basis_points / 4);
        let id = state.next_ids.chronicle();
        state.chronicle.push(ChronicleEntry {
            id,
            day: state.clock.day(),
            kind: ChronicleKind::Succession,
            summary: format!(
                "Dynasty {dynasty_id} passed from character {outgoing_head_id} to {incoming_head_id}."
            ),
        });
    }
}

#[cfg(test)]
#[path = "simulation_tests.rs"]
mod tests;
