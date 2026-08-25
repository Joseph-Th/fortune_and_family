//! Business ownership transactions: capitalization, distributions, acquisitions, and dividends.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessAcquisitionQuote {
    pub business_id: BusinessId,
    pub seller_dynasty_id: DynastyId,
    pub purchase_price: Money,
    pub minimum_recapitalization: Money,
}

/// Transfers dynasty treasury into one of its businesses and rehabilitates operating condition.
///
/// This is the canonical capitalization path used by both player commands and autonomous houses.
///
/// # Errors
///
/// Returns an error when the dynasty or business is missing, ownership does not match, the amount
/// is non-positive, the business is closed, funds are insufficient, or the resulting cash/version
/// would exceed supported ranges. Failed capitalization leaves state unchanged.
pub(crate) fn capitalize_owned_business(
    state: &mut AppState,
    dynasty_id: DynastyId,
    business_id: BusinessId,
    amount: Money,
) -> Result<u16, StrategicError> {
    if amount <= Money::ZERO {
        return Err(StrategicError::NonPositiveAmount);
    }
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .ok_or(StrategicError::MissingDynasty { dynasty_id })?;
    let dynasty_treasury = dynasty.treasury();
    if dynasty_treasury < amount {
        return Err(StrategicError::InsufficientDynastyFunds {
            dynasty_id,
            available: dynasty_treasury,
            required: amount,
        });
    }
    let business = state
        .businesses
        .get(business_id)
        .ok_or(StrategicError::MissingBusiness { business_id })?;
    if business.owner_dynasty_id() != dynasty_id {
        return Err(StrategicError::BusinessNotOwnedByDynasty {
            business_id,
            dynasty_id,
        });
    }
    if business.status() == BusinessStatus::Closed {
        return Err(StrategicError::BusinessInactive { business_id });
    }
    let resulting_cash =
        business
            .cash()
            .checked_add(amount)
            .ok_or(StrategicError::BusinessCashOverflow {
                business_id,
                current: business.cash(),
                incoming: amount,
            })?;
    let finance_version = checked_next_business_finance_version(business)
        .ok_or(StrategicError::BusinessFinanceVersionExhausted { business_id })?;
    let rehabilitation = u16::try_from((amount.copper() / 2).clamp(0, 3_000))
        .expect("bounded rehabilitation must fit u16");

    state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("validated dynasty must exist")
        .resources
        .treasury = dynasty_treasury
        .checked_sub(amount)
        .expect("validated dynasty funds must cover capitalization");
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.finance.cash = resulting_cash;
    business.finance.version = finance_version;
    business.operations.condition_basis_points = business
        .operations
        .condition_basis_points
        .saturating_add(rehabilitation)
        .min(10_000);
    business.operations.quality_basis_points = business
        .operations
        .quality_basis_points
        .saturating_add(rehabilitation / 2)
        .min(10_000);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessCapitalization,
        subject: format!("business:{business_id}").into(),
        detail: format!(
            "dynasty={dynasty_id};amount={};rehabilitation_basis_points={rehabilitation}",
            amount.copper()
        )
        .into(),
    });
    Ok(rehabilitation)
}

