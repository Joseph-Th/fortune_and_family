//! Real estate, tenancy, rents, district conditions, and public works.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyLiquidationQuote {
    pub price: Money,
    pub buyer_contribution: Money,
    pub civic_guarantee: Money,
    pub lien_payoff: Money,
    pub seller_proceeds: Money,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PropertyLienSettlement {
    loan_id: crate::ids::LoanId,
    lender_dynasty_id: DynastyId,
    payoff: Money,
}

/// Transfers an unowned property to a dynasty after validating price and ownership.
///
/// # Errors
///
/// Returns an error when the property or buyer is missing, the property is owned, funds are
/// insufficient, or durable feedback identifiers are exhausted.
pub fn buy_unowned_property(
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
) -> Result<(), StrategicError> {
    // Validation — including durable-feedback headroom — completes before any
    // mutation runs, so no defensive whole-campaign copy is needed for
    // rollback.
    let price = validate_unowned_property_purchase(state, buyer_dynasty_id, property_id)?;
    let outbox_id = state.next_ids.try_outbox()?;
    commit_unowned_property_purchase(state, buyer_dynasty_id, property_id, price, outbox_id);
    Ok(())
}

/// Validates an unowned-property purchase and returns its price.
pub(crate) fn validate_unowned_property_purchase(
    state: &AppState,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
) -> Result<Money, StrategicError> {
    let property = state
        .properties
        .get(&property_id)
        .ok_or(StrategicError::MissingProperty { property_id })?;
    if property.owner_dynasty_id.is_some() {
        return Err(StrategicError::PropertyAlreadyOwned { property_id });
    }
    let price = property.value;
    let buyer = state
        .dynasties
        .get(&buyer_dynasty_id)
        .ok_or(StrategicError::MissingDynasty {
            dynasty_id: buyer_dynasty_id,
        })?;
    if buyer.treasury() < price {
        return Err(StrategicError::InsufficientDynastyFunds {
            dynasty_id: buyer_dynasty_id,
            available: buyer.treasury(),
            required: price,
        });
    }
    // Unowned stock is effectively city real estate, so the purchase price
    // flows into the market clearing pool like every other payment into the
    // city's commercial sector instead of vanishing from the economy.
    if state.market.clearing_account.checked_add(price).is_none() {
        return Err(StrategicError::Simulation(
            SimulationError::MarketClearingAccountOverflow {
                current: state.market.clearing_account,
                change: price,
            },
        ));
    }
    Ok(price)
}

/// Applies a fully validated unowned-property purchase; infallible.
pub(crate) fn commit_unowned_property_purchase(
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
    price: Money,
    outbox_id: crate::ids::OutboxMessageId,
) {
    let buyer = state
        .dynasties
        .get_mut(&buyer_dynasty_id)
        .expect("validated buyer must exist");
    buyer.resources.treasury = buyer
        .treasury()
        .checked_sub(price)
        .expect("validated property buyer must cover the purchase price");
    credit_market_clearing_account(state, price)
        .expect("pre-validated clearing-account credit must succeed");
    state
        .properties
        .get_mut(&property_id)
        .expect("validated property must exist")
        .owner_dynasty_id = Some(buyer_dynasty_id);
    state.outbox.push(OutboxMessage {
        id: outbox_id,
        day: state.clock.day(),
        kind: OutboxKind::Property,
        subject: format!("Property {property_id} acquired"),
        body: format!("Dynasty {buyer_dynasty_id} acquired the property for {price}."),
        acknowledged: false,
    });
}

pub(crate) fn property_liquidation_lien(
    state: &AppState,
    seller_dynasty_id: DynastyId,
    property_id: PropertyId,
    collateral_loan_id: Option<crate::ids::LoanId>,
    price: Money,
) -> Result<Option<PropertyLienSettlement>, StrategicError> {
    let Some(loan_id) = collateral_loan_id else {
        return Ok(None);
    };
    let loan = state
        .loans
        .get(&loan_id)
        .ok_or(StrategicError::MissingCollateralLoan { loan_id })?;
    if loan.borrower_dynasty_id != seller_dynasty_id {
        return Err(StrategicError::PropertyLienBorrowerMismatch {
            property_id,
            loan_id,
            borrower_dynasty_id: loan.borrower_dynasty_id,
            seller_dynasty_id,
        });
    }
    if loan.balance > price {
        return Err(StrategicError::PropertySaleCannotSettleLien {
            property_id,
            loan_id,
            price,
            balance: loan.balance,
        });
    }
    state
        .dynasties
        .get(&loan.lender_dynasty_id)
        .ok_or(StrategicError::MissingDynasty {
            dynasty_id: loan.lender_dynasty_id,
        })?;
    Ok(Some(PropertyLienSettlement {
        loan_id,
        lender_dynasty_id: loan.lender_dynasty_id,
        payoff: loan.balance,
    }))
}

