//! Weekly employment settlement, market wage fairness, and workforce disputes.
//!
//! Purpose: own the weekly wage settlement where business cash funds
//! household income, wage posture's standing commitment drives loyalty
//! toward dispute or recovery, and market bread-price discipline keeps
//! `ReferenceWage` fair.
//! Owns: `settle_employment`, `LaborEnvironment`,
//! `REFERENCE_WEEKLY_WORKER_WAGE_COPPER` and stingy/generous thresholds,
//! `business_labor_utilization_basis_points`, wage-pressure
//! auto-adjustment for non-player rivals.
//! Reads: `Registry` recipes, `AppState` businesses/employment/households/
//! market + `DailyCapacityScratch`.
//! Mutates: `AppState` employment (loyalty/conditions/status), business and
//! household cash, audit/outbox.
//! Does not own: business wage-policy commands — `commands/holdings.rs`.
//! Invariants: every employer retains a week of operating cover during
//! settlement; sub-fair wages erode loyalty toward dispute, generous wages
//! build a buffer; wage stall keeps disputed crews from reconciling.
//! Focused tests: `src/systems/strategic/strategic_tests.rs` employment and
//! labor-response behavior.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn settle_employment(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    // One capacity collection for the whole settlement pass: per-agreement
    // collection would rescan employment and contracts for every crew.
    let capacity_scratch = crate::systems::DailyCapacityScratch::collect(state);
    let agreements: Vec<_> = state
        .employment
        .values()
        .filter(|agreement| {
            matches!(
                agreement.status,
                EmploymentStatus::Active | EmploymentStatus::Disputed
            )
        })
        .map(|agreement| {
            (
                agreement.id,
                agreement.business_id,
                agreement.household_id,
                agreement.weekly_wage,
                agreement.status,
            )
        })
        .collect();
    for (id, business_id, household_id, wage, prior_status) in agreements {
        settle_employment_agreement(
            registry,
            state,
            &capacity_scratch,
            id,
            business_id,
            household_id,
            wage,
            prior_status,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LaborEnvironment {
    pub(crate) utilization: u16,
    pub(crate) business_condition: u16,
    pub(crate) maintenance: u16,
    /// Per-worker weekly wage relative to the market reference wage, in basis
    /// points. Drives slow loyalty and condition drift toward wage fairness.
    pub(crate) wage_ratio_basis_points: u16,
}

/// Weekly per-worker wage the labor market treats as fair at base staple
/// prices. It tracks the bread price so sustained food inflation or scarcity
/// turns yesterday's fair wage into a stingy one.
pub(crate) const REFERENCE_WEEKLY_WORKER_WAGE_COPPER: i64 = 35;
/// At or above this wage-to-reference ratio a workforce considers its pay
/// generous and builds loyalty that absorbs operating strain.
pub(crate) const WAGE_ADEQUACY_GENEROUS_BASIS_POINTS: u16 = 12_000;
/// Below this ratio a workforce considers its pay stingy and slowly withdraws
/// cooperation until wages recover or resistance organizes.
pub(crate) const WAGE_ADEQUACY_STINGY_BASIS_POINTS: u16 = 9_000;
pub(crate) const WAGE_ADEQUACY_MAX_LOYALTY_LOSS_PER_WEEK: u16 = 150;
pub(crate) const WAGE_ADEQUACY_MAX_CONDITION_LOSS_PER_WEEK: u16 = 50;
pub(crate) const WAGE_ADEQUACY_GENEROUS_LOYALTY_GAIN_PER_WEEK: u16 = 40;
pub(crate) const WAGE_ADEQUACY_GENEROUS_CONDITION_GAIN_PER_WEEK: u16 = 15;

pub(crate) fn market_reference_weekly_wage(registry: &Registry, state: &AppState) -> Option<Money> {
    let bread_id = registry.get_good_id("bread")?;
    let base_price = registry.get_good(bread_id)?.base_price();
    if base_price <= Money::ZERO {
        return None;
    }
    let quote_price = state.market.quotes.get(&bread_id)?.price;
    let ratio_basis_points = quote_price
        .copper()
        .saturating_mul(10_000)
        .checked_div(base_price.copper())?;
    let clamped = ratio_basis_points.clamp(6_000, 18_000);
    Some(Money::from_copper(
        REFERENCE_WEEKLY_WORKER_WAGE_COPPER
            .saturating_mul(clamped)
            .div_euclid(10_000),
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn settle_employment_agreement(
    registry: &Registry,
    state: &mut AppState,
    capacity_scratch: &crate::systems::DailyCapacityScratch,
    employment_id: EmploymentId,
    business_id: BusinessId,
    household_id: HouseholdId,
    wage: Money,
    prior_status: EmploymentStatus,
) -> Result<(), SimulationError> {
    let utilization_basis_points =
        business_labor_utilization_basis_points(registry, state, business_id, capacity_scratch);
    let wage_ratio_basis_points =
        market_reference_weekly_wage(registry, state).map_or(10_000, |reference| {
            let agreement = state
                .employment
                .get(&employment_id)
                .expect("employment must exist");
            let workers = i64::from(agreement.workers().max(1));
            let per_worker_copper = agreement.weekly_wage().copper().max(0) / workers;
            let reference_copper = reference.copper().max(1);
            u16::try_from(
                per_worker_copper
                    .saturating_mul(10_000)
                    .div_euclid(reference_copper),
            )
            .unwrap_or(u16::MAX)
        });
    let labor_environment = {
        let business = state
            .businesses
            .get(business_id)
            .expect("employment business must exist");
        LaborEnvironment {
            utilization: utilization_basis_points,
            business_condition: business.operations.condition_basis_points,
            maintenance: business.policy.maintenance_basis_points,
            wage_ratio_basis_points,
        }
    };
    let wage_due = wage.saturating_mul_ratio(i64::from(utilization_basis_points), 10_000);
    let paid = pay_employment_wage(registry, state, business_id, household_id, wage_due)?;
    let (recovered, became_disputed) = update_employment_after_payment(
        state,
        employment_id,
        prior_status,
        labor_environment,
        paid,
        wage_due,
    );
    emit_employment_outcome(state, business_id, recovered, became_disputed)?;
    if paid == wage_due && wage_due > Money::ZERO && prior_status == EmploymentStatus::Active {
        respond_to_market_wage_pressure(registry, state, employment_id, business_id)?;
    }
    Ok(())
}

/// Rival employers answer market wage pressure the way real competitors do:
/// when food inflation turns a standing wage unfair and the workforce is
/// souring, they raise pay toward the fair reference while their cash buffer
/// allows it. This keeps the city's labor economy adaptive instead of letting
/// ambient disputes silently erode district employment.
pub(crate) fn respond_to_market_wage_pressure(
    registry: &Registry,
    state: &mut AppState,
    employment_id: EmploymentId,
    business_id: BusinessId,
) -> Result<(), SimulationError> {
    if state
        .businesses
        .get(business_id)
        .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
    {
        // The player sets wage posture deliberately through
        // `SetBusinessWages`; the simulation never renegotiates for them.
        return Ok(());
    }
    let Some(reference) = market_reference_weekly_wage(registry, state) else {
        return Ok(());
    };
    let (current_per_worker, workers) = {
        let agreement = state
            .employment
            .get(&employment_id)
            .expect("employment must exist");
        if agreement.status != EmploymentStatus::Active || agreement.loyalty_basis_points >= 5_000 {
            return Ok(());
        }
        (
            agreement.weekly_wage.copper().max(0) / i64::from(agreement.workers.max(1)),
            i64::from(agreement.workers.max(1)),
        )
    };
    if current_per_worker.saturating_mul(10_000)
        >= i64::from(WAGE_ADEQUACY_STINGY_BASIS_POINTS).saturating_mul(reference.copper().max(1))
    {
        return Ok(());
    }
    let target_per_worker = reference
        .copper()
        .min(current_per_worker.saturating_add(current_per_worker / 10 + 1));
    let new_total = Money::from_copper(target_per_worker.saturating_mul(workers));
    let old_total = Money::from_copper(current_per_worker.saturating_mul(workers));
    let weekly_increase = new_total.saturating_sub(old_total);
    if weekly_increase <= Money::ZERO {
        return Ok(());
    }
    let next_finance_version = {
        let business = state
            .businesses
            .get(business_id)
            .expect("employment business must exist");
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("employment business recipe must exist");
        let payroll_reserve = recipe.daily_operating_cost().saturating_mul(7);
        let required_buffer = payroll_reserve.saturating_add(weekly_increase.saturating_mul(6));
        if business.cash() < required_buffer {
            return Ok(());
        }
        next_business_finance_version(business)?
    };
    let agreement = state
        .employment
        .get_mut(&employment_id)
        .expect("employment must exist");
    agreement.weekly_wage = new_total;
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("employment business must exist");
    business.finance.version = next_finance_version;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::LaborSettlement,
        subject: format!("business:{business_id}").into(),
        detail: format!(
            "market_wage_adjustment={}; per_worker={old_total}/{new_total}",
            weekly_increase.copper()
        )
        .into(),
    });
    Ok(())
}

pub(crate) fn pay_employment_wage(
    registry: &Registry,
    state: &mut AppState,
    business_id: BusinessId,
    household_id: HouseholdId,
    wage_due: Money,
) -> Result<Money, SimulationError> {
    let business = state
        .businesses
        .get(business_id)
        .expect("employment business must exist");
    let business_cash = business.cash();
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("employment business recipe must exist");
    // Wages settle weekly, so every employer retains one week of operating
    // funds — or its policy reserve, whichever protects more — instead of
    // letting payroll spend a healthy firm into distress. Distressed firms
    // ignore the policy minimum (matching purchase/maintenance/distress
    // logic) and keep only the operating week, so recovery payroll can
    // still be met while the firm stabilizes.
    let payroll_reserve = if business.status() == BusinessStatus::Distressed {
        recipe.daily_operating_cost().saturating_mul(7)
    } else {
        business
            .policy
            .minimum_cash_reserve
            .max(recipe.daily_operating_cost().saturating_mul(7))
    };
    let spendable = business_cash.saturating_sub(payroll_reserve);
    if wage_due <= Money::ZERO || spendable <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let household_cash = state
        .households
        .get(household_id)
        .expect("employment household must exist")
        .cash;
    let paid = wage_due.min(spendable);
    if paid <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    household_cash
        .checked_add(paid)
        .ok_or(SimulationError::HouseholdCashOverflow {
            household_id,
            current: household_cash,
            incoming: paid,
        })?;
    let (resulting_lifetime_costs, next_finance_version) = {
        let business = state
            .businesses
            .get(business_id)
            .expect("employment business must exist");
        (
            business.finance.lifetime_costs.checked_add(paid).ok_or(
                SimulationError::BusinessLifetimeCostsOverflow {
                    business_id,
                    current: business.finance.lifetime_costs,
                    incoming: paid,
                },
            )?,
            next_business_finance_version(business)?,
        )
    };
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("employment business must exist");
    business.finance.cash = business_cash
        .checked_sub(paid)
        .expect("bounded wage must fit business cash");
    business.finance.lifetime_costs = resulting_lifetime_costs;
    business.finance.version = next_finance_version;
    let household = state
        .households
        .get_mut(household_id)
        .expect("employment household must exist");
    household.cash = household
        .cash
        .checked_add(paid)
        .expect("bounded wage must fit household cash");
    Ok(paid)
}

pub(crate) fn update_employment_after_payment(
    state: &mut AppState,
    employment_id: EmploymentId,
    prior_status: EmploymentStatus,
    environment: LaborEnvironment,
    paid: Money,
    wage_due: Money,
) -> (bool, bool) {
    let agreement = state
        .employment
        .get_mut(&employment_id)
        .expect("employment must exist");
    if paid == wage_due && wage_due > Money::ZERO {
        return update_fully_paid_employment(agreement, prior_status, environment);
    }
    if wage_due <= Money::ZERO {
        // A week with no work due pays nothing and drifts nothing: an idle
        // firm owes no payroll, so payment accounting must not read a zero
        // paycheck as either reconciliation or a missed one.
        return (false, false);
    }
    let loyalty_loss = if prior_status == EmploymentStatus::Disputed {
        100
    } else {
        250
    };
    let condition_loss = if prior_status == EmploymentStatus::Disputed {
        50
    } else {
        100
    };
    agreement.loyalty_basis_points = agreement.loyalty_basis_points.saturating_sub(loyalty_loss);
    agreement.conditions_basis_points = agreement
        .conditions_basis_points
        .saturating_sub(condition_loss);
    let became_disputed =
        prior_status == EmploymentStatus::Active && agreement.loyalty_basis_points < 2_000;
    if became_disputed {
        agreement.status = EmploymentStatus::Disputed;
    }
    (false, became_disputed)
}

pub(crate) fn update_fully_paid_employment(
    agreement: &mut EmploymentAgreement,
    prior_status: EmploymentStatus,
    environment: LaborEnvironment,
) -> (bool, bool) {
    if prior_status != EmploymentStatus::Disputed {
        let wage_dispute =
            apply_wage_adequacy_drift(agreement, environment.wage_ratio_basis_points);
        if wage_dispute {
            return (false, true);
        }
        let strain = labor_strain_basis_points(agreement, environment);
        if strain > 0 {
            agreement.conditions_basis_points =
                agreement.conditions_basis_points.saturating_sub(strain);
            agreement.loyalty_basis_points = agreement
                .loyalty_basis_points
                .saturating_sub(strain.saturating_div(2));
            let became_disputed =
                agreement.conditions_basis_points < 3_000 || agreement.loyalty_basis_points < 2_000;
            if became_disputed {
                agreement.status = EmploymentStatus::Disputed;
            }
            return (false, became_disputed);
        }
        if environment.utilization == 10_000 {
            agreement.loyalty_basis_points = agreement
                .loyalty_basis_points
                .saturating_add(30)
                .min(10_000);
            agreement.conditions_basis_points = agreement
                .conditions_basis_points
                .saturating_add(10)
                .min(10_000);
        }
        return (false, false);
    }
    // Disputed workforces reconcile toward fair pay. Stingy wages stall the
    // recovery entirely — the same under-reference pay that erodes an active
    // workforce's loyalty cannot buy goodwill from a disputing one, so the
    // dispute holds until wages become adequate again.
    let wage_ratio = environment.wage_ratio_basis_points;
    if wage_ratio < WAGE_ADEQUACY_STINGY_BASIS_POINTS {
        return (false, false);
    }
    let (loyalty_gain, condition_gain) = (180, 60);
    agreement.loyalty_basis_points = agreement
        .loyalty_basis_points
        .saturating_add(loyalty_gain)
        .min(10_000);
    agreement.conditions_basis_points = agreement
        .conditions_basis_points
        .saturating_add(condition_gain)
        .min(10_000);
    let recovered = agreement.loyalty_basis_points
        >= crate::systems::EMPLOYMENT_RECOVERY_BASIS_POINTS
        && agreement.conditions_basis_points >= crate::systems::EMPLOYMENT_RECOVERY_BASIS_POINTS;
    if recovered {
        agreement.status = EmploymentStatus::Active;
    }
    (recovered, false)
}

/// Weekly loyalty and condition drift from wage fairness. Stingy wages erode
/// cooperation and can eventually provoke a dispute on their own; generous
/// wages build the loyal buffer that absorbs periods of operating strain.
/// Returns whether the drift alone pushed an active workforce into dispute.
pub(crate) fn apply_wage_adequacy_drift(
    agreement: &mut EmploymentAgreement,
    wage_ratio_basis_points: u16,
) -> bool {
    if wage_ratio_basis_points >= WAGE_ADEQUACY_GENEROUS_BASIS_POINTS {
        agreement.loyalty_basis_points = agreement
            .loyalty_basis_points
            .saturating_add(WAGE_ADEQUACY_GENEROUS_LOYALTY_GAIN_PER_WEEK)
            .min(10_000);
        agreement.conditions_basis_points = agreement
            .conditions_basis_points
            .saturating_add(WAGE_ADEQUACY_GENEROUS_CONDITION_GAIN_PER_WEEK)
            .min(10_000);
        return false;
    }
    if wage_ratio_basis_points >= WAGE_ADEQUACY_STINGY_BASIS_POINTS {
        return false;
    }
    let deficit = WAGE_ADEQUACY_STINGY_BASIS_POINTS
        .saturating_sub(wage_ratio_basis_points)
        .max(1);
    agreement.loyalty_basis_points = agreement.loyalty_basis_points.saturating_sub(
        deficit
            .saturating_div(25)
            .min(WAGE_ADEQUACY_MAX_LOYALTY_LOSS_PER_WEEK),
    );
    agreement.conditions_basis_points = agreement.conditions_basis_points.saturating_sub(
        deficit
            .saturating_div(75)
            .min(WAGE_ADEQUACY_MAX_CONDITION_LOSS_PER_WEEK),
    );
    let became_disputed = agreement.status == EmploymentStatus::Active
        && (agreement.conditions_basis_points < 3_000 || agreement.loyalty_basis_points < 2_000);
    if became_disputed {
        agreement.status = EmploymentStatus::Disputed;
    }
    became_disputed
}

pub(crate) fn labor_strain_basis_points(
    agreement: &EmploymentAgreement,
    environment: LaborEnvironment,
) -> u16 {
    if environment.utilization < 9_000 {
        return 0;
    }
    let maintenance_strain = 1_000_u16
        .saturating_sub(environment.maintenance)
        .saturating_div(5);
    let condition_strain = 7_000_u16
        .saturating_sub(environment.business_condition)
        .saturating_div(20);
    let raw_strain = maintenance_strain.saturating_add(condition_strain).min(180);

    // A workforce with accumulated loyalty and decent conditions can absorb ordinary periods of
    // high utilization without turning every growth policy into a predictable dispute timer.
    // Extreme under-maintenance still erodes that buffer and eventually creates resistance, while
    // missed payroll continues to bypass this path and directly damages the relationship.
    let social_resilience = 68_u16.saturating_add(
        agreement
            .loyalty_basis_points
            .min(agreement.conditions_basis_points)
            .saturating_div(200),
    );
    raw_strain.saturating_sub(social_resilience)
}

pub(crate) fn emit_employment_outcome(
    state: &mut AppState,
    business_id: BusinessId,
    recovered: bool,
    became_disputed: bool,
) -> Result<(), SimulationError> {
    if recovered {
        try_push_outbox(
            state,
            OutboxKind::District,
            format!("Labor dispute at business {business_id} settled"),
            "Sustained full wage payments restored a workable labor agreement.".to_owned(),
        )?;
    }
    if became_disputed {
        try_push_outbox(
            state,
            OutboxKind::District,
            format!("Labor dispute at business {business_id}"),
            "Accumulated wage, workload, or workplace-condition pressure caused organized resistance."
                .to_owned(),
        )?;
    }
    Ok(())
}

pub(crate) fn business_labor_utilization_basis_points(
    registry: &Registry,
    state: &AppState,
    business_id: BusinessId,
    capacity_scratch: &crate::systems::DailyCapacityScratch,
) -> u16 {
    // Retainer keeps a skeleton crew paid during idle weeks so a healthy firm
    // retains workers through short troughs. At 4% it covers a minimal
    // standby wage without paying a tenth of payroll to an entirely idle
    // workshop — an idle active firm should idle cheaply, not bleed like
    // it were fully producing.
    const RETAINER_BASIS_POINTS: i64 = 400;
    let business = state
        .businesses
        .get(business_id)
        .expect("employment business must exist");
    if matches!(
        business.status(),
        BusinessStatus::Closed | BusinessStatus::Insolvent
    ) {
        return 0;
    }
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("employment business recipe must exist");
    let output_good_id = recipe.output_good_id();
    let output_per_batch = recipe.output_quantity().milliunits();
    if output_per_batch <= 0 {
        return 0;
    }
    let reserve_shortfall =
        crate::systems::business_policy_reserve(business, recipe.output_quantity())
            .saturating_sub(business.inventory_quantity(output_good_id))
            .max(Quantity::ZERO);
    let contract_reserve = capacity_scratch.business_contract_reserve(business_id, output_good_id);
    // Crisis demand is pure price pressure: no buyer stands behind it and no
    // money moves with it. Wages follow work that real demand funds, so the
    // phantom share is excluded from the weekly demand estimate.
    let phantom_daily_demand =
        crisis_phantom_demand(state, registry, output_good_id).saturating_mul_ratio(7, 1);
    let weekly_market_demand = state
        .market
        .quotes
        .get(&output_good_id)
        .map_or(Quantity::ZERO, |quote| {
            quote.demand_today.saturating_mul_ratio(7, 1)
        })
        .saturating_sub(phantom_daily_demand);
    let required_output = reserve_shortfall
        .saturating_add(contract_reserve)
        .saturating_add(weekly_market_demand);
    let required_batches =
        crate::money::ceil_div_nonnegative(required_output.milliunits(), output_per_batch);
    let weekly_capacity_batches =
        i64::from(business.operations.capacity_batches_per_day).saturating_mul(7);
    if weekly_capacity_batches <= 0 {
        return 0;
    }
    // Wages follow the work the present workforce can actually run: a firm
    // whose counted workers cannot assemble one batch produces nothing and
    // pays nothing, and a partially staffed firm pays proportionally.
    let workforce_coverage = (i64::from(capacity_scratch.worker_limited_batches(business_id))
        .saturating_mul(10_000)
        / i64::from(business.operations.capacity_batches_per_day))
    .min(10_000);
    if workforce_coverage == 0 {
        return 0;
    }
    let utilization_numerator = required_batches.saturating_mul(10_000);
    let utilization =
        crate::money::ceil_div_nonnegative(utilization_numerator, weekly_capacity_batches)
            .clamp(RETAINER_BASIS_POINTS, 10_000)
            .saturating_mul(workforce_coverage)
            / 10_000;
    u16::try_from(utilization).expect("clamped utilization must fit u16")
}
