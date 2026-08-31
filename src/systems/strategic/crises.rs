//! Crisis detection, escalation, response effects, route risk, guild standing.
//!
//! Purpose: own the monthly crisis detector and the daily crisis-effect
//! applier so route disruption, fiscal pressure, and district distress remain
//! causally coherent.
//! Owns: `CRISIS_RESPONSE_WINDOW_DAYS`, `apply_crisis_daily_effects`,
//! monthly detection/escalation, `capacity_weighted_route_disruption` reuse,
//! and response containment logic (paid responses never inflate severity).
//! Reads: `Registry` routes, `AppState` market/route/district state.
//! Mutates: `AppState.crises` severity/status, route disruption, audit.
//! Does not own: crisis-response command validation (`commands/response.rs`)
//! or household income scaling (reads the shared `capacity_weighted_...`).
//! Focused tests: `strategic_tests` crisis lifecycle, gameplay world-stress
//! aggregates.

#[allow(clippy::wildcard_imports)]
use super::*;

/// How long a player response counts as an ongoing containment effort. One
/// cheap response years ago must neither grant a crisis permanent immunity
/// against escalation nor permanently lock the house out of responding while
/// the underlying condition persists; once this window closes on a crisis
/// that is still active, organized responses are legitimate again.
pub(crate) const CRISIS_RESPONSE_WINDOW_DAYS: i64 = OFFICE_DUTY_FORFEITURE_WINDOW_DAYS;

pub(crate) fn apply_crisis_daily_effects(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let active: Vec<_> = state
        .crises
        .values()
        .filter(|crisis| crisis.status.is_active())
        .map(|crisis| {
            (
                crisis.kind,
                crisis.severity_basis_points,
                crisis.district_id,
            )
        })
        .collect();
    for (kind, severity, district_id) in active {
        match kind {
            CrisisKind::GrainShortage => {
                if let Some(bread_id) = registry.get_good_id("bread") {
                    let quote = state
                        .market
                        .quotes
                        .get_mut(&bread_id)
                        .expect("bread quote must exist");
                    let crisis_demand = quote
                        .target_stock
                        .saturating_mul_ratio(i64::from(severity), 100_000);
                    quote.demand_today = quote.demand_today.checked_add(crisis_demand).ok_or(
                        SimulationError::MarketDemandOverflow {
                            good_id: bread_id,
                            current: quote.demand_today,
                            incoming: crisis_demand,
                        },
                    )?;
                }
            }
            CrisisKind::UrbanFire => {
                if let Some(district_id) = district_id {
                    for property in state
                        .properties
                        .values_mut()
                        .filter(|property| property.district_id == district_id)
                    {
                        property.condition_basis_points = property
                            .condition_basis_points
                            .saturating_sub((severity / 200).max(1));
                    }
                }
            }
            CrisisKind::Epidemic => {
                apply_epidemic_household_pressure(
                    state,
                    district_id,
                    (severity / EPIDEMIC_DAILY_WELFARE_DIVISOR).max(1),
                );
            }
            CrisisKind::TradeDisruption => {
                // The crisis tracks disrupted routes rather than driving them:
                // elevated `disruption_basis_points` already throttles route
                // supply directly, and re-pinning routes to crisis severity
                // every day would make the underlying condition impossible to
                // heal, locking external trade permanently.
            }
            CrisisKind::GuildRevolt => {
                if let Some(district_id) = district_id {
                    // Unrest accumulates while the revolt runs; the monthly
                    // smoothing pass decays it after resolution.
                    if let Some(district) = state.districts.get_mut(&district_id) {
                        district.unrest_basis_points = district
                            .unrest_basis_points
                            .saturating_add((severity / 200).max(1))
                            .min(10_000);
                    }
                    // Employment is idempotent under the crisis rather than
                    // compounding: it holds the same crisis-adjusted level the
                    // monthly recompute derives, so a long revolt cannot grind
                    // stored employment to zero and then snap back at the
                    // month boundary.
                    let pressure = (severity / 100).max(1);
                    let employment = district_employment_basis_points(state, district_id)
                        .saturating_sub(pressure);
                    if let Some(district) = state.districts.get_mut(&district_id) {
                        district.employment_basis_points = employment;
                    }
                }
            }
            CrisisKind::BankingPanic => {
                apply_banking_panic_losses(state, severity)?;
            }
            CrisisKind::NobleDemand => {
                if let Some(treasury_id) = registry.get_institution_id("treasury")
                    && let Some(treasury) = state.institutions.get_mut(&treasury_id)
                {
                    // The levy is the one deliberate external money leak in the
                    // simulation: the payment leaves Rivergate for the prince's
                    // court, so no internal record receives the counterparty
                    // credit. Every other debit in the economy must have a
                    // credited counterparty.
                    let levy = Money::from_copper(i64::from(severity) / 20).min(treasury.budget);
                    treasury.budget = treasury
                        .budget
                        .checked_sub(levy)
                        .expect("bounded noble levy must not exceed civic treasury");
                }
                if let Some(district_id) = district_id
                    && let Some(district) = state.districts.get_mut(&district_id)
                {
                    district.unrest_basis_points = district
                        .unrest_basis_points
                        .saturating_add((severity / 500).max(1))
                        .min(10_000);
                }
            }
        }
    }
    Ok(())
}

