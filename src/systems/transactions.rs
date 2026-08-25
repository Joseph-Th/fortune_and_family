//! Validated multi-record transactions and shared simulation errors.

use crate::core::{AppState, AuditKind, AuditRecord, Business, BusinessStatus};
use crate::ids::{
    BusinessId, CivicDebtId, DynastyId, GoodId, HouseholdId, IdentifierAllocationError,
    InstitutionId, LoanId,
};
use crate::money::{Money, Quantity};
use thiserror::Error;

/// Errors produced when a runtime schedule cannot be represented on the simulation timeline.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum TimelineError {
    #[error(
        "scheduling {offset_days} days after day {base_day} exceeds the supported simulation range"
    )]
    FutureDayOutOfRange { base_day: i64, offset_days: i64 },
}

pub(crate) fn checked_future_day(base_day: i64, offset_days: i64) -> Result<i64, TimelineError> {
    if offset_days < 0 {
        return Err(TimelineError::FutureDayOutOfRange {
            base_day,
            offset_days,
        });
    }
    base_day
        .checked_add(offset_days)
        .filter(|day| *day < i64::MAX)
        .ok_or(TimelineError::FutureDayOutOfRange {
            base_day,
            offset_days,
        })
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SimulationError {
    #[error(transparent)]
    IdentifierAllocation(#[from] IdentifierAllocationError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
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
    #[error(
        "business {business_id} inventory for good {good_id} cannot receive {incoming}; current quantity {current} would exceed the supported quantity range"
    )]
    BusinessInventoryOverflow {
        business_id: BusinessId,
        good_id: GoodId,
        current: Quantity,
        incoming: Quantity,
    },
    #[error(
        "business {business_id} cannot record cost {incoming}; lifetime costs {current} would exceed the supported money range"
    )]
    BusinessLifetimeCostsOverflow {
        business_id: BusinessId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "business {business_id} cannot record revenue {incoming}; lifetime revenue {current} would exceed the supported money range"
    )]
    BusinessLifetimeRevenueOverflow {
        business_id: BusinessId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "business {business_id} finance changed after validation: expected version {expected_version}, found {actual_version}"
    )]
    StaleBusinessFinance {
        business_id: BusinessId,
        expected_version: u64,
        actual_version: u64,
    },
    #[error("business {business_id} finance version is exhausted")]
    BusinessFinanceVersionExhausted { business_id: BusinessId },
    #[error("dynasty {dynasty_id} family charter version is exhausted")]
    FamilyCharterVersionExhausted { dynasty_id: DynastyId },
    #[error("dynasty {dynasty_id} generation is exhausted")]
    DynastyGenerationExhausted { dynasty_id: DynastyId },
    #[error(
        "dynasty {dynasty_id} cannot record civic contribution {incoming}; current total {current} would exceed the supported money range"
    )]
    DynastyCivicContributionsOverflow {
        dynasty_id: DynastyId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "dynasty {dynasty_id} cannot receive {incoming}; current treasury {current} would exceed the supported money range"
    )]
    DynastyTreasuryOverflow {
        dynasty_id: DynastyId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "household {household_id} cannot receive {incoming}; current cash {current} would exceed the supported money range"
    )]
    HouseholdCashOverflow {
        household_id: HouseholdId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "institution {institution_id} cannot receive {incoming}; current budget {current} would exceed the supported money range"
    )]
    InstitutionBudgetOverflow {
        institution_id: InstitutionId,
        current: Money,
        incoming: Money,
    },
    #[error("institution {institution_id} term number is exhausted")]
    InstitutionTermNumberExhausted { institution_id: InstitutionId },
    #[error("market quote is missing for good {good_id}")]
    MarketQuoteMissing { good_id: GoodId },
    #[error("market clearing debit must be nonnegative, received {outgoing}")]
    NegativeMarketDebit { outgoing: Money },
    #[error("market clearing credit must be nonnegative, received {incoming}")]
    NegativeMarketCredit { incoming: Money },
    #[error("market supply for good {good_id} must be nonnegative, received {incoming}")]
    NegativeMarketSupply { good_id: GoodId, incoming: Quantity },
    #[error(
        "market demand for good {good_id} cannot receive {incoming}; current demand {current} would exceed the supported quantity range"
    )]
    MarketDemandOverflow {
        good_id: GoodId,
        current: Quantity,
        incoming: Quantity,
    },
    #[error(
        "market stock for good {good_id} cannot receive {incoming}; current stock {current} would exceed the supported quantity range"
    )]
    MarketStockOverflow {
        good_id: GoodId,
        current: Quantity,
        incoming: Quantity,
    },
    #[error(
        "market supply flow for good {good_id} cannot receive {incoming}; current supply {current} would exceed the supported quantity range"
    )]
    MarketSupplyOverflow {
        good_id: GoodId,
        current: Quantity,
        incoming: Quantity,
    },
    #[error(
        "market trade for good {good_id} at quantity {quantity} and unit price {unit_price} exceeds the supported money range"
    )]
    MarketTradeValueOverflow {
        good_id: GoodId,
        quantity: Quantity,
        unit_price: Money,
    },
    #[error(
        "weekly external income cannot include payment {incoming}; accumulated total {accumulated} would exceed the supported money range"
    )]
    WeeklyExternalIncomeOverflow { accumulated: Money, incoming: Money },
    #[error(
        "loan {loan_id} cannot accrue interest {incoming}; current balance {current} would exceed the supported money range"
    )]
    LoanBalanceOverflow {
        loan_id: LoanId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "civic debt {civic_debt_id} cannot accrue interest {incoming}; current balance {current} would exceed the supported money range"
    )]
    CivicDebtBalanceOverflow {
        civic_debt_id: CivicDebtId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "market clearing account {current} cannot apply change {change} within the supported money range"
    )]
    MarketClearingAccountOverflow { current: Money, change: Money },
}

