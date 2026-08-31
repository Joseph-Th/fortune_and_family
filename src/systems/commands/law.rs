//! Law sponsorship and municipal debt issuance.
//!
//! Purpose: own the validated player path for `EnactLaw`, including civic
//! debt authorization that credits the treasury institution.
//! Owns: `apply_law`, civic-debt `principal`/`weekly_payment` derivation,
//! `OfficePower`-gated law power mapping, and audit/outbox.
//! Reads: `Registry` institutions, `AppState` laws + dynasties + market.
//! Mutates: `EnactedLaw` active set, `CivicDebt` issuance, dynasty
//! treasury/legitimacy, market clearing pool via `credit_*`.
//! Does not own: law price-control effects (simulation) or debt
//! delinquency (credit strategic).
//! Invariants: `PublicDebtAuthorization` value is issuance principal;
//! other law values validate via `LawKind::is_value_valid`; office +
//! power + establishment-day gates before sponsorship; cooldown audited.
//! Focused tests: `src/systems/commands/commands_tests.rs` law paths.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedCivicDebtIssuance {
    treasury_id: InstitutionId,
    creditor_dynasty_id: DynastyId,
    principal: Money,
    creditor_treasury_after: Money,
    treasury_budget_after: Money,
    weekly_payment: Money,
    next_due_day: i64,
}

pub(crate) fn validate_civic_debt_issuance(
    registry: &Registry,
    state: &AppState,
    principal: Money,
) -> Result<ValidatedCivicDebtIssuance, CommandError> {
    let treasury_id = registry
        .get_institution_id("treasury")
        .ok_or(CommandError::MissingCivicTreasury)?;
    let treasury = state
        .institutions
        .get(&treasury_id)
        .ok_or(CommandError::MissingCivicTreasury)?;
    let treasury_budget_after =
        treasury
            .budget
            .checked_add(principal)
            .ok_or(CommandError::CivicTreasuryOverflow {
                current: treasury.budget,
                incoming: principal,
            })?;
    let creditor = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .filter(|dynasty| {
            dynasty
                .treasury()
                .saturating_sub(CIVIC_DEBT_CREDITOR_RESERVE)
                >= principal
        })
        .max_by_key(|dynasty| (dynasty.treasury(), std::cmp::Reverse(dynasty.id())))
        .ok_or(CommandError::NoCivicDebtCreditor {
            required: principal,
        })?;
    Ok(ValidatedCivicDebtIssuance {
        treasury_id,
        creditor_dynasty_id: creditor.id(),
        principal,
        creditor_treasury_after: creditor
            .treasury()
            .checked_sub(principal)
            .expect("validated civic debt creditor must cover the principal"),
        treasury_budget_after,
        weekly_payment: principal.ceil_div_positive(CIVIC_DEBT_TERM_WEEKS),
        next_due_day: checked_future_day(state.clock.day(), 7)?,
    })
}