/// Moves surplus cash from an active business to its owning dynasty while preserving the same
/// operating floor used by automatic dividends.
///
/// # Errors
///
/// Returns an error when the dynasty or business is missing, ownership does not match, the amount
/// is non-positive, the business is not active, the requested distribution would breach its
/// operating reserve, or the resulting treasury/version would exceed supported ranges. Failed
/// distributions leave state unchanged.
pub(crate) fn distribute_owned_business_cash(
    registry: &Registry,
    state: &mut AppState,
    dynasty_id: DynastyId,
    business_id: BusinessId,
    amount: Money,
) -> Result<(), StrategicError> {
    if amount <= Money::ZERO {
        return Err(StrategicError::NonPositiveAmount);
    }
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .ok_or(StrategicError::MissingDynasty { dynasty_id })?;
    let business = state
        .businesses
        .get(business_id)
        .ok_or(StrategicError::MissingBusiness { business_id })?;
    if business.owner_dynasty_id() != dynasty_id {
        return Err(StrategicError::BusinessNotOwnedByDynasty {
            business_id,
            dynasty_id,
        });
    }
    if matches!(
        business.status(),
        BusinessStatus::Insolvent | BusinessStatus::Closed
    ) {
        return Err(StrategicError::BusinessInactive { business_id });
    }
    // An operating but Distressed firm may still return true surplus to its
    // owner: the distribution reserve below already protects 21 days of
    // operating cost on top of the minimum cash reserve, so withdrawal cannot
    // strip the cushion its recovery depends on.
    let reserve = business_owner_distribution_reserve(registry, business);
    let available = business.cash().saturating_sub(reserve).max(Money::ZERO);
    if amount > available {
        return Err(StrategicError::BusinessDistributionExceedsSurplus {
            business_id,
            available,
            required_reserve: reserve,
            requested: amount,
        });
    }
    let treasury_after =
        dynasty
            .treasury()
            .checked_add(amount)
            .ok_or(StrategicError::DynastyTreasuryOverflow {
                dynasty_id,
                current: dynasty.treasury(),
                incoming: amount,
            })?;
    let business_cash_after = business
        .cash()
        .checked_sub(amount)
        .expect("validated business distribution must fit business cash");
    let finance_version = checked_next_business_finance_version(business)
        .ok_or(StrategicError::BusinessFinanceVersionExhausted { business_id })?;

    state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("validated dynasty must exist")
        .resources
        .treasury = treasury_after;
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.finance.cash = business_cash_after;
    business.finance.version = finance_version;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessDividend,
        subject: format!("business:{business_id}").into(),
        detail: format!(
            "owner_distribution={};reserve={}",
            amount.copper(),
            reserve.copper()
        )
        .into(),
    });
    Ok(())
}

pub(crate) fn business_owner_distribution_reserve(
    registry: &Registry,
    business: &crate::core::Business,
) -> Money {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must exist");
    business
        .policy
        .minimum_cash_reserve
        .saturating_add(recipe.daily_operating_cost().saturating_mul(21))
}

pub(crate) fn business_recapitalization_target(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
) -> Money {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must resolve");
    let payroll_buffer = state
        .employment
        .values()
        .filter(|agreement| {
            agreement.business_id == business.id() && agreement.status != EmploymentStatus::Ended
        })
        .fold(Money::ZERO, |total, agreement| {
            total.saturating_add(agreement.weekly_wage.saturating_mul(2))
        });
    let input_buffer = recipe.inputs().iter().fold(Money::ZERO, |total, input| {
        let price = state
            .market
            .get_quote(input.good_id())
            .expect("recipe input good must have a market quote")
            .price();
        let quantity = input.quantity().saturating_mul_ratio(
            i64::from(business.operations.capacity_batches_per_day).saturating_mul(7),
            1,
        );
        total.saturating_add(cost_for(quantity, price))
    });
    business
        .policy
        .minimum_cash_reserve
        .saturating_add(recipe.daily_operating_cost().saturating_mul(14))
        .saturating_add(payroll_buffer)
        .saturating_add(input_buffer)
}

