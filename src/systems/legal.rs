//! Grounded legal claims shared by player commands and autonomous rival behavior.

use crate::core::{AppState, LegalCaseKind, LegalClaimSource, LoanStatus};
use crate::ids::DynastyId;
use crate::money::Money;
use crate::registry::Registry;

pub(crate) const LEGAL_CASE_FILING_INTERVAL_DAYS: i64 = 90;
pub(crate) const LEGAL_CASE_FILING_COST: Money = Money::from_copper(300);
pub(crate) const LEGAL_CASE_HEARING_DELAY_DAYS: i64 = 60;
pub(crate) const LEGAL_DELINQUENT_DEBT_EVIDENCE_BASIS_POINTS: u16 = 7_500;
pub(crate) const LEGAL_DEFAULTED_DEBT_EVIDENCE_BASIS_POINTS: u16 = 9_000;
pub(crate) const LEGAL_CONTRACT_BREACH_EVIDENCE_BASIS_POINTS: u16 = 8_500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegalClaimQuote {
    pub defendant_dynasty_id: DynastyId,
    pub kind: LegalCaseKind,
    pub claim_source: LegalClaimSource,
    pub evidence_basis_points: u16,
    pub maximum_damages: Money,
    pub description: String,
}

pub(crate) fn is_valid_legal_hearing_day(filed_day: i64, hearing_day: i64) -> bool {
    hearing_day != i64::MAX
        && hearing_day
            .checked_sub(filed_day)
            .is_some_and(|delay| (0..=LEGAL_CASE_HEARING_DELAY_DAYS).contains(&delay))
}

/// Routes a filed case's filing fee into the Civic Court's budget, so the
/// plaintiff's debit keeps a credited counterparty instead of vanishing from
/// the economy. Callers must have already validated the fee with
/// [`court_filing_fee_headroom`] and debited the plaintiff, so this credit
/// cannot fail.
pub(crate) fn collect_court_filing_fee(registry: &Registry, state: &mut AppState) {
    let court_id = registry
        .get_institution_id("civic_court")
        .expect("civic court institution must be registered");
    let court = state
        .institutions
        .get_mut(&court_id)
        .expect("civic court runtime must exist");
    court.budget = court
        .budget
        .checked_add(LEGAL_CASE_FILING_COST)
        .expect("validated court filing fee must fit the court budget");
}

/// Validates that the Civic Court can accept one filing fee before any caller
/// mutates state, so filing rejects with a typed error instead of failing on a
/// credit after the plaintiff's debit.
pub(crate) fn court_filing_fee_headroom(
    registry: &Registry,
    state: &AppState,
) -> Result<(), super::SimulationError> {
    let court_id = registry
        .get_institution_id("civic_court")
        .expect("civic court institution must be registered");
    let court = state
        .institutions
        .get(&court_id)
        .expect("civic court runtime must exist");
    court.budget.checked_add(LEGAL_CASE_FILING_COST).ok_or(
        super::SimulationError::InstitutionBudgetOverflow {
            institution_id: court_id,
            current: court.budget,
            incoming: LEGAL_CASE_FILING_COST,
        },
    )?;
    Ok(())
}

pub(crate) fn quote_grounded_legal_claim(
    state: &AppState,
    plaintiff_dynasty_id: DynastyId,
    defendant_dynasty_id: DynastyId,
    kind: LegalCaseKind,
) -> Option<LegalClaimQuote> {
    if plaintiff_dynasty_id == defendant_dynasty_id
        || !state.dynasties.contains_key(&plaintiff_dynasty_id)
        || !state.dynasties.contains_key(&defendant_dynasty_id)
    {
        return None;
    }
    match kind {
        LegalCaseKind::Debt => state
            .loans
            .values()
            .filter(|loan| {
                loan.lender_dynasty_id == plaintiff_dynasty_id
                    && loan.borrower_dynasty_id == defendant_dynasty_id
                    && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
                    && legal_claim_source_unused(
                        state,
                        plaintiff_dynasty_id,
                        LegalClaimSource::Loan { loan_id: loan.id },
                    )
            })
            .max_by_key(|loan| {
                (
                    u8::from(loan.status == LoanStatus::Defaulted),
                    loan.balance,
                    loan.id,
                )
            })
            .map(|loan| {
                let evidence_basis_points = if loan.status == LoanStatus::Defaulted {
                    LEGAL_DEFAULTED_DEBT_EVIDENCE_BASIS_POINTS
                } else {
                    LEGAL_DELINQUENT_DEBT_EVIDENCE_BASIS_POINTS
                };
                LegalClaimQuote {
                    defendant_dynasty_id,
                    kind,
                    claim_source: LegalClaimSource::Loan { loan_id: loan.id },
                    evidence_basis_points,
                    maximum_damages: loan.balance,
                    description: format!(
                        "enforce {:?} loan {} with {} outstanding",
                        loan.status, loan.id, loan.balance
                    ),
                }
            }),
        LegalCaseKind::ContractBreach => state
            .contracts
            .values()
            .filter(|contract| {
                // Recoverable breach debt carries its own attributed
                // breacher/victim pair from the first attributable miss, so
                // the claim grounds on the debt rather than on the contract's
                // lifecycle status.
                contract.breaching_dynasty_id == Some(defendant_dynasty_id)
                    && contract.breach_victim_dynasty_id == Some(plaintiff_dynasty_id)
                    && contract.unpaid_breach_penalty > Money::ZERO
                    && legal_claim_source_unused(
                        state,
                        plaintiff_dynasty_id,
                        LegalClaimSource::Contract {
                            contract_id: contract.id,
                        },
                    )
            })
            .max_by_key(|contract| (contract.unpaid_breach_penalty, contract.id))
            .map(|contract| LegalClaimQuote {
                defendant_dynasty_id,
                kind,
                claim_source: LegalClaimSource::Contract {
                    contract_id: contract.id,
                },
                evidence_basis_points: LEGAL_CONTRACT_BREACH_EVIDENCE_BASIS_POINTS,
                maximum_damages: contract.unpaid_breach_penalty,
                description: format!(
                    "recover {} still unpaid on attributed supply contract {}",
                    contract.unpaid_breach_penalty, contract.id
                ),
            }),
    }
}

fn legal_claim_source_unused(
    state: &AppState,
    plaintiff_dynasty_id: DynastyId,
    claim_source: LegalClaimSource,
) -> bool {
    !state.legal_cases.values().any(|legal_case| {
        legal_case.plaintiff_dynasty_id == plaintiff_dynasty_id
            && legal_case.claim_source == Some(claim_source)
    })
}
