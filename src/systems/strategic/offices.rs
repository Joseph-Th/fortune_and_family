//! Political-office lifecycle: duties, stipends, powers, directives, and elections.
//!
//! Purpose: own the monthly political layer — office term timing, duty
//! funding from holder treasury into institutional budget, stipend repayment,
//! power application (licenses, tolls, inspections etc.), directive
//! lifecycle, and deterministic elections with coalition backlash.
//! Owns: `apply_office_duties`, `apply_office_stipends`,
//! `apply_active_office_directives`, `resolve_institution_selections`,
//! election scoring / forfeiture on repeated duty shortfall.
//! Reads: `Registry` institutions, `AppState` institutions/characters/
//! dynasties/relationships.
//! Mutates: `AppState` institutions, dynasties (treasury/legitimacy),
//! relationships, audit/outbox.
//! Does not own: command-side nomination/directives — `commands/politics.rs`.
//! Invariants: every term advances `term_number`/`next_selection_day`;
//! monthly fee is repaid from institutional budget; administrative load
//! scales with power count × office count.
//! Focused tests: `src/systems/strategic/strategic_tests.rs` office and
//! institution selection.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn apply_active_office_directives(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let directives: Vec<_> = state
        .institutions
        .values_mut()
        .filter_map(|institution| {
            let directive = institution.active_directive?;
            if day > directive.expires_day {
                institution.active_directive = None;
                return None;
            }
            Some((
                institution.institution_id,
                directive.power,
                institution.office_holder_id,
            ))
        })
        .collect();
    for (institution_id, power, office_holder_id) in directives {
        let district_id = registry
            .get_institution(institution_id)
            .expect("active office directive institution must remain registered")
            .district_id();
        let holder_dynasty_id = office_holder_id
            .and_then(|character_id| state.characters.get(character_id))
            .map(crate::core::Character::dynasty_id);
        apply_office_directive_momentum(
            registry,
            state,
            institution_id,
            district_id,
            power,
            holder_dynasty_id,
        )?;
    }
    Ok(())
}

pub(crate) fn apply_office_directive_momentum(
    registry: &Registry,
    state: &mut AppState,
    institution_id: InstitutionId,
    district_id: DistrictId,
    power: OfficePower,
    holder_dynasty_id: Option<DynastyId>,
) -> Result<(), SimulationError> {
    match power {
        OfficePower::Licenses => adjust_directive_businesses(state, district_id, 10, 10),
        OfficePower::Inspections => adjust_directive_businesses(state, district_id, 15, 25),
        OfficePower::MarketTolls => adjust_directive_household_welfare(state, district_id, -15),
        OfficePower::DebtEnforcement => {
            // Enforcement breeds respect and fear toward whichever house wields
            // the office, not toward an unrelated bystander house.
            let Some(holder_dynasty_id) = holder_dynasty_id else {
                return Ok(());
            };
            for (pair, relationship) in &mut state.relationships {
                if pair.first == holder_dynasty_id || pair.second == holder_dynasty_id {
                    relationship.respect_basis_points = relationship
                        .respect_basis_points
                        .saturating_add(10)
                        .min(10_000);
                    relationship.fear_basis_points =
                        relationship.fear_basis_points.saturating_add(5).min(10_000);
                }
            }
        }
        OfficePower::CityContracts => adjust_directive_businesses(state, district_id, 20, 10),
        OfficePower::PublicWorks => adjust_directive_businesses(state, district_id, 20, 5),
        OfficePower::WatchPriorities => {
            adjust_directive_household_welfare(state, district_id, 10);
            for crisis in state.crises.values_mut().filter(|crisis| {
                crisis.district_id == Some(district_id) && crisis.status.is_active()
            }) {
                crisis.severity_basis_points = crisis.severity_basis_points.saturating_sub(60);
            }
        }
        OfficePower::Taxation => adjust_directive_household_welfare(state, district_id, -20),
        OfficePower::EmergencyImports => {
            adjust_directive_household_welfare(state, district_id, 50);
            if let Some(grain_id) = registry.get_good_id("grain") {
                add_market_supply(state, grain_id, Quantity::from_units(5))?;
            }
        }
    }
    if matches!(power, OfficePower::MarketTolls | OfficePower::Taxation)
        && let Some(institution) = state.institutions.get_mut(&institution_id)
    {
        institution.legitimacy_basis_points = institution
            .legitimacy_basis_points
            .saturating_add(10)
            .min(10_000);
    }
    Ok(())
}

