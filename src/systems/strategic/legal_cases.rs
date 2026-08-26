//! Grounded legal-case hearings, judgments, negotiated settlements, and claim discharge.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn advance_legal_case_hearings(state: &mut AppState) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let entering_hearing: Vec<_> = state
        .legal_cases
        .values()
        .filter(|legal_case| {
            legal_case.status == LegalCaseStatus::Filed
                && legal_case.hearing_day > day
                && legal_case.hearing_day.saturating_sub(day) <= 30
        })
        .map(|legal_case| {
            (
                legal_case.id,
                legal_case.plaintiff_dynasty_id == state.player_dynasty_id
                    || legal_case.defendant_dynasty_id == state.player_dynasty_id,
            )
        })
        .collect();
    for (legal_case_id, player_is_party) in entering_hearing {
        state
            .legal_cases
            .get_mut(&legal_case_id)
            .expect("legal case must exist")
            .status = LegalCaseStatus::Hearing;
        if player_is_party {
            try_push_outbox(
                state,
                OutboxKind::Legal,
                format!("Legal case {legal_case_id} entered hearing"),
                "The court began formal proceedings ahead of judgment.".to_owned(),
            )?;
        }
    }
    Ok(())
}

pub(crate) fn resolve_legal_cases(state: &mut AppState) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let due: Vec<_> = state
        .legal_cases
        .values()
        .filter(|case| {
            matches!(
                case.status,
                LegalCaseStatus::Filed | LegalCaseStatus::Hearing
            ) && case.hearing_day <= day
        })
        .map(|case| {
            (
                case.id,
                case.plaintiff_dynasty_id,
                case.defendant_dynasty_id,
                case.kind,
                case.claim_source,
                case.evidence_basis_points,
                case.public_attention_basis_points,
                case.damages,
            )
        })
        .collect();
    for (id, plaintiff_id, defendant_id, kind, claim_source, evidence, attention, damages) in due {
        let plaintiff_legitimacy = state
            .dynasties
            .get(&plaintiff_id)
            .expect("legal plaintiff must exist")
            .resources
            .legitimacy_basis_points;
        let defendant_legitimacy = state
            .dynasties
            .get(&defendant_id)
            .expect("legal defendant must exist")
            .resources
            .legitimacy_basis_points;
        let plaintiff_score = u32::from(evidence)
            .saturating_mul(2)
            .saturating_add(u32::from(attention))
            .saturating_add(u32::from(plaintiff_legitimacy));
        let defendant_score = 10_000_u32
            .saturating_sub(u32::from(evidence))
            .saturating_mul(2)
            .saturating_add(u32::from(defendant_legitimacy));
        let plaintiff_wins = plaintiff_score >= defendant_score;
        let (awarded, paid) = if plaintiff_wins {
            let awarded = recoverable_legal_damages(state, claim_source, damages);
            let paid = settle_legal_damages(state, plaintiff_id, defendant_id, awarded)?;
            settle_legal_claim_source(
                state,
                claim_source,
                plaintiff_id,
                defendant_id,
                paid,
                false,
                true,
            );
            (awarded, paid)
        } else {
            (Money::ZERO, Money::ZERO)
        };
        // Winning a grounded claim over an obligation that no longer exists is
        // a hollow victory: the court rules on the paperwork, but a dispute
        // over a cured debt must not poison the relationship as if real
        // damages had been suffered.
        let hollow_victory = plaintiff_wins && claim_source.is_some() && awarded == Money::ZERO;
        state
            .legal_cases
            .get_mut(&id)
            .expect("legal case must exist")
            .status = if plaintiff_wins {
            LegalCaseStatus::DecidedForPlaintiff
        } else {
            LegalCaseStatus::DecidedForDefendant
        };
        adjust_dynasty_relationship(
            state,
            plaintiff_id,
            defendant_id,
            if hollow_victory {
                RelationshipDelta::new(-5, 5, 5, 20, 0)
            } else {
                RelationshipDelta::new(-60, 20, 50, 120, 0)
            },
        );
        if plaintiff_id == state.player_dynasty_id || defendant_id == state.player_dynasty_id {
            try_push_outbox(
                state,
                OutboxKind::Legal,
                format!("Legal case {id} decided"),
                if !plaintiff_wins {
                    format!(
                        "The court decided the {kind:?} claim for dynasty {defendant_id}; no damages were awarded."
                    )
                } else if hollow_victory {
                    format!(
                        "The court decided the {kind:?} claim for dynasty {plaintiff_id}, but the underlying obligation had already been cured, so no damages were due."
                    )
                } else {
                    let settlement_note = if claim_source.is_some() {
                        " The grounded source obligation is settled by the judgment."
                    } else {
                        ""
                    };
                    format!(
                        "The court decided the {kind:?} claim for dynasty {plaintiff_id}, awarded {awarded}, and recovered {paid} immediately.{settlement_note}"
                    )
                },
            )?;
        }
    }
    Ok(())
}