/// Demand an active grain shortage injects into the bread quote as pure price
/// pressure: no household or business stands behind it and no money moves with
/// it. Wage and utilization math excludes this share, because pay must follow
/// work that real buyers fund, not scarcity panic.
pub(crate) fn crisis_phantom_demand(
    state: &AppState,
    registry: &Registry,
    good_id: GoodId,
) -> Quantity {
    if Some(good_id) != registry.get_good_id("bread") {
        return Quantity::ZERO;
    }
    let target_stock = state
        .market
        .quotes
        .get(&good_id)
        .map_or(Quantity::ZERO, |quote| quote.target_stock);
    state
        .crises
        .values()
        .filter(|crisis| crisis.kind == CrisisKind::GrainShortage && crisis.status.is_active())
        .fold(Quantity::ZERO, |total, crisis| {
            total.saturating_add(
                target_stock.saturating_mul_ratio(i64::from(crisis.severity_basis_points), 100_000),
            )
        })
}

pub(crate) fn apply_banking_panic_losses(
    state: &mut AppState,
    severity: u16,
) -> Result<(), SimulationError> {
    let mut total_loss = Money::ZERO;
    for business in state.businesses.iter_mut() {
        let loss = business
            .finance
            .cash
            .saturating_mul_ratio(i64::from(severity), 400_000);
        if loss > Money::ZERO {
            let resulting_cash = business
                .finance
                .cash
                .checked_sub(loss)
                .expect("banking-panic loss must not exceed business cash");
            let next_finance_version = next_business_finance_version(business)?;
            business.finance.cash = resulting_cash;
            business.finance.version = next_finance_version;
            total_loss = total_loss
                .checked_add(loss)
                .expect("total banking-panic loss must fit Money");
        }
    }
    // Households also hold deposits in the same banking system: a panic that
    // only hits business vaults while household savings sit untouched is
    // incoherent and makes the crisis wealth-destroying for one sector only.
    // Household exposure is half the business rate — personal savings are less
    // leveraged than commercial operating cash — but still material so a panic
    // tightens the whole city's liquidity, not just the workshop ledger.
    for household in state.households.iter_mut() {
        let loss = household
            .cash
            .saturating_mul_ratio(i64::from(severity), 800_000);
        if loss > Money::ZERO {
            household.cash = household
                .cash
                .checked_sub(loss)
                .expect("banking-panic household loss must not exceed cash");
            total_loss = total_loss
                .checked_add(loss)
                .expect("total banking-panic loss must fit Money");
        }
    }
    if total_loss > Money::ZERO {
        // Deposits flee to the pooled market sector rather than vanishing:
        // every business and household debit keeps a credited counterparty.
        // The loss is also deliberately kept out of `lifetime_costs`, which
        // measures operating history — a one-day panic must not permanently
        // brand a recovered house as structurally unprofitable for dividends
        // and reputation.
        credit_market_clearing_account(state, total_loss)?;
    }
    Ok(())
}

pub(crate) fn recover_external_routes(state: &mut AppState) {
    for route in state.external_routes.values_mut() {
        route.disruption_basis_points = route
            .disruption_basis_points
            .saturating_sub(ROUTE_DISRUPTION_HEALING_BASIS_POINTS);
    }
}