/// Returns the canonical price and minimum working-capital requirement for acquiring a troubled
/// business.
///
/// # Errors
///
/// Returns an error when the business or buyer is missing, the buyer already owns the business,
/// the business is still active and therefore not available for acquisition, or the discounted
/// valuation cannot fit the supported money range.
///
/// # Panics
///
/// Panics when previously validated business recipe or market references are missing.
pub fn quote_business_acquisition(
    registry: &Registry,
    state: &AppState,
    buyer_dynasty_id: DynastyId,
    business_id: BusinessId,
) -> Result<BusinessAcquisitionQuote, StrategicError> {
    ensure_registry_matches(registry, state)?;
    if !state.dynasties.contains_key(&buyer_dynasty_id) {
        return Err(StrategicError::MissingDynasty {
            dynasty_id: buyer_dynasty_id,
        });
    }
    let business = state
        .businesses
        .get(business_id)
        .ok_or(StrategicError::MissingBusiness { business_id })?;
    let seller_dynasty_id = business.owner_dynasty_id();
    if seller_dynasty_id == buyer_dynasty_id {
        return Err(StrategicError::BusinessAlreadyOwned {
            business_id,
            buyer_dynasty_id,
        });
    }
    let discount_basis_points = match business.status() {
        BusinessStatus::Distressed => 7_000_i64,
        BusinessStatus::Insolvent => 4_000,
        BusinessStatus::Closed => 2_500,
        BusinessStatus::Active => {
            return Err(StrategicError::BusinessNotAcquirable {
                business_id,
                status: business.status(),
            });
        }
    };
    let purchase_price =
        resolve_business_purchase_price(registry, state, business, discount_basis_points)?;
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe references must be validated");
    let operating_floor = recipe.daily_operating_cost().saturating_mul(2);
    let minimum_recapitalization = Money::from_copper(
        operating_floor
            .copper()
            .saturating_sub(business.cash().copper())
            .max(0),
    );
    Ok(BusinessAcquisitionQuote {
        business_id,
        seller_dynasty_id,
        purchase_price,
        minimum_recapitalization,
    })
}