pub(crate) fn commit_civic_debt_issuance(
    state: &mut AppState,
    law_id: crate::ids::LawId,
    civic_debt_id: crate::ids::CivicDebtId,
    sponsor_dynasty_id: DynastyId,
    issuance: ValidatedCivicDebtIssuance,
) -> Result<crate::ids::CivicDebtId, CommandError> {
    state
        .dynasties
        .get_mut(&issuance.creditor_dynasty_id)
        .expect("validated civic debt creditor must exist")
        .resources
        .treasury = issuance.creditor_treasury_after;
    state
        .institutions
        .get_mut(&issuance.treasury_id)
        .expect("validated civic treasury must exist")
        .budget = issuance.treasury_budget_after;
    state.civic_debts.insert(
        civic_debt_id,
        CivicDebt {
            id: civic_debt_id,
            creditor_dynasty_id: issuance.creditor_dynasty_id,
            authorizing_law_id: law_id,
            sponsor_dynasty_id: Some(sponsor_dynasty_id),
            principal: issuance.principal,
            balance: issuance.principal,
            weekly_payment: issuance.weekly_payment,
            interest_basis_points: CIVIC_DEBT_INTEREST_BASIS_POINTS,
            issued_day: state.clock.day(),
            next_due_day: issuance.next_due_day,
            missed_payments: 0,
            status: CivicDebtStatus::Current,
        },
    );
    crate::systems::strategic::adjust_dynasty_relationship(
        state,
        sponsor_dynasty_id,
        issuance.creditor_dynasty_id,
        crate::systems::strategic::RelationshipDelta::new(40, 30, 0, -20, 1),
    );
    crate::systems::strategic::remember_dynasty_interaction(
        state,
        sponsor_dynasty_id,
        issuance.creditor_dynasty_id,
        &format!("Civic debt {civic_debt_id} financed the city treasury."),
    );
    crate::systems::strategic::try_record_counterparty_information(
        state,
        sponsor_dynasty_id,
        issuance.creditor_dynasty_id,
        "Municipal debt underwriting and treasury records",
    )?;
    Ok(civic_debt_id)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedLawSponsorship {
    legitimacy: u16,
    civic_debt_issuance: Option<ValidatedCivicDebtIssuance>,
}

pub(crate) fn validate_law_sponsorship(
    registry: &Registry,
    state: &AppState,
    kind: LawKind,
    value: i64,
) -> Result<ValidatedLawSponsorship, CommandError> {
    if !kind.is_value_valid(value) {
        return Err(CommandError::InvalidLawValue { kind, value });
    }
    if state
        .laws
        .values()
        .any(|law| law.active && law.kind == kind && law.value == value)
    {
        return Err(CommandError::UnchangedLaw { kind, value });
    }
    if let Some(last_enactment_day) = state
        .laws
        .values()
        .filter(|law| law.sponsor_dynasty_id == Some(state.player_dynasty_id))
        .map(|law| law.enacted_day)
        .max()
    {
        let next_enactment_day =
            checked_future_day(last_enactment_day, LAW_SPONSORSHIP_INTERVAL_DAYS)?;
        if state.clock.day() < next_enactment_day {
            return Err(CommandError::LawCooldown { next_enactment_day });
        }
    }
    let legitimacy = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points;
    if legitimacy < LAW_LEGITIMACY_REQUIREMENT {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: LAW_LEGITIMACY_REQUIREMENT,
        });
    }
    if !has_player_office(state) {
        return Err(CommandError::LawSponsorshipRequiresOffice);
    }
    let required_power = required_office_power_for_law(kind);
    if !has_player_office_power(state, required_power) {
        return Err(CommandError::LawSponsorshipRequiresPower {
            kind,
            required: required_power,
        });
    }
    let available_day = checked_player_office_power_available_day(state, required_power)?
        .expect("validated office power must have an availability day");
    if state.clock.day() < available_day {
        return Err(CommandError::LawSponsorshipPowerNotEstablished {
            kind,
            required: required_power,
            available_day,
        });
    }
    let civic_debt_issuance = (kind == LawKind::PublicDebtAuthorization)
        .then(|| validate_civic_debt_issuance(registry, state, Money::from_copper(value)))
        .transpose()?;
    Ok(ValidatedLawSponsorship {
        legitimacy,
        civic_debt_issuance,
    })
}

pub(crate) fn apply_law(
    registry: &Registry,
    state: &mut AppState,
    kind: LawKind,
    value: i64,
) -> Result<CommandOutcome, CommandError> {
    let validation = validate_law_sponsorship(registry, state, kind, value)?;
    // Reserve every durable identifier the commit path needs so allocation
    // exhaustion surfaces while state is still untouched. The trailing
    // feedback pushes remain fallible and are covered by the caller's
    // transactional working copy.
    let id = state.next_ids.try_law()?;
    let reserved_civic_debt_id = validation
        .civic_debt_issuance
        .as_ref()
        .map(|_| state.next_ids.try_civic_debt())
        .transpose()?;
    spend_player_treasury_to_market(state, LAW_SPONSORSHIP_COST)?;
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points = validation.legitimacy.saturating_sub(LAW_LEGITIMACY_COST);
    for law in state
        .laws
        .values_mut()
        .filter(|law| law.kind == kind && law.active)
    {
        law.active = false;
    }
    state.laws.insert(
        id,
        EnactedLaw {
            id,
            kind,
            enacted_day: state.clock.day(),
            sponsor_dynasty_id: Some(state.player_dynasty_id),
            value,
            active: kind.remains_active_after_enactment(),
        },
    );
    let civic_debt_id = validation
        .civic_debt_issuance
        .map(|issuance| {
            commit_civic_debt_issuance(
                state,
                id,
                reserved_civic_debt_id.expect("reserved civic debt id must exist with issuance"),
                state.player_dynasty_id,
                issuance,
            )
        })
        .transpose()?;
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Law,
        format!("Law {id} enacted"),
        civic_debt_id.map_or_else(
            || format!("The player dynasty sponsored {kind:?} with value {value}."),
            |debt_id| {
                format!(
                    "The player dynasty sponsored {kind:?}; civic debt {debt_id} issued {value} copper to the treasury."
                )
            },
        ),
    )?;
    Ok(CommandOutcome {
        summary: civic_debt_id.map_or_else(
            || format!("Enacted law {id}: {kind:?}."),
            |debt_id| format!("Enacted law {id}: {kind:?}, issuing civic debt {debt_id}."),
        ),
    })
}