pub(crate) fn update_external_route_risk(state: &mut AppState) {
    for route in state.external_routes.values_mut() {
        // Route risk is an annual hazard expressed as a monthly spike chance.
        // Sustained bad luck must be able to accumulate into a trade
        // disruption crisis, so spikes are large relative to routine decay;
        // otherwise the TradeDisruption crisis could never occur in live play.
        if state.rng.is_chance_success(route.risk_basis_points) {
            let spike = u16::try_from(
                i32::from(ROUTE_DISRUPTION_SPIKE_MIN_BASIS_POINTS)
                    + i32::try_from(
                        state
                            .rng
                            .range_u32(ROUTE_DISRUPTION_SPIKE_RANGE_BASIS_POINTS),
                    )
                    .unwrap_or(0),
            )
            .unwrap_or(u16::MAX);
            route.disruption_basis_points = route
                .disruption_basis_points
                .saturating_add(spike)
                .min(9_500);
        } else {
            route.disruption_basis_points = route
                .disruption_basis_points
                .saturating_sub(ROUTE_DISRUPTION_CALM_RECOVERY_BASIS_POINTS);
        }
    }
}

pub(crate) fn detect_and_advance_crises(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    advance_existing_crises(registry, state)?;
    let has_grain_crisis = state
        .crises
        .values()
        .any(|crisis| crisis.kind == CrisisKind::GrainShortage && crisis.status.is_active());
    if !has_grain_crisis {
        // The staple chain starts at the granary: a blockade drains grain
        // long before bakers stop producing bread, so detection watches both
        // staples against their own target stock instead of an absolute
        // shelf floor that normal fluctuation can never approach.
        // At 65% the shortage declares while shelves thin rather than
        // after they have collapsed to 40%, leaving response routes
        // something to protect.
        let staple_thinning = ["grain", "bread"].iter().any(|good_key| {
            registry
                .get_good_id(good_key)
                .and_then(|id| state.market.get_quote(id))
                .is_some_and(|quote| {
                    quote.stock() < quote.target_stock.saturating_mul_ratio(6_500, 10_000)
                })
        });
        // Detection must precede empty shelves, or response routes have
        // nothing left to protect. A shortage is declared while the squeeze is
        // still building: regional access is failing or daily resupply has
        // fallen visibly behind consumption.
        let supply_stressed =
            crate::systems::simulation::import_trade_availability_basis_points(state) < 5_000;
        if staple_thinning && supply_stressed {
            insert_crisis(
                state,
                CrisisKind::GrainShortage,
                None,
                4_500,
                "Disrupted regional supply left city staple stores thinning against demand.",
            )?;
        }
    }
    let distressed_loans = state
        .loans
        .values()
        .filter(|loan| matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted))
        .count()
        .saturating_add(
            state
                .civic_debts
                .values()
                .filter(|debt| debt.status == CivicDebtStatus::Defaulted)
                .count(),
        );
    let recent_writeoffs = state
        .loans
        .values()
        .filter(|loan| {
            loan.status == LoanStatus::WrittenOff
                && day - loan.next_due_day <= BANKING_PANIC_MEMORY_DAYS
        })
        .count();
    let distressed_loans = distressed_loans.saturating_add(recent_writeoffs);
    let active_panic = state
        .crises
        .values()
        .any(|crisis| crisis.kind == CrisisKind::BankingPanic && crisis.status.is_active());
    let prior_panics = state
        .crises
        .values()
        .filter(|crisis| {
            crisis.kind == CrisisKind::BankingPanic
                // Only recent panics raise the bar for the next one: without a
                // lookback window the threshold ratchets forever because
                // resolved crises stay in state, and after a few events no
                // reachable default count could ever trigger detection again.
                && day - crisis.started_day <= BANKING_PANIC_MEMORY_DAYS
        })
        .count();
    let next_panic_threshold = prior_panics.saturating_add(2);
    if distressed_loans >= next_panic_threshold && !active_panic {
        insert_crisis(
            state,
            CrisisKind::BankingPanic,
            None,
            3_800,
            "Multiple defaults damaged confidence in city credit.",
        )?;
    } else if !active_panic
        && prior_panics == 0
        && day > 0
        && day % 180 == 0
        && state.loans.len() >= 4
        && state
            .businesses
            .iter()
            .filter(|b| b.status() == crate::core::BusinessStatus::Distressed)
            .count()
            >= 1
        && state.rng.is_chance_success(1_500)
    {
        insert_crisis(
            state,
            CrisisKind::BankingPanic,
            None,
            3_500,
            "Sustained business distress and strained credit sparked a banking panic.",
        )?;
    }
    detect_trade_disruption(state)?;
    if day > 0
        && day % NOBLE_DEMAND_CHECK_INTERVAL_DAYS == 0
        && !has_active_crisis(state, CrisisKind::NobleDemand)
        && state
            .rng
            .is_chance_success(NOBLE_DEMAND_CHANCE_BASIS_POINTS)
    {
        // The prince's extraordinary levy lands where the money is: the
        // district with the highest current rent index, ties broken by the
        // stable district order.
        let district_id = state
            .districts
            .values()
            .max_by_key(|district| (district.rent_index_basis_points, district.district_id))
            .map(|district| district.district_id);
        insert_crisis(
            state,
            CrisisKind::NobleDemand,
            district_id,
            3_000,
            "The regional prince demanded an extraordinary payment from the city.",
        )?;
    }
    detect_periodic_crises(registry, state, day)?;
    Ok(())
}