pub(crate) fn property_auction_funding(
    registry: &Registry,
    state: &AppState,
    seller_dynasty_id: DynastyId,
    buyer_dynasty_id: DynastyId,
    price: Money,
) -> Result<(Money, Money), StrategicError> {
    let seller = state
        .dynasties
        .get(&seller_dynasty_id)
        .ok_or(StrategicError::MissingDynasty {
            dynasty_id: seller_dynasty_id,
        })?;
    let buyer = state
        .dynasties
        .get(&buyer_dynasty_id)
        .ok_or(StrategicError::MissingDynasty {
            dynasty_id: buyer_dynasty_id,
        })?;
    let buyer_contribution = buyer.treasury().min(price);
    let civic_guarantee = price.saturating_sub(buyer_contribution);
    if civic_guarantee == Money::ZERO {
        return Ok((buyer_contribution, civic_guarantee));
    }
    let treasury_id = registry
        .get_institution_id("treasury")
        .ok_or(StrategicError::MissingCivicTreasury)?;
    let civic_available = state
        .institutions
        .get(&treasury_id)
        .ok_or(StrategicError::MissingCivicTreasury)?
        .budget;
    let distressed_seller = seller.treasury() < PROPERTY_AUCTION_DISTRESS_TREASURY_LIMIT
        && state.businesses.iter().any(|business| {
            business.owner_dynasty_id() == seller_dynasty_id
                && (matches!(
                    business.status(),
                    BusinessStatus::Distressed | BusinessStatus::Insolvent
                ) || business.cash() == Money::ZERO
                    || business.operations.condition_basis_points < 2_000)
        });
    if !distressed_seller || civic_available < civic_guarantee {
        return Err(StrategicError::InsufficientPropertyAuctionLiquidity {
            buyer_available: buyer.treasury(),
            civic_available: if distressed_seller {
                civic_available
            } else {
                Money::ZERO
            },
            required: price,
        });
    }
    Ok((buyer_contribution, civic_guarantee))
}

/// Returns the cash price available for a voluntary property liquidation.
///
/// # Errors
///
/// Returns an error when either dynasty or the property is missing, ownership does not match,
/// the parties are identical, a lien cannot be settled from the sale, or the transfer cannot fit
/// or be funded.
pub fn quote_property_liquidation(
    registry: &Registry,
    state: &AppState,
    seller_dynasty_id: DynastyId,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
) -> Result<PropertyLiquidationQuote, StrategicError> {
    ensure_registry_matches(registry, state)?;
    if seller_dynasty_id == buyer_dynasty_id {
        return Err(StrategicError::SamePropertyParty);
    }
    let property = state
        .properties
        .get(&property_id)
        .ok_or(StrategicError::MissingProperty { property_id })?;
    if property.owner_dynasty_id != Some(seller_dynasty_id) {
        return Err(StrategicError::PropertyNotOwnedBySeller {
            property_id,
            seller_dynasty_id,
        });
    }
    let seller = state
        .dynasties
        .get(&seller_dynasty_id)
        .ok_or(StrategicError::MissingDynasty {
            dynasty_id: seller_dynasty_id,
        })?;
    let price = property
        .value
        .saturating_mul_ratio(PROPERTY_LIQUIDATION_BASIS_POINTS, 10_000)
        .max(Money::from_copper(1));
    let lien = property_liquidation_lien(
        state,
        seller_dynasty_id,
        property_id,
        property.collateral_loan_id,
        price,
    )?;
    let lien_payoff = lien.map_or(Money::ZERO, |settlement| settlement.payoff);
    let seller_proceeds = price.saturating_sub(lien_payoff);
    let (buyer_contribution, civic_guarantee) =
        property_auction_funding(registry, state, seller_dynasty_id, buyer_dynasty_id, price)?;
    // A lender buying the collateral is debited before receiving the payoff. Because the payoff
    // cannot exceed the price, that combined balance transition cannot overflow.
    if let Some(lien) = lien
        && lien.lender_dynasty_id != buyer_dynasty_id
    {
        let lender =
            state
                .dynasties
                .get(&lien.lender_dynasty_id)
                .ok_or(StrategicError::MissingDynasty {
                    dynasty_id: lien.lender_dynasty_id,
                })?;
        if lender.treasury().checked_add(lien.payoff).is_none() {
            return Err(StrategicError::DynastyTreasuryOverflow {
                dynasty_id: lien.lender_dynasty_id,
                current: lender.treasury(),
                incoming: lien.payoff,
            });
        }
    }
    if seller.treasury().checked_add(seller_proceeds).is_none() {
        return Err(StrategicError::DynastyTreasuryOverflow {
            dynasty_id: seller_dynasty_id,
            current: seller.treasury(),
            incoming: seller_proceeds,
        });
    }
    Ok(PropertyLiquidationQuote {
        price,
        buyer_contribution,
        civic_guarantee,
        lien_payoff,
        seller_proceeds,
    })
}