pub(crate) fn adjust_directive_businesses(
    state: &mut AppState,
    district_id: DistrictId,
    condition: u16,
    quality: u16,
) {
    for business in state.businesses.iter_mut().filter(|business| {
        business.district_id() == district_id
            && matches!(
                business.status(),
                BusinessStatus::Active | BusinessStatus::Distressed
            )
    }) {
        business.operations.condition_basis_points = business
            .operations
            .condition_basis_points
            .saturating_add(condition)
            .min(10_000);
        business.operations.quality_basis_points = business
            .operations
            .quality_basis_points
            .saturating_add(quality)
            .min(10_000);
    }
}

pub(crate) fn adjust_directive_household_welfare(
    state: &mut AppState,
    district_id: DistrictId,
    delta: i16,
) {
    for household in state
        .households
        .iter_mut()
        .filter(|household| household.district_id() == district_id)
    {
        household.food_satisfaction_basis_points = if delta >= 0 {
            household
                .food_satisfaction_basis_points
                .saturating_add(delta.unsigned_abs())
                .min(10_000)
        } else {
            household
                .food_satisfaction_basis_points
                .saturating_sub(delta.unsigned_abs())
        };
    }
}

pub(crate) fn dynasty_office_administrative_load(state: &AppState, dynasty_id: DynastyId) -> u16 {
    state
        .institutions
        .values()
        .filter(|institution| {
            institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == dynasty_id)
            })
        })
        .fold(0_u16, |load, institution| {
            let power_count = u16::try_from(institution.powers.len()).unwrap_or(u16::MAX);
            load.saturating_add(power_count.saturating_mul(OFFICE_ADMINISTRATIVE_LOAD_PER_POWER))
        })
}

/// Office administrative load for every dynasty in one pass over the
/// institutions, so per-business daily planning resolves each holder once
/// instead of rescanning every institution per business.
pub(crate) fn dynasty_office_administrative_loads(state: &AppState) -> BTreeMap<DynastyId, u16> {
    let mut loads: BTreeMap<DynastyId, u16> = BTreeMap::new();
    for institution in state.institutions.values() {
        let Some(holder_id) = institution.office_holder_id else {
            continue;
        };
        let Some(holder) = state.characters.get(holder_id) else {
            continue;
        };
        let power_count = u16::try_from(institution.powers.len()).unwrap_or(u16::MAX);
        let entry = loads.entry(holder.dynasty_id()).or_default();
        *entry =
            entry.saturating_add(power_count.saturating_mul(OFFICE_ADMINISTRATIVE_LOAD_PER_POWER));
    }
    loads
}

pub(crate) fn office_duty_required(power_count: usize, office_count: usize) -> Money {
    if power_count == 0 || office_count == 0 {
        return Money::ZERO;
    }
    let power_count = i64::try_from(power_count).unwrap_or(i64::MAX);
    let additional_offices = i64::try_from(office_count.saturating_sub(1)).unwrap_or(i64::MAX);
    OFFICE_DUTY_COST_PER_POWER
        .saturating_mul(power_count)
        .saturating_add(
            OFFICE_DUTY_PORTFOLIO_SURCHARGE_PER_ADDITIONAL_OFFICE
                .saturating_mul(additional_offices),
        )
}

pub(crate) fn projected_dynasty_monthly_office_duty(
    state: &AppState,
    dynasty_id: DynastyId,
    additional_office_power_count: usize,
) -> Money {
    let additional_offices = (additional_office_power_count > 0)
        .then_some(additional_office_power_count)
        .into_iter()
        .collect::<Vec<_>>();
    projected_dynasty_monthly_office_duty_with_additional_offices(
        state,
        dynasty_id,
        &additional_offices,
    )
}

pub(crate) fn projected_dynasty_monthly_office_duty_with_additional_offices(
    state: &AppState,
    dynasty_id: DynastyId,
    additional_office_power_counts: &[usize],
) -> Money {
    let held_power_counts: Vec<_> = state
        .institutions
        .values()
        .filter(|institution| {
            institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == dynasty_id)
            })
        })
        .map(|institution| institution.powers.len())
        .collect();
    let office_count = held_power_counts
        .len()
        .saturating_add(additional_office_power_counts.len());
    held_power_counts
        .into_iter()
        .chain(additional_office_power_counts.iter().copied())
        .fold(Money::ZERO, |total, power_count| {
            total.saturating_add(office_duty_required(power_count, office_count))
        })
}

#[derive(Clone, Copy)]
pub(crate) struct OfficeDutyPlan {
    institution_id: InstitutionId,
    dynasty_id: DynastyId,
    power_count: usize,
    office_count: usize,
}