pub(crate) fn advance_existing_crises(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let mut resolved = Vec::new();
    let mut escalated = Vec::new();
    let day = state.clock.day();
    // Chronologically ordered history: reverse iteration stops at the window
    // boundary instead of sweeping the whole audit log for every crisis.
    let cutoff = day.saturating_sub(CRISIS_RESPONSE_WINDOW_DAYS);
    let addressed_subjects: BTreeSet<_> = state
        .audit_log
        .iter()
        .rev()
        .take_while(|record| record.day() >= cutoff)
        .filter(|record| crisis_response_contains_crisis(record))
        .map(|record| record.subject().to_owned())
        .collect();
    // A standing watch directive is an ongoing institutional response in its
    // district, so crises it is actively suppressing count as addressed even
    // without a player-issued response record.
    let watch_directive_districts: BTreeSet<DistrictId> = state
        .institutions
        .values()
        .filter_map(|institution| {
            let directive = institution.active_directive?;
            if directive.power != OfficePower::WatchPriorities || directive.expires_day < day {
                return None;
            }
            Some(
                registry
                    .get_institution(institution.institution_id)
                    .expect("institution runtime must remain registered")
                    .district_id(),
            )
        })
        .collect();
    let weighted_disruption = capacity_weighted_route_disruption(state);
    for crisis in state.crises.values_mut() {
        if !crisis.status.is_active() {
            continue;
        }
        let previous_status = crisis.status;
        let subject = format!("crisis:{}", crisis.id);
        // A trade disruption tracks the condition that spawned it instead of
        // escalating on its own: while any route remains above the detection
        // threshold the crisis holds at that disruption level, and once every
        // route has healed below it the crisis recovers month over month even
        // without a player response.
        let next_severity = if crisis.kind == CrisisKind::TradeDisruption {
            if weighted_disruption >= TRADE_DISRUPTION_ROUTE_DISRUPTION_THRESHOLD {
                crisis.severity_basis_points.max(weighted_disruption)
            } else {
                // Every route has healed below the detection threshold: the
                // tracked condition is gone, so the disruption ends with it
                // instead of decaying for seasons as a phantom active threat.
                0
            }
        } else if addressed_subjects.contains(&subject)
            || crisis
                .district_id
                .is_some_and(|district_id| watch_directive_districts.contains(&district_id))
        {
            crisis
                .severity_basis_points
                .saturating_sub(ADDRESSED_CRISIS_MONTHLY_RECOVERY_BASIS_POINTS)
        } else {
            crisis
                .severity_basis_points
                .saturating_add(UNADDRESSED_CRISIS_MONTHLY_ESCALATION_BASIS_POINTS)
                .min(10_000)
        };
        crisis.severity_basis_points = next_severity;
        crisis.status = CrisisStatus::from_severity(crisis.severity_basis_points);
        if crisis.status == CrisisStatus::Resolved {
            resolved.push((crisis.id, crisis.kind));
        } else if previous_status != CrisisStatus::Escalated
            && crisis.status == CrisisStatus::Escalated
        {
            escalated.push((crisis.id, crisis.kind));
        }
    }
    for (crisis_id, kind) in escalated {
        try_push_outbox(
            state,
            OutboxKind::Crisis,
            format!("Crisis {crisis_id} escalated"),
            format!(
                "The {kind:?} crisis intensified because no effective response had contained it."
            ),
        )?;
    }
    for (crisis_id, kind) in resolved {
        try_push_outbox(
            state,
            OutboxKind::Crisis,
            format!("Crisis {crisis_id} resolved"),
            format!("The {kind:?} crisis has subsided below an active threat level."),
        )?;
    }
    prune_expired_crisis_history(state);
    Ok(())
}