fn settle_property_sale_finances(
    registry: &Registry,
    state: &mut AppState,
    seller_dynasty_id: DynastyId,
    buyer_dynasty_id: DynastyId,
    quote: PropertyLiquidationQuote,
    lien: Option<PropertyLienSettlement>,
) {
    let buyer = state
        .dynasties
        .get_mut(&buyer_dynasty_id)
        .expect("validated property buyer must exist");
    buyer.resources.treasury = buyer
        .resources
        .treasury
        .checked_sub(quote.buyer_contribution)
        .expect("validated property buyer must cover its contribution");
    if quote.civic_guarantee > Money::ZERO {
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("validated civic treasury definition must exist");
        let treasury = state
            .institutions
            .get_mut(&treasury_id)
            .expect("validated civic treasury runtime must exist");
        treasury.budget = treasury
            .budget
            .checked_sub(quote.civic_guarantee)
            .expect("validated civic treasury must cover the guarantee");
    }
    if let Some(lien) = lien {
        let lender = state
            .dynasties
            .get_mut(&lien.lender_dynasty_id)
            .expect("validated collateral lender must exist");
        lender.resources.treasury = lender
            .resources
            .treasury
            .checked_add(lien.payoff)
            .expect("validated lien payoff must fit lender treasury");
        let loan = state
            .loans
            .get_mut(&lien.loan_id)
            .expect("validated collateral loan must exist");
        loan.balance = Money::ZERO;
        loan.missed_payments = 0;
        loan.collateral_property_id = None;
        loan.status = LoanStatus::Repaid;
    }
    let seller = state
        .dynasties
        .get_mut(&seller_dynasty_id)
        .expect("validated property seller must exist");
    seller.resources.treasury = seller
        .resources
        .treasury
        .checked_add(quote.seller_proceeds)
        .expect("validated property sale must fit seller treasury");
}

/// Sells an owned property on an exclusively owned transactional scratch state.
///
/// Callers must discard `state` if this function returns an error. Player
/// commands already execute inside a whole-command scratch state and should
/// not clone it again.
pub(crate) fn sell_owned_property_scratch(
    registry: &Registry,
    state: &mut AppState,
    seller_dynasty_id: DynastyId,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
) -> Result<PropertyLiquidationQuote, StrategicError> {
    commit_owned_property_sale(
        registry,
        state,
        seller_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )
}

pub(crate) fn commit_owned_property_sale(
    registry: &Registry,
    state: &mut AppState,
    seller_dynasty_id: DynastyId,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
) -> Result<PropertyLiquidationQuote, StrategicError> {
    let quote = quote_property_liquidation(
        registry,
        state,
        seller_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )?;
    let occupant_owner_id = state
        .properties
        .get(&property_id)
        .and_then(|property| property.occupant_business_id)
        .and_then(|business_id| state.businesses.get(business_id))
        .map(crate::core::Business::owner_dynasty_id);
    let collateral_loan_id = state
        .properties
        .get(&property_id)
        .expect("validated property must exist")
        .collateral_loan_id;
    let lien = property_liquidation_lien(
        state,
        seller_dynasty_id,
        property_id,
        collateral_loan_id,
        quote.price,
    )
    .expect("validated property lien must remain valid");
    settle_property_sale_finances(
        registry,
        state,
        seller_dynasty_id,
        buyer_dynasty_id,
        quote,
        lien,
    );
    if let Some(lien) = lien {
        adjust_reliability_reputation(state, seller_dynasty_id, 10);
        record_completed_loan_repayment(
            state,
            lien.lender_dynasty_id,
            seller_dynasty_id,
            lien.loan_id,
        )?;
    }
    let property = state
        .properties
        .get_mut(&property_id)
        .expect("validated property must exist");
    property.collateral_loan_id = None;
    property.owner_dynasty_id = Some(buyer_dynasty_id);
    property.tenant_dynasty_id = occupant_owner_id.filter(|owner_id| *owner_id != buyer_dynasty_id);
    try_push_outbox(
        state,
        OutboxKind::Property,
        format!("Property {property_id} sold"),
        if quote.civic_guarantee > Money::ZERO {
            format!(
                "Dynasty {seller_dynasty_id} sold property {property_id} to dynasty {buyer_dynasty_id} for {}; the civic treasury guaranteed {} and {} settled the property lien.",
                quote.price, quote.civic_guarantee, quote.lien_payoff
            )
        } else {
            format!(
                "Dynasty {seller_dynasty_id} sold property {property_id} to dynasty {buyer_dynasty_id} for {}; {} settled the property lien.",
                quote.price, quote.lien_payoff
            )
        },
    )?;
    adjust_dynasty_relationship(
        state,
        seller_dynasty_id,
        buyer_dynasty_id,
        RelationshipDelta::new(35, 20, 0, -5, 0),
    );
    remember_dynasty_interaction(
        state,
        seller_dynasty_id,
        buyer_dynasty_id,
        &format!("Property {property_id} changed hands for {}.", quote.price),
    );
    Ok(quote)
}