pub(crate) fn apply_office_duties(state: &mut AppState) -> Result<(), SimulationError> {
    let office_counts = state
        .institutions
        .values()
        .filter_map(|institution| {
            let holder_id = institution.office_holder_id?;
            state
                .characters
                .get(holder_id)
                .map(crate::core::Character::dynasty_id)
        })
        .fold(
            BTreeMap::<DynastyId, usize>::new(),
            |mut counts, dynasty_id| {
                *counts.entry(dynasty_id).or_default() += 1;
                counts
            },
        );
    let duties: Vec<_> = active_officeholders(state)
        .into_iter()
        .map(|(institution_id, dynasty_id, power_count)| {
            let office_count = office_counts.get(&dynasty_id).copied().unwrap_or(1);
            OfficeDutyPlan {
                institution_id,
                dynasty_id,
                power_count,
                office_count,
            }
        })
        .collect();
    preflight_office_duty_contributions(state, &duties)?;
    for duty in duties {
        apply_office_duty(
            state,
            duty.institution_id,
            duty.dynasty_id,
            duty.power_count,
            duty.office_count,
        )?;
    }
    Ok(())
}

/// Sitting officeholders in stable institution order as
/// `(institution_id, holder dynasty, power count)`. Institutions whose holder
/// does not resolve to a character are skipped.
pub(crate) fn active_officeholders(state: &AppState) -> Vec<(InstitutionId, DynastyId, usize)> {
    state
        .institutions
        .values()
        .filter_map(|institution| {
            let holder_id = institution.office_holder_id?;
            let dynasty_id = state.characters.get(holder_id)?.dynasty_id();
            Some((
                institution.institution_id,
                dynasty_id,
                institution.powers.len(),
            ))
        })
        .collect()
}

pub(crate) fn apply_office_stipends(state: &mut AppState) -> Result<(), SimulationError> {
    let stipends: Vec<_> = active_officeholders(state)
        .into_iter()
        .map(|(institution_id, dynasty_id, power_count)| {
            (
                institution_id,
                dynasty_id,
                OFFICE_STIPEND_PER_POWER
                    .saturating_mul(i64::try_from(power_count).unwrap_or(i64::MAX)),
            )
        })
        .collect();
    for (institution_id, dynasty_id, stipend) in stipends {
        let paid_stipend = {
            let institution = state
                .institutions
                .get_mut(&institution_id)
                .expect("stipend institution must exist");
            let paid = stipend.min(institution.budget);
            if paid == Money::ZERO {
                continue;
            }
            // The stipend is clamped to the budget above, so the withdrawal is
            // always representable.
            institution.budget = institution
                .budget
                .checked_sub(paid)
                .expect("clamped office stipend must not exceed the institutional budget");
            paid
        };
        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("officeholder dynasty must exist");
        let treasury = dynasty.treasury();
        let next_treasury =
            treasury
                .checked_add(paid_stipend)
                .ok_or(SimulationError::DynastyTreasuryOverflow {
                    dynasty_id,
                    current: treasury,
                    incoming: paid_stipend,
                })?;
        dynasty.resources.treasury = next_treasury;
    }
    Ok(())
}

pub(crate) fn preflight_office_duty_contributions(
    state: &AppState,
    duties: &[OfficeDutyPlan],
) -> Result<(), SimulationError> {
    let mut projected_treasuries = BTreeMap::new();
    let mut projected_institution_budgets = BTreeMap::new();
    let mut projected_contributions = BTreeMap::new();
    for duty in duties {
        let required = office_duty_required(duty.power_count, duty.office_count);
        let treasury = projected_treasuries
            .entry(duty.dynasty_id)
            .or_insert_with(|| {
                state
                    .dynasties
                    .get(&duty.dynasty_id)
                    .expect("officeholder dynasty must exist")
                    .treasury()
            });
        let paid = required.min(*treasury);
        *treasury = treasury
            .checked_sub(paid)
            .expect("projected office-duty payment must not exceed treasury");
        if paid == Money::ZERO {
            continue;
        }
        let institution_budget = projected_institution_budgets
            .entry(duty.institution_id)
            .or_insert_with(|| {
                state
                    .institutions
                    .get(&duty.institution_id)
                    .expect("office institution must exist")
                    .budget
            });
        *institution_budget = institution_budget.checked_add(paid).ok_or(
            SimulationError::InstitutionBudgetOverflow {
                institution_id: duty.institution_id,
                current: *institution_budget,
                incoming: paid,
            },
        )?;
        let contributions = projected_contributions
            .entry(duty.dynasty_id)
            .or_insert_with(|| {
                state
                    .dynasties
                    .get(&duty.dynasty_id)
                    .expect("officeholder dynasty must exist")
                    .resources
                    .civic_contributions
            });
        *contributions = contributions.checked_add(paid).ok_or(
            SimulationError::DynastyCivicContributionsOverflow {
                dynasty_id: duty.dynasty_id,
                current: *contributions,
                incoming: paid,
            },
        )?;
    }
    Ok(())
}