/// Resolved crises leave `state.crises` only long enough to remain visible
/// history; beyond the retention horizon they are dropped so the record map
/// (and every monthly scan over it) stays bounded on long campaigns. No other
/// record references crises, so pruning cannot desynchronize derived state.
pub(crate) fn prune_expired_crisis_history(state: &mut AppState) {
    let day = state.clock.day();
    state.crises.retain(|_, crisis| {
        crisis.status.is_active() || day - crisis.started_day <= CRISIS_HISTORY_RETENTION_DAYS
    });
}

/// The single canonical audit-detail encoding for a crisis response. Readers
/// must go through [`audit_record_crisis_response`] so the writer and the
/// idempotence guards cannot drift apart.
pub(crate) fn crisis_response_audit_detail(response: CrisisResponse) -> String {
    format!("response={response:?}")
}

pub(crate) fn audit_record_crisis_response(record: &AuditRecord) -> Option<CrisisResponse> {
    if record.kind() != AuditKind::CrisisResponse {
        return None;
    }
    match record.detail().strip_prefix("response=")? {
        "Relief" => Some(CrisisResponse::Relief),
        "Reform" => Some(CrisisResponse::Reform),
        "Suppress" => Some(CrisisResponse::Suppress),
        "Exploit" => Some(CrisisResponse::Exploit),
        _ => None,
    }
}

pub(crate) fn crisis_response_contains_crisis(record: &AuditRecord) -> bool {
    matches!(
        audit_record_crisis_response(record),
        Some(CrisisResponse::Relief | CrisisResponse::Reform | CrisisResponse::Suppress)
    )
}

pub(crate) fn has_active_crisis(state: &AppState, kind: CrisisKind) -> bool {
    state
        .crises
        .values()
        .any(|crisis| crisis.kind == kind && crisis.status.is_active())
}

pub(crate) fn detect_periodic_crises(
    registry: &Registry,
    state: &mut AppState,
    day: i64,
) -> Result<(), SimulationError> {
    if day <= 0 || day % 180 != 0 {
        return Ok(());
    }
    detect_urban_fire(state)?;
    detect_epidemic(state)?;
    detect_guild_revolt(registry, state)?;
    Ok(())
}

pub(crate) fn detect_urban_fire(state: &mut AppState) -> Result<(), SimulationError> {
    if has_active_crisis(state, CrisisKind::UrbanFire) {
        return Ok(());
    }
    let Some((district_id, safety)) = state
        .districts
        .iter()
        .min_by_key(|(_, district)| district.safety_basis_points)
        .map(|(id, district)| (*id, district.safety_basis_points))
    else {
        return Ok(());
    };
    let fire_code = active_law_value(state, LawKind::FireCode)
        .unwrap_or(0)
        .clamp(0, 10_000);
    let chance = urban_fire_probability_basis_points(safety, fire_code);
    if state.rng.is_chance_success(chance) {
        insert_crisis(
            state,
            CrisisKind::UrbanFire,
            Some(district_id),
            urban_fire_severity_basis_points(safety, fire_code),
            "Unsafe buildings and weak fire prevention allowed an urban fire to spread.",
        )?;
    }
    Ok(())
}

pub(crate) fn urban_fire_probability_basis_points(safety: u16, fire_code: i64) -> u16 {
    let deficiency = 10_000_u16.saturating_sub(safety);
    let chance = i64::from(deficiency)
        .saturating_div(4)
        .saturating_add(500)
        .saturating_sub(fire_code / 5)
        .clamp(0, 10_000);
    u16::try_from(chance).unwrap_or(0)
}