/// A tenancy attached to a business premise ends when that business stops
/// operating; the former tenant must not keep paying rent on premises nobody
/// can use.
pub(crate) fn terminate_stale_tenancy(
    state: &mut AppState,
    property_id: PropertyId,
    tenant_id: DynastyId,
) -> Result<(), SimulationError> {
    if let Some(property) = state.properties.get_mut(&property_id) {
        property.tenant_dynasty_id = None;
    }
    if tenant_id == state.player_dynasty_id {
        try_push_outbox(
            state,
            OutboxKind::Property,
            format!("Tenancy at property {property_id} ended"),
            format!(
                "The business occupying property {property_id} stopped operating, so its tenancy was terminated and weekly rent no longer accrues."
            ),
        )?;
    }
    Ok(())
}

/// A firm evicted during insolvency re-occupies its premises once it trades
/// again: the workshop was built for it, and leaving it vacant would pay the
/// owner a vacancy-income windfall for their own tenant's recovery.
pub(crate) fn reoccupy_recovered_premises(state: &mut AppState) {
    let reoccupations: Vec<(PropertyId, BusinessId)> = state
        .businesses
        .iter()
        .filter(|business| business.status() == BusinessStatus::Active)
        .filter_map(|business| {
            let property_id = business.premises_property_id()?;
            let property = state.properties.get(&property_id)?;
            (property.occupant_business_id.is_none()).then_some((property_id, business.id()))
        })
        .collect();
    for (property_id, business_id) in reoccupations {
        let Some(business_owner_id) = state
            .businesses
            .get(business_id)
            .map(crate::core::Business::owner_dynasty_id)
        else {
            continue;
        };
        if let Some(property) = state.properties.get_mut(&property_id) {
            property.occupant_business_id = Some(business_id);
            // An outside owner must keep collecting rent from the recovered
            // occupant; only owner-occupied premises trade without a tenancy.
            property.tenant_dynasty_id = property
                .owner_dynasty_id
                .filter(|property_owner_id| *property_owner_id != business_owner_id)
                .map(|_| business_owner_id);
        }
    }
}

/// Vacancy income is an abstraction funded by the market's own clearing
/// pool; it is bounded by what that pool holds so the weekly settlement can
/// never overdraw it. Returns the amount actually paid.
pub(crate) fn collect_vacancy_income(
    state: &mut AppState,
    owner_id: DynastyId,
    rent: Money,
) -> Result<Money, SimulationError> {
    let paid = rent.min(state.market.clearing_account.max(Money::ZERO));
    if paid <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let owner_treasury = state
        .dynasties
        .get(&owner_id)
        .expect("property owner dynasty must exist")
        .treasury();
    owner_treasury
        .checked_add(paid)
        .ok_or(SimulationError::DynastyTreasuryOverflow {
            dynasty_id: owner_id,
            current: owner_treasury,
            incoming: paid,
        })?;
    debit_market_clearing_account(state, paid)?;
    Ok(paid)
}

pub(crate) fn settle_property_rents(state: &mut AppState) -> Result<(), SimulationError> {
    reoccupy_recovered_premises(state);
    let rents: Vec<_> = state
        .properties
        .values()
        .filter_map(|property| {
            Some((
                property.id,
                property.owner_dynasty_id?,
                property.tenant_dynasty_id,
                property.occupant_business_id,
                effective_property_weekly_rent(state, property),
            ))
        })
        .collect();
    for (property_id, owner_id, tenant_id, occupant_business_id, rent) in rents {
        let occupant_is_closed = occupant_business_id.is_some_and(|business_id| {
            state.businesses.get(business_id).is_some_and(|business| {
                matches!(
                    business.status(),
                    crate::core::BusinessStatus::Closed | crate::core::BusinessStatus::Insolvent
                )
            })
        });
        if occupant_is_closed {
            // A closed or insolvent business no longer occupies its premises:
            // evict it so the unit genuinely returns to the market instead of
            // a dead firm blocking vacancy income indefinitely.
            if let Some(property) = state.properties.get_mut(&property_id) {
                property.occupant_business_id = None;
            }
        }
        let paid = if let Some(tenant_id) = tenant_id {
            if owner_id == tenant_id {
                continue;
            }
            if occupant_is_closed {
                terminate_stale_tenancy(state, property_id, tenant_id)?;
                continue;
            }
            let tenant_cash = state
                .dynasties
                .get(&tenant_id)
                .expect("property tenant dynasty must exist")
                .treasury();
            let paid = rent.min(tenant_cash);
            if paid <= Money::ZERO {
                continue;
            }
            let owner_treasury = state
                .dynasties
                .get(&owner_id)
                .expect("property owner dynasty must exist")
                .treasury();
            owner_treasury
                .checked_add(paid)
                .ok_or(SimulationError::DynastyTreasuryOverflow {
                    dynasty_id: owner_id,
                    current: owner_treasury,
                    incoming: paid,
                })?;
            state
                .dynasties
                .get_mut(&tenant_id)
                .expect("property tenant dynasty must exist")
                .resources
                .treasury = tenant_cash
                .checked_sub(paid)
                .expect("bounded rent payment must not exceed tenant treasury");
            paid
        } else if occupant_business_id.is_none() {
            collect_vacancy_income(state, owner_id, rent)?
        } else {
            Money::ZERO
        };
        if paid == Money::ZERO {
            continue;
        }
        let owner = state
            .dynasties
            .get_mut(&owner_id)
            .expect("property owner dynasty must exist");
        owner.resources.treasury = owner
            .resources
            .treasury
            .checked_add(paid)
            .expect("bounded rent must fit owner treasury");
    }
    Ok(())
}