pub(crate) fn apply_office_duty(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    power_count: usize,
    office_count: usize,
) -> Result<(), SimulationError> {
    let required = office_duty_required(power_count, office_count);
    let institution_budget = state
        .institutions
        .get(&institution_id)
        .expect("office institution must exist")
        .budget;
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .expect("officeholder dynasty must exist")
        .treasury();
    let paid = required.min(treasury);
    transfer_office_duty_payment(
        state,
        institution_id,
        dynasty_id,
        institution_budget,
        treasury,
        paid,
    )?;
    if paid < required {
        record_office_duty_shortfall(
            state,
            institution_id,
            dynasty_id,
            required,
            paid,
            required.saturating_sub(paid),
        )?;
    }
    Ok(())
}

pub(crate) fn transfer_office_duty_payment(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    institution_budget: Money,
    treasury: Money,
    paid: Money,
) -> Result<(), SimulationError> {
    if paid == Money::ZERO {
        return Ok(());
    }
    let current_contributions = state
        .dynasties
        .get(&dynasty_id)
        .expect("officeholder dynasty must exist")
        .resources
        .civic_contributions;
    let next_contributions = current_contributions.checked_add(paid).ok_or(
        SimulationError::DynastyCivicContributionsOverflow {
            dynasty_id,
            current: current_contributions,
            incoming: paid,
        },
    )?;
    let next_institution_budget =
        institution_budget
            .checked_add(paid)
            .ok_or(SimulationError::InstitutionBudgetOverflow {
                institution_id,
                current: institution_budget,
                incoming: paid,
            })?;
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("officeholder dynasty must exist");
    dynasty.resources.treasury = treasury
        .checked_sub(paid)
        .expect("validated office-duty payment must not exceed dynasty treasury");
    dynasty.resources.civic_contributions = next_contributions;
    state
        .institutions
        .get_mut(&institution_id)
        .expect("office institution must exist")
        .budget = next_institution_budget;
    Ok(())
}

pub(crate) fn record_office_duty_shortfall(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    required: Money,
    paid: Money,
    shortfall: Money,
) -> Result<(), SimulationError> {
    let subject = office_duty_subject(institution_id, dynasty_id);
    let recent_shortfalls = recent_office_duty_shortfalls(state, &subject);
    let should_notify = should_notify_office_duty_shortfall(state, &subject);
    penalize_office_duty_shortfall(state, institution_id, dynasty_id);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::OfficeDutyShortfall,
        subject: subject.clone().into(),
        detail: format!("required={required};paid={paid};shortfall={shortfall}").into(),
    });
    let forfeited = recent_shortfalls.saturating_add(1) >= OFFICE_DUTY_FORFEITURE_THRESHOLD;
    if forfeited {
        forfeit_office_for_unmet_duties(
            state,
            institution_id,
            &subject,
            recent_shortfalls.saturating_add(1),
        )?;
    }
    notify_player_office_duty_outcome(
        state,
        OfficeDutyOutcome {
            institution_id,
            dynasty_id,
            required,
            paid,
            shortfall,
            forfeited,
            should_notify,
        },
    )?;
    Ok(())
}

pub(crate) fn recent_office_duty_shortfalls(state: &AppState, subject: &str) -> usize {
    // Audit-record days are chronologically nondecreasing (an enforced
    // invariant), so reverse iteration can stop as soon as records fall
    // outside the forfeiture window instead of sweeping the entire history.
    let cutoff = state
        .clock
        .day()
        .saturating_sub(OFFICE_DUTY_FORFEITURE_WINDOW_DAYS);
    let mut count = 0_usize;
    for record in state.audit_log.iter().rev() {
        if record.day() < cutoff {
            break;
        }
        if record.kind() == AuditKind::OfficeDutyShortfall && record.subject() == subject {
            count += 1;
        }
    }
    count
}

pub(crate) fn should_notify_office_duty_shortfall(state: &AppState, subject: &str) -> bool {
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::OfficeDutyShortfall && record.subject() == subject
        })
        .is_none_or(|record| {
            checked_future_day(record.day(), OFFICE_DUTY_FAILURE_NOTIFICATION_INTERVAL_DAYS)
                .is_ok_and(|next_notification_day| state.clock.day() >= next_notification_day)
        })
}