pub(crate) fn debit_market_clearing_account(
    state: &mut AppState,
    outgoing: Money,
) -> Result<(), SimulationError> {
    if outgoing < Money::ZERO {
        return Err(SimulationError::NegativeMarketDebit { outgoing });
    }
    let current = state.market.clearing_account;
    let change = Money::from_copper(-outgoing.copper());
    state.market.clearing_account = current
        .checked_sub(outgoing)
        .ok_or(SimulationError::MarketClearingAccountOverflow { current, change })?;
    Ok(())
}

/// Credits incoming money to the market clearing account so every payment
/// into the market sector conserves copper against its debited counterparty.
///
/// # Errors
///
/// Returns [`SimulationError::MarketClearingAccountOverflow`] when the credit
/// would exceed the supported money range.
pub(crate) fn credit_market_clearing_account(
    state: &mut AppState,
    incoming: Money,
) -> Result<(), SimulationError> {
    if incoming < Money::ZERO {
        return Err(SimulationError::NegativeMarketCredit { incoming });
    }
    let current = state.market.clearing_account;
    state.market.clearing_account =
        current
            .checked_add(incoming)
            .ok_or(SimulationError::MarketClearingAccountOverflow {
                current,
                change: incoming,
            })?;
    Ok(())
}

pub(crate) fn add_market_supply(
    state: &mut AppState,
    good_id: GoodId,
    incoming: Quantity,
) -> Result<(), SimulationError> {
    if incoming < Quantity::ZERO {
        return Err(SimulationError::NegativeMarketSupply { good_id, incoming });
    }
    let quote = state
        .market
        .quotes
        .get_mut(&good_id)
        .ok_or(SimulationError::MarketQuoteMissing { good_id })?;
    let resulting_stock =
        quote
            .stock
            .checked_add(incoming)
            .ok_or(SimulationError::MarketStockOverflow {
                good_id,
                current: quote.stock,
                incoming,
            })?;
    let resulting_supply =
        quote
            .supply_today
            .checked_add(incoming)
            .ok_or(SimulationError::MarketSupplyOverflow {
                good_id,
                current: quote.supply_today,
                incoming,
            })?;
    quote.stock = resulting_stock;
    quote.supply_today = resulting_supply;
    Ok(())
}

pub(crate) fn checked_next_business_finance_version(business: &Business) -> Option<u64> {
    business
        .finance
        .version
        .checked_add(1)
        .filter(|next| *next < u64::MAX)
}

