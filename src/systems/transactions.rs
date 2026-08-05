//! Validated multi-record transactions and shared simulation errors.

use crate::core::{AppState, AuditKind, AuditRecord, BusinessStatus};
use crate::ids::{BusinessId, GoodId};
use crate::money::Money;
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SimulationError {
    #[error("day count must be positive, received {days}")]
    InvalidDayCount { days: u32 },
    #[error(
        "advancing {requested_days} days from day {current_day} exceeds the supported simulation range"
    )]
    DayRangeExhausted {
        current_day: i64,
        requested_days: u32,
    },
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
    #[error(
        "business {business_id} cannot receive {incoming}; current cash {current} would exceed the supported money range"
    )]
    BusinessCashOverflow {
        business_id: BusinessId,
        current: Money,
        incoming: Money,
    },
    #[error("market quote is missing for good {good_id}")]
    MarketQuoteMissing { good_id: GoodId },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedCashTransfer {
    from_business_id: BusinessId,
    to_business_id: BusinessId,
    amount: Money,
}

impl ValidatedCashTransfer {
    /// Revalidates and commits a previously validated two-business cash transfer exactly once.
    ///
    /// # Errors
    ///
    /// Returns the current validation error if application state changed after the token was
    /// created.
    ///
    /// # Panics
    ///
    /// Panics only if a validated business disappears between the internal revalidation and commit
    /// steps within this synchronous call.
    pub fn commit(self, state: &mut AppState) -> Result<(), SimulationError> {
        let Self {
            from_business_id,
            to_business_id,
            amount,
        } = self;
        validate_business_cash_transfer(state, from_business_id, to_business_id, amount)?;

        {
            let source = state
                .businesses
                .get_mut(from_business_id)
                .expect("validated transfer source must exist");
            source.finance.cash = source
                .finance
                .cash
                .checked_sub(amount)
                .expect("revalidated transfer source must cover the amount");
            source.finance.version = source.finance.version.saturating_add(1);
        }
        {
            let target = state
                .businesses
                .get_mut(to_business_id)
                .expect("validated transfer target must exist");
            target.finance.cash = target
                .finance
                .cash
                .checked_add(amount)
                .expect("revalidated transfer target cash must fit the supported range");
            target.finance.version = target.finance.version.saturating_add(1);
        }

        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CashTransfer,
            subject: format!("business:{from_business_id}->business:{to_business_id}"),
            detail: format!("amount={}", amount.copper()),
        });
        Ok(())
    }
}

/// Validates and commits a cash transfer between two active businesses.
///
/// # Errors
///
/// Returns a dedicated error for invalid amounts, missing or inactive businesses, identical
/// endpoints, insufficient source cash, or target cash overflow.
pub fn transfer_business_cash(
    state: &mut AppState,
    from_business_id: BusinessId,
    to_business_id: BusinessId,
    amount: Money,
) -> Result<(), SimulationError> {
    validate_business_cash_transfer(state, from_business_id, to_business_id, amount)?.commit(state)
}

/// Validates a two-business cash transfer without mutating state.
///
/// # Errors
///
/// Returns a dedicated error for invalid amounts, missing or inactive businesses, identical
/// endpoints, insufficient source cash, or target cash overflow.
pub fn validate_business_cash_transfer(
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
    if target.cash().checked_add(amount).is_none() {
        return Err(SimulationError::BusinessCashOverflow {
            business_id: to_business_id,
            current: target.cash(),
            incoming: amount,
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
    use crate::test_support::{assert_state_unchanged, make_test_campaign};

    #[test]
    fn rejects_insufficient_cash_without_mutation() {
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
        assert_state_unchanged(
            &before,
            &state,
            "failed transfers must not mutate balances, versions, or the audit log",
        );
    }

    #[test]
    fn validated_transfer_rechecks_changed_state_before_commit() {
        let mut state = make_test_campaign();
        let (from_business_id, to_business_id) = {
            let mut businesses = state.businesses().iter().map(crate::core::Business::id);
            (
                businesses.next().expect("source business must exist"),
                businesses.next().expect("target business must exist"),
            )
        };
        let amount = Money::from_copper(1);
        let token =
            validate_business_cash_transfer(&state, from_business_id, to_business_id, amount)
                .expect("initial transfer must validate");
        state
            .businesses
            .get_mut(from_business_id)
            .expect("source business must exist")
            .finance
            .cash = Money::ZERO;
        let before = state.clone();

        let result = token.commit(&mut state);

        assert_eq!(
            result,
            Err(SimulationError::InsufficientBusinessCash {
                business_id: from_business_id,
                available: Money::ZERO,
                required: amount,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "stale validation tokens must fail before mutating either business",
        );
    }

    #[test]
    fn rejects_target_cash_overflow_without_mutation() {
        let mut state = make_test_campaign();
        let (from_business_id, to_business_id) = {
            let mut businesses = state.businesses().iter().map(crate::core::Business::id);
            (
                businesses.next().expect("source business must exist"),
                businesses.next().expect("target business must exist"),
            )
        };
        let amount = Money::from_copper(1);
        state
            .businesses
            .get_mut(from_business_id)
            .expect("source business must exist")
            .finance
            .cash = amount;
        state
            .businesses
            .get_mut(to_business_id)
            .expect("target business must exist")
            .finance
            .cash = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = transfer_business_cash(&mut state, from_business_id, to_business_id, amount);

        assert_eq!(
            result,
            Err(SimulationError::BusinessCashOverflow {
                business_id: to_business_id,
                current: Money::from_copper(i64::MAX),
                incoming: amount,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing transfers must not debit the source or append audit records",
        );
    }
}