pub(crate) fn penalize_office_duty_shortfall(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
) {
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("officeholder dynasty must exist");
    dynasty.resources.unmet_office_duties = dynasty.resources.unmet_office_duties.saturating_add(1);
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_sub(120);
    dynasty.resources.reputation_reliability_basis_points = dynasty
        .resources
        .reputation_reliability_basis_points
        .saturating_sub(80);
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("office institution must exist");
    institution.legitimacy_basis_points = institution.legitimacy_basis_points.saturating_sub(100);
}

pub(crate) fn forfeit_office_for_unmet_duties(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    subject: &str,
    recent_shortfalls: usize,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let next_selection_day = checked_future_day(day, 30)?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("office institution must exist");
    institution.office_holder_id = None;
    // A forfeited office leaves no ghost administration: its active directive
    // ends with the officeholder instead of continuing without a holder.
    institution.active_directive = None;
    institution.next_selection_day = next_selection_day;
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::OfficeDutyForfeiture,
        subject: subject.into(),
        detail: format!("office forfeited after {recent_shortfalls} recent duty shortfalls").into(),
    });
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct OfficeDutyOutcome {
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    required: Money,
    paid: Money,
    shortfall: Money,
    forfeited: bool,
    should_notify: bool,
}

pub(crate) fn notify_player_office_duty_outcome(
    state: &mut AppState,
    outcome: OfficeDutyOutcome,
) -> Result<(), SimulationError> {
    if outcome.dynasty_id != state.player_dynasty_id {
        return Ok(());
    }
    if outcome.forfeited {
        try_push_outbox(
            state,
            OutboxKind::Politics,
            format!("Office forfeited at institution {}", outcome.institution_id),
            "Repeatedly unmet civic duties forced the dynasty to surrender the office. The institution will select a replacement next month, and the dynasty cannot immediately return to the same office."
                .to_owned(),
        )?;
    } else if outcome.should_notify {
        try_push_outbox(
            state,
            OutboxKind::Politics,
            format!(
                "Office duty shortfall at institution {}",
                outcome.institution_id
            ),
            format!(
                "The dynasty funded {} of a {} monthly civic duty. The {} shortfall reduced institutional and dynastic standing.",
                outcome.paid, outcome.required, outcome.shortfall
            ),
        )?;
    }
    Ok(())
}

pub(crate) fn office_duty_subject(
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
) -> String {
    format!(
        "institution:{};dynasty:{}",
        institution_id.value(),
        dynasty_id.value()
    )
}