pub(crate) fn effective_property_weekly_rent(state: &AppState, property: &Property) -> Money {
    // District desirability reprices every lease, occupied or vacant: the
    // same premises cannot sit at a flat rent while everything around them
    // moves with the district's fortunes. Building condition then discounts
    // that indexed rent — a fire-scarred workshop cannot command the price
    // of a pristine one, but routine upkeep restores the discount as the
    // structure heals.
    let rent_index = state
        .districts
        .get(&property.district_id)
        .expect("property district runtime must exist")
        .rent_index_basis_points;
    let indexed_rent = property
        .weekly_rent
        .saturating_mul_ratio(i64::from(rent_index), 10_000);
    // Condition gates rent only when the building is materially damaged:
    // above 7000 bp (≈70% condition) the premises rent at full indexed
    // price; below that, rent scales linearly from 0% at total ruin to
    // full at the 7000 threshold, so a fire-gutted workshop commands no
    // rent until repaired, mirroring the monthly 180 bp repair step that
    // needs ~3.2 years to heal a fully destroyed property from 0 to 7000.
    let condition_basis_points = property.condition_basis_points;
    let condition_adjusted = if condition_basis_points >= 7_000 {
        indexed_rent
    } else {
        let factor = 10_000_i64 * i64::from(condition_basis_points) / 7_000;
        indexed_rent.saturating_mul_ratio(factor, 10_000)
    };
    let indexed_rent = condition_adjusted;
    active_law_value(state, LawKind::RentRestriction).map_or(indexed_rent, |limit| {
        let annual_cap = property
            .value
            .saturating_mul_ratio(limit.clamp(0, 10_000), 10_000);
        // The canonical calendar is a 360-day year settled weekly, matching
        // loan interest accrual.
        let weekly_cap = if annual_cap.copper() > 0 {
            Money::from_copper(crate::money::ceil_div_nonnegative(
                annual_cap.copper().saturating_mul(7),
                360,
            ))
        } else {
            Money::ZERO
        };
        indexed_rent.min(weekly_cap)
    })
}

pub(crate) fn apply_public_work_completion(
    state: &mut AppState,
    district_id: DistrictId,
    kind: PublicWorkKind,
) {
    let Some(district) = state.districts.get_mut(&district_id) else {
        return;
    };
    let employment_bonus = public_work_employment_bonus_basis_points(kind);
    if employment_bonus > 0 {
        district.employment_basis_points = district
            .employment_basis_points
            .saturating_add(employment_bonus)
            .min(10_000);
    }
    match kind {
        PublicWorkKind::Drainage => {
            district.sanitation_basis_points = district
                .sanitation_basis_points
                .saturating_add(1_200)
                .min(10_000);
        }
        PublicWorkKind::Hospital => {
            district.sanitation_basis_points = district
                .sanitation_basis_points
                .saturating_add(900)
                .min(10_000);
        }
        PublicWorkKind::WatchStation => {
            district.safety_basis_points = district
                .safety_basis_points
                .saturating_add(1_200)
                .min(10_000);
        }
        PublicWorkKind::Road | PublicWorkKind::Bridge => {
            district.safety_basis_points =
                district.safety_basis_points.saturating_add(250).min(10_000);
        }
        PublicWorkKind::Granary => {
            district.sanitation_basis_points = district
                .sanitation_basis_points
                .saturating_add(250)
                .min(10_000);
        }
        PublicWorkKind::Market | PublicWorkKind::School => {}
    }
    let unrest_relief = match kind {
        PublicWorkKind::WatchStation => 250,
        PublicWorkKind::Granary | PublicWorkKind::Hospital | PublicWorkKind::School => 700,
        PublicWorkKind::Road
        | PublicWorkKind::Bridge
        | PublicWorkKind::Market
        | PublicWorkKind::Drainage => 500,
    };
    district.unrest_basis_points = district.unrest_basis_points.saturating_sub(unrest_relief);
    if kind == PublicWorkKind::Granary {
        for household in state
            .households
            .iter_mut()
            .filter(|household| household.district_id() == district_id)
        {
            household.food_satisfaction_basis_points = household
                .food_satisfaction_basis_points
                .saturating_add(500)
                .min(10_000);
        }
    }
}