pub(crate) fn resolve_business_purchase_price(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
    discount_basis_points: i64,
) -> Result<Money, StrategicError> {
    let business_id = business.id();
    let overflow = || StrategicError::BusinessValuationOverflow { business_id };
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe references must be validated");
    let mut gross_value = i128::from(business.cash().copper());

    for (good_id, quantity) in business.inventory() {
        let unit_price = state
            .market
            .quotes
            .get(good_id)
            .expect("business inventory good must have a market quote")
            .price;
        let inventory_value = rounded_cost_copper_wide(*quantity, unit_price);
        gross_value = gross_value
            .checked_add(inventory_value)
            .ok_or_else(&overflow)?;
    }

    let capacity = i128::from(business.operations.capacity_batches_per_day);
    let equipment_scale = capacity
        .checked_mul(60)
        .and_then(|value| {
            value.checked_mul(i128::from(
                business.operations.condition_basis_points.max(1_000),
            ))
        })
        .ok_or_else(&overflow)?;
    let operating_cost = i128::from(recipe.daily_operating_cost().copper());
    let equipment_value = operating_cost
        .checked_mul(equipment_scale)
        .ok_or_else(&overflow)?
        / 10_000;
    gross_value = gross_value
        .checked_add(equipment_value)
        .ok_or_else(&overflow)?;

    let goodwill_scale = capacity
        .checked_mul(30)
        .and_then(|value| value.checked_mul(i128::from(business.operations.quality_basis_points)))
        .ok_or_else(&overflow)?;
    let goodwill_value = operating_cost
        .checked_mul(goodwill_scale)
        .ok_or_else(&overflow)?
        / 10_000;
    gross_value = gross_value
        .checked_add(goodwill_value)
        .ok_or_else(&overflow)?;

    let discounted_value = gross_value
        .checked_mul(i128::from(discount_basis_points))
        .ok_or_else(&overflow)?
        / 10_000;
    let purchase_price = i64::try_from(discounted_value.max(500)).map_err(|_| overflow())?;
    Ok(Money::from_copper(purchase_price))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedBusinessAcquisition {
    quote: BusinessAcquisitionQuote,
    buyer_treasury: Money,
    total_required: Money,
    seller_treasury_after: Money,
    business_cash_after: Money,
    business_finance_version_after: u64,
    seller_administrative_load_after: u16,
    buyer_administrative_load_after: u16,
}

/// Acquires a troubled business, installs an eligible manager, and supplies enough working
/// capital for it to resume active operation.
///
/// # Errors
///
/// Returns an error for an unavailable business, invalid manager, insufficient recapitalization,
/// insufficient buyer treasury funds, or identifier-allocation exhaustion while recording the
/// acquisition and related feedback. Failed acquisitions leave state unchanged.
///
/// # Panics
///
/// Panics only if synchronized business, dynasty, character, or property records violate internal
/// invariants after successful validation.
pub fn acquire_business(
    registry: &Registry,
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    business_id: BusinessId,
    manager_id: CharacterId,
    recapitalization: Money,
) -> Result<BusinessAcquisitionQuote, StrategicError> {
    let validated = validate_business_acquisition(
        registry,
        state,
        buyer_dynasty_id,
        business_id,
        manager_id,
        recapitalization,
    )?;
    let mut next_state = state.clone();
    commit_business_acquisition(
        &mut next_state,
        buyer_dynasty_id,
        manager_id,
        recapitalization,
        validated,
    )?;
    *state = next_state;
    Ok(validated.quote)
}

pub(crate) fn validate_business_acquisition(
    registry: &Registry,
    state: &AppState,
    buyer_dynasty_id: DynastyId,
    business_id: BusinessId,
    manager_id: CharacterId,
    recapitalization: Money,
) -> Result<ValidatedBusinessAcquisition, StrategicError> {
    let quote = quote_business_acquisition(registry, state, buyer_dynasty_id, business_id)?;
    let manager =
        state
            .characters
            .get(manager_id)
            .ok_or(StrategicError::InvalidAcquisitionManager {
                manager_id,
                buyer_dynasty_id,
            })?;
    if manager.dynasty_id() != buyer_dynasty_id || manager.status() != CharacterStatus::Active {
        return Err(StrategicError::InvalidAcquisitionManager {
            manager_id,
            buyer_dynasty_id,
        });
    }
    if recapitalization < quote.minimum_recapitalization {
        return Err(StrategicError::InsufficientBusinessRecapitalization {
            business_id,
            provided: recapitalization,
            required: quote.minimum_recapitalization,
        });
    }
    let total_required = quote.purchase_price.checked_add(recapitalization).ok_or(
        StrategicError::AcquisitionCostOverflow {
            purchase_price: quote.purchase_price,
            recapitalization,
        },
    )?;
    let buyer_treasury = state
        .dynasties
        .get(&buyer_dynasty_id)
        .expect("quoted buyer dynasty must exist")
        .treasury();
    if buyer_treasury < total_required {
        return Err(StrategicError::InsufficientDynastyFunds {
            dynasty_id: buyer_dynasty_id,
            available: buyer_treasury,
            required: total_required,
        });
    }
    let seller_treasury = state
        .dynasties
        .get(&quote.seller_dynasty_id)
        .expect("business owner dynasty must exist")
        .treasury();
    let seller_treasury_after = seller_treasury.checked_add(quote.purchase_price).ok_or(
        StrategicError::DynastyTreasuryOverflow {
            dynasty_id: quote.seller_dynasty_id,
            current: seller_treasury,
            incoming: quote.purchase_price,
        },
    )?;
    let business = state
        .businesses
        .get(business_id)
        .expect("quoted business must exist");
    let business_cash_after = business.cash().checked_add(recapitalization).ok_or(
        StrategicError::BusinessCashOverflow {
            business_id,
            current: business.cash(),
            incoming: recapitalization,
        },
    )?;
    let business_finance_version_after = checked_next_business_finance_version(business)
        .ok_or(StrategicError::BusinessFinanceVersionExhausted { business_id })?;
    let recipe_id = business.recipe_id();
    let administrative_load = registry
        .get_recipe(recipe_id)
        .expect("business recipe references must be validated")
        .administrative_load();
    let (seller_administrative_load_after, buyer_administrative_load_after) =
        validate_acquisition_administrative_load(
            state,
            quote.seller_dynasty_id,
            buyer_dynasty_id,
            administrative_load,
        )?;
    Ok(ValidatedBusinessAcquisition {
        quote,
        buyer_treasury,
        total_required,
        seller_treasury_after,
        business_cash_after,
        business_finance_version_after,
        seller_administrative_load_after,
        buyer_administrative_load_after,
    })
}

pub(crate) fn validate_acquisition_administrative_load(
    state: &AppState,
    seller_dynasty_id: DynastyId,
    buyer_dynasty_id: DynastyId,
    administrative_load: u16,
) -> Result<(u16, u16), StrategicError> {
    let seller_current = state
        .dynasties
        .get(&seller_dynasty_id)
        .expect("business owner dynasty must exist")
        .administrative_load();
    let seller_after = seller_current.checked_sub(administrative_load).ok_or(
        StrategicError::DynastyAdministrativeLoadUnderflow {
            dynasty_id: seller_dynasty_id,
            current: seller_current,
            outgoing: administrative_load,
        },
    )?;
    let buyer_current = state
        .dynasties
        .get(&buyer_dynasty_id)
        .expect("quoted buyer dynasty must exist")
        .administrative_load();
    let buyer_after = buyer_current.checked_add(administrative_load).ok_or(
        StrategicError::DynastyAdministrativeLoadOverflow {
            dynasty_id: buyer_dynasty_id,
            current: buyer_current,
            incoming: administrative_load,
        },
    )?;
    Ok((seller_after, buyer_after))
}

pub(crate) fn commit_business_acquisition(
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    manager_id: CharacterId,
    recapitalization: Money,
    validated: ValidatedBusinessAcquisition,
) -> Result<(), StrategicError> {
    let quote = validated.quote;
    let business_id = quote.business_id;
    state
        .dynasties
        .get_mut(&buyer_dynasty_id)
        .expect("validated buyer must exist")
        .resources
        .treasury = validated
        .buyer_treasury
        .checked_sub(validated.total_required)
        .expect("validated acquisition buyer must cover the total cost");
    let seller = state
        .dynasties
        .get_mut(&quote.seller_dynasty_id)
        .expect("business owner dynasty must exist");
    seller.resources.treasury = validated.seller_treasury_after;
    seller.resources.administrative_load = validated.seller_administrative_load_after;
    let buyer = state
        .dynasties
        .get_mut(&buyer_dynasty_id)
        .expect("validated buyer must exist");
    buyer.resources.administrative_load = validated.buyer_administrative_load_after;

    let prior_owner = state
        .businesses
        .transfer_ownership(business_id, buyer_dynasty_id, manager_id)
        .expect("validated business must exist");
    debug_assert_eq!(prior_owner, quote.seller_dynasty_id);
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("transferred business must exist");
    business.finance.cash = validated.business_cash_after;
    business.finance.version = validated.business_finance_version_after;
    let rehabilitation = u16::try_from((recapitalization.copper() / 2).clamp(0, 3_000))
        .expect("bounded acquisition rehabilitation must fit u16");
    business.operations.condition_basis_points = business
        .operations
        .condition_basis_points
        .saturating_add(rehabilitation)
        .min(10_000);
    business.operations.quality_basis_points = business
        .operations
        .quality_basis_points
        .saturating_add(rehabilitation / 2)
        .min(10_000);
    business.operations.status = BusinessStatus::Active;
    synchronize_business_property_tenancy(state, business_id, buyer_dynasty_id);
    crate::systems::synchronize_employment_for_business_status(
        state,
        business_id,
        BusinessStatus::Active,
    );
    cancel_internalized_contracts(state, business_id, buyer_dynasty_id)?;

    record_business_acquisition(state, buyer_dynasty_id, manager_id, recapitalization, quote)?;
    Ok(())
}

pub(crate) fn cancel_internalized_contracts(
    state: &mut AppState,
    acquired_business_id: BusinessId,
    buyer_dynasty_id: DynastyId,
) -> Result<(), StrategicError> {
    let contract_ids: Vec<_> = state
        .contracts
        .iter()
        .filter_map(|(contract_id, contract)| {
            if contract.status != ContractStatus::Active {
                return None;
            }
            let counterparty_business_id = if contract.buyer_business_id == acquired_business_id {
                contract.seller_business_id
            } else if contract.seller_business_id == acquired_business_id {
                contract.buyer_business_id
            } else {
                return None;
            };
            state
                .businesses
                .get(counterparty_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == buyer_dynasty_id)
                .then_some(*contract_id)
        })
        .collect();

    for contract_id in &contract_ids {
        state
            .contracts
            .get_mut(contract_id)
            .expect("selected internalized contract must exist")
            .status = ContractStatus::Cancelled;
    }
    if !contract_ids.is_empty() {
        let ids = contract_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        try_push_outbox(
            state,
            OutboxKind::Contract,
            format!("Contracts cancelled after business {acquired_business_id} acquisition"),
            format!(
                "Contracts {ids} became internal to dynasty {buyer_dynasty_id} and were cancelled rather than counted as external commercial performance."
            ),
        )?;
    }
    Ok(())
}

pub(crate) fn synchronize_business_property_tenancy(
    state: &mut AppState,
    business_id: BusinessId,
    business_owner_id: DynastyId,
) {
    for property in state
        .properties
        .values_mut()
        .filter(|property| property.occupant_business_id == Some(business_id))
    {
        property.tenant_dynasty_id = property
            .owner_dynasty_id
            .filter(|property_owner_id| *property_owner_id != business_owner_id)
            .map(|_| business_owner_id);
    }
}

pub(crate) fn record_business_acquisition(
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    manager_id: CharacterId,
    recapitalization: Money,
    quote: BusinessAcquisitionQuote,
) -> Result<(), StrategicError> {
    let business_id = quote.business_id;
    let chronicle_id = state.next_ids.try_chronicle()?;
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::BusinessAcquired,
        summary: format!(
            "Dynasty {buyer_dynasty_id} acquired business {business_id} from dynasty {} for {} and supplied {} working capital.",
            quote.seller_dynasty_id, quote.purchase_price, recapitalization
        ),
    });
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessAcquisition,
        subject: format!("business:{business_id}").into(),
        detail: format!(
            "buyer={buyer_dynasty_id}; seller={}; price={}; recapitalization={}; manager={manager_id}",
            quote.seller_dynasty_id,
            quote.purchase_price.copper(),
            recapitalization.copper()
        ).into(),
    });
    try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} acquired"),
        format!(
            "The dynasty paid {} and supplied {} working capital. Character {manager_id} now manages the enterprise.",
            quote.purchase_price, recapitalization
        ),
    )?;
    Ok(())
}