pub(crate) fn next_business_finance_version(business: &Business) -> Result<u64, SimulationError> {
    checked_next_business_finance_version(business).ok_or(
        SimulationError::BusinessFinanceVersionExhausted {
            business_id: business.id(),
        },
    )
}

pub(crate) fn next_family_charter_version(
    dynasty_id: DynastyId,
    current: u64,
) -> Result<u64, SimulationError> {
    current
        .checked_add(1)
        .filter(|next| *next < u64::MAX)
        .ok_or(SimulationError::FamilyCharterVersionExhausted { dynasty_id })
}

#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedCashTransfer {
    from_business_id: BusinessId,
    to_business_id: BusinessId,
    amount: Money,
    from_finance_version: u64,
    to_finance_version: u64,
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
            from_finance_version,
            to_finance_version,
        } = self;
        let current =
            validate_business_cash_transfer(state, from_business_id, to_business_id, amount)?;
        if current.from_finance_version != from_finance_version {
            return Err(SimulationError::StaleBusinessFinance {
                business_id: from_business_id,
                expected_version: from_finance_version,
                actual_version: current.from_finance_version,
            });
        }
        if current.to_finance_version != to_finance_version {
            return Err(SimulationError::StaleBusinessFinance {
                business_id: to_business_id,
                expected_version: to_finance_version,
                actual_version: current.to_finance_version,
            });
        }

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
            source.finance.version = source
                .finance
                .version
                .checked_add(1)
                .expect("revalidated transfer source finance version must have headroom");
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
            target.finance.version = target
                .finance
                .version
                .checked_add(1)
                .expect("revalidated transfer target finance version must have headroom");
        }

        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CashTransfer,
            subject: format!("business:{from_business_id}->business:{to_business_id}").into(),
            detail: format!("amount={}", amount.copper()).into(),
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
    next_business_finance_version(source)?;
    next_business_finance_version(target)?;

    Ok(ValidatedCashTransfer {
        from_business_id,
        to_business_id,
        amount,
        from_finance_version: source.finance.version,
        to_finance_version: target.finance.version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{assert_state_unchanged, make_test_campaign};

    #[test]
    fn future_day_rejects_the_reserved_terminal_day() {
        assert_eq!(
            checked_future_day(i64::MAX - 1, 1),
            Err(TimelineError::FutureDayOutOfRange {
                base_day: i64::MAX - 1,
                offset_days: 1,
            })
        );
    }

    #[test]
    fn future_day_rejects_integer_overflow() {
        assert_eq!(
            checked_future_day(i64::MAX - 2, 7),
            Err(TimelineError::FutureDayOutOfRange {
                base_day: i64::MAX - 2,
                offset_days: 7,
            })
        );
    }

    #[test]
    fn future_day_allows_historical_bases_but_rejects_negative_offsets() {
        assert_eq!(
            checked_future_day(-180, 180),
            Ok(0),
            "pre-campaign history is part of the date domain"
        );
        assert_eq!(
            checked_future_day(7, -1),
            Err(TimelineError::FutureDayOutOfRange {
                base_day: 7,
                offset_days: -1,
            })
        );
    }

    #[test]
    fn rejects_negative_market_debit_without_mutation() {
        let mut state = make_test_campaign();
        let outgoing = Money::from_copper(-1);
        let before = state.clone();

        let result = debit_market_clearing_account(&mut state, outgoing);

        assert_eq!(
            result,
            Err(SimulationError::NegativeMarketDebit { outgoing })
        );
        assert_state_unchanged(
            &before,
            &state,
            "negative market debits must be rejected in release and debug builds alike",
        );
    }

    #[test]
    fn rejects_negative_market_supply_without_mutation() {
        let mut state = make_test_campaign();
        let good_id = *state
            .market
            .quotes
            .keys()
            .next()
            .expect("campaign must contain a market quote");
        let incoming = Quantity::from_milliunits(-1);
        let before = state.clone();

        let result = add_market_supply(&mut state, good_id, incoming);

        assert_eq!(
            result,
            Err(SimulationError::NegativeMarketSupply { good_id, incoming })
        );
        assert_state_unchanged(
            &before,
            &state,
            "negative market supply must be rejected before stock or flow state changes",
        );
    }

    #[test]
    fn rejects_market_clearing_debit_overflow_without_mutation() {
        let mut state = make_test_campaign();
        state.market.clearing_account = Money::from_copper(i64::MIN);
        let before = state.clone();

        let result = debit_market_clearing_account(&mut state, Money::from_copper(1));

        assert_eq!(
            result,
            Err(SimulationError::MarketClearingAccountOverflow {
                current: Money::from_copper(i64::MIN),
                change: Money::from_copper(-1),
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing market debits must not clamp the external account",
        );
    }

    #[test]
    fn rejects_market_stock_overflow_without_mutation() {
        let mut state = make_test_campaign();
        let good_id = *state
            .market
            .quotes
            .keys()
            .next()
            .expect("campaign must contain a market quote");
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("market quote must exist")
            .stock = Quantity::from_milliunits(i64::MAX);
        let incoming = Quantity::from_milliunits(1);
        let before = state.clone();

        let result = add_market_supply(&mut state, good_id, incoming);

        assert_eq!(
            result,
            Err(SimulationError::MarketStockOverflow {
                good_id,
                current: Quantity::from_milliunits(i64::MAX),
                incoming,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing market stock must not alter stock or daily supply",
        );
    }

    #[test]
    fn rejects_market_supply_flow_overflow_without_mutation() {
        let mut state = make_test_campaign();
        let good_id = *state
            .market
            .quotes
            .keys()
            .next()
            .expect("campaign must contain a market quote");
        let quote = state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("market quote must exist");
        quote.stock = Quantity::ZERO;
        quote.supply_today = Quantity::from_milliunits(i64::MAX);
        let incoming = Quantity::from_milliunits(1);
        let before = state.clone();

        let result = add_market_supply(&mut state, good_id, incoming);

        assert_eq!(
            result,
            Err(SimulationError::MarketSupplyOverflow {
                good_id,
                current: Quantity::from_milliunits(i64::MAX),
                incoming,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing daily supply must not commit the otherwise valid stock addition",
        );
    }

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
    fn validated_transfer_rejects_stale_finance_even_when_balances_still_cover_it() {
        let mut state = make_test_campaign();
        let business_ids: Vec<_> = state
            .businesses()
            .iter()
            .map(crate::core::Business::id)
            .collect();
        let [from_business_id, to_business_id, intervening_target_id, ..] = business_ids.as_slice()
        else {
            panic!("test campaign must contain at least three businesses: {business_ids:?}");
        };
        let from_business_id = *from_business_id;
        let to_business_id = *to_business_id;
        let intervening_target_id = *intervening_target_id;
        let amount = Money::from_copper(1);
        let token =
            validate_business_cash_transfer(&state, from_business_id, to_business_id, amount)
                .expect("initial transfer must validate");
        let expected_version = state
            .businesses()
            .get(from_business_id)
            .expect("source business must exist")
            .finance
            .version;
        transfer_business_cash(&mut state, from_business_id, intervening_target_id, amount)
            .expect("intervening transfer must succeed");
        let actual_version = state
            .businesses()
            .get(from_business_id)
            .expect("source business must exist")
            .finance
            .version;
        let before = state.clone();

        let result = token.commit(&mut state);

        assert_eq!(
            result,
            Err(SimulationError::StaleBusinessFinance {
                business_id: from_business_id,
                expected_version,
                actual_version,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "stale finance tokens must not apply after an intervening valid mutation",
        );
    }

    #[test]
    fn rejects_reserved_finance_version_without_mutation() {
        let mut state = make_test_campaign();
        let (from_business_id, to_business_id) = {
            let mut businesses = state.businesses().iter().map(crate::core::Business::id);
            (
                businesses.next().expect("source business must exist"),
                businesses.next().expect("target business must exist"),
            )
        };
        state
            .businesses
            .get_mut(from_business_id)
            .expect("source business must exist")
            .finance
            .version = u64::MAX - 1;
        let before = state.clone();

        let result = transfer_business_cash(
            &mut state,
            from_business_id,
            to_business_id,
            Money::from_copper(1),
        );

        assert_eq!(
            result,
            Err(SimulationError::BusinessFinanceVersionExhausted {
                business_id: from_business_id,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "exhausted finance versions must be rejected before balance mutation",
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