pub(crate) fn apply_office_power_effects(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let offices: Vec<_> = state
        .institutions
        .values()
        .filter_map(|institution| {
            let holder = state.characters.get(institution.office_holder_id?)?;
            let district_id = registry
                .get_institution(institution.institution_id)?
                .district_id();
            Some((
                institution.institution_id,
                holder.dynasty_id(),
                district_id,
                institution.powers.iter().copied().collect::<Vec<_>>(),
            ))
        })
        .collect();
    for (institution_id, dynasty_id, district_id, powers) in offices {
        for power in powers {
            apply_office_power(
                registry,
                state,
                institution_id,
                dynasty_id,
                district_id,
                power,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn apply_office_power(
    registry: &Registry,
    state: &mut AppState,
    institution_id: InstitutionId,
    dynasty_id: DynastyId,
    district_id: DistrictId,
    power: OfficePower,
) -> Result<(), SimulationError> {
    match power {
        OfficePower::Licenses => {
            let dynasty = state
                .dynasties
                .get_mut(&dynasty_id)
                .expect("officeholder dynasty must exist");
            dynasty.resources.legitimacy_basis_points = dynasty
                .resources
                .legitimacy_basis_points
                .saturating_add(15)
                .min(10_000);
        }
        OfficePower::Inspections => {
            let dynasty = state
                .dynasties
                .get_mut(&dynasty_id)
                .expect("officeholder dynasty must exist");
            dynasty.resources.reputation_quality_basis_points = dynasty
                .resources
                .reputation_quality_basis_points
                .saturating_add(15)
                .min(10_000);
        }
        OfficePower::MarketTolls | OfficePower::Taxation => {
            // Like vacancy income, toll and taxation revenue is funded by the
            // market's own clearing pool; it is bounded by what that pool
            // holds so a depleted pool degrades the office's take instead of
            // aborting the monthly settlement.
            let revenue =
                Money::from_copper(100).min(state.market.clearing_account.max(Money::ZERO));
            if revenue <= Money::ZERO {
                return Ok(());
            }
            let institution_budget = state
                .institutions
                .get(&institution_id)
                .expect("office institution must exist")
                .budget;
            let next_budget = institution_budget.checked_add(revenue).ok_or(
                SimulationError::InstitutionBudgetOverflow {
                    institution_id,
                    current: institution_budget,
                    incoming: revenue,
                },
            )?;
            debit_market_clearing_account(state, revenue)?;
            state
                .institutions
                .get_mut(&institution_id)
                .expect("office institution must exist")
                .budget = next_budget;
        }
        OfficePower::DebtEnforcement => adjust_reliability_reputation(state, dynasty_id, 15),
        OfficePower::CityContracts => award_city_contract(state, institution_id, dynasty_id)?,
        OfficePower::PublicWorks => {
            let district = state
                .districts
                .get_mut(&district_id)
                .expect("office district must exist");
            district.employment_basis_points = district
                .employment_basis_points
                .saturating_add(20)
                .min(10_000);
        }
        OfficePower::WatchPriorities => {
            let district = state
                .districts
                .get_mut(&district_id)
                .expect("office district must exist");
            district.safety_basis_points =
                district.safety_basis_points.saturating_add(40).min(10_000);
        }
        OfficePower::EmergencyImports => {
            if let Some(grain_id) = registry.get_good_id("grain") {
                let quantity = Quantity::from_units(20);
                add_market_supply(state, grain_id, quantity)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn award_city_contract(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
) -> Result<(), SimulationError> {
    let business_id = state
        .businesses
        .ids_for_owner(dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|business_id| state.businesses.get(*business_id))
        .filter(|business| business.status() == BusinessStatus::Active)
        .min_by_key(|business| (business.cash(), business.id()))
        .map(crate::core::Business::id);
    let Some(business_id) = business_id else {
        return Ok(());
    };
    let institution_budget = state
        .institutions
        .get(&institution_id)
        .expect("city contract institution must exist")
        .budget;
    let award = Money::from_copper(250).min(institution_budget);
    if award == Money::ZERO {
        return Ok(());
    }
    let (resulting_cash, resulting_lifetime_revenue, next_finance_version) = {
        let business = state
            .businesses
            .get(business_id)
            .expect("city contract business must exist");
        (
            business
                .cash()
                .checked_add(award)
                .ok_or(SimulationError::BusinessCashOverflow {
                    business_id,
                    current: business.cash(),
                    incoming: award,
                })?,
            business.finance.lifetime_revenue.checked_add(award).ok_or(
                SimulationError::BusinessLifetimeRevenueOverflow {
                    business_id,
                    current: business.finance.lifetime_revenue,
                    incoming: award,
                },
            )?,
            next_business_finance_version(business)?,
        )
    };
    state
        .institutions
        .get_mut(&institution_id)
        .expect("city contract institution must exist")
        .budget = institution_budget
        .checked_sub(award)
        .expect("bounded city-contract award must not exceed institution budget");
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("city contract business must exist");
    business.finance.cash = resulting_cash;
    business.finance.lifetime_revenue = resulting_lifetime_revenue;
    business.finance.version = next_finance_version;
    Ok(())
}

pub(crate) fn resolve_institution_selections(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let due = due_institution_selections(state, day);
    if due.is_empty() {
        return Ok(());
    }
    let next_term_day = checked_future_day(day, crate::systems::OFFICE_TERM_DAYS)?;
    let next_retry_day = checked_future_day(day, crate::systems::OFFICE_VACANCY_RETRY_DAYS)?;

    // Decide the complete selection result before committing any term change.
    let mut planned_office_holders = BTreeSet::new();
    let mut selections = Vec::new();
    for institution_id in due {
        let winner = select_institution_officeholder(
            registry,
            state,
            institution_id,
            day,
            &planned_office_holders,
        );
        // A vacancy retries on the short forfeiture cadence instead of
        // locking the office — its powers and stipend flow — away for a full
        // term, and an unfilled term does not consume a term number.
        let (term_number, next_selection_day) = {
            let institution = state
                .institutions
                .get(&institution_id)
                .expect("institution runtime must exist");
            match winner {
                Some(_) => (
                    institution
                        .term_number
                        .checked_add(1)
                        .filter(|next| *next < u32::MAX)
                        .ok_or(SimulationError::InstitutionTermNumberExhausted {
                            institution_id,
                        })?,
                    next_term_day,
                ),
                None => (institution.term_number, next_retry_day),
            }
        };
        if let Some(winner) = winner {
            planned_office_holders.insert(winner);
        }
        selections.push((institution_id, winner, term_number, next_selection_day));
    }

    for (institution_id, winner, term_number, next_selection_day) in &selections {
        let institution = state
            .institutions
            .get_mut(institution_id)
            .expect("institution runtime must exist");
        institution.office_holder_id = *winner;
        institution.term_started_day = day;
        institution.next_selection_day = *next_selection_day;
        institution.term_number = *term_number;
    }
    announce_institution_selections(state, selections)?;
    Ok(())
}

/// Scores every eligible member and returns the winning officeholder for one
/// institution's election, or `None` when no member qualifies.
pub(crate) fn select_institution_officeholder(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    day: i64,
    planned_office_holders: &BTreeSet<CharacterId>,
) -> Option<CharacterId> {
    let institution_kind = registry
        .get_institution(institution_id)
        .expect("runtime institution must have a registry definition")
        .kind();
    let institution = state
        .institutions
        .get(&institution_id)
        .expect("institution runtime must exist");
    let incumbent_id = institution.office_holder_id;
    let member_ids: Vec<_> = institution.members.iter().copied().collect();
    let candidates: Vec<_> = member_ids
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .filter(|character| character.status() == crate::core::CharacterStatus::Active)
        .filter(|character| !planned_office_holders.contains(&character.id()))
        .filter(|character| {
            !state.institutions.values().any(|other| {
                other.institution_id != institution_id
                    && other.office_holder_id == Some(character.id())
            })
        })
        .filter(|character| {
            !has_recent_office_duty_forfeiture(state, institution_id, character.dynasty_id(), day)
                && (character.dynasty_id() != state.player_dynasty_id
                    || incumbent_id == Some(character.id())
                    || has_recent_office_nomination(state, institution_id, character.id(), day))
        })
        .map(|character| {
            let dynasty = state
                .dynasties
                .get(&character.dynasty_id())
                .expect("candidate dynasty must exist");
            let campaign_bonus =
                if has_recent_office_nomination(state, institution_id, character.id(), day) {
                    OFFICE_NOMINATION_CAMPAIGN_BONUS
                } else {
                    0
                };
            let relationship_support =
                institution_relationship_support(state, institution_id, character.dynasty_id());
            let score = institution_capability_score(character, institution_kind)
                .saturating_add(u32::from(dynasty.resources.legitimacy_basis_points))
                .saturating_add(campaign_bonus)
                .saturating_add(relationship_support);
            (score, character.id())
        })
        .collect();
    candidates
        .into_iter()
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
        .map(|(_, character_id)| character_id)
}

/// Records concentration backlash and durable feedback for each committed
/// office selection.
pub(crate) fn announce_institution_selections(
    state: &mut AppState,
    selections: Vec<(InstitutionId, Option<CharacterId>, u32, i64)>,
) -> Result<(), SimulationError> {
    for (institution_id, winner, term_number, _) in selections {
        if let Some(winner) = winner {
            apply_office_concentration_backlash(state, institution_id, winner);
            let winner_dynasty_id = state
                .characters
                .get(winner)
                .expect("selected officeholder must exist")
                .dynasty_id();
            // Winning an election is what earns public standing: the reward
            // lands on victory, not on filing, and scales with the office's
            // institutional standing so endowed guilds confer more weight.
            let standing_bonus = state
                .institutions
                .get(&institution_id)
                .expect("selected institution must exist")
                .legitimacy_basis_points
                / u16::try_from(INSTITUTION_STANDING_VICTORY_DIVISOR)
                    .expect("divisor must fit u16");
            if let Some(dynasty) = state.dynasties.get_mut(&winner_dynasty_id) {
                dynasty.resources.legitimacy_basis_points = dynasty
                    .resources
                    .legitimacy_basis_points
                    .saturating_add(150)
                    .saturating_add(standing_bonus)
                    .min(10_000);
            }
            let fees_of_office = if winner_dynasty_id == state.player_dynasty_id {
                let power_count = state
                    .institutions
                    .get(&institution_id)
                    .expect("selected institution must exist")
                    .powers
                    .len();
                format!(
                    " The position pays {} in monthly fees of office, funded by the institution's budget.",
                    OFFICE_STIPEND_PER_POWER
                        .saturating_mul(i64::try_from(power_count).unwrap_or(i64::MAX)),
                )
            } else {
                String::new()
            };
            try_push_outbox(
                state,
                OutboxKind::Politics,
                format!("Institution {institution_id} selected a new officeholder"),
                format!(
                    "Character {winner} now holds the office for term {term_number}.{fees_of_office}"
                ),
            )?;
        }
    }
    Ok(())
}

pub(crate) fn due_institution_selections(state: &AppState, day: i64) -> Vec<InstitutionId> {
    state
        .institutions
        .values()
        .filter(|institution| institution.next_selection_day <= day)
        .map(|institution| institution.institution_id)
        .collect()
}

pub(crate) fn apply_office_concentration_backlash(
    state: &mut AppState,
    institution_id: InstitutionId,
    winner_id: CharacterId,
) {
    let winner_dynasty_id = state
        .characters
        .get(winner_id)
        .expect("selected officeholder must exist")
        .dynasty_id();
    let office_count = state
        .institutions
        .values()
        .filter(|institution| {
            institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == winner_dynasty_id)
            })
        })
        .count();
    let additional_offices = office_count.saturating_sub(1);
    if additional_offices == 0 {
        return;
    }
    let backlash = i16::try_from(additional_offices)
        .unwrap_or(i16::MAX)
        .saturating_mul(OFFICE_CONCENTRATION_BACKLASH_PER_ADDITIONAL_OFFICE)
        .min(MAX_OFFICE_CONCENTRATION_BACKLASH);
    let member_dynasties: BTreeSet<_> = state
        .institutions
        .get(&institution_id)
        .expect("selected institution must exist")
        .members
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .map(crate::core::Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != winner_dynasty_id)
        .collect();
    for member_dynasty_id in member_dynasties {
        adjust_dynasty_relationship(
            state,
            winner_dynasty_id,
            member_dynasty_id,
            RelationshipDelta::new(-(backlash / 2), 30, backlash / 3, backlash, 0),
        );
        remember_dynasty_interaction(
            state,
            winner_dynasty_id,
            member_dynasty_id,
            &format!(
                "house {winner_dynasty_id} consolidated {office_count} offices after winning institution {institution_id}, increasing coalition resistance"
            ),
        );
    }
}

pub(crate) fn institution_capability_score(
    character: &crate::core::Character,
    institution_kind: InstitutionKind,
) -> u32 {
    let capabilities = &character.capabilities;
    let (primary, secondary) = match institution_kind {
        InstitutionKind::CraftGuild => (capabilities.craft, capabilities.commerce),
        InstitutionKind::MerchantGuild | InstitutionKind::MarketOffice => {
            (capabilities.commerce, capabilities.administration)
        }
        InstitutionKind::Council | InstitutionKind::Charity => {
            (capabilities.social, capabilities.administration)
        }
        InstitutionKind::Court | InstitutionKind::Watch => {
            (capabilities.administration, capabilities.social)
        }
        InstitutionKind::Treasury => (capabilities.administration, capabilities.commerce),
    };
    u32::from(primary)
        .saturating_mul(100)
        .saturating_add(u32::from(secondary).saturating_mul(30))
}

pub(crate) fn has_recent_office_nomination(
    state: &AppState,
    institution_id: crate::ids::InstitutionId,
    character_id: CharacterId,
    day: i64,
) -> bool {
    let nomination_subject =
        crate::systems::commands::office_nomination_subject(institution_id, character_id);
    // Chronologically ordered history: stop once records predate the
    // nomination-recency window.
    for record in state.audit_log.iter().rev() {
        if day.saturating_sub(record.day()) > 180 {
            break;
        }
        if record.kind() == AuditKind::OfficeNomination && record.subject() == nomination_subject {
            return true;
        }
    }
    false
}

pub(crate) fn has_recent_office_duty_forfeiture(
    state: &AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    day: i64,
) -> bool {
    let subject = office_duty_subject(institution_id, dynasty_id);
    // Chronologically ordered history: stop once records predate the
    // reelection-ban window.
    for record in state.audit_log.iter().rev() {
        if day.saturating_sub(record.day()) > OFFICE_DUTY_REELECTION_BAN_DAYS {
            break;
        }
        if record.kind() == AuditKind::OfficeDutyForfeiture && record.subject() == subject {
            return true;
        }
    }
    false
}

pub(crate) fn institution_relationship_support(
    state: &AppState,
    institution_id: crate::ids::InstitutionId,
    candidate_dynasty_id: DynastyId,
) -> u32 {
    let member_dynasties: BTreeSet<_> = state
        .institutions
        .get(&institution_id)
        .expect("institution runtime must exist")
        .members
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .map(crate::core::Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != candidate_dynasty_id)
        .collect();
    let mut total = 0_u32;
    let mut count = 0_u32;
    for dynasty_id in member_dynasties {
        let relationship = state
            .relationships
            .get(&DynastyPair::new(candidate_dynasty_id, dynasty_id))
            .expect("every dynasty pair must have a relationship record");
        let positive = u32::from(relationship.trust_basis_points)
            .saturating_add(u32::from(relationship.respect_basis_points))
            .saturating_add(u32::from(relationship.fear_basis_points) / 2);
        total = total.saturating_add(
            positive.saturating_sub(u32::from(relationship.resentment_basis_points)),
        );
        count = count.saturating_add(1);
    }
    total
        .checked_div(count)
        .map_or(0, |average| (average / 4).min(3_000))
}
