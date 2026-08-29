//! Private and municipal credit: loans, civic debts, weekly interest, and collateral seizure.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoanTerms {
    pub lender_dynasty_id: DynastyId,
    pub borrower_dynasty_id: DynastyId,
    pub principal: Money,
    pub weekly_payment: Money,
    pub interest_basis_points: u16,
    pub collateral_property_id: Option<PropertyId>,
}

/// Newest defaulted loan for one lender/borrower pair.
///
/// Restructuring is pair-owned: a default can only be cured through the
/// creditor that already owns that claim. Keeping the lookup in the credit
/// system prevents command negotiation, gameplay policy, and AI underwriting
/// from each reimplementing slightly different default semantics.
pub(crate) fn latest_defaulted_loan_for_pair(
    state: &AppState,
    lender_dynasty_id: DynastyId,
    borrower_dynasty_id: DynastyId,
) -> Option<&Loan> {
    state
        .loans
        .values()
        .filter(|loan| {
            loan.lender_dynasty_id == lender_dynasty_id
                && loan.borrower_dynasty_id == borrower_dynasty_id
                && loan.status == LoanStatus::Defaulted
        })
        .max_by_key(|loan| (loan.next_due_day, loan.id))
}

#[must_use]
pub(crate) fn defaulted_loan_restructuring_available(state: &AppState, loan: &Loan) -> bool {
    loan.status == LoanStatus::Defaulted
        && state.clock.day()
            >= loan
                .next_due_day
                .saturating_add(DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS)
}

#[must_use]
pub(crate) fn borrower_has_unresolved_default(
    state: &AppState,
    borrower_dynasty_id: DynastyId,
) -> bool {
    state.loans.values().any(|loan| {
        loan.borrower_dynasty_id == borrower_dynasty_id && loan.status == LoanStatus::Defaulted
    })
}

/// Defaulted claim held by somebody other than `proposed_lender_dynasty_id`.
///
/// A creditor may restructure its own claim even when the borrower has other
/// defaults, allowing a badly damaged house to work through creditors one at
/// a time. An unrelated lender uses this predicate to refuse fresh credit
/// while another house still owns an unresolved default.
pub(crate) fn unresolved_default_owed_elsewhere(
    state: &AppState,
    borrower_dynasty_id: DynastyId,
    proposed_lender_dynasty_id: DynastyId,
) -> Option<&Loan> {
    state
        .loans
        .values()
        .filter(|loan| {
            loan.borrower_dynasty_id == borrower_dynasty_id
                && loan.lender_dynasty_id != proposed_lender_dynasty_id
                && loan.status == LoanStatus::Defaulted
        })
        .min_by_key(|loan| (loan.next_due_day, loan.id))
}