pub(crate) fn settle_legal_damages(
    state: &mut AppState,
    plaintiff_id: DynastyId,
    defendant_id: DynastyId,
    damages: Money,
) -> Result<Money, SimulationError> {
    let defendant_cash = state
        .dynasties
        .get(&defendant_id)
        .expect("legal defendant must exist")
        .treasury();
    let plaintiff_treasury = state
        .dynasties
        .get(&plaintiff_id)
        .expect("legal plaintiff must exist")
        .treasury();
    let paid = damages.min(defendant_cash);
    plaintiff_treasury
        .checked_add(paid)
        .ok_or(SimulationError::DynastyTreasuryOverflow {
            dynasty_id: plaintiff_id,
            current: plaintiff_treasury,
            incoming: paid,
        })?;
    state
        .dynasties
        .get_mut(&defendant_id)
        .expect("legal defendant must exist")
        .resources
        .treasury = defendant_cash
        .checked_sub(paid)
        .expect("bounded damages must not exceed defendant treasury");
    let plaintiff = state
        .dynasties
        .get_mut(&plaintiff_id)
        .expect("legal plaintiff must exist");
    plaintiff.resources.treasury = plaintiff
        .resources
        .treasury
        .checked_add(paid)
        .expect("prevalidated damages must fit plaintiff treasury");
    Ok(paid)
}

pub(crate) fn recoverable_legal_damages(
    state: &AppState,
    claim_source: Option<LegalClaimSource>,
    requested: Money,
) -> Money {
    match claim_source {
        Some(LegalClaimSource::Loan { loan_id }) => state
            .loans
            .get(&loan_id)
            .map_or(Money::ZERO, |loan| requested.min(loan.balance)),
        Some(LegalClaimSource::Contract { contract_id }) => state
            .contracts
            .get(&contract_id)
            .map_or(Money::ZERO, |contract| {
                requested.min(contract.unpaid_breach_penalty)
            }),
        None => requested,
    }
}

/// Returns the full outstanding obligation behind a grounded claim source.
///
/// A negotiated settlement extinguishes the claim in whole only when its
/// payment covers this amount; anything less discharges just what was paid,
/// mirroring how a court judgment treats a judgment-proof defendant.
pub(crate) fn outstanding_legal_claim_obligation(
    state: &AppState,
    claim_source: Option<LegalClaimSource>,
) -> Money {
    match claim_source {
        Some(LegalClaimSource::Loan { loan_id }) => state
            .loans
            .get(&loan_id)
            .map_or(Money::ZERO, |loan| loan.balance),
        Some(LegalClaimSource::Contract { contract_id }) => state
            .contracts
            .get(&contract_id)
            .map_or(Money::ZERO, |contract| contract.unpaid_breach_penalty),
        None => Money::ZERO,
    }
}