pub(crate) fn urban_fire_severity_basis_points(safety: u16, fire_code: i64) -> u16 {
    let deficiency = 10_000_u16.saturating_sub(safety);
    let severity = 4_000_i64
        .saturating_add(i64::from(deficiency) / 5)
        .saturating_sub(fire_code / 4)
        .clamp(1_000, 9_000);
    u16::try_from(severity).unwrap_or(9_000)
}

pub(crate) fn detect_epidemic(state: &mut AppState) -> Result<(), SimulationError> {
    if has_active_crisis(state, CrisisKind::Epidemic) {
        return Ok(());
    }
    let Some((district_id, sanitation)) = state
        .districts
        .iter()
        .min_by_key(|(_, district)| district.sanitation_basis_points)
        .map(|(id, district)| (*id, district.sanitation_basis_points))
    else {
        return Ok(());
    };
    let deficiency = 10_000_u16.saturating_sub(sanitation);
    let chance = deficiency.saturating_div(4).saturating_add(250).min(10_000);
    if state.rng.is_chance_success(chance) {
        let severity = 3_000_u16.saturating_add(deficiency / 5).min(9_000);
        insert_crisis(
            state,
            CrisisKind::Epidemic,
            Some(district_id),
            severity,
            "Poor sanitation allowed an epidemic to take hold.",
        )?;
        apply_epidemic_household_pressure(
            state,
            Some(district_id),
            (severity / EPIDEMIC_ONSET_WELFARE_DIVISOR).max(1),
        );
    }
    Ok(())
}

pub(crate) fn apply_epidemic_household_pressure(
    state: &mut AppState,
    district_id: Option<DistrictId>,
    welfare_loss: u16,
) {
    for household in state.households.iter_mut().filter(|household| {
        district_id.is_none_or(|district_id| household.district_id() == district_id)
    }) {
        household.food_satisfaction_basis_points = household
            .food_satisfaction_basis_points
            .saturating_sub(welfare_loss);
    }
}

pub(crate) use crate::systems::capacity_weighted_route_disruption;

pub(crate) fn detect_trade_disruption(state: &mut AppState) -> Result<(), SimulationError> {
    if has_active_crisis(state, CrisisKind::TradeDisruption) {
        return Ok(());
    }
    let disruption = capacity_weighted_route_disruption(state);
    if disruption >= TRADE_DISRUPTION_ROUTE_DISRUPTION_THRESHOLD {
        insert_crisis(
            state,
            CrisisKind::TradeDisruption,
            None,
            disruption,
            "External trade routes became too disrupted to sustain normal commerce.",
        )?;
    }
    Ok(())
}

pub(crate) fn detect_guild_revolt(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    if has_active_crisis(state, CrisisKind::GuildRevolt) {
        return Ok(());
    }
    let disputed_district = state.employment.values().find_map(|agreement| {
        (agreement.status == EmploymentStatus::Disputed)
            .then(|| state.businesses.get(agreement.business_id))
            .flatten()
            .map(crate::core::Business::district_id)
    });
    let disputed_count = state
        .employment
        .values()
        .filter(|agreement| agreement.status == EmploymentStatus::Disputed)
        .count();
    let restriction = active_law_value(state, LawKind::GuildEntryRestriction)
        .unwrap_or(0)
        .clamp(0, 10_000);
    let guild_deficit = chartered_guild_legitimacy_deficit(registry, state);
    let chance = guild_revolt_probability_basis_points(disputed_count, restriction, guild_deficit);
    if disputed_count >= 2 || (chance > 0 && state.rng.is_chance_success(chance)) {
        let district_id = disputed_district.or_else(|| {
            state
                .districts
                .iter()
                .max_by_key(|(_, district)| district.unrest_basis_points)
                .map(|(id, _)| *id)
        });
        insert_crisis(
            state,
            CrisisKind::GuildRevolt,
            district_id,
            2_500_u16
                .saturating_add(
                    u16::try_from(disputed_count)
                        .unwrap_or(u16::MAX)
                        .saturating_mul(500),
                )
                .min(9_000),
            "Labor disputes and restrictive guild rules triggered organized resistance.",
        )?;
    }
    Ok(())
}

