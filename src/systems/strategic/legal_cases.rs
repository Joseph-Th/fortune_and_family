//! Grounded legal-case hearings, judgments, negotiated settlements, and claim discharge.
//!
//! Purpose: own the monthly adjudication of debt/contract claims — filing
//! against a concrete obligation, hearing delay, evidence-capped judgment,
//! installment execution, settlement, and discharge of the underlying claim
//! so no dynasty is trapped by an uncollectible procedure.
//! Owns: filing validation, hearing scheduling, judgment execution,
//! settlement path, and terminal write-off when a judgment proves
//! uncollectible.
//! Reads: `Registry` + `AppState` loans/contracts/civic debts.
//! Mutates: `AppState` legal cases, associated loans/contracts, treasuries,
//! district conditions, legitimacy, and audit/outbox.
//! Does not own: law sponsorship — `strategic/mod.rs` law appliers.
//! Invariants: every case traces to a grounded claim; damages ≤ claim cap;
//! filing fees credit the Civic Court; settled claims discharge their
//! backing obligation per DESIGN.md recovery guarantee.
//! Focused tests: `src/systems/strategic/strategic_tests.rs` legal and
//! `commands_tests.rs` filing/settlement validation.

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
    let due: Vec<_> = state
        .legal_cases
        .values()
        .filter(|case| {
            matches!(
                case.status,
                LegalCaseStatus::Filed | LegalCaseStatus::Hearing
            ) && case.hearing_day <= state.clock.day()
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
        decide_legal_case(
            state,
            id,
            plaintiff_id,
            defendant_id,
            kind,
            claim_source,
            evidence,
            attention,
            damages,
        )?;
    }
    execute_decided_debt_judgments(state)?;
    Ok(())
}

/// Continues execution of already-won debt judgments as the borrower's asset
/// position changes after the hearing.
///
/// A legal claim is intentionally one-shot, but its judgment is durable. If a
/// borrower still had collectible assets at judgment and only later becomes
/// truly judgment-proof, the existing decision must be able to finish the
/// credit lifecycle without filing the same claim a second time.
fn execute_decided_debt_judgments(state: &mut AppState) -> Result<(), SimulationError> {
    let mut judgments: Vec<_> = state
        .legal_cases
        .values()
        .filter(|legal_case| legal_case.status == LegalCaseStatus::DecidedForPlaintiff)
        .filter_map(|legal_case| match legal_case.claim_source {
            Some(LegalClaimSource::Loan { loan_id }) => Some((
                loan_id,
                legal_case.plaintiff_dynasty_id,
                legal_case.defendant_dynasty_id,
            )),
            Some(LegalClaimSource::Contract { .. }) | None => None,
        })
        .collect();
    judgments.sort_unstable_by_key(|(loan_id, plaintiff_id, defendant_id)| {
        (*loan_id, *plaintiff_id, *defendant_id)
    });
    judgments.dedup_by_key(|(loan_id, _, _)| *loan_id);

    for (loan_id, plaintiff_id, defendant_id) in judgments {
        let written_off = write_off_judgment_proof_loan_deficiency(state, loan_id);
        if written_off <= Money::ZERO
            || (plaintiff_id != state.player_dynasty_id && defendant_id != state.player_dynasty_id)
        {
            continue;
        }
        try_push_outbox(
            state,
            OutboxKind::Legal,
            format!("Loan {loan_id} judgment completed"),
            format!(
                "Final execution found no remaining collectible assets. The unpaid deficiency of {written_off} was written off as the lender's loss."
            ),
        )?;
    }
    Ok(())
}