pub(crate) fn settle_legal_claim_source(
    state: &mut AppState,
    claim_source: Option<LegalClaimSource>,
    plaintiff_id: DynastyId,
    defendant_id: DynastyId,
    discharged: Money,
    full_satisfaction: bool,
    enforce_against_collateral: bool,
) {
    match claim_source {
        Some(LegalClaimSource::Loan { loan_id }) => {
            let collateral_property_id = {
                let Some(loan) = state.loans.get_mut(&loan_id) else {
                    return;
                };
                if loan.lender_dynasty_id != plaintiff_id
                    || loan.borrower_dynasty_id != defendant_id
                {
                    return;
                }
                // A negotiated settlement extinguishes the whole claim; a
                // court judgment discharges only what it actually recovered,
                // never more: a judgment-proof defendant cannot have the debt
                // erased cleaner than a bankruptcy.
                if full_satisfaction {
                    loan.balance = Money::ZERO;
                } else {
                    loan.balance = loan.balance.saturating_sub(discharged);
                }
                if loan.balance == Money::ZERO {
                    loan.status = LoanStatus::Repaid;
                    loan.missed_payments = 0;
                }
                loan.collateral_property_id
            };
            let outstanding = state
                .loans
                .get(&loan_id)
                .map_or(Money::ZERO, |loan| loan.balance);
            if outstanding > Money::ZERO {
                if enforce_against_collateral {
                    // A court judgment executes against pledged collateral;
                    // only when that still leaves a deficiency does the
                    // lender retain a live claim against the borrower.
                    execute_judgment_against_collateral(state, loan_id);
                }
                // Without enforcement the remainder simply stands: a
                // negotiated settlement is amicable, so the pledged property
                // stays pledged and the loan's own delinquency machinery —
                // not the courtroom — decides what happens if it fails again.
            } else if let Some(property_id) = collateral_property_id
                && let Some(property) = state.properties.get_mut(&property_id)
                && property.collateral_loan_id == Some(loan_id)
            {
                property.collateral_loan_id = None;
            }
        }
        Some(LegalClaimSource::Contract { contract_id }) => {
            let Some(contract) = state.contracts.get_mut(&contract_id) else {
                return;
            };
            if contract.breaching_dynasty_id != Some(defendant_id)
                || contract.breach_victim_dynasty_id != Some(plaintiff_id)
            {
                return;
            }
            // Only the discharged part of the penalty leaves the recoverable
            // breach debt unless the parties agreed to a full settlement.
            if full_satisfaction {
                contract.unpaid_breach_penalty = Money::ZERO;
            } else {
                contract.unpaid_breach_penalty =
                    contract.unpaid_breach_penalty.saturating_sub(discharged);
            }
        }
        None => {}
    }
}

/// Executes a won debt judgment against the loan's pledged collateral when
/// immediate payment could not cover it, through the same seizure accounting
/// as the default-seizure path. Equity the lender cannot fund is recorded as a
/// borrower grievance rather than silently enriching the lender.
pub(crate) fn execute_judgment_against_collateral(
    state: &mut AppState,
    loan_id: crate::ids::LoanId,
) {
    let parties = state
        .loans
        .get(&loan_id)
        .map(|loan| (loan.lender_dynasty_id, loan.borrower_dynasty_id));
    let seizure = seize_pledged_collateral(state, loan_id);
    if seizure.equity_withheld > Money::ZERO
        && let Some((lender_id, borrower_id)) = parties
    {
        // The executing lender could not fund the full surplus, so it kept a
        // windfall above its claim; the borrower remembers the grievance.
        adjust_dynasty_relationship(
            state,
            borrower_id,
            lender_id,
            RelationshipDelta::new(-150, -100, 0, 250, 0),
        );
    }
}

/// A single bad month adds between these bounds of route disruption.
pub(crate) const ROUTE_DISRUPTION_SPIKE_MIN_BASIS_POINTS: u16 = 1_500;
pub(crate) const ROUTE_DISRUPTION_SPIKE_RANGE_BASIS_POINTS: u32 = 1_500;
/// Routine calm months remove this much accumulated route disruption.
pub(crate) const ROUTE_DISRUPTION_CALM_RECOVERY_BASIS_POINTS: u16 = 150;
/// Post-crisis healing removes this much accumulated route disruption.
pub(crate) const ROUTE_DISRUPTION_HEALING_BASIS_POINTS: u16 = 250;