/// Average legitimacy shortfall of the chartered guild institutions, in basis
/// points. Guilds whose members no longer trust or fund them cannot keep their
/// trades calm, so a legitimacy deficit feeds the revolt chance just like
/// labor disputes and entry restrictions do; endowments and office stewardship
/// that restore guild standing therefore suppress future revolts.
pub(crate) fn chartered_guild_legitimacy_deficit(registry: &Registry, state: &AppState) -> i64 {
    let guild_legitimacies: Vec<i64> = chartered_guild_ids(registry, state)
        .into_iter()
        .filter_map(|institution_id| state.institutions.get(&institution_id))
        .map(|institution| i64::from(institution.legitimacy_basis_points))
        .collect();
    if guild_legitimacies.is_empty() {
        return 0;
    }
    let total: i64 = guild_legitimacies.iter().sum();
    (10_000 - total / i64::try_from(guild_legitimacies.len()).unwrap_or(1)).max(0)
}

/// Runtime IDs of every chartered guild institution: the four craft guilds
/// and the merchant guild.
pub(crate) fn chartered_guild_ids(registry: &Registry, state: &AppState) -> Vec<InstitutionId> {
    use crate::registry::InstitutionKind;

    state
        .institutions
        .keys()
        .copied()
        .filter(|institution_id| {
            matches!(
                registry
                    .get_institution(*institution_id)
                    .map(crate::registry::InstitutionDef::kind),
                Some(InstitutionKind::CraftGuild | InstitutionKind::MerchantGuild)
            )
        })
        .collect()
}

/// A guild revolt is a crisis of guild standing, so how the dynasty answers
/// it changes how every chartered guild is seen. Relief and reform restore
/// trust in the charters; suppression and profiteering spend it. The shift
/// persists in institutional legitimacy and therefore feeds back into future
/// revolt pressure.
pub(crate) fn apply_guild_revolt_standing_response(
    registry: &Registry,
    state: &mut AppState,
    response: CrisisResponse,
) -> i32 {
    let delta: i32 = match response {
        CrisisResponse::Relief => 75,
        CrisisResponse::Reform => 150,
        CrisisResponse::Suppress => -200,
        CrisisResponse::Exploit => -250,
    };
    let magnitude = u16::try_from(delta.abs()).unwrap_or(u16::MAX);
    for institution_id in chartered_guild_ids(registry, state) {
        let Some(institution) = state.institutions.get_mut(&institution_id) else {
            continue;
        };
        institution.legitimacy_basis_points = if delta > 0 {
            institution
                .legitimacy_basis_points
                .saturating_add(magnitude)
                .min(10_000)
        } else {
            institution
                .legitimacy_basis_points
                .saturating_sub(magnitude)
        };
    }
    delta
}

pub(crate) fn guild_revolt_probability_basis_points(
    disputed_count: usize,
    restriction: i64,
    guild_deficit: i64,
) -> u16 {
    if disputed_count == 0 && restriction <= 0 && guild_deficit <= 0 {
        return 0;
    }
    let chance = 400_i64
        .saturating_add(restriction.clamp(0, 10_000) / 5)
        .saturating_add(guild_deficit.clamp(0, 10_000) / 8)
        .saturating_add(
            i64::try_from(disputed_count)
                .unwrap_or(i64::MAX)
                .saturating_mul(800),
        )
        .clamp(0, 10_000);
    u16::try_from(chance).unwrap_or(10_000)
}

pub(crate) fn insert_crisis(
    state: &mut AppState,
    kind: CrisisKind,
    district_id: Option<DistrictId>,
    severity_basis_points: u16,
    cause: &str,
) -> Result<crate::ids::CrisisId, SimulationError> {
    let id = state.next_ids.try_crisis()?;
    state.crises.insert(
        id,
        Crisis {
            id,
            kind,
            district_id,
            started_day: state.clock.day(),
            severity_basis_points,
            status: CrisisStatus::Emerging,
            cause: cause.to_owned(),
        },
    );
    try_push_outbox(
        state,
        OutboxKind::Crisis,
        format!("Crisis emerged: {kind:?}"),
        cause.to_owned(),
    )?;
    Ok(id)
}
