//! Supply-contract and private-credit negotiation with NPC counterparties.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn apply_contract(
    registry: &Registry,
    state: &mut AppState,
    terms: &SupplyContractTerms,
) -> Result<CommandOutcome, CommandError> {
    ensure_player_contract_party(state, terms)?;
    let validated = validate_supply_contract(registry, state, terms.clone())?;
    ensure_non_player_contract_counterparty_accepts(registry, state, terms)?;
    let id = validated.commit(registry, state)?;
    Ok(CommandOutcome {
        summary: format!("Created supply contract {id}."),
    })
}

pub(crate) fn apply_loan(
    registry: &Registry,
    state: &mut AppState,
    terms: &LoanTerms,
) -> Result<CommandOutcome, CommandError> {
    ensure_player_loan_party(state, terms)?;
    let validated = validate_loan(state, terms.clone())?;
    ensure_non_player_loan_counterparty_accepts(state, terms)?;
    let restructured = validated.restructures_defaulted_loan();
    let id = validated.commit(state)?;
    deploy_non_player_financing_package(registry, state, terms)?;
    let summary = if restructured {
        format!("Restructured loan {id}.")
    } else {
        format!("Issued loan {id}.")
    };
    Ok(CommandOutcome { summary })
}

pub(crate) fn ensure_player_contract_party(
    state: &AppState,
    terms: &SupplyContractTerms,
) -> Result<(), CommandError> {
    let buyer =
        state
            .businesses
            .get(terms.buyer_business_id)
            .ok_or(CommandError::MissingBusiness {
                business_id: terms.buyer_business_id,
            })?;
    let seller =
        state
            .businesses
            .get(terms.seller_business_id)
            .ok_or(CommandError::MissingBusiness {
                business_id: terms.seller_business_id,
            })?;
    if buyer.owner_dynasty_id() != state.player_dynasty_id
        && seller.owner_dynasty_id() != state.player_dynasty_id
    {
        return Err(CommandError::PlayerNotParty);
    }
    Ok(())
}