/// Hears one due case: weighs evidence and standing, settles whatever the
/// losing party can pay against the grounded claim source, and records the
/// relational aftermath.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
fn decide_legal_case(
    state: &mut AppState,
    id: crate::ids::LegalCaseId,
    plaintiff_id: DynastyId,
    defendant_id: DynastyId,
    kind: LegalCaseKind,
    claim_source: Option<LegalClaimSource>,
    evidence: u16,
    attention: u16,
    damages: Money,
) -> Result<(), SimulationError> {
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
    let (awarded, paid, written_off, rejected_claim) = if plaintiff_wins {
        let awarded = recoverable_legal_damages(state, claim_source, damages);
        let paid = settle_legal_damages(state, plaintiff_id, defendant_id, awarded)?;
        let written_off = settle_legal_claim_source(
            state,
            claim_source,
            plaintiff_id,
            defendant_id,
            paid,
            false,
            true,
        );
        (awarded, paid, written_off, Money::ZERO)
    } else {
        // Grounded claim sources are deliberately one-shot. A final judgment
        // for the defendant therefore has to resolve the source's legal
        // enforceability as well: leaving the obligation live after consuming
        // its only court route would create permanent zombie debt or breach
        // penalties that can never be adjudicated again.
        let rejected_claim = dismiss_grounded_claim_after_defendant_judgment(
            state,
            claim_source,
            plaintiff_id,
            defendant_id,
        );
        (Money::ZERO, Money::ZERO, Money::ZERO, rejected_claim)
    };
    // A grounded claim whose underlying obligation is already cured is a
    // hollow victory: the court rules on the paperwork, but the relationship
    // cost stays minimal when no damages are due.
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
                match claim_source {
                    Some(LegalClaimSource::Loan { .. }) if rejected_claim > Money::ZERO => format!(
                        "The court decided the {kind:?} claim for dynasty {defendant_id}; no damages were awarded. The rejected loan claim of {rejected_claim} is written off as the creditor's loss and is no longer enforceable."
                    ),
                    Some(LegalClaimSource::Contract { .. }) if rejected_claim > Money::ZERO => {
                        format!(
                            "The court decided the {kind:?} claim for dynasty {defendant_id}; no damages were awarded. The rejected breach claim of {rejected_claim} is no longer enforceable."
                        )
                    }
                    _ => format!(
                        "The court decided the {kind:?} claim for dynasty {defendant_id}; no damages were awarded."
                    ),
                }
            } else if hollow_victory {
                format!(
                    "The court decided the {kind:?} claim for dynasty {plaintiff_id}, but the underlying obligation had already been cured, so no damages were due."
                )
            } else {
                let outstanding = outstanding_legal_claim_obligation(state, claim_source);
                let settlement_note = if written_off > Money::ZERO {
                    format!(
                        " Final enforcement found no remaining collectible assets, so {written_off} was written off as the lender's loss."
                    )
                } else if claim_source.is_some() && outstanding == Money::ZERO {
                    " The grounded source obligation is satisfied.".to_owned()
                } else if claim_source.is_some() {
                    format!(" The grounded source obligation still has {outstanding} outstanding.")
                } else {
                    String::new()
                };
                format!(
                    "The court decided the {kind:?} claim for dynasty {plaintiff_id}, awarded {awarded}, and recovered {paid} immediately.{settlement_note}"
                )
            },
        )?;
    }
    Ok(())
}

