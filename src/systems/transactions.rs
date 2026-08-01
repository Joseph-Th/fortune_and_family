//! Validated multi-record transactions and shared simulation errors.

use crate::core::{AppState, AuditKind, AuditRecord, BusinessStatus};
use crate::ids::{BusinessId, GoodId};
use crate::money::Money;
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SimulationError {
    #[error("day count must be positive, received {days}")]
    InvalidDayCount { days: u32 },
    #[error("state scenario {state_scenario} does not match registry scenario {registry_scenario}")]
    RegistryMismatch {
        state_scenario: String,
        registry_scenario: String,
    },
    #[error("business {business_id} does not exist")]
    BusinessNotFound { business_id: BusinessId },
    #[error("business {business_id} is not active")]
    BusinessInactive { business_id: BusinessId },
    #[error("cash transfer source and target are both business {business_id}")]
    SameBusiness { business_id: BusinessId },
    #[error("cash transfer amount must be positive, received {amount}")]
    NonPositiveAmount { amount: Money },
    #[error("business {business_id} has {available}, below required transfer {required}")]
    InsufficientBusinessCash {
        business_id: BusinessId,
        available: Money,
        required: Money,
    },
    #[error("market quote is missing for good {good_id}")]
    MarketQuoteMissing { good_id: GoodId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidatedCashTransfer {
    from_business_id: BusinessId,
    to_business_id: BusinessId,
    amount: Money,
}

impl ValidatedCashTransfer {
    /// Commits a previously validated two-business cash transfer exactly once.
    ///
    /// # Panics
    ///
    /// Panics only when application state changed after validation and a validated business no
    /// longer exists.
    pub fn commit(self, state: &mut AppState) {
        let Self {
            from_business_id,
            to_business_id,
            amount,
        } = self;

        {
            let source = state
                .businesses
                .get_mut(from_business_id)
                .expect("validated transfer source must exist");
            source.finance.cash = source.finance.cash.saturating_sub(amount);
            source.finance.version = source.finance.version.saturating_add(1);
        }
        {
            let target = state
                .businesses
                .get_mut(to_business_id)
                .expect("validated transfer target must exist");
            target.finance.cash = target.finance.cash.saturating_add(amount);
            target.finance.version = target.finance.version.saturating_add(1);
        }

        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CashTransfer,
            subject: format!("business:{from_business_id}->business:{to_business_id}"),
            detail: format!("amount={}", amount.copper()),
        });
    }
}

/// Validates and commits a cash transfer between two active businesses.
///
/// # Errors
///
/// Returns a dedicated error for invalid amounts, missing or inactive businesses, identical
/// endpoints, or insufficient source cash.
pub fn transfer_business_cash(
    state: &mut AppState,
    from_business_id: BusinessId,
    to_business_id: BusinessId,
    amount: Money,
) -> Result<(), SimulationError> {
    let validated =
        validate_business_cash_transfer(state, from_business_id, to_business_id, amount)?;
    validated.commit(state);
    Ok(())
}

fn validate_business_cash_transfer(
    state: &AppState,
    from_business_id: BusinessId,
    to_business_id: BusinessId,
    amount: Money,
) -> Result<ValidatedCashTransfer, SimulationError> {
    if amount.copper() <= 0 {
        return Err(SimulationError::NonPositiveAmount { amount });
    }
    if from_business_id == to_business_id {
        return Err(SimulationError::SameBusiness {
            business_id: from_business_id,
        });
    }

    let source =
        state
            .businesses
            .get(from_business_id)
            .ok_or(SimulationError::BusinessNotFound {
                business_id: from_business_id,
            })?;
    let target = state
        .businesses
        .get(to_business_id)
        .ok_or(SimulationError::BusinessNotFound {
            business_id: to_business_id,
        })?;

    if source.status() == BusinessStatus::Closed || source.status() == BusinessStatus::Insolvent {
        return Err(SimulationError::BusinessInactive {
            business_id: from_business_id,
        });
    }
    if target.status() == BusinessStatus::Closed || target.status() == BusinessStatus::Insolvent {
        return Err(SimulationError::BusinessInactive {
            business_id: to_business_id,
        });
    }
    if source.cash() < amount {
        return Err(SimulationError::InsufficientBusinessCash {
            business_id: from_business_id,
            available: source.cash(),
            required: amount,
        });
    }

    Ok(ValidatedCashTransfer {
        from_business_id,
        to_business_id,
        amount,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{assert_state_eq, make_test_campaign};

    #[test]
    fn transfers_reject_insufficient_cash_without_mutation() {
        let mut state = make_test_campaign();
        let (from_business_id, to_business_id) = {
            let mut businesses = state.businesses().iter().map(crate::core::Business::id);
            (
                businesses.next().expect("source business must exist"),
                businesses.next().expect("target business must exist"),
            )
        };
        let available = state
            .businesses()
            .get(from_business_id)
            .expect("source business must exist")
            .cash();
        let required = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = transfer_business_cash(&mut state, from_business_id, to_business_id, required);

        assert_eq!(
            result,
            Err(SimulationError::InsufficientBusinessCash {
                business_id: from_business_id,
                available,
                required,
            })
        );
        assert_state_eq(
            &before,
            &state,
            "failed transfers must not mutate balances, versions, or the audit log",
        );
    }
}