pub(crate) fn distribute_business_dividends(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let mut projected_owner_treasuries = BTreeMap::new();
    let mut dividends = Vec::new();
    for business in state.businesses.iter() {
        if business.status() != BusinessStatus::Active
            || business.finance.lifetime_revenue <= business.finance.lifetime_costs
        {
            continue;
        }
        let operating_floor = business_owner_distribution_reserve(registry, business);
        let excess = business.cash().saturating_sub(operating_floor);
        let owner_id = business.owner_dynasty_id();
        let owner_treasury = projected_owner_treasuries
            .entry(owner_id)
            .or_insert_with(|| {
                state
                    .dynasties
                    .get(&owner_id)
                    .expect("dividend owner dynasty must exist")
                    .treasury()
            });
        let dividend = Money::from_copper(excess.copper() / 10).min(Money::from_copper(1_000));
        if dividend <= Money::ZERO {
            continue;
        }
        let owner_treasury_after = owner_treasury.checked_add(dividend).ok_or(
            SimulationError::DynastyTreasuryOverflow {
                dynasty_id: owner_id,
                current: *owner_treasury,
                incoming: dividend,
            },
        )?;
        let resulting_cash = business
            .finance
            .cash
            .checked_sub(dividend)
            .expect("planned dividend must fit business cash");
        let next_finance_version = next_business_finance_version(business)?;
        *owner_treasury = owner_treasury_after;
        dividends.push((
            business.id(),
            owner_id,
            dividend,
            resulting_cash,
            next_finance_version,
        ));
    }
    let mut total_copper = 0_i128;
    for (business_id, owner_id, dividend, resulting_cash, next_finance_version) in dividends {
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("dividend business must exist");
        business.finance.cash = resulting_cash;
        business.finance.version = next_finance_version;
        let owner = state
            .dynasties
            .get_mut(&owner_id)
            .expect("dividend owner dynasty must exist");
        owner.resources.treasury = owner
            .resources
            .treasury
            .checked_add(dividend)
            .expect("bounded dividend must fit owner treasury");
        total_copper += i128::from(dividend.copper());
    }
    if total_copper > 0 {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::BusinessDividend,
            subject: "business-portfolio".into(),
            detail: format!("dividends={total_copper}").into(),
        });
    }
    Ok(())
}