/// Applies the economic consequence of a final defendant judgment to its
/// one-shot grounded claim source.
///
/// No payment occurs. A rejected debt claim becomes a lender write-off and a
/// rejected contract claim loses its remaining recoverable breach penalty.
/// This keeps legal finality synchronized with the underlying obligation
/// instead of permanently consuming the only litigation route while leaving
/// an enforceable-looking balance behind.
fn dismiss_grounded_claim_after_defendant_judgment(
    state: &mut AppState,
    claim_source: Option<LegalClaimSource>,
    plaintiff_id: DynastyId,
    defendant_id: DynastyId,
) -> Money {
    match claim_source {
        Some(LegalClaimSource::Loan { loan_id }) => {
            let (balance, collateral_property_id) = {
                let Some(loan) = state.loans.get_mut(&loan_id) else {
                    return Money::ZERO;
                };
                if loan.lender_dynasty_id != plaintiff_id
                    || loan.borrower_dynasty_id != defendant_id
                {
                    return Money::ZERO;
                }
                let balance = loan.balance;
                let collateral_property_id = loan.collateral_property_id;
                loan.balance = Money::ZERO;
                loan.missed_payments = 0;
                loan.collateral_property_id = None;
                loan.status = if balance > Money::ZERO {
                    LoanStatus::WrittenOff
                } else {
                    LoanStatus::Repaid
                };
                (balance, collateral_property_id)
            };
            if let Some(property_id) = collateral_property_id
                && let Some(property) = state.properties.get_mut(&property_id)
                && property.collateral_loan_id == Some(loan_id)
            {
                property.collateral_loan_id = None;
            }
            if balance > Money::ZERO {
                remember_dynasty_interaction(
                    state,
                    plaintiff_id,
                    defendant_id,
                    &format!(
                        "The court rejected the creditor's claim on loan {loan_id}; the remaining {balance} was written off without payment."
                    ),
                );
            }
            balance
        }
        Some(LegalClaimSource::Contract { contract_id }) => {
            let Some(contract) = state.contracts.get_mut(&contract_id) else {
                return Money::ZERO;
            };
            if contract.breaching_dynasty_id != Some(defendant_id)
                || contract.breach_victim_dynasty_id != Some(plaintiff_id)
            {
                return Money::ZERO;
            }
            let rejected = contract.unpaid_breach_penalty;
            contract.unpaid_breach_penalty = Money::ZERO;
            if rejected > Money::ZERO {
                remember_dynasty_interaction(
                    state,
                    plaintiff_id,
                    defendant_id,
                    &format!(
                        "The court rejected the breach claim on contract {contract_id}; {rejected} of claimed penalty is no longer enforceable."
                    ),
                );
            }
            rejected
        }
        None => Money::ZERO,
    }
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
) -> Money {
    match claim_source {
        Some(LegalClaimSource::Loan { loan_id }) => {
            let collateral_property_id = {
                let Some(loan) = state.loans.get_mut(&loan_id) else {
                    return Money::ZERO;
                };
                if loan.lender_dynasty_id != plaintiff_id
                    || loan.borrower_dynasty_id != defendant_id
                {
                    return Money::ZERO;
                }
                // A negotiated settlement may extinguish the whole claim. A
                // court judgment first discharges only what it actually
                // recovered; any deficiency is handled separately below as an
                // explicit lender write-off only after final enforcement finds
                // the borrower judgment-proof.
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
                    return write_off_judgment_proof_loan_deficiency(state, loan_id);
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
            Money::ZERO
        }
        Some(LegalClaimSource::Contract { contract_id }) => {
            let Some(contract) = state.contracts.get_mut(&contract_id) else {
                return Money::ZERO;
            };
            if contract.breaching_dynasty_id != Some(defendant_id)
                || contract.breach_victim_dynasty_id != Some(plaintiff_id)
            {
                return Money::ZERO;
            }
            // Only the discharged part of the penalty leaves the recoverable
            // breach debt unless the parties agreed to a full settlement.
            if full_satisfaction {
                contract.unpaid_breach_penalty = Money::ZERO;
            } else {
                contract.unpaid_breach_penalty =
                    contract.unpaid_breach_penalty.saturating_sub(discharged);
            }
            Money::ZERO
        }
        None => Money::ZERO,
    }
}

/// Converts an uncollectible post-judgment deficiency into an explicit lender
/// loss once the borrower has no treasury, property, or operating business
/// left for the model to treat as a credible source of recovery.
///
/// The loan balance is not paid. It is extinguished as a write-off, preserving
/// the economic loss while removing a permanently unresolvable default from
/// the borrower's future credit lifecycle. The borrower pays through a severe
/// reliability and legitimacy penalty instead of receiving a cost-free reset.
fn write_off_judgment_proof_loan_deficiency(
    state: &mut AppState,
    loan_id: crate::ids::LoanId,
) -> Money {
    let Some(loan) = state.loans.get(&loan_id) else {
        return Money::ZERO;
    };
    if loan.status != LoanStatus::Defaulted || loan.balance <= Money::ZERO {
        return Money::ZERO;
    }
    let borrower_id = loan.borrower_dynasty_id;
    let lender_id = loan.lender_dynasty_id;
    let balance = loan.balance;
    let borrower_treasury = state
        .dynasties
        .get(&borrower_id)
        .expect("judgment borrower must exist")
        .treasury();
    let owns_property = state
        .properties
        .values()
        .any(|property| property.owner_dynasty_id == Some(borrower_id));
    let has_operating_business = state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == borrower_id && business.status() == BusinessStatus::Active
    });
    if borrower_treasury > Money::ZERO || owns_property || has_operating_business {
        return Money::ZERO;
    }

    let loan = state
        .loans
        .get_mut(&loan_id)
        .expect("judgment loan must remain present");
    loan.balance = Money::ZERO;
    loan.missed_payments = 0;
    loan.collateral_property_id = None;
    loan.status = LoanStatus::WrittenOff;

    adjust_reliability_reputation(state, borrower_id, -1_200);
    let borrower = state
        .dynasties
        .get_mut(&borrower_id)
        .expect("judgment borrower must exist");
    borrower.resources.legitimacy_basis_points = borrower
        .resources
        .legitimacy_basis_points
        .saturating_sub(600);
    adjust_dynasty_relationship(
        state,
        lender_id,
        borrower_id,
        RelationshipDelta::new(-250, -120, 40, 350, -1),
    );
    remember_dynasty_interaction(
        state,
        lender_id,
        borrower_id,
        &format!(
            "Loan {loan_id} ended with an uncollectible deficiency of {balance}; the lender wrote it off after final judgment."
        ),
    );
    balance
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
///
/// The spike must outweigh routine calm-month recovery by a wide margin on
/// the risky routes: with the seeded 9-15% monthly spike chances, a typical
/// month drifts upward by roughly 150-360 basis points, so a standard
/// multi-year session can realistically push a route past the trade-
/// disruption detection threshold while calm years still heal it.
pub(crate) const ROUTE_DISRUPTION_SPIKE_MIN_BASIS_POINTS: u16 = 2_600;
pub(crate) const ROUTE_DISRUPTION_SPIKE_RANGE_BASIS_POINTS: u32 = 2_900;
/// Routine calm months remove this much accumulated route disruption.
pub(crate) const ROUTE_DISRUPTION_CALM_RECOVERY_BASIS_POINTS: u16 = 130;
/// Post-crisis healing removes this much accumulated route disruption.
pub(crate) const ROUTE_DISRUPTION_HEALING_BASIS_POINTS: u16 = 250;

/// The prince's levy is checked at this cadence, each check passing this
/// often. A standard three-year session should see roughly one demand in a
/// third of campaigns instead of the demand being effectively unreachable.
pub(crate) const NOBLE_DEMAND_CHECK_INTERVAL_DAYS: i64 = 360;
pub(crate) const NOBLE_DEMAND_CHANCE_BASIS_POINTS: u16 = 1_200;