#[must_use]
pub(crate) fn credit_pair_blocks_new_loan(
    state: &AppState,
    lender_dynasty_id: DynastyId,
    borrower_dynasty_id: DynastyId,
) -> bool {
    state.loans.values().any(|loan| {
        loan.lender_dynasty_id == lender_dynasty_id
            && loan.borrower_dynasty_id == borrower_dynasty_id
            && (loan.status.is_repayment_active()
                || (loan.status == LoanStatus::Defaulted
                    && !defaulted_loan_restructuring_available(state, loan)))
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DueLoan {
    pub(crate) id: crate::ids::LoanId,
    pub(crate) lender_id: DynastyId,
    pub(crate) borrower_id: DynastyId,
    pub(crate) weekly_payment: Money,
    pub(crate) balance: Money,
    pub(crate) interest_basis_points: u16,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct DueCivicDebt {
    pub(crate) id: CivicDebtId,
    pub(crate) creditor_dynasty_id: DynastyId,
    pub(crate) sponsor_dynasty_id: Option<DynastyId>,
    pub(crate) weekly_payment: Money,
    pub(crate) balance: Money,
    pub(crate) interest_basis_points: u16,
}
#[derive(Debug)]
pub struct ValidatedLoan {
    terms: LoanTerms,
    restructures_defaulted_loan: bool,
}
impl ValidatedLoan {
    /// Whether committing this loan restructures an existing defaulted loan in place
    /// instead of issuing a new loan record.
    pub fn restructures_defaulted_loan(&self) -> bool {
        self.restructures_defaulted_loan
    }

    /// Revalidates and commits a previously validated loan atomically.
    ///
    /// # Errors
    ///
    /// Returns the current validation error if state changed after the token was created, or an
    /// allocation or timeline error if durable loan feedback can no longer be recorded.
    pub fn commit(self, state: &mut AppState) -> Result<crate::ids::LoanId, StrategicError> {
        let defaulted_loan_id = validate_loan_terms(state, &self.terms)?;
        // Every durable identifier and schedule this commit consumes is
        // reserved up front (the allocator snapshot is restored on failure),
        // so the mutation phase below is infallible. That removes the
        // defensive whole-campaign copy a mid-mutation failure would
        // otherwise need for rollback.
        let reserved = reserve_loan_commit(state, &self.terms, defaulted_loan_id)?;
        Ok(commit_loan_reserved(state, &self.terms, reserved))
    }
}
/// Durable identifiers and schedule results a loan commit will consume.
#[derive(Clone, Copy)]
pub(crate) struct ReservedLoanCommit {
    /// New loan identifier, or the restructured defaulted loan's identifier.
    loan_id: crate::ids::LoanId,
    /// Whether `loan_id` targets an existing defaulted loan being
    /// restructured rather than a freshly issued record.
    restructured: bool,
    next_due_day: i64,
    outbox_id: crate::ids::OutboxMessageId,
    counterparty_report: Option<ReservedCounterpartyReport>,
}
/// Reserves, in the exact order [`commit_loan_reserved`] consumes them, every
/// durable identifier the loan commit needs. Nothing has mutated when a
/// reservation fails, so the allocator snapshot alone restores state.
pub(crate) fn reserve_loan_commit(
    state: &mut AppState,
    terms: &LoanTerms,
    defaulted_loan_id: Option<crate::ids::LoanId>,
) -> Result<ReservedLoanCommit, StrategicError> {
    let day = state.clock.day();
    let next_due_day = checked_future_day(day, 7)?;
    // Mirrors `reserve_counterparty_report`'s targeting: loans between two AI
    // houses never touch the player's intelligence ledger.
    let report_is_due = (terms.lender_dynasty_id == state.player_dynasty_id)
        != (terms.borrower_dynasty_id == state.player_dynasty_id);
    let ids_before = state.next_ids.clone();
    let reservation = (|| -> Result<ReservedLoanCommit, StrategicError> {
        let (loan_id, restructured) = match defaulted_loan_id {
            Some(id) => (id, true),
            None => (state.next_ids.try_loan()?, false),
        };
        let outbox_id = state.next_ids.try_outbox()?;
        let counterparty_report = if report_is_due {
            Some(ReservedCounterpartyReport {
                id: state.next_ids.try_information_report()?,
                expires_day: checked_future_day(day, COUNTERPARTY_REPORT_EXPIRY_DAYS)?,
            })
        } else {
            None
        };
        Ok(ReservedLoanCommit {
            loan_id,
            restructured,
            next_due_day,
            outbox_id,
            counterparty_report,
        })
    })();
    match reservation {
        Ok(reserved) => Ok(reserved),
        Err(error) => {
            state.next_ids = ids_before;
            Err(error)
        }
    }
}

/// Applies a validated loan with every durable identifier pre-reserved.
///
/// Infallible by construction: reservation has already consumed the loan,
/// outbox, and counterparty-report identifiers and resolved the schedule, so
/// no step below can fail.
#[expect(
    clippy::too_many_lines,
    reason = "the commit keeps every ledger, index, and feedback write in one auditable sequence"
)]
pub(crate) fn commit_loan_reserved(
    state: &mut AppState,
    terms: &LoanTerms,
    reserved: ReservedLoanCommit,
) -> crate::ids::LoanId {
    let ReservedLoanCommit {
        loan_id: id,
        restructured,
        next_due_day,
        outbox_id,
        counterparty_report,
    } = reserved;
    let &LoanTerms {
        lender_dynasty_id,
        borrower_dynasty_id,
        principal,
        collateral_property_id,
        ..
    } = terms;
    let lender = state
        .dynasties
        .get_mut(&lender_dynasty_id)
        .expect("validated lender must exist");
    lender.resources.treasury = lender
        .resources
        .treasury
        .checked_sub(principal)
        .expect("revalidated lender treasury must cover the principal");
    let borrower = state
        .dynasties
        .get_mut(&borrower_dynasty_id)
        .expect("validated borrower must exist");
    borrower.resources.treasury = borrower
        .resources
        .treasury
        .checked_add(principal)
        .expect("revalidated borrower treasury must fit the supported range");
    if let Some(property_id) = collateral_property_id {
        state
            .properties
            .get_mut(&property_id)
            .expect("validated collateral must exist")
            .collateral_loan_id = Some(id);
    }
    commit_loan_record(
        state,
        terms,
        id,
        if restructured { Some(id) } else { None },
        next_due_day,
    );
    let lender_name = state
        .dynasties
        .get(&lender_dynasty_id)
        .expect("validated lender must exist")
        .name()
        .to_owned();
    let borrower_name = state
        .dynasties
        .get(&borrower_dynasty_id)
        .expect("validated borrower must exist")
        .name()
        .to_owned();
    state.outbox.push(OutboxMessage {
        id: outbox_id,
        day: state.clock.day(),
        kind: OutboxKind::Finance,
        subject: if restructured {
            format!("Loan {id} restructured")
        } else {
            format!("Loan {id} issued")
        },
        body: if restructured {
            if principal > Money::ZERO {
                format!(
                    "House {lender_name} restructured loan {id} and advanced {principal} to House {borrower_name}."
                )
            } else {
                format!(
                    "House {lender_name} restructured loan {id} for House {borrower_name} on revised repayment terms without increasing the debt."
                )
            }
        } else {
            format!("House {lender_name} lent {principal} to House {borrower_name}.")
        },
        acknowledged: false,
    });
    adjust_dynasty_relationship(
        state,
        lender_dynasty_id,
        borrower_dynasty_id,
        RelationshipDelta::new(60, 40, 0, -10, 1),
    );
    remember_dynasty_interaction(
        state,
        lender_dynasty_id,
        borrower_dynasty_id,
        &if restructured {
            if principal > Money::ZERO {
                format!("Loan {id} was restructured with a {principal} advance.")
            } else {
                format!("Loan {id} was restructured without increasing principal.")
            }
        } else {
            format!("Loan {id} was issued for {principal}.")
        },
    );
    if let Some(report) = counterparty_report {
        emit_counterparty_report(
            state,
            report,
            lender_dynasty_id,
            borrower_dynasty_id,
            "Credit underwriting and repayment records",
        );
    }
    id
}
pub(crate) fn commit_loan_record(
    state: &mut AppState,
    terms: &LoanTerms,
    id: crate::ids::LoanId,
    defaulted_loan_id: Option<crate::ids::LoanId>,
    next_due_day: i64,
) {
    if let Some(defaulted_loan_id) = defaulted_loan_id {
        let prior_collateral = state
            .loans
            .get(&defaulted_loan_id)
            .and_then(|loan| loan.collateral_property_id);
        if prior_collateral != terms.collateral_property_id
            && let Some(property_id) = prior_collateral
            && let Some(property) = state.properties.get_mut(&property_id)
            && property.collateral_loan_id == Some(defaulted_loan_id)
        {
            property.collateral_loan_id = None;
        }
        let loan = state
            .loans
            .get_mut(&defaulted_loan_id)
            .expect("validated defaulted loan must exist");
        loan.principal = loan
            .principal
            .checked_add(terms.principal)
            .expect("revalidated loan principal must fit the supported range");
        loan.balance = loan
            .balance
            .checked_add(terms.principal)
            .expect("revalidated loan balance must fit the supported range");
        loan.weekly_payment = terms.weekly_payment;
        loan.interest_basis_points = terms.interest_basis_points;
        loan.next_due_day = next_due_day;
        loan.missed_payments = 0;
        loan.collateral_property_id = terms.collateral_property_id;
        loan.status = LoanStatus::Restructured;
    } else {
        state.loans.insert(
            id,
            Loan {
                id,
                lender_dynasty_id: terms.lender_dynasty_id,
                borrower_dynasty_id: terms.borrower_dynasty_id,
                principal: terms.principal,
                balance: terms.principal,
                weekly_payment: terms.weekly_payment,
                interest_basis_points: terms.interest_basis_points,
                next_due_day,
                missed_payments: 0,
                collateral_property_id: terms.collateral_property_id,
                status: LoanStatus::Current,
            },
        );
    }
}
/// Validates a loan without mutating state.
///
/// # Errors
///
/// Returns an error for missing parties, invalid terms, insufficient lender funds, or invalid collateral.
pub fn validate_loan(state: &AppState, terms: LoanTerms) -> Result<ValidatedLoan, StrategicError> {
    let defaulted_loan_id = validate_loan_terms(state, &terms)?;
    Ok(ValidatedLoan {
        restructures_defaulted_loan: defaulted_loan_id.is_some(),
        terms,
    })
}
pub(crate) fn validate_loan_terms(
    state: &AppState,
    terms: &LoanTerms,
) -> Result<Option<crate::ids::LoanId>, StrategicError> {
    if terms.lender_dynasty_id == terms.borrower_dynasty_id {
        return Err(StrategicError::SameLoanParty);
    }
    // A restructuring may change repayment terms without forcing the
    // defaulting house to borrow even more. Fresh loans still require a
    // positive principal, and negative principal is never meaningful.
    if terms.principal < Money::ZERO || terms.weekly_payment <= Money::ZERO {
        return Err(StrategicError::NonPositiveAmount);
    }
    if terms.interest_basis_points > 10_000 {
        return Err(StrategicError::InterestOutOfRange {
            interest_basis_points: terms.interest_basis_points,
        });
    }
    checked_future_day(state.clock.day(), 7)?;
    let lender =
        state
            .dynasties
            .get(&terms.lender_dynasty_id)
            .ok_or(StrategicError::MissingDynasty {
                dynasty_id: terms.lender_dynasty_id,
            })?;
    let borrower =
        state
            .dynasties
            .get(&terms.borrower_dynasty_id)
            .ok_or(StrategicError::MissingDynasty {
                dynasty_id: terms.borrower_dynasty_id,
            })?;
    if borrower.treasury().checked_add(terms.principal).is_none() {
        return Err(StrategicError::DynastyTreasuryOverflow {
            dynasty_id: terms.borrower_dynasty_id,
            current: borrower.treasury(),
            incoming: terms.principal,
        });
    }
    if let Some(existing) = state.loans.values().find(|loan| {
        loan.lender_dynasty_id == terms.lender_dynasty_id
            && loan.borrower_dynasty_id == terms.borrower_dynasty_id
            && loan.status.is_repayment_active()
    }) {
        return Err(StrategicError::ExistingUnsettledLoan {
            lender_dynasty_id: terms.lender_dynasty_id,
            borrower_dynasty_id: terms.borrower_dynasty_id,
            loan_id: existing.id,
        });
    }
    let defaulted_loan_id = validate_defaulted_loan_restructuring(state, terms)?;
    if terms.principal == Money::ZERO && defaulted_loan_id.is_none() {
        return Err(StrategicError::NonPositiveAmount);
    }
    if lender.treasury() < terms.principal {
        return Err(StrategicError::InsufficientDynastyFunds {
            dynasty_id: terms.lender_dynasty_id,
            available: lender.treasury(),
            required: terms.principal,
        });
    }
    if let Some(property_id) = terms.collateral_property_id {
        let property = state
            .properties
            .get(&property_id)
            .ok_or(StrategicError::MissingProperty { property_id })?;
        if property.owner_dynasty_id != Some(terms.borrower_dynasty_id) {
            return Err(StrategicError::CollateralNotOwned {
                property_id,
                borrower_dynasty_id: terms.borrower_dynasty_id,
            });
        }
        if let Some(loan_id) = property.collateral_loan_id
            && Some(loan_id) != defaulted_loan_id
        {
            return Err(StrategicError::PropertyAlreadyPledged {
                property_id,
                loan_id,
            });
        }
    }
    Ok(defaulted_loan_id)
}
pub(crate) fn validate_defaulted_loan_restructuring(
    state: &AppState,
    terms: &LoanTerms,
) -> Result<Option<crate::ids::LoanId>, StrategicError> {
    let defaulted_loan =
        latest_defaulted_loan_for_pair(state, terms.lender_dynasty_id, terms.borrower_dynasty_id);
    let Some(defaulted_loan) = defaulted_loan else {
        return Ok(None);
    };
    let available_day = checked_future_day(
        defaulted_loan.next_due_day,
        DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS,
    )?;
    if state.clock.day() < available_day {
        return Err(StrategicError::DefaultedLoanRestructuringCooldown {
            loan_id: defaulted_loan.id,
            available_day,
        });
    }
    if defaulted_loan
        .balance
        .checked_add(terms.principal)
        .is_none()
        || defaulted_loan
            .principal
            .checked_add(terms.principal)
            .is_none()
    {
        return Err(StrategicError::LoanBalanceOverflow {
            loan_id: defaulted_loan.id,
            current: defaulted_loan.balance,
            incoming: terms.principal,
        });
    }
    Ok(Some(defaulted_loan.id))
}
/// Validates and issues a loan through its canonical commit token.
///
/// # Errors
///
/// Returns the same errors as [`validate_loan`], plus allocation or timeline exhaustion while
/// committing the loan and its durable feedback.
#[cfg(test)]
pub(crate) fn issue_loan(
    state: &mut AppState,
    terms: LoanTerms,
) -> Result<crate::ids::LoanId, StrategicError> {
    validate_loan(state, terms)?.commit(state)
}
pub(crate) fn record_completed_loan_repayment(
    state: &mut AppState,
    lender_dynasty_id: DynastyId,
    borrower_dynasty_id: DynastyId,
    loan_id: crate::ids::LoanId,
) -> Result<(), DurableFeedbackError> {
    adjust_dynasty_relationship(
        state,
        lender_dynasty_id,
        borrower_dynasty_id,
        RelationshipDelta::new(30, 20, 0, -25, -1),
    );
    remember_dynasty_interaction(
        state,
        lender_dynasty_id,
        borrower_dynasty_id,
        &format!("Loan {loan_id} was repaid in full."),
    );
    try_record_counterparty_information(
        state,
        lender_dynasty_id,
        borrower_dynasty_id,
        "Completed loan repayment records",
    )?;
    Ok(())
}
pub(crate) fn settle_loans(state: &mut AppState) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let interest_limit = active_interest_limit(state);
    let due: Vec<_> = state
        .loans
        .values()
        .filter(|loan| loan.status.is_repayment_active() && loan.next_due_day <= day)
        .map(|loan| DueLoan {
            id: loan.id,
            lender_id: loan.lender_dynasty_id,
            borrower_id: loan.borrower_dynasty_id,
            weekly_payment: loan.weekly_payment,
            balance: loan.balance,
            interest_basis_points: loan.interest_basis_points,
        })
        .collect();
    for due_loan in due {
        settle_due_loan(state, due_loan, interest_limit)?;
    }
    Ok(())
}
pub(crate) fn settle_civic_debts(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let Some(treasury_id) = registry.get_institution_id("treasury") else {
        return Ok(());
    };
    let day = state.clock.day();
    let interest_limit = active_interest_limit(state);
    let due: Vec<_> = state
        .civic_debts
        .values()
        .filter(|debt| {
            matches!(
                debt.status,
                CivicDebtStatus::Current | CivicDebtStatus::Delinquent
            ) && debt.next_due_day <= day
        })
        .map(|debt| DueCivicDebt {
            id: debt.id,
            creditor_dynasty_id: debt.creditor_dynasty_id,
            sponsor_dynasty_id: debt.sponsor_dynasty_id,
            weekly_payment: debt.weekly_payment,
            balance: debt.balance,
            interest_basis_points: debt.interest_basis_points,
        })
        .collect();
    for due_debt in due {
        settle_due_civic_debt(state, treasury_id, due_debt, interest_limit)?;
    }
    Ok(())
}
pub(crate) fn settle_due_civic_debt(
    state: &mut AppState,
    treasury_id: InstitutionId,
    due: DueCivicDebt,
    interest_limit: Option<u16>,
) -> Result<(), SimulationError> {
    let effective_interest = interest_limit.map_or(due.interest_basis_points, |limit| {
        due.interest_basis_points.min(limit)
    });
    let interest_due = weekly_interest_due(due.balance, effective_interest);
    let accrued_balance =
        due.balance
            .checked_add(interest_due)
            .ok_or(SimulationError::CivicDebtBalanceOverflow {
                civic_debt_id: due.id,
                current: due.balance,
                incoming: interest_due,
            })?;
    let amount_due = due.weekly_payment.min(accrued_balance);
    let treasury_budget = state
        .institutions
        .get(&treasury_id)
        .expect("civic treasury must exist")
        .budget;
    let remaining_balance = accrued_balance
        .checked_sub(amount_due)
        .expect("civic debt payment cannot exceed accrued balance");
    // The same rule the private-loan machinery enforces: a payment that
    // cannot cover more than the week's interest never reduces the balance,
    // producing a debt that is mathematically unrepayable yet perpetually
    // "current". Such an installment counts as missed so the delinquency and
    // default machinery handles unsustainable terms instead of collecting
    // interest forever.
    let payment_is_productive = remaining_balance == Money::ZERO || amount_due > interest_due;
    let payable = treasury_budget >= amount_due && payment_is_productive;
    if payable {
        let creditor_treasury = state
            .dynasties
            .get(&due.creditor_dynasty_id)
            .expect("civic debt creditor must exist")
            .treasury();
        creditor_treasury.checked_add(amount_due).ok_or(
            SimulationError::DynastyTreasuryOverflow {
                dynasty_id: due.creditor_dynasty_id,
                current: creditor_treasury,
                incoming: amount_due,
            },
        )?;
    }
    let next_due_day = {
        let debt = state
            .civic_debts
            .get(&due.id)
            .expect("civic debt must exist");
        if payable {
            if remaining_balance == Money::ZERO {
                None
            } else {
                Some(checked_future_day(debt.next_due_day, 7)?)
            }
        } else if debt.missed_payments.saturating_add(1) >= 3 {
            None
        } else {
            Some(checked_future_day(debt.next_due_day, 7)?)
        }
    };
    state
        .civic_debts
        .get_mut(&due.id)
        .expect("civic debt must exist")
        .balance = accrued_balance;
    if payable {
        settle_successful_civic_debt_payment(state, treasury_id, due, amount_due, next_due_day)?;
    } else {
        settle_missed_civic_debt_payment(state, treasury_id, due, next_due_day)?;
    }
    Ok(())
}
pub(crate) fn settle_successful_civic_debt_payment(
    state: &mut AppState,
    treasury_id: InstitutionId,
    due: DueCivicDebt,
    payment: Money,
    next_due_day: Option<i64>,
) -> Result<(), SimulationError> {
    let remaining_balance = {
        let debt = state
            .civic_debts
            .get(&due.id)
            .expect("civic debt must exist");
        debt.balance
            .checked_sub(payment)
            .expect("validated civic debt payment must not exceed debt balance")
    };
    {
        let treasury = state
            .institutions
            .get_mut(&treasury_id)
            .expect("civic treasury must exist");
        treasury.budget = treasury
            .budget
            .checked_sub(payment)
            .expect("validated civic debt payment must not exceed treasury budget");
    }
    {
        let creditor = state
            .dynasties
            .get_mut(&due.creditor_dynasty_id)
            .expect("civic debt creditor must exist");
        creditor.resources.treasury = creditor
            .resources
            .treasury
            .checked_add(payment)
            .expect("prevalidated civic debt payment must fit creditor treasury");
    }
    let repaid = {
        let debt = state
            .civic_debts
            .get_mut(&due.id)
            .expect("civic debt must exist");
        debt.balance = remaining_balance;
        if let Some(next_due_day) = next_due_day {
            debt.next_due_day = next_due_day;
        }
        debt.missed_payments = 0;
        if debt.balance == Money::ZERO {
            debt.status = CivicDebtStatus::Repaid;
            true
        } else {
            debt.status = CivicDebtStatus::Current;
            false
        }
    };
    let treasury = state
        .institutions
        .get_mut(&treasury_id)
        .expect("civic treasury must exist");
    treasury.legitimacy_basis_points = treasury
        .legitimacy_basis_points
        .saturating_add(if repaid { 100 } else { 10 })
        .min(10_000);
    if let Some(sponsor_dynasty_id) = due.sponsor_dynasty_id {
        adjust_dynasty_relationship(
            state,
            sponsor_dynasty_id,
            due.creditor_dynasty_id,
            RelationshipDelta::new(
                if repaid { 30 } else { 3 },
                10,
                0,
                -5,
                if repaid { -1 } else { 0 },
            ),
        );
        if repaid {
            remember_dynasty_interaction(
                state,
                sponsor_dynasty_id,
                due.creditor_dynasty_id,
                &format!("Civic debt {} was repaid in full.", due.id),
            );
            try_record_counterparty_information(
                state,
                sponsor_dynasty_id,
                due.creditor_dynasty_id,
                "Completed municipal debt repayment records",
            )?;
        }
    }
    if repaid {
        try_push_outbox(
            state,
            OutboxKind::Finance,
            format!("Civic debt {} repaid", due.id),
            format!(
                "The city treasury repaid dynasty {} in full.",
                due.creditor_dynasty_id
            ),
        )?;
    }
    Ok(())
}
pub(crate) fn settle_missed_civic_debt_payment(
    state: &mut AppState,
    treasury_id: InstitutionId,
    due: DueCivicDebt,
    next_due_day: Option<i64>,
) -> Result<(), SimulationError> {
    let missed_payments = {
        let debt = state
            .civic_debts
            .get(&due.id)
            .expect("civic debt must exist");
        debt.missed_payments.saturating_add(1)
    };
    let defaulted = {
        let debt = state
            .civic_debts
            .get_mut(&due.id)
            .expect("civic debt must exist");
        debt.missed_payments = missed_payments;
        if let Some(next_due_day) = next_due_day {
            debt.next_due_day = next_due_day;
        }
        debt.status = if debt.missed_payments >= 3 {
            CivicDebtStatus::Defaulted
        } else {
            CivicDebtStatus::Delinquent
        };
        debt.status == CivicDebtStatus::Defaulted
    };
    // Unlike a defaulted private loan, a defaulted civic debt is terminal: the
    // creditor's principal is extinguished with only these one-time legitimacy
    // and unrest consequences. This asymmetry is deliberate — municipal
    // default is a political event resolved at the ballot and in the street,
    // not a restructuring case for the Civic Court, so no grounded claim
    // source exists for civic debts.
    let treasury = state
        .institutions
        .get_mut(&treasury_id)
        .expect("civic treasury must exist");
    treasury.legitimacy_basis_points = treasury
        .legitimacy_basis_points
        .saturating_sub(if defaulted { 500 } else { 100 });
    for district in state.districts.values_mut() {
        district.unrest_basis_points = district
            .unrest_basis_points
            .saturating_add(if defaulted { 200 } else { 25 })
            .min(10_000);
    }
    if let Some(sponsor_dynasty_id) = due.sponsor_dynasty_id {
        let sponsor = state
            .dynasties
            .get_mut(&sponsor_dynasty_id)
            .expect("civic debt sponsor must exist");
        sponsor.resources.legitimacy_basis_points = sponsor
            .resources
            .legitimacy_basis_points
            .saturating_sub(if defaulted { 300 } else { 40 });
        adjust_dynasty_relationship(
            state,
            sponsor_dynasty_id,
            due.creditor_dynasty_id,
            RelationshipDelta::new(
                if defaulted { -180 } else { -30 },
                if defaulted { -80 } else { -10 },
                if defaulted { 40 } else { 0 },
                if defaulted { 250 } else { 40 },
                if defaulted { -1 } else { 0 },
            ),
        );
        if defaulted {
            remember_dynasty_interaction(
                state,
                sponsor_dynasty_id,
                due.creditor_dynasty_id,
                &format!("Civic debt {} defaulted.", due.id),
            );
            try_record_counterparty_information(
                state,
                sponsor_dynasty_id,
                due.creditor_dynasty_id,
                "Municipal debt default and civic treasury records",
            )?;
        }
    }
    if defaulted {
        try_push_outbox(
            state,
            OutboxKind::Finance,
            format!("Civic debt {} defaulted", due.id),
            format!(
                "The city treasury defaulted on its obligation to dynasty {}.",
                due.creditor_dynasty_id
            ),
        )?;
    }
    Ok(())
}
pub(crate) fn settle_due_loan(
    state: &mut AppState,
    due: DueLoan,
    interest_limit: Option<u16>,
) -> Result<(), SimulationError> {
    let effective_interest = interest_limit.map_or(due.interest_basis_points, |limit| {
        due.interest_basis_points.min(limit)
    });
    let interest_due = weekly_interest_due(due.balance, effective_interest);
    let accrued_balance =
        due.balance
            .checked_add(interest_due)
            .ok_or(SimulationError::LoanBalanceOverflow {
                loan_id: due.id,
                current: due.balance,
                incoming: interest_due,
            })?;
    let amount_due = due.weekly_payment.min(accrued_balance);
    let borrower_treasury = state
        .dynasties
        .get(&due.borrower_id)
        .expect("loan borrower must exist")
        .treasury();
    let remaining_balance = accrued_balance.saturating_sub(amount_due);
    // A payment that cannot cover more than the week's interest never
    // reduces the balance, producing a debt that is mathematically unrepayable
    // yet perpetually "current" — collecting interest forever with collateral
    // locked away. Such an installment counts as missed so the delinquency
    // and default machinery handles unsustainable terms.
    let payment_is_productive = remaining_balance == Money::ZERO || amount_due > interest_due;
    let payable = borrower_treasury >= amount_due && payment_is_productive;
    if payable {
        let lender_treasury = state
            .dynasties
            .get(&due.lender_id)
            .expect("loan lender must exist")
            .treasury();
        lender_treasury.checked_add(amount_due).ok_or(
            SimulationError::DynastyTreasuryOverflow {
                dynasty_id: due.lender_id,
                current: lender_treasury,
                incoming: amount_due,
            },
        )?;
    }
    let next_due_day = {
        let loan = state.loans.get(&due.id).expect("loan must exist");
        if payable {
            let remaining_balance = accrued_balance
                .checked_sub(amount_due)
                .expect("loan payment cannot exceed accrued balance");
            if remaining_balance == Money::ZERO {
                None
            } else {
                Some(checked_future_day(loan.next_due_day, 7)?)
            }
        } else if loan.missed_payments.saturating_add(1) >= 3 {
            None
        } else {
            Some(checked_future_day(loan.next_due_day, 7)?)
        }
    };
    state
        .loans
        .get_mut(&due.id)
        .expect("loan must exist")
        .balance = accrued_balance;
    if payable {
        settle_successful_loan_payment(state, due, amount_due, next_due_day)?;
    } else {
        settle_missed_loan_payment(state, due, next_due_day)?;
    }
    Ok(())
}
pub(crate) fn settle_successful_loan_payment(
    state: &mut AppState,
    due: DueLoan,
    amount_due: Money,
    next_due_day: Option<i64>,
) -> Result<(), SimulationError> {
    apply_loan_payment(state, due.id, amount_due)?;
    let loan = state.loans.get_mut(&due.id).expect("loan must exist");
    if let Some(next_due_day) = next_due_day {
        loan.next_due_day = next_due_day;
    }
    loan.missed_payments = 0;
    if !loan.status.is_settled() {
        loan.status = LoanStatus::Current;
    }
    adjust_reliability_reputation(state, due.borrower_id, 10);
    Ok(())
}
pub(crate) fn settle_missed_loan_payment(
    state: &mut AppState,
    due: DueLoan,
    next_due_day: Option<i64>,
) -> Result<(), SimulationError> {
    let missed_payments = {
        let loan = state.loans.get(&due.id).expect("loan must exist");
        loan.missed_payments.saturating_add(1)
    };
    let defaulted = {
        let loan = state.loans.get_mut(&due.id).expect("loan must exist");
        loan.missed_payments = missed_payments;
        if let Some(next_due_day) = next_due_day {
            loan.next_due_day = next_due_day;
        }
        loan.status = if loan.missed_payments >= 3 {
            LoanStatus::Defaulted
        } else {
            LoanStatus::Delinquent
        };
        loan.status == LoanStatus::Defaulted
    };
    if defaulted {
        let CollateralSeizure {
            recovery: collateral_recovery,
            equity_returned,
            equity_withheld,
        } = seize_defaulted_collateral(state, due);
        let remaining_balance = state
            .loans
            .get(&due.id)
            .expect("defaulted loan must exist")
            .balance;
        let mut surplus_note = String::new();
        if equity_returned > Money::ZERO {
            let _ = write!(
                surplus_note,
                " Surplus collateral equity of {equity_returned} was returned to the borrower."
            );
        }
        if equity_withheld > Money::ZERO {
            // The seizing lender could not fund the full surplus, so it kept a
            // windfall above its claim; the borrower remembers the grievance.
            let _ = write!(
                surplus_note,
                " Surplus collateral equity of {equity_withheld} was withheld because the lender lacked treasury funds."
            );
            adjust_dynasty_relationship(
                state,
                due.borrower_id,
                due.lender_id,
                RelationshipDelta::new(-150, -100, 0, 250, 0),
            );
        }
        try_push_outbox(
            state,
            OutboxKind::Finance,
            format!("Loan {} defaulted", due.id),
            format!(
                "Dynasty {} defaulted on its obligation to dynasty {}. Collateral recovered {}; remaining balance {}.{}",
                due.borrower_id,
                due.lender_id,
                collateral_recovery,
                remaining_balance,
                surplus_note
            ),
        )?;
    }
    adjust_reliability_reputation(state, due.borrower_id, if defaulted { -400 } else { -60 });
    adjust_dynasty_relationship(
        state,
        due.lender_id,
        due.borrower_id,
        RelationshipDelta::new(
            if defaulted { -180 } else { -40 },
            if defaulted { -80 } else { -10 },
            if defaulted { 50 } else { 0 },
            if defaulted { 250 } else { 50 },
            if defaulted { -1 } else { 0 },
        ),
    );
    if defaulted {
        remember_dynasty_interaction(
            state,
            due.lender_id,
            due.borrower_id,
            &format!("Loan {} defaulted.", due.id),
        );
        try_record_counterparty_information(
            state,
            due.lender_id,
            due.borrower_id,
            "Loan default and collateral records",
        )?;
    }
    Ok(())
}
/// Outcome of seizing defaulted collateral.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CollateralSeizure {
    /// Balance cancelled by the collateral's liquidation value.
    pub(crate) recovery: Money,
    /// Liquidation equity above the settled debt that was actually returned
    /// to the borrower in cash.
    pub(crate) equity_returned: Money,
    /// Liquidation equity above the settled debt that the lender's treasury
    /// could not fund. The lender keeps the corresponding windfall, so it is
    /// recorded as a grievance against them rather than silently vanishing.
    pub(crate) equity_withheld: Money,
}
impl CollateralSeizure {
    const fn none() -> Self {
        Self {
            recovery: Money::ZERO,
            equity_returned: Money::ZERO,
            equity_withheld: Money::ZERO,
        }
    }
}
pub(crate) fn seize_defaulted_collateral(state: &mut AppState, due: DueLoan) -> CollateralSeizure {
    let defaulted = state
        .loans
        .get(&due.id)
        .is_some_and(|loan| loan.status == LoanStatus::Defaulted);
    if defaulted {
        seize_pledged_collateral(state, due.id)
    } else {
        CollateralSeizure::none()
    }
}
/// Executes recovery against a loan's still-pledged collateral: ownership of
/// the premises passes to the lender, its liquidation value credits the
/// balance, and any surplus equity flows back to the borrower. Shared by the
/// extrajudicial default-seizure path and the court-judgment execution path so
/// both keep identical accounting and equity handling.
pub(crate) fn seize_pledged_collateral(
    state: &mut AppState,
    loan_id: crate::ids::LoanId,
) -> CollateralSeizure {
    let (lender_id, property_id, balance) = {
        let Some(loan) = state.loans.get(&loan_id) else {
            return CollateralSeizure::none();
        };
        match loan.collateral_property_id {
            Some(property_id) => (loan.lender_dynasty_id, property_id, loan.balance),
            None => return CollateralSeizure::none(),
        }
    };
    if balance <= Money::ZERO {
        return CollateralSeizure::none();
    }
    let pledged = state
        .properties
        .get(&property_id)
        .is_some_and(|property| property.collateral_loan_id == Some(loan_id));
    if !pledged {
        return CollateralSeizure::none();
    }
    let (occupant_owner_id, existing_tenant_id) = {
        let property = state
            .properties
            .get(&property_id)
            .expect("loan collateral must exist");
        let occupant_owner_id = property.occupant_business_id.map(|business_id| {
            state
                .businesses
                .get(business_id)
                .expect("collateral occupant business must exist")
                .owner_dynasty_id()
        });
        (occupant_owner_id, property.tenant_dynasty_id)
    };
    let (liquidation_value, equity_surplus) = {
        let property = state
            .properties
            .get_mut(&property_id)
            .expect("loan collateral must exist");
        property.owner_dynasty_id = Some(lender_id);
        property.tenant_dynasty_id = occupant_owner_id
            .or(existing_tenant_id)
            .filter(|tenant_id| *tenant_id != lender_id);
        property.collateral_loan_id = None;
        let liquidation_value = property
            .value
            .saturating_mul_ratio(PROPERTY_LIQUIDATION_BASIS_POINTS, 10_000);
        (liquidation_value, liquidation_value.saturating_sub(balance))
    };
    let (equity_returned, equity_withheld) = if equity_surplus > Money::ZERO {
        return_collateral_equity_surplus(state, loan_id, equity_surplus)
    } else {
        (Money::ZERO, Money::ZERO)
    };
    let credited = liquidation_value.min(balance);
    let loan = state
        .loans
        .get_mut(&loan_id)
        .expect("judgment loan must exist");
    loan.balance = loan
        .balance
        .checked_sub(credited)
        .expect("collateral recovery must not exceed the judgment balance");
    if loan.balance == Money::ZERO {
        loan.status = LoanStatus::Repaid;
        loan.missed_payments = 0;
    }
    CollateralSeizure {
        recovery: credited,
        equity_returned,
        equity_withheld,
    }
}
/// Pays collateral liquidation equity above the settled debt back to the
/// borrower. The payment is bounded by the lender's treasury: whatever the
/// lender cannot fund is reported back so it can be recorded as a grievance
/// instead of silently enriching the lender beyond its claim.
pub(crate) fn return_collateral_equity_surplus(
    state: &mut AppState,
    loan_id: crate::ids::LoanId,
    surplus: Money,
) -> (Money, Money) {
    let (lender_id, borrower_id) = {
        let loan = state
            .loans
            .get(&loan_id)
            .expect("defaulted loan must exist");
        (loan.lender_dynasty_id, loan.borrower_dynasty_id)
    };
    let lender_treasury = state
        .dynasties
        .get(&lender_id)
        .expect("collateral lender must exist")
        .treasury();
    let paid = surplus.min(lender_treasury);
    if paid <= Money::ZERO {
        return (Money::ZERO, surplus);
    }
    let borrower_treasury = state
        .dynasties
        .get(&borrower_id)
        .expect("collateral borrower must exist")
        .treasury();
    let resulting_borrower = borrower_treasury
        .checked_add(paid)
        .expect("bounded collateral equity must fit borrower treasury");
    state
        .dynasties
        .get_mut(&borrower_id)
        .expect("collateral borrower must exist")
        .resources
        .treasury = resulting_borrower;
    let lender = state
        .dynasties
        .get_mut(&lender_id)
        .expect("collateral lender must exist");
    lender.resources.treasury = lender.resources.treasury.saturating_sub(paid);
    (paid, surplus.saturating_sub(paid))
}
pub(crate) fn active_interest_limit(state: &AppState) -> Option<u16> {
    active_law_value(state, LawKind::InterestLimit)
        .map(|value| u16::try_from(value.clamp(0, 10_000)).unwrap_or(10_000))
}
pub(crate) fn weekly_interest_due(balance: Money, annual_interest_basis_points: u16) -> Money {
    if balance <= Money::ZERO || annual_interest_basis_points == 0 {
        return Money::ZERO;
    }
    let annual_interest =
        balance.saturating_mul_ratio(i64::from(annual_interest_basis_points), 10_000);
    if annual_interest <= Money::ZERO {
        return Money::ZERO;
    }
    // The calendar year has 360 days settled on a global 7-day cadence, so the
    // weekly charge is one week's share of the annual interest (rounded up),
    // not a 52-week approximation of it.
    let scaled = i128::from(annual_interest.copper()) * 7;
    let weekly_copper = scaled / 360 + i128::from(scaled % 360 != 0);
    Money::from_copper(i64::try_from(weekly_copper).unwrap_or(i64::MAX))
}
pub(crate) fn apply_loan_payment(
    state: &mut AppState,
    loan_id: crate::ids::LoanId,
    amount: Money,
) -> Result<Money, SimulationError> {
    if amount <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let (lender_id, borrower_id, balance, collateral) = {
        let loan = state.loans.get(&loan_id).expect("loan must exist");
        (
            loan.lender_dynasty_id,
            loan.borrower_dynasty_id,
            loan.balance,
            loan.collateral_property_id,
        )
    };
    let payment = amount.min(balance);
    if payment <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let borrower_treasury = state
        .dynasties
        .get(&borrower_id)
        .expect("loan borrower must exist")
        .treasury();
    debug_assert!(
        borrower_treasury >= payment,
        "validated loan payment exceeds borrower treasury"
    );
    let lender_treasury = state
        .dynasties
        .get(&lender_id)
        .expect("loan lender must exist")
        .treasury();
    let lender_treasury_after =
        lender_treasury
            .checked_add(payment)
            .ok_or(SimulationError::DynastyTreasuryOverflow {
                dynasty_id: lender_id,
                current: lender_treasury,
                incoming: payment,
            })?;
    let borrower_treasury_after = borrower_treasury
        .checked_sub(payment)
        .expect("validated loan payment must not exceed borrower treasury");
    state
        .dynasties
        .get_mut(&borrower_id)
        .expect("loan borrower must exist")
        .resources
        .treasury = borrower_treasury_after;
    let lender = state
        .dynasties
        .get_mut(&lender_id)
        .expect("loan lender must exist");
    lender.resources.treasury = lender_treasury_after;
    let repaid = {
        let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
        loan.balance = loan
            .balance
            .checked_sub(payment)
            .expect("validated loan payment must not exceed loan balance");
        if loan.balance == Money::ZERO {
            loan.status = LoanStatus::Repaid;
            loan.missed_payments = 0;
            true
        } else {
            false
        }
    };
    if repaid
        && let Some(property_id) = collateral
        && let Some(property) = state.properties.get_mut(&property_id)
    {
        property.collateral_loan_id = None;
    }
    if repaid {
        record_completed_loan_repayment(state, lender_id, borrower_id, loan_id)?;
    } else {
        adjust_dynasty_relationship(
            state,
            lender_id,
            borrower_id,
            RelationshipDelta::new(4, 2, 0, -1, 0),
        );
    }
    Ok(payment)
}