pub(crate) fn ensure_non_player_contract_counterparty_accepts(
    registry: &Registry,
    state: &AppState,
    terms: &SupplyContractTerms,
) -> Result<(), CommandError> {
    let buyer = state
        .businesses
        .get(terms.buyer_business_id)
        .expect("validated contract buyer must exist");
    let seller = state
        .businesses
        .get(terms.seller_business_id)
        .expect("validated contract seller must exist");
    let market_price = state
        .market
        .get_quote(terms.good_id)
        .ok_or(CommandError::MissingMarketQuote {
            good_id: terms.good_id,
        })?
        .price();
    let price_bounds = contract_counterparty_price_bounds(
        state,
        terms.buyer_business_id,
        terms.seller_business_id,
        market_price,
    );
    let minimum_price = price_bounds.minimum_seller_price;
    let maximum_price = price_bounds.maximum_buyer_price;
    if seller.owner_dynasty_id() != state.player_dynasty_id && terms.unit_price < minimum_price {
        return Err(CommandError::ContractCounterpartyPriceTooLow {
            unit_price: terms.unit_price,
            minimum_price,
        });
    }
    if buyer.owner_dynasty_id() != state.player_dynasty_id && terms.unit_price > maximum_price {
        return Err(CommandError::ContractCounterpartyPriceTooHigh {
            unit_price: terms.unit_price,
            maximum_price,
        });
    }

    let weekly_payment = crate::money::checked_cost_for(terms.quantity_per_week, terms.unit_price)
        .expect("validated contract payment must fit the supported money range");
    let minimum_penalty = weekly_payment.ceil_div_positive(4);
    let maximum_penalty = weekly_payment.saturating_mul(4);
    if terms.penalty < minimum_penalty || terms.penalty > maximum_penalty {
        return Err(CommandError::ContractCounterpartyPenaltyOutOfRange {
            penalty: terms.penalty,
            minimum_penalty,
            maximum_penalty,
        });
    }

    let capacity = available_supply_contract_capacity(
        registry,
        state,
        terms.buyer_business_id,
        terms.seller_business_id,
        terms.good_id,
    )
    .expect("validated contract parties must have compatible capacity");
    if seller.owner_dynasty_id() != state.player_dynasty_id
        && terms.quantity_per_week > capacity.seller
    {
        return Err(CommandError::ContractCounterpartyCapacity {
            business_id: terms.seller_business_id,
            requested: terms.quantity_per_week,
            available: capacity.seller,
        });
    }
    if buyer.owner_dynasty_id() != state.player_dynasty_id
        && terms.quantity_per_week > capacity.buyer
    {
        return Err(CommandError::ContractCounterpartyCapacity {
            business_id: terms.buyer_business_id,
            requested: terms.quantity_per_week,
            available: capacity.buyer,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContractCounterpartyPriceBounds {
    pub(crate) minimum_seller_price: Money,
    pub(crate) maximum_buyer_price: Money,
    pub(crate) relationship_pressure_basis_points: u16,
}

/// Returns the price band an NPC counterparty will accept against the player.
///
/// Neutral houses tolerate a modest bargaining band around the market quote. Distrust and
/// resentment narrow that band and can eventually require a premium from an NPC seller or a
/// discount for an NPC buyer. The relationship surcharge is capped so hostility cannot move a
/// negotiated price more than 15% away from market solely through this rule.
pub(crate) fn contract_counterparty_price_bounds(
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    market_price: Money,
) -> ContractCounterpartyPriceBounds {
    let player_id = state.player_dynasty_id;
    let buyer_owner = state
        .businesses
        .get(buyer_business_id)
        .map(crate::core::Business::owner_dynasty_id);
    let seller_owner = state
        .businesses
        .get(seller_business_id)
        .map(crate::core::Business::owner_dynasty_id);
    let counterparty_id = match (buyer_owner, seller_owner) {
        (Some(buyer_owner), Some(seller_owner)) if buyer_owner == player_id => {
            (seller_owner != player_id).then_some(seller_owner)
        }
        (Some(buyer_owner), Some(seller_owner)) if seller_owner == player_id => {
            (buyer_owner != player_id).then_some(buyer_owner)
        }
        (Some(_) | None, Some(_) | None) => None,
    };
    let relationship_pressure_basis_points = counterparty_id.map_or(0, |counterparty_id| {
        contract_relationship_pressure_basis_points(state, counterparty_id)
    });
    let seller_factor = 9_000_i64.saturating_add(i64::from(relationship_pressure_basis_points));
    let buyer_factor = 11_000_i64.saturating_sub(i64::from(relationship_pressure_basis_points));
    ContractCounterpartyPriceBounds {
        minimum_seller_price: market_price.saturating_mul_ratio(seller_factor, 10_000),
        maximum_buyer_price: market_price.saturating_mul_ratio(buyer_factor, 10_000),
        relationship_pressure_basis_points,
    }
}

pub(crate) fn contract_relationship_pressure_basis_points(
    state: &AppState,
    counterparty_id: DynastyId,
) -> u16 {
    let Some(relationship) = state
        .relationships
        .get(&DynastyPair::new(state.player_dynasty_id, counterparty_id))
    else {
        return 0;
    };
    let distrust = 4_000_u16.saturating_sub(relationship.trust_basis_points);
    let resentment = relationship.resentment_basis_points.saturating_sub(3_500);
    distrust
        .saturating_add(resentment)
        .saturating_div(2)
        .min(2_500)
}

pub(crate) fn ensure_player_loan_party(
    state: &AppState,
    terms: &LoanTerms,
) -> Result<(), CommandError> {
    if terms.lender_dynasty_id != state.player_dynasty_id
        && terms.borrower_dynasty_id != state.player_dynasty_id
    {
        return Err(CommandError::PlayerNotParty);
    }
    Ok(())
}

pub(crate) fn private_loan_borrower_financing_pressure(
    state: &AppState,
    dynasty_id: DynastyId,
) -> u8 {
    if state.loans.values().any(|loan| {
        loan.borrower_dynasty_id == dynasty_id
            && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
    }) {
        return 3;
    }
    if state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == dynasty_id
            && matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            )
    }) {
        return 2;
    }
    u8::from(state.dynasties.get(&dynasty_id).is_some_and(|dynasty| {
        dynasty.treasury() < PRIVATE_LOAN_COUNTERPARTY_BORROWER_LIQUIDITY_TARGET
    }))
}

pub(crate) fn ensure_non_player_loan_counterparty_accepts(
    state: &AppState,
    terms: &LoanTerms,
) -> Result<(), CommandError> {
    let player_id = state.player_dynasty_id;
    let exposure = negotiated_loan_exposure(state, terms);

    if terms.lender_dynasty_id != player_id {
        // Reworking this lender's own default is a recovery negotiation, not
        // fresh credit. Unrelated lenders, however, refuse to let a borrower
        // shop around an unresolved default and turn one failed obligation
        // into a chain of new creditors.
        let restructures_own_default = latest_defaulted_loan_for_pair(
            state,
            terms.lender_dynasty_id,
            terms.borrower_dynasty_id,
        )
        .is_some();
        if !restructures_own_default
            && let Some(defaulted) = unresolved_default_owed_elsewhere(
                state,
                terms.borrower_dynasty_id,
                terms.lender_dynasty_id,
            )
        {
            return Err(CommandError::LoanCounterpartyBorrowerInDefault {
                lender_dynasty_id: terms.lender_dynasty_id,
                borrower_dynasty_id: terms.borrower_dynasty_id,
                creditor_dynasty_id: defaulted.lender_dynasty_id,
                loan_id: defaulted.id,
            });
        }
        let lender = state
            .dynasties
            .get(&terms.lender_dynasty_id)
            .expect("validated loan lender must exist");
        let lender_after = lender.treasury().saturating_sub(terms.principal);
        if terms.principal > Money::ZERO && lender_after < PRIVATE_LOAN_COUNTERPARTY_RESERVE {
            return Err(CommandError::LoanCounterpartyLenderReserve {
                lender_dynasty_id: terms.lender_dynasty_id,
                available: lender.treasury(),
                principal: terms.principal,
                required_reserve: PRIVATE_LOAN_COUNTERPARTY_RESERVE,
            });
        }
        if terms.interest_basis_points < PRIVATE_LOAN_COUNTERPARTY_MIN_INTEREST_BASIS_POINTS {
            return Err(CommandError::LoanCounterpartyInterestTooLow {
                interest_basis_points: terms.interest_basis_points,
                minimum_basis_points: PRIVATE_LOAN_COUNTERPARTY_MIN_INTEREST_BASIS_POINTS,
            });
        }
        let minimum_payment =
            exposure.ceil_div_positive(PRIVATE_LOAN_COUNTERPARTY_MAX_AMORTIZATION_WEEKS);
        if terms.weekly_payment < minimum_payment {
            return Err(CommandError::LoanCounterpartyPaymentTooLow {
                weekly_payment: terms.weekly_payment,
                minimum_payment,
            });
        }
    }

    if terms.borrower_dynasty_id != player_id {
        if terms.interest_basis_points > PRIVATE_LOAN_COUNTERPARTY_MAX_INTEREST_BASIS_POINTS {
            return Err(CommandError::LoanCounterpartyInterestTooHigh {
                interest_basis_points: terms.interest_basis_points,
                maximum_basis_points: PRIVATE_LOAN_COUNTERPARTY_MAX_INTEREST_BASIS_POINTS,
            });
        }
        let minimum_amortization_weeks =
            if private_loan_borrower_financing_pressure(state, terms.borrower_dynasty_id) >= 2 {
                PRIVATE_LOAN_DISTRESSED_BORROWER_MIN_AMORTIZATION_WEEKS
            } else {
                PRIVATE_LOAN_COUNTERPARTY_MIN_AMORTIZATION_WEEKS
            };
        let maximum_payment = exposure.ceil_div_positive(minimum_amortization_weeks);
        if terms.weekly_payment > maximum_payment {
            return Err(CommandError::LoanCounterpartyPaymentTooHigh {
                weekly_payment: terms.weekly_payment,
                maximum_payment,
            });
        }
        if let Some(property_id) = terms.collateral_property_id {
            let property = state
                .properties
                .get(&property_id)
                .expect("validated loan collateral must exist");
            let minimum_exposure = ceil_basis_point_share(
                property.value,
                PRIVATE_LOAN_COUNTERPARTY_MIN_COLLATERAL_LTV_BASIS_POINTS,
            );
            if exposure < minimum_exposure {
                return Err(CommandError::LoanCounterpartyCollateralTooLarge {
                    property_id,
                    property_value: property.value,
                    exposure,
                    minimum_exposure,
                });
            }
        }
        if private_loan_borrower_financing_pressure(state, terms.borrower_dynasty_id) == 0 {
            return Err(CommandError::LoanCounterpartyNoFinancingNeed {
                borrower_dynasty_id: terms.borrower_dynasty_id,
            });
        }
    }
    Ok(())
}

pub(crate) fn deploy_non_player_financing_package(
    registry: &Registry,
    state: &mut AppState,
    terms: &LoanTerms,
) -> Result<(), CommandError> {
    if terms.borrower_dynasty_id == state.player_dynasty_id {
        return Ok(());
    }
    // One pass resolves each candidate's recapitalization shortfall; the
    // neediest operable firm wins by lifecycle severity, then cash, then ID.
    let selected = state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() == terms.borrower_dynasty_id)
        // Closed premises cannot be recapitalized; deploying funds there would
        // fail after the loan itself has committed.
        .filter(|business| business.status() != BusinessStatus::Closed)
        .filter_map(|business| {
            let target_cash = business_recapitalization_target(registry, state, business);
            let shortfall = target_cash.saturating_sub(business.cash());
            if shortfall <= Money::ZERO {
                return None;
            }
            // Closed premises are filtered out above; that arm exists for
            // exhaustiveness only.
            let severity = match business.status() {
                BusinessStatus::Insolvent => 0_u8,
                BusinessStatus::Distressed => 1,
                BusinessStatus::Active | BusinessStatus::Closed => 2,
            };
            Some((severity, business.cash(), business.id(), shortfall))
        })
        .min_by_key(|&(severity, cash, id, _)| (severity, cash, id));
    let Some((_, _, business_id, shortfall)) = selected else {
        return Ok(());
    };
    // The new principal is the financing package being deployed. The
    // borrower's existing treasury remains its household reserve; requiring
    // the post-loan treasury to clear that reserve would make small, valid
    // rescue loans inert before they can reach the business.
    let amount = shortfall.min(terms.principal);
    if amount > Money::ZERO {
        capitalize_owned_business(state, terms.borrower_dynasty_id, business_id, amount)?;
    }
    Ok(())
}

pub(crate) fn negotiated_loan_exposure(state: &AppState, terms: &LoanTerms) -> Money {
    let prior_default = state
        .loans
        .values()
        .filter(|loan| {
            loan.lender_dynasty_id == terms.lender_dynasty_id
                && loan.borrower_dynasty_id == terms.borrower_dynasty_id
                && loan.status == LoanStatus::Defaulted
        })
        .max_by_key(|loan| (loan.next_due_day, loan.id))
        .map_or(Money::ZERO, |loan| loan.balance);
    prior_default
        .checked_add(terms.principal)
        .expect("validated loan exposure must fit the supported money range")
}

pub(crate) fn ceil_basis_point_share(value: Money, basis_points: i64) -> Money {
    debug_assert!(value >= Money::ZERO);
    debug_assert!((0..=10_000).contains(&basis_points));
    let numerator = i128::from(value.copper()) * i128::from(basis_points);
    let copper = (numerator + 9_999) / 10_000;
    Money::from_copper(
        i64::try_from(copper).expect("basis-point share of supported money must fit money"),
    )
}