pub(crate) const fn public_work_employment_bonus_basis_points(kind: PublicWorkKind) -> u16 {
    match kind {
        PublicWorkKind::Market => 800,
        PublicWorkKind::Road | PublicWorkKind::Bridge => 500,
        PublicWorkKind::Granary | PublicWorkKind::School => 300,
        PublicWorkKind::Drainage | PublicWorkKind::WatchStation | PublicWorkKind::Hospital => 0,
    }
}

pub(crate) fn completed_public_work_employment_bonus_basis_points(
    state: &AppState,
    district_id: DistrictId,
) -> u16 {
    state
        .public_works
        .values()
        .filter(|work| {
            work.district_id == district_id && work.status == PublicWorkStatus::Completed
        })
        .fold(0_u16, |bonus, work| {
            bonus.saturating_add(public_work_employment_bonus_basis_points(work.kind))
        })
        .min(8_000)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn progress_public_works(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let treasury_id = registry.get_institution_id("treasury");
    let tools_id = registry.get_good_id("tools");
    let ids: Vec<_> = state
        .public_works
        .values()
        .filter(|work| work.status.is_unfinished())
        .map(|work| work.id)
        .collect();
    for id in ids {
        let (remaining, was_suspended) = {
            let work = state.public_works.get(&id).expect("public work must exist");
            (
                work.budget.saturating_sub(work.spent),
                work.status == PublicWorkStatus::Suspended,
            )
        };
        let requested = Money::from_copper(1_500).min(remaining);
        let weekly_spend = treasury_id
            .and_then(|treasury_id| state.institutions.get(&treasury_id))
            .map_or(Money::ZERO, |treasury| requested.min(treasury.budget));
        let tool_purchase = if let Some(tools_id) = tools_id {
            plan_public_work_tool_purchase(state, tools_id, weekly_spend)?
        } else {
            None
        };
        if weekly_spend > Money::ZERO
            && let Some(treasury_id) = treasury_id
        {
            let treasury = state
                .institutions
                .get_mut(&treasury_id)
                .expect("civic treasury runtime must exist");
            treasury.budget = treasury
                .budget
                .checked_sub(weekly_spend)
                .expect("bounded public-work spending must not exceed treasury budget");
        }
        if let Some(tool_purchase) = tool_purchase {
            apply_public_work_tool_purchase(state, tool_purchase);
        }
        // The share of the weekly spend not spent on tools pays construction
        // labor and materials from the unmodeled sector, so the residual is
        // credited to the market clearing pool instead of vanishing from the
        // economy: every treasury debit keeps a credited counterparty.
        let tool_cost = tool_purchase.map_or(Money::ZERO, |purchase| purchase.cost);
        let labor_residual = weekly_spend.saturating_sub(tool_cost);
        if labor_residual > Money::ZERO {
            credit_market_clearing_account(state, labor_residual)?;
        }

        let completion = {
            let work = state
                .public_works
                .get_mut(&id)
                .expect("public work must exist");
            if remaining > Money::ZERO && weekly_spend == Money::ZERO {
                work.status = PublicWorkStatus::Suspended;
                None
            } else {
                work.status = PublicWorkStatus::Building;
                work.spent = work
                    .spent
                    .checked_add(weekly_spend)
                    .expect("bounded public-work spending must fit project total");
                work.progress_basis_points =
                    crate::systems::public_work_progress_basis_points(work.spent, work.budget);
                (work.progress_basis_points >= 10_000).then_some((work.district_id, work.kind))
            }
        };

        if !was_suspended
            && completion.is_none()
            && state
                .public_works
                .get(&id)
                .is_some_and(|work| work.status == PublicWorkStatus::Suspended)
        {
            // A suspended civic project is not free limbo: stalled works
            // erode public trust and local order while they sit unfinished.
            if let Some(treasury_id) = treasury_id
                && let Some(treasury) = state.institutions.get_mut(&treasury_id)
            {
                treasury.legitimacy_basis_points =
                    treasury.legitimacy_basis_points.saturating_sub(15);
            }
            let district_id = state
                .public_works
                .get(&id)
                .expect("suspended work must exist")
                .district_id;
            if let Some(district) = state.districts.get_mut(&district_id) {
                district.unrest_basis_points =
                    district.unrest_basis_points.saturating_add(10).min(10_000);
            }
            try_push_outbox(
                state,
                OutboxKind::Politics,
                format!("Public work {id} suspended"),
                "Civic treasury funding is insufficient to continue construction.".to_owned(),
            )?;
        }
        if let Some((district_id, kind)) = completion {
            state
                .public_works
                .get_mut(&id)
                .expect("public work must exist")
                .status = PublicWorkStatus::Completed;
            apply_public_work_completion(state, district_id, kind);
            try_push_outbox(
                state,
                OutboxKind::Politics,
                format!("Public work {id} completed"),
                "A civic construction project has permanently changed district conditions."
                    .to_owned(),
            )?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PublicWorkToolPurchase {
    tools_id: GoodId,
    market_stock_after: Quantity,
    clearing_after: Money,
    cost: Money,
}

pub(crate) fn plan_public_work_tool_purchase(
    state: &AppState,
    tools_id: GoodId,
    weekly_spend: Money,
) -> Result<Option<PublicWorkToolPurchase>, SimulationError> {
    if weekly_spend <= Money::ZERO {
        return Ok(None);
    }
    let quote = state
        .market
        .quotes
        .get(&tools_id)
        .expect("registered public-work tools quote must exist");
    let tool_budget =
        weekly_spend.saturating_mul_ratio(PUBLIC_WORK_TOOL_SHARE_BASIS_POINTS, 10_000);
    let quantity = quote
        .stock
        .min(affordable_quantity(tool_budget, quote.price));
    if quantity <= Quantity::ZERO {
        return Ok(None);
    }
    let cost = cost_for(quantity, quote.price);
    let market_stock_after = quote
        .stock
        .checked_sub(quantity)
        .expect("planned public-work tool purchase must not exceed market stock");
    let clearing_after = state.market.clearing_account.checked_add(cost).ok_or(
        SimulationError::MarketClearingAccountOverflow {
            current: state.market.clearing_account,
            change: cost,
        },
    )?;
    // Weekly settlement runs after the day's price update and before the next
    // market-flow reset, so this off-hours purchase records stock and money
    // movement only; a `demand_today` write here would be dead state.
    Ok(Some(PublicWorkToolPurchase {
        tools_id,
        market_stock_after,
        clearing_after,
        cost,
    }))
}

pub(crate) fn apply_public_work_tool_purchase(
    state: &mut AppState,
    purchase: PublicWorkToolPurchase,
) {
    let quote = state
        .market
        .quotes
        .get_mut(&purchase.tools_id)
        .expect("planned public-work tools quote must exist");
    quote.stock = purchase.market_stock_after;
    state.market.clearing_account = purchase.clearing_after;
}

pub(crate) fn update_district_conditions(state: &mut AppState) {
    let district_ids: Vec<_> = state.districts.keys().copied().collect();
    for district_id in district_ids {
        let households: Vec<_> = state
            .households
            .ids_for_district(district_id)
            .into_iter()
            .flatten()
            .filter_map(|id| state.households.get(*id))
            .collect();
        let satisfaction = crate::core::population_weighted_food_satisfaction_basis_points(
            households.iter().copied(),
        )
        .unwrap_or(crate::core::NEUTRAL_FOOD_SATISFACTION_BASIS_POINTS);
        // An ongoing guild revolt suppresses formal employment for as long as
        // it is active, so the monthly model applies the same pressure the
        // daily crisis effect erodes with instead of resetting it away.
        let revolt_employment_pressure = state
            .crises
            .values()
            .filter(|crisis| {
                crisis.kind == CrisisKind::GuildRevolt
                    && crisis.status.is_active()
                    && crisis.district_id == Some(district_id)
            })
            .map(|crisis| (crisis.severity_basis_points / 100).max(1))
            .max()
            .unwrap_or(0);
        let employment = district_employment_basis_points(state, district_id)
            .saturating_sub(revolt_employment_pressure);
        let rent_index = {
            let district = state
                .districts
                .get_mut(&district_id)
                .expect("district runtime must exist");
            district.employment_basis_points = employment;
            district.unrest_basis_points =
                district_unrest_next_basis_points(district, satisfaction);
            let desirability = u32::from(district.safety_basis_points)
                .saturating_add(u32::from(district.sanitation_basis_points));
            district.rent_index_basis_points = u16::try_from(
                u32::from(crate::systems::MIN_DISTRICT_RENT_INDEX_BASIS_POINTS)
                    .saturating_add(desirability / 3)
                    .min(u32::from(
                        crate::systems::MAX_DISTRICT_RENT_INDEX_BASIS_POINTS,
                    )),
            )
            .expect("bounded district rent index must fit u16");
            district.rent_index_basis_points
        };
        revalue_district_properties(state, district_id, rent_index);
        repair_district_properties(state, district_id);
    }
}

/// Monthly wear reversal: routine upkeep restores condition toward full
/// repair, so an urban fire's damage heals over the following year instead
/// of degrading every building in the district permanently. The step stays
/// smaller than active fire erosion (600+/mo at moderate severity) so
/// fires remain costly while making condition a two-way statistic.
pub(crate) fn repair_district_properties(state: &mut AppState, district_id: DistrictId) {
    const MONTHLY_REPAIR_BASIS_POINTS: u16 = 180;
    for property in state
        .properties
        .values_mut()
        .filter(|property| property.district_id == district_id)
    {
        if property.condition_basis_points < 10_000 {
            property.condition_basis_points = property
                .condition_basis_points
                .saturating_add(MONTHLY_REPAIR_BASIS_POINTS)
                .min(10_000);
        }
    }
}

/// Drifts property values toward what the district's current desirability
/// would command, so real estate is a two-way asset: buildings in a decaying
/// district lose value, and a prosperous district appreciates them. The anchor
/// is each property's neutral-district `anchor_value` scaled by the district
/// rent index, so a one-time change in conditions reprices a property once
/// toward a stable level instead of compounding against its own drifting value
/// month after month.
pub(crate) fn revalue_district_properties(
    state: &mut AppState,
    district_id: DistrictId,
    rent_index: u16,
) {
    for property in state
        .properties
        .values_mut()
        .filter(|property| property.district_id == district_id)
    {
        let target_value = property
            .anchor_value
            .saturating_mul_ratio(i64::from(rent_index), 10_000);
        // A small monthly step toward the target prevents wild swings while still
        // letting sustained district decay pull values down. Cap the step so a
        // sudden extreme gap cannot move a property by more than 5% of its value
        // in a single month.
        let gap = target_value.copper().abs_diff(property.value.copper());
        let raw_step = gap / 12;
        let cap = property
            .value
            .saturating_mul_ratio(500, 10_000)
            .copper()
            .unsigned_abs();
        let step = Money::from_copper(
            i64::try_from(raw_step.min(cap)).expect("capped revaluation step must fit i64"),
        );
        if step == Money::ZERO {
            continue;
        }
        property.value = if target_value > property.value {
            property.value.saturating_add(step)
        } else {
            property
                .value
                .saturating_sub(step)
                .max(Money::from_copper(1))
        };
    }
}

pub(crate) fn district_employment_basis_points(state: &AppState, district_id: DistrictId) -> u16 {
    let active_jobs = crate::systems::saturating_worker_count(
        state
            .employment
            .values()
            .filter(|employment| {
                employment.status == EmploymentStatus::Active
                    && state
                        .businesses
                        .get(employment.business_id)
                        .is_some_and(|business| business.district_id() == district_id)
            })
            .map(|employment| u32::from(employment.workers)),
    );
    // Population-weighted formal employment: the same number of formal jobs
    // covers a smaller share of a populous district, so per-capita pressure
    // dilutes the bonus. A 6.9k reference keeps the single-district average
    // near the previous magnitude while making 9.9k Southern Reach and 4.3k
    // Temple Hill diverge realistically.
    let total_members: u32 = state
        .households
        .ids_for_district(district_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.households.get(*id))
        .map(|h| u32::from(h.members()))
        .sum();
    let reference_members: u32 = 6_900;
    let population_adjusted_jobs = if total_members == 0 {
        active_jobs
    } else {
        active_jobs
            .saturating_mul(reference_members)
            .checked_div(total_members)
            .unwrap_or(active_jobs)
    };
    let formal_employment_bonus = population_adjusted_jobs
        .saturating_mul(DISTRICT_FORMAL_EMPLOYMENT_BASIS_POINTS_PER_WORKER)
        .min(DISTRICT_MAX_FORMAL_EMPLOYMENT_BONUS_BASIS_POINTS);
    let formal_employment_bonus =
        u16::try_from(formal_employment_bonus).expect("bounded employment bonus must fit u16");
    DISTRICT_BACKGROUND_EMPLOYMENT_BASIS_POINTS
        .saturating_add(formal_employment_bonus)
        .saturating_add(completed_public_work_employment_bonus_basis_points(
            state,
            district_id,
        ))
        .min(10_000)
}

/// Per-cause unrest pressure shares used by the monthly district model, in
/// unrest-weighted basis points. Projection reads these same values so the
/// dashboard's district drivers cannot drift from the simulation's own
/// definition of what strains a district.
pub(crate) struct DistrictUnrestPressures {
    pub food: u16,
    pub safety: u16,
    pub employment: u16,
    pub sanitation: u16,
    pub rent: u16,
}

pub(crate) fn district_unrest_pressures(
    district: &DistrictRuntime,
    food_satisfaction: u16,
) -> DistrictUnrestPressures {
    DistrictUnrestPressures {
        food: 10_000_u16.saturating_sub(food_satisfaction),
        safety: 10_000_u16.saturating_sub(district.safety_basis_points) / 3,
        employment: 6_000_u16.saturating_sub(district.employment_basis_points),
        sanitation: 7_000_u16.saturating_sub(district.sanitation_basis_points) / 2,
        rent: district.rent_index_basis_points.saturating_sub(11_000) / 2,
    }
}

pub(crate) fn district_unrest_next_basis_points(
    district: &DistrictRuntime,
    food_satisfaction: u16,
) -> u16 {
    let pressures = district_unrest_pressures(district, food_satisfaction);
    let pressure = u32::from(pressures.food)
        .saturating_add(u32::from(pressures.safety))
        .saturating_add(u32::from(pressures.employment))
        .saturating_add(u32::from(pressures.sanitation))
        .saturating_add(u32::from(pressures.rent));
    u16::try_from(
        (u32::from(district.unrest_basis_points)
            .saturating_mul(3)
            .saturating_add(pressure)
            / 5)
        .min(10_000),
    )
    .expect("bounded district unrest must fit u16")
}
