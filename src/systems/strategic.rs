//! Strategic initialization, periodic systems, and validated cross-record operations.

use crate::core::{
    AiObjective, AppState, AuditKind, AuditRecord, BusinessStatus, CharacterRole, CharacterStatus,
    ChronicleEntry, ChronicleKind, ContractStatus, Crisis, CrisisKind, CrisisStatus,
    DistrictRuntime, DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw, ExternalRoute,
    FamilyCouncilState, FamilyLink, FamilyLinkKind, HouseGovernance, InformationConfidence,
    InformationReport, InstitutionRuntime, LawKind, LegalCase, LegalCaseKind, LegalCaseStatus,
    Loan, LoanStatus, ObjectiveKind, ObjectiveStatus, OfficePower, OutboxKind, OutboxMessage,
    Property, PropertyKind, PublicWork, PublicWorkKind, PublicWorkStatus, RelationshipState,
    SupplyContract,
};
use crate::ids::{
    BusinessId, CharacterId, DistrictId, DynastyId, EmploymentId, GoodId, HouseholdId, PropertyId,
};
use crate::money::{Money, Quantity, cost_for};
use crate::registry::{InstitutionKind, Registry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StrategicError {
    #[error("business {business_id} does not exist")]
    MissingBusiness { business_id: BusinessId },
    #[error("business {business_id} is not active")]
    BusinessInactive { business_id: BusinessId },
    #[error("dynasty {dynasty_id} does not exist")]
    MissingDynasty { dynasty_id: DynastyId },
    #[error("property {property_id} does not exist")]
    MissingProperty { property_id: PropertyId },
    #[error("contract parties must be different businesses")]
    SameContractParty,
    #[error("loan parties must be different dynasties")]
    SameLoanParty,
    #[error("amount must be positive")]
    NonPositiveAmount,
    #[error("quantity must be positive")]
    NonPositiveQuantity,
    #[error("contract duration must contain at least one week")]
    EmptyContractDuration,
    #[error("seller business {seller_business_id} cannot produce good {good_id}")]
    SellerCannotProduce {
        seller_business_id: BusinessId,
        good_id: GoodId,
    },
    #[error("buyer business {buyer_business_id} does not consume good {good_id}")]
    BuyerDoesNotConsume {
        buyer_business_id: BusinessId,
        good_id: GoodId,
    },
    #[error("dynasty {dynasty_id} has only {available} available, requires {required}")]
    InsufficientDynastyFunds {
        dynasty_id: DynastyId,
        available: Money,
        required: Money,
    },
    #[error("loan interest {interest_basis_points} is outside the 0..=10000 basis-point range")]
    InterestOutOfRange { interest_basis_points: u16 },
    #[error("property {property_id} is not owned by borrower dynasty {borrower_dynasty_id}")]
    CollateralNotOwned {
        property_id: PropertyId,
        borrower_dynasty_id: DynastyId,
    },
    #[error("property {property_id} is already pledged to loan {loan_id}")]
    PropertyAlreadyPledged {
        property_id: PropertyId,
        loan_id: crate::ids::LoanId,
    },
    #[error("property {property_id} is already owned")]
    PropertyAlreadyOwned { property_id: PropertyId },
    #[error("business {business_id} is already owned by dynasty {buyer_dynasty_id}")]
    BusinessAlreadyOwned {
        business_id: BusinessId,
        buyer_dynasty_id: DynastyId,
    },
    #[error("business {business_id} with status {status:?} is not available for acquisition")]
    BusinessNotAcquirable {
        business_id: BusinessId,
        status: BusinessStatus,
    },
    #[error("character {manager_id} is not an active member of buyer dynasty {buyer_dynasty_id}")]
    InvalidAcquisitionManager {
        manager_id: CharacterId,
        buyer_dynasty_id: DynastyId,
    },
    #[error(
        "business {business_id} requires at least {required} recapitalization, but {provided} was provided"
    )]
    InsufficientBusinessRecapitalization {
        business_id: BusinessId,
        provided: Money,
        required: Money,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyContractTerms {
    pub buyer_business_id: BusinessId,
    pub seller_business_id: BusinessId,
    pub good_id: GoodId,
    pub quantity_per_week: Quantity,
    pub unit_price: Money,
    pub penalty: Money,
    pub duration_weeks: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoanTerms {
    pub lender_dynasty_id: DynastyId,
    pub borrower_dynasty_id: DynastyId,
    pub principal: Money,
    pub weekly_payment: Money,
    pub interest_basis_points: u16,
    pub collateral_property_id: Option<PropertyId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessAcquisitionQuote {
    pub business_id: BusinessId,
    pub seller_dynasty_id: DynastyId,
    pub purchase_price: Money,
    pub minimum_recapitalization: Money,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ObjectiveProgress {
    Pending,
    Achieved,
}

impl ObjectiveProgress {
    const fn from_achieved(achieved: bool) -> Self {
        if achieved {
            Self::Achieved
        } else {
            Self::Pending
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DueContract {
    id: crate::ids::ContractId,
    buyer_id: BusinessId,
    seller_id: BusinessId,
    good_id: GoodId,
    quantity: Quantity,
    unit_price: Money,
    penalty: Money,
    end_day: i64,
}

#[derive(Clone, Copy, Debug)]
struct ContractSettlementState {
    buyer_owner_id: DynastyId,
    seller_owner_id: DynastyId,
    seller_can_deliver: bool,
    buyer_can_pay: bool,
}

#[derive(Debug)]
pub struct ValidatedSupplyContract {
    terms: SupplyContractTerms,
}

impl ValidatedSupplyContract {
    /// Revalidates and commits a supply contract exactly once.
    ///
    /// # Errors
    ///
    /// Returns the current validation error if state changed after the token was created.
    pub fn commit(
        self,
        registry: &Registry,
        state: &mut AppState,
    ) -> Result<crate::ids::ContractId, StrategicError> {
        validate_supply_contract_terms(registry, state, &self.terms)?;
        Ok(commit_supply_contract(state, &self.terms))
    }
}

fn commit_supply_contract(
    state: &mut AppState,
    terms: &SupplyContractTerms,
) -> crate::ids::ContractId {
    let &SupplyContractTerms {
        buyer_business_id,
        seller_business_id,
        good_id,
        quantity_per_week,
        unit_price,
        penalty,
        duration_weeks,
    } = terms;
    let id = state.next_ids.contract();
    let day = state.clock.day();
    let end_day = day.saturating_add(i64::from(duration_weeks).saturating_mul(7));
    state.contracts.insert(
        id,
        SupplyContract {
            id,
            buyer_business_id,
            seller_business_id,
            good_id,
            quantity_per_week,
            unit_price,
            penalty,
            next_due_day: day.saturating_add(7),
            end_day,
            fulfilled_deliveries: 0,
            missed_deliveries: 0,
            status: ContractStatus::Active,
        },
    );
    push_outbox(
        state,
        OutboxKind::Contract,
        format!("Supply contract {id} signed"),
        format!(
            "Business {seller_business_id} will deliver {quantity_per_week} of good {good_id} to business {buyer_business_id} each week."
        ),
    );
    id
}

#[derive(Debug)]
pub struct ValidatedLoan {
    terms: LoanTerms,
}

impl ValidatedLoan {
    /// Revalidates and commits a previously validated loan atomically.
    ///
    /// # Errors
    ///
    /// Returns the current validation error if state changed after the token was created.
    pub fn commit(self, state: &mut AppState) -> Result<crate::ids::LoanId, StrategicError> {
        validate_loan_terms(state, &self.terms)?;
        Ok(commit_loan(state, &self.terms))
    }
}

fn commit_loan(state: &mut AppState, terms: &LoanTerms) -> crate::ids::LoanId {
    let &LoanTerms {
        lender_dynasty_id,
        borrower_dynasty_id,
        principal,
        weekly_payment,
        interest_basis_points,
        collateral_property_id,
    } = terms;
    let id = state.next_ids.loan();
    let lender = state
        .dynasties
        .get_mut(&lender_dynasty_id)
        .expect("validated lender must exist");
    lender.resources.treasury = lender.resources.treasury.saturating_sub(principal);
    let borrower = state
        .dynasties
        .get_mut(&borrower_dynasty_id)
        .expect("validated borrower must exist");
    borrower.resources.treasury = borrower.resources.treasury.saturating_add(principal);
    if let Some(property_id) = collateral_property_id {
        state
            .properties
            .get_mut(&property_id)
            .expect("validated collateral must exist")
            .collateral_loan_id = Some(id);
    }
    state.loans.insert(
        id,
        Loan {
            id,
            lender_dynasty_id,
            borrower_dynasty_id,
            principal,
            balance: principal,
            weekly_payment,
            interest_basis_points,
            next_due_day: state.clock.day().saturating_add(7),
            missed_payments: 0,
            collateral_property_id,
            status: LoanStatus::Current,
        },
    );
    push_outbox(
        state,
        OutboxKind::Finance,
        format!("Loan {id} issued"),
        format!("Dynasty {lender_dynasty_id} lent {principal} to dynasty {borrower_dynasty_id}."),
    );
    id
}

/// Validates a supply contract without mutating state.
///
/// # Errors
///
/// Returns an error for missing parties, invalid quantities, or incompatible production chains.
///
/// # Panics
///
/// Panics if an existing business contains an invalid recipe reference.
pub fn validate_supply_contract(
    registry: &Registry,
    state: &AppState,
    terms: SupplyContractTerms,
) -> Result<ValidatedSupplyContract, StrategicError> {
    validate_supply_contract_terms(registry, state, &terms)?;
    Ok(ValidatedSupplyContract { terms })
}

fn validate_supply_contract_terms(
    registry: &Registry,
    state: &AppState,
    terms: &SupplyContractTerms,
) -> Result<(), StrategicError> {
    if terms.buyer_business_id == terms.seller_business_id {
        return Err(StrategicError::SameContractParty);
    }
    if terms.quantity_per_week <= Quantity::ZERO {
        return Err(StrategicError::NonPositiveQuantity);
    }
    if terms.unit_price <= Money::ZERO || terms.penalty < Money::ZERO {
        return Err(StrategicError::NonPositiveAmount);
    }
    if terms.duration_weeks == 0 {
        return Err(StrategicError::EmptyContractDuration);
    }
    let buyer =
        state
            .businesses
            .get(terms.buyer_business_id)
            .ok_or(StrategicError::MissingBusiness {
                business_id: terms.buyer_business_id,
            })?;
    let seller =
        state
            .businesses
            .get(terms.seller_business_id)
            .ok_or(StrategicError::MissingBusiness {
                business_id: terms.seller_business_id,
            })?;
    for business in [buyer, seller] {
        if matches!(
            business.status(),
            crate::core::BusinessStatus::Insolvent | crate::core::BusinessStatus::Closed
        ) {
            return Err(StrategicError::BusinessInactive {
                business_id: business.id(),
            });
        }
    }
    let seller_recipe = registry
        .get_recipe(seller.recipe_id())
        .expect("business recipe references must be validated");
    if seller_recipe.output_good_id() != terms.good_id {
        return Err(StrategicError::SellerCannotProduce {
            seller_business_id: terms.seller_business_id,
            good_id: terms.good_id,
        });
    }
    let buyer_recipe = registry
        .get_recipe(buyer.recipe_id())
        .expect("business recipe references must be validated");
    if !buyer_recipe
        .inputs()
        .iter()
        .any(|input| input.good_id() == terms.good_id)
    {
        return Err(StrategicError::BuyerDoesNotConsume {
            buyer_business_id: terms.buyer_business_id,
            good_id: terms.good_id,
        });
    }
    Ok(())
}

/// Validates and creates a supply contract through its canonical commit token.
///
/// # Errors
///
/// Returns the same errors as [`validate_supply_contract`].
pub fn sign_supply_contract(
    registry: &Registry,
    state: &mut AppState,
    terms: SupplyContractTerms,
) -> Result<crate::ids::ContractId, StrategicError> {
    validate_supply_contract(registry, state, terms)?.commit(registry, state)
}

/// Validates a loan without mutating state.
///
/// # Errors
///
/// Returns an error for missing parties, invalid terms, insufficient lender funds, or invalid collateral.
pub fn validate_loan(state: &AppState, terms: LoanTerms) -> Result<ValidatedLoan, StrategicError> {
    validate_loan_terms(state, &terms)?;
    Ok(ValidatedLoan { terms })
}

fn validate_loan_terms(state: &AppState, terms: &LoanTerms) -> Result<(), StrategicError> {
    if terms.lender_dynasty_id == terms.borrower_dynasty_id {
        return Err(StrategicError::SameLoanParty);
    }
    if terms.principal <= Money::ZERO || terms.weekly_payment <= Money::ZERO {
        return Err(StrategicError::NonPositiveAmount);
    }
    if terms.interest_basis_points > 10_000 {
        return Err(StrategicError::InterestOutOfRange {
            interest_basis_points: terms.interest_basis_points,
        });
    }
    let lender =
        state
            .dynasties
            .get(&terms.lender_dynasty_id)
            .ok_or(StrategicError::MissingDynasty {
                dynasty_id: terms.lender_dynasty_id,
            })?;
    if !state.dynasties.contains_key(&terms.borrower_dynasty_id) {
        return Err(StrategicError::MissingDynasty {
            dynasty_id: terms.borrower_dynasty_id,
        });
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
        if let Some(loan_id) = property.collateral_loan_id {
            return Err(StrategicError::PropertyAlreadyPledged {
                property_id,
                loan_id,
            });
        }
    }
    Ok(())
}

/// Validates and issues a loan through its canonical commit token.
///
/// # Errors
///
/// Returns the same errors as [`validate_loan`].
pub fn issue_loan(
    state: &mut AppState,
    terms: LoanTerms,
) -> Result<crate::ids::LoanId, StrategicError> {
    validate_loan(state, terms)?.commit(state)
}

/// Transfers an unowned property to a dynasty after validating price and ownership.
///
/// # Errors
///
/// Returns an error when the property or buyer is missing, the property is owned, or funds are insufficient.
///
/// # Panics
///
/// Panics if validated records are removed between validation and commit within this call.
pub fn buy_unowned_property(
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
) -> Result<(), StrategicError> {
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
    state
        .dynasties
        .get_mut(&buyer_dynasty_id)
        .expect("validated buyer must exist")
        .resources
        .treasury = buyer.treasury().saturating_sub(price);
    state
        .properties
        .get_mut(&property_id)
        .expect("validated property must exist")
        .owner_dynasty_id = Some(buyer_dynasty_id);
    push_outbox(
        state,
        OutboxKind::Property,
        format!("Property {property_id} acquired"),
        format!("Dynasty {buyer_dynasty_id} acquired the property for {price}."),
    );
    Ok(())
}

/// Returns the canonical price and minimum working-capital requirement for acquiring a troubled
/// business.
///
/// # Errors
///
/// Returns an error when the business or buyer is missing, the buyer already owns the business,
/// or the business is still active and therefore not available for acquisition.
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
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe references must be validated");
    let inventory_value =
        business
            .inventory()
            .iter()
            .fold(Money::ZERO, |total, (good_id, quantity)| {
                let unit_price = state
                    .market
                    .quotes
                    .get(good_id)
                    .expect("business inventory good must have a market quote")
                    .price;
                total.saturating_add(cost_for(*quantity, unit_price))
            });
    let capacity = i64::from(business.operations.capacity_batches_per_day);
    let equipment_value = recipe
        .daily_operating_cost()
        .saturating_mul(capacity)
        .saturating_mul(60)
        .saturating_mul(i64::from(
            business.operations.condition_basis_points.max(1_000),
        ));
    let equipment_value = Money::from_copper(equipment_value.copper() / 10_000);
    let goodwill_value = recipe
        .daily_operating_cost()
        .saturating_mul(capacity)
        .saturating_mul(30)
        .saturating_mul(i64::from(business.operations.quality_basis_points));
    let goodwill_value = Money::from_copper(goodwill_value.copper() / 10_000);
    let gross_value = business
        .cash()
        .saturating_add(inventory_value)
        .saturating_add(equipment_value)
        .saturating_add(goodwill_value);
    let discounted_value = gross_value.saturating_mul(discount_basis_points);
    let purchase_price = Money::from_copper((discounted_value.copper() / 10_000).max(500));
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

#[derive(Clone, Copy, Debug)]
struct ValidatedBusinessAcquisition {
    quote: BusinessAcquisitionQuote,
    buyer_treasury: Money,
    administrative_load: u16,
}

/// Acquires a troubled business, installs an eligible manager, and supplies enough working
/// capital for it to resume active operation.
///
/// # Errors
///
/// Returns an error for an unavailable business, invalid manager, insufficient recapitalization,
/// or insufficient buyer treasury funds. Failed acquisitions leave state unchanged.
///
/// # Panics
///
/// Panics only if validated records are removed or altered between validation and commit within
/// this call.
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
    commit_business_acquisition(
        state,
        buyer_dynasty_id,
        manager_id,
        recapitalization,
        validated,
    );
    Ok(validated.quote)
}

fn validate_business_acquisition(
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
    let total_required = quote.purchase_price.saturating_add(recapitalization);
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
    let recipe_id = state
        .businesses
        .get(business_id)
        .expect("quoted business must exist")
        .recipe_id();
    let administrative_load = registry
        .get_recipe(recipe_id)
        .expect("business recipe references must be validated")
        .administrative_load();
    Ok(ValidatedBusinessAcquisition {
        quote,
        buyer_treasury,
        administrative_load,
    })
}

fn commit_business_acquisition(
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    manager_id: CharacterId,
    recapitalization: Money,
    validated: ValidatedBusinessAcquisition,
) {
    let quote = validated.quote;
    let business_id = quote.business_id;
    let total_required = quote.purchase_price.saturating_add(recapitalization);
    state
        .dynasties
        .get_mut(&buyer_dynasty_id)
        .expect("validated buyer must exist")
        .resources
        .treasury = validated.buyer_treasury.saturating_sub(total_required);
    let seller = state
        .dynasties
        .get_mut(&quote.seller_dynasty_id)
        .expect("business owner dynasty must exist");
    seller.resources.treasury = seller
        .resources
        .treasury
        .saturating_add(quote.purchase_price);
    seller.resources.administrative_load = seller
        .resources
        .administrative_load
        .saturating_sub(validated.administrative_load);
    let buyer = state
        .dynasties
        .get_mut(&buyer_dynasty_id)
        .expect("validated buyer must exist");
    buyer.resources.administrative_load = buyer
        .resources
        .administrative_load
        .saturating_add(validated.administrative_load);

    let prior_owner = state
        .businesses
        .transfer_ownership(business_id, buyer_dynasty_id, manager_id)
        .expect("validated business must exist");
    debug_assert_eq!(prior_owner, quote.seller_dynasty_id);
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("transferred business must exist");
    business.finance.cash = business.finance.cash.saturating_add(recapitalization);
    business.finance.version = business.finance.version.saturating_add(1);
    business.operations.status = BusinessStatus::Active;

    record_business_acquisition(state, buyer_dynasty_id, manager_id, recapitalization, quote);
}

fn record_business_acquisition(
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    manager_id: CharacterId,
    recapitalization: Money,
    quote: BusinessAcquisitionQuote,
) {
    let business_id = quote.business_id;
    let chronicle_id = state.next_ids.chronicle();
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
        subject: format!("business:{business_id}"),
        detail: format!(
            "buyer={buyer_dynasty_id}; seller={}; price={}; recapitalization={}; manager={manager_id}",
            quote.seller_dynasty_id,
            quote.purchase_price.copper(),
            recapitalization.copper()
        ),
    });
    push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} acquired"),
        format!(
            "The dynasty paid {} and supplied {} working capital. Character {manager_id} now manages the enterprise.",
            quote.purchase_price, recapitalization
        ),
    );
}

pub(crate) fn initialize_strategic_state(registry: &Registry, state: &mut AppState) {
    initialize_districts(registry, state);
    initialize_institutions(registry, state);
    initialize_properties(registry, state);
    initialize_employment(state);
    initialize_family_governance(state);
    initialize_relationships(state);
    initialize_laws(state);
    initialize_routes(registry, state);
    initialize_contracts(registry, state);
    initialize_loans(state);
    initialize_objectives(state);
    initialize_public_works(registry, state);
    initialize_legal_cases(state);
    initialize_information(state);
}

fn initialize_districts(registry: &Registry, state: &mut AppState) {
    for district in registry.districts() {
        state.districts.insert(
            district.id(),
            DistrictRuntime {
                district_id: district.id(),
                rent_index_basis_points: 10_000,
                employment_basis_points: 7_200,
                sanitation_basis_points: if district.key() == "southern_reach" {
                    4_200
                } else {
                    6_500
                },
                safety_basis_points: if district.key() == "riverside" {
                    5_400
                } else {
                    6_800
                },
                unrest_basis_points: if district.key() == "southern_reach" {
                    2_800
                } else {
                    1_200
                },
                dynasty_support: Vec::new(),
            },
        );
    }
}

fn powers_for(kind: InstitutionKind) -> BTreeSet<OfficePower> {
    let values: &[OfficePower] = match kind {
        InstitutionKind::CraftGuild => &[OfficePower::Licenses, OfficePower::Inspections],
        InstitutionKind::MerchantGuild => &[OfficePower::CityContracts, OfficePower::MarketTolls],
        InstitutionKind::Council => &[
            OfficePower::Taxation,
            OfficePower::PublicWorks,
            OfficePower::CityContracts,
        ],
        InstitutionKind::Court => &[OfficePower::DebtEnforcement],
        InstitutionKind::Watch => &[OfficePower::WatchPriorities],
        InstitutionKind::Treasury => &[OfficePower::Taxation, OfficePower::CityContracts],
        InstitutionKind::Charity => &[OfficePower::EmergencyImports],
        InstitutionKind::MarketOffice => &[
            OfficePower::Inspections,
            OfficePower::MarketTolls,
            OfficePower::EmergencyImports,
        ],
    };
    values.iter().copied().collect()
}

fn initialize_institutions(registry: &Registry, state: &mut AppState) {
    for definition in registry.institutions() {
        let mut members = BTreeSet::new();
        for dynasty in state.dynasties.values() {
            members.insert(dynasty.head_id());
        }
        let office_holder_id = if definition.key() == "city_council" {
            state
                .dynasties
                .values()
                .find(|dynasty| dynasty.id() != state.player_dynasty_id)
                .map(crate::core::Dynasty::head_id)
        } else {
            None
        };
        state.institutions.insert(
            definition.id(),
            InstitutionRuntime {
                institution_id: definition.id(),
                members,
                office_holder_id,
                powers: powers_for(definition.kind()),
                budget: Money::from_copper(120_000),
                legitimacy_basis_points: 7_000,
                next_selection_day: 360,
                term_number: 1,
            },
        );
    }
}

fn initialize_properties(registry: &Registry, state: &mut AppState) {
    let businesses: Vec<_> = state
        .businesses
        .iter()
        .map(|business| {
            (
                business.id(),
                business.name().to_owned(),
                business.owner_dynasty_id(),
                business.district_id(),
            )
        })
        .collect();
    for (business_id, name, owner_dynasty_id, district_id) in businesses {
        let property_id = state.next_ids.property();
        state.properties.insert(
            property_id,
            Property {
                id: property_id,
                name: format!("{name} Premises"),
                kind: PropertyKind::Workshop,
                district_id,
                owner_dynasty_id: Some(owner_dynasty_id),
                occupant_business_id: Some(business_id),
                tenant_dynasty_id: None,
                value: Money::from_copper(28_000),
                weekly_rent: Money::from_copper(340),
                condition_basis_points: 8_000,
                collateral_loan_id: None,
            },
        );
    }
    for dynasty in state.dynasties.values() {
        let district_id = state
            .businesses
            .ids_for_owner(dynasty.id())
            .and_then(|ids| ids.iter().next())
            .and_then(|id| state.businesses.get(*id))
            .map_or_else(
                || registry.districts()[0].id(),
                crate::core::Business::district_id,
            );
        let property_id = state.next_ids.property();
        state.properties.insert(
            property_id,
            Property {
                id: property_id,
                name: format!("House {} Residence", dynasty.name()),
                kind: PropertyKind::Residence,
                district_id,
                owner_dynasty_id: Some(dynasty.id()),
                occupant_business_id: None,
                tenant_dynasty_id: None,
                value: Money::from_copper(45_000),
                weekly_rent: Money::ZERO,
                condition_basis_points: 8_500,
                collateral_loan_id: None,
            },
        );
    }
    for district in registry.districts() {
        let property_id = state.next_ids.property();
        state.properties.insert(
            property_id,
            Property {
                id: property_id,
                name: format!("Vacant {} Warehouse", district.name()),
                kind: PropertyKind::Warehouse,
                district_id: district.id(),
                owner_dynasty_id: None,
                occupant_business_id: None,
                tenant_dynasty_id: None,
                value: Money::from_copper(34_000),
                weekly_rent: Money::from_copper(420),
                condition_basis_points: 6_500,
                collateral_loan_id: None,
            },
        );
    }
}

fn initialize_employment(state: &mut AppState) {
    let businesses: Vec<_> = state
        .businesses
        .iter()
        .map(|business| {
            (
                business.id(),
                business.district_id(),
                business
                    .operations
                    .capacity_batches_per_day
                    .saturating_mul(super::WORKERS_PER_BATCH),
            )
        })
        .collect();
    for (business_id, district_id, workers) in businesses {
        let Some(household_id) = state
            .households
            .ids_for_district(district_id)
            .and_then(|ids| {
                ids.iter().find(|id| {
                    super::available_household_workers(state, **id, None) >= u32::from(workers)
                })
            })
            .copied()
        else {
            continue;
        };
        let id = state.next_ids.employment();
        state.employment.insert(
            id,
            EmploymentAgreement {
                id,
                business_id,
                household_id,
                workers,
                weekly_wage: Money::from_copper(i64::from(workers).saturating_mul(35)),
                loyalty_basis_points: 6_500,
                conditions_basis_points: 6_800,
                status: EmploymentStatus::Active,
            },
        );
    }
}

fn initialize_family_governance(state: &mut AppState) {
    let dynasties: Vec<_> = state
        .dynasties
        .values()
        .map(|dynasty| (dynasty.id(), dynasty.head_id(), dynasty.heir_id()))
        .collect();
    for (dynasty_id, head_id, heir_id) in dynasties {
        let mut members = BTreeSet::from([head_id]);
        if let Some(heir_id) = heir_id {
            members.insert(heir_id);
            let id = state.next_ids.family_link();
            state.family_links.insert(
                id,
                FamilyLink {
                    id,
                    first_character_id: head_id,
                    second_character_id: heir_id,
                    kind: FamilyLinkKind::ParentChild,
                    active: true,
                    property_claim_basis_points: 8_000,
                },
            );
        }
        state.family_councils.insert(
            dynasty_id,
            FamilyCouncilState {
                dynasty_id,
                governance: HouseGovernance::Primogeniture,
                members,
                unity_basis_points: 7_500,
                charter_version: 1,
            },
        );
    }
}

fn initialize_relationships(state: &mut AppState) {
    let dynasty_ids: Vec<_> = state.dynasties.keys().copied().collect();
    for (index, left) in dynasty_ids.iter().enumerate() {
        for right in dynasty_ids.iter().skip(index + 1) {
            let pair = DynastyPair::new(*left, *right);
            state.relationships.insert(
                pair,
                RelationshipState {
                    pair,
                    trust_basis_points: 4_000
                        + u16::try_from(state.rng.range_u32(2_500)).expect("random trust fits"),
                    fear_basis_points: 1_000
                        + u16::try_from(state.rng.range_u32(1_500)).expect("random fear fits"),
                    respect_basis_points: 4_000
                        + u16::try_from(state.rng.range_u32(2_500)).expect("random respect fits"),
                    obligation: 0,
                    resentment_basis_points: 1_500
                        + u16::try_from(state.rng.range_u32(1_500))
                            .expect("random resentment fits"),
                    last_interaction_day: 0,
                    memories: Vec::new(),
                },
            );
        }
    }
}

fn initialize_laws(state: &mut AppState) {
    for (kind, value) in [
        (LawKind::ForeignMerchantToll, 500),
        (LawKind::FireCode, 6_000),
        (LawKind::GuildEntryRestriction, 1),
    ] {
        let id = state.next_ids.law();
        state.laws.insert(
            id,
            EnactedLaw {
                id,
                kind,
                enacted_day: 0,
                sponsor_dynasty_id: None,
                value,
                active: true,
            },
        );
    }
}

fn initialize_routes(registry: &Registry, state: &mut AppState) {
    let routes = [
        ("Western Grain Road", "grain", 20, 900),
        ("Upland Wool Road", "wool", 10, 1_100),
        ("Northern Timber Road", "timber", 14, 1_300),
        ("Valley Ore Road", "iron", 7, 1_500),
    ];
    for (name, good_key, capacity, risk) in routes {
        let good_id = registry
            .get_good_id(good_key)
            .unwrap_or_else(|| panic!("missing required route good {good_key}"));
        let id = state.next_ids.external_route();
        state.external_routes.insert(
            id,
            ExternalRoute {
                id,
                name: name.to_owned(),
                good_id,
                daily_capacity: Quantity::from_units(capacity),
                risk_basis_points: risk,
                disruption_basis_points: 0,
                toll_basis_points: 500,
                active: true,
            },
        );
    }
}

fn initialize_contracts(registry: &Registry, state: &mut AppState) {
    let businesses: Vec<_> = state
        .businesses
        .iter()
        .map(|business| {
            (
                business.id(),
                business.owner_dynasty_id(),
                business.recipe_id(),
            )
        })
        .collect();
    let mut created = 0_u16;
    for (buyer_id, buyer_owner, buyer_recipe_id) in &businesses {
        let buyer_recipe = registry
            .get_recipe(*buyer_recipe_id)
            .expect("business recipes must resolve");
        for input in buyer_recipe.inputs() {
            let seller = businesses
                .iter()
                .find(|(_, seller_owner, seller_recipe_id)| {
                    if seller_owner == buyer_owner {
                        return false;
                    }
                    registry
                        .get_recipe(*seller_recipe_id)
                        .is_some_and(|recipe| recipe.output_good_id() == input.good_id())
                });
            let Some((seller_id, _, _)) = seller else {
                continue;
            };
            let price = state
                .market
                .get_quote(input.good_id())
                .expect("market quote must exist")
                .price();
            let terms = SupplyContractTerms {
                buyer_business_id: *buyer_id,
                seller_business_id: *seller_id,
                good_id: input.good_id(),
                quantity_per_week: input.quantity().saturating_mul_ratio(5, 1),
                unit_price: price,
                penalty: cost_for(input.quantity(), price).saturating_mul(2),
                duration_weeks: 52,
            };
            if let Ok(token) = validate_supply_contract(registry, state, terms) {
                token.commit(registry, state).expect(
                    "validated bootstrap contract must commit without intervening mutation",
                );
                created = created.saturating_add(1);
            }
            if created >= 8 {
                return;
            }
        }
    }
}

fn initialize_loans(state: &mut AppState) {
    let dynasty_ids: Vec<_> = state.dynasties.keys().copied().collect();
    for pair in dynasty_ids.windows(2).take(2) {
        let [lender, borrower] = pair else {
            continue;
        };
        let terms = LoanTerms {
            lender_dynasty_id: *lender,
            borrower_dynasty_id: *borrower,
            principal: Money::from_copper(8_000),
            weekly_payment: Money::from_copper(450),
            interest_basis_points: 900,
            collateral_property_id: state
                .properties
                .values()
                .find(|property| property.owner_dynasty_id == Some(*borrower))
                .map(|property| property.id),
        };
        let token = validate_loan(state, terms)
            .expect("authored bootstrap loan must satisfy strategic validation");
        token
            .commit(state)
            .expect("validated bootstrap loan must commit without intervening mutation");
    }
}

fn initialize_objectives(state: &mut AppState) {
    let dynasty_ids: Vec<_> = state
        .dynasties
        .keys()
        .copied()
        .filter(|id| *id != state.player_dynasty_id)
        .collect();
    for (index, dynasty_id) in dynasty_ids.into_iter().enumerate() {
        let kind = match index % 5 {
            0 => ObjectiveKind::AcquireProperty,
            1 => ObjectiveKind::WinOffice,
            2 => ObjectiveKind::SecureSupply,
            3 => ObjectiveKind::ImproveLegitimacy,
            _ => ObjectiveKind::AccumulateCash,
        };
        let id = state.next_ids.objective();
        state.ai_objectives.insert(
            id,
            AiObjective {
                id,
                dynasty_id,
                kind,
                target_dynasty_id: Some(state.player_dynasty_id),
                priority: 60 + u16::try_from(index).unwrap_or(0),
                created_day: 0,
                status: ObjectiveStatus::Pursuing,
                rationale: format!("House strategy selected from current assets and institutional access: {kind:?}."),
            },
        );
    }
}

fn initialize_public_works(registry: &Registry, state: &mut AppState) {
    let district_id = registry
        .get_district_id("southern_reach")
        .expect("Rivergate registry must define southern_reach");
    let id = state.next_ids.public_work();
    state.public_works.insert(
        id,
        PublicWork {
            id,
            district_id,
            kind: PublicWorkKind::Drainage,
            sponsor_dynasty_id: None,
            budget: Money::from_copper(60_000),
            spent: Money::ZERO,
            progress_basis_points: 0,
            status: PublicWorkStatus::Building,
        },
    );
}

fn initialize_legal_cases(state: &mut AppState) {
    let dynasty_ids: Vec<_> = state.dynasties.keys().copied().collect();
    let Some(plaintiff_dynasty_id) = dynasty_ids.first().copied() else {
        return;
    };
    let Some(defendant_dynasty_id) = dynasty_ids.get(1).copied() else {
        return;
    };
    let id = state.next_ids.legal_case();
    state.legal_cases.insert(
        id,
        LegalCase {
            id,
            plaintiff_dynasty_id,
            defendant_dynasty_id,
            kind: LegalCaseKind::Debt,
            evidence_basis_points: 6_500,
            public_attention_basis_points: 2_000,
            filed_day: 0,
            hearing_day: 90,
            damages: Money::from_copper(2_500),
            status: LegalCaseStatus::Filed,
        },
    );
}

fn initialize_information(state: &mut AppState) {
    let id = state.next_ids.information_report();
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: state.player_dynasty_id,
            subject: "Rivergate opening conditions".to_owned(),
            confidence: InformationConfidence::Confirmed,
            created_day: 0,
            expires_day: 90,
            source: "Household account books and market inspection".to_owned(),
            summary: "Food prices are politically sensitive, the southern district lacks sanitation, and the treasury remains indebted after wall repairs.".to_owned(),
        },
    );
    push_outbox(
        state,
        OutboxKind::Information,
        "Rivergate briefing available".to_owned(),
        "The dynasty ledger now includes contracts, property, credit, institutional power, district conditions, and strategic reports.".to_owned(),
    );
}

pub(crate) fn run_daily_strategic_systems(registry: &Registry, state: &mut AppState) {
    apply_route_laws(state);
    apply_crisis_daily_effects(registry, state);
    apply_external_route_supply(state);
}

fn active_law_value(state: &AppState, kind: LawKind) -> Option<i64> {
    state
        .laws
        .values()
        .find(|law| law.active && law.kind == kind)
        .map(|law| law.value)
}

fn apply_route_laws(state: &mut AppState) {
    let Some(toll) = active_law_value(state, LawKind::ForeignMerchantToll) else {
        return;
    };
    let toll = u16::try_from(toll.clamp(0, 10_000)).unwrap_or(10_000);
    for route in state.external_routes.values_mut() {
        route.toll_basis_points = toll;
    }
}

fn apply_external_route_supply(state: &mut AppState) {
    let routes: Vec<_> = state
        .external_routes
        .values()
        .filter(|route| route.active)
        .map(|route| {
            let disruption_availability = 10_000_u16.saturating_sub(route.disruption_basis_points);
            let toll_availability = 10_000_u16.saturating_sub(route.toll_basis_points);
            (
                route.good_id,
                route
                    .daily_capacity
                    .saturating_mul_ratio(i64::from(disruption_availability), 10_000)
                    .saturating_mul_ratio(i64::from(toll_availability), 10_000),
            )
        })
        .collect();
    for (good_id, quantity) in routes {
        let quote = state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("route good must have a market quote");
        quote.stock = quote.stock.saturating_add(quantity);
        quote.supply_today = quote.supply_today.saturating_add(quantity);
    }
}

pub(crate) fn apply_law_price_controls(registry: &Registry, state: &mut AppState) {
    let ceiling = active_law_value(state, LawKind::BreadPriceCeiling);
    let Some(ceiling) = ceiling else {
        return;
    };
    let Some(bread_id) = registry.get_good_id("bread") else {
        return;
    };
    let quote = state
        .market
        .quotes
        .get_mut(&bread_id)
        .expect("bread quote must exist");
    quote.price = quote.price.min(Money::from_copper(ceiling));
}

fn apply_crisis_daily_effects(registry: &Registry, state: &mut AppState) {
    let active: Vec<_> = state
        .crises
        .values()
        .filter(|crisis| crisis.status.is_active())
        .map(|crisis| {
            (
                crisis.kind,
                crisis.severity_basis_points,
                crisis.district_id,
            )
        })
        .collect();
    for (kind, severity, district_id) in active {
        match kind {
            CrisisKind::GrainShortage => {
                if let Some(bread_id) = registry.get_good_id("bread") {
                    let quote = state
                        .market
                        .quotes
                        .get_mut(&bread_id)
                        .expect("bread quote must exist");
                    let crisis_demand = quote
                        .target_stock
                        .saturating_mul_ratio(i64::from(severity), 100_000);
                    quote.demand_today = quote.demand_today.saturating_add(crisis_demand);
                }
            }
            CrisisKind::UrbanFire => {
                if let Some(district_id) = district_id {
                    for property in state
                        .properties
                        .values_mut()
                        .filter(|property| property.district_id == district_id)
                    {
                        property.condition_basis_points = property
                            .condition_basis_points
                            .saturating_sub((severity / 200).max(1));
                    }
                }
            }
            CrisisKind::Epidemic => {
                for household in state.households.iter_mut() {
                    household.food_satisfaction_basis_points = household
                        .food_satisfaction_basis_points
                        .saturating_sub((severity / 500).max(1));
                }
            }
            CrisisKind::TradeDisruption => {
                for route in state.external_routes.values_mut() {
                    route.disruption_basis_points = route.disruption_basis_points.max(severity);
                }
            }
            CrisisKind::GuildRevolt => {
                if let Some(district_id) = district_id
                    && let Some(district) = state.districts.get_mut(&district_id)
                {
                    district.employment_basis_points = district
                        .employment_basis_points
                        .saturating_sub((severity / 100).max(1));
                    district.unrest_basis_points = district
                        .unrest_basis_points
                        .saturating_add((severity / 200).max(1))
                        .min(10_000);
                }
            }
            CrisisKind::BankingPanic => {
                apply_banking_panic_losses(state, severity);
            }
            CrisisKind::NobleDemand => {
                if let Some(treasury_id) = registry.get_institution_id("treasury")
                    && let Some(treasury) = state.institutions.get_mut(&treasury_id)
                {
                    let levy = Money::from_copper(i64::from(severity) / 20).min(treasury.budget);
                    treasury.budget = treasury.budget.saturating_sub(levy);
                }
                if let Some(district_id) = district_id
                    && let Some(district) = state.districts.get_mut(&district_id)
                {
                    district.unrest_basis_points = district
                        .unrest_basis_points
                        .saturating_add((severity / 500).max(1))
                        .min(10_000);
                }
            }
        }
    }
}

fn apply_banking_panic_losses(state: &mut AppState, severity: u16) {
    for business in state.businesses.iter_mut() {
        let loss = Money::from_copper(
            business
                .finance
                .cash
                .copper()
                .saturating_mul(i64::from(severity))
                / 1_000_000,
        );
        if loss > Money::ZERO {
            business.finance.cash = business.finance.cash.saturating_sub(loss);
            business.finance.lifetime_costs = business.finance.lifetime_costs.saturating_add(loss);
            business.finance.version = business.finance.version.saturating_add(1);
        }
    }
}

pub(crate) fn run_weekly_strategic_systems(registry: &Registry, state: &mut AppState) {
    settle_contracts(state);
    settle_loans(state);
    settle_property_rents(state);
    settle_employment(registry, state);
    distribute_business_dividends(registry, state);
    progress_public_works(registry, state);
    update_relationships_from_obligations(state);
    update_quality_reputations(state);
    apply_law_economic_effects(registry, state);
}

fn settle_contracts(state: &mut AppState) {
    let day = state.clock.day();
    let due: Vec<_> = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active && contract.next_due_day <= day
        })
        .map(|contract| DueContract {
            id: contract.id,
            buyer_id: contract.buyer_business_id,
            seller_id: contract.seller_business_id,
            good_id: contract.good_id,
            quantity: contract.quantity_per_week,
            unit_price: contract.unit_price,
            penalty: contract.penalty,
            end_day: contract.end_day,
        })
        .collect();
    for due_contract in due {
        settle_due_contract(state, due_contract);
    }
}

fn settle_due_contract(state: &mut AppState, due: DueContract) {
    let payment = cost_for(due.quantity, due.unit_price);
    let seller = state
        .businesses
        .get(due.seller_id)
        .expect("contract seller must exist");
    let buyer = state
        .businesses
        .get(due.buyer_id)
        .expect("contract buyer must exist");
    let seller_active = !matches!(
        seller.status(),
        BusinessStatus::Insolvent | BusinessStatus::Closed
    );
    let buyer_active = !matches!(
        buyer.status(),
        BusinessStatus::Insolvent | BusinessStatus::Closed
    );
    if !seller_active || !buyer_active {
        let buyer_owner_id = buyer.owner_dynasty_id();
        let seller_owner_id = seller.owner_dynasty_id();
        let contract = state
            .contracts
            .get_mut(&due.id)
            .expect("contract must exist");
        contract.missed_deliveries = contract.missed_deliveries.saturating_add(1);
        contract.status = ContractStatus::Breached;
        if buyer_owner_id != seller_owner_id {
            if !seller_active {
                adjust_reliability_reputation(state, seller_owner_id, -120);
            }
            if !buyer_active {
                adjust_reliability_reputation(state, buyer_owner_id, -120);
            }
        }
        push_outbox(
            state,
            OutboxKind::Contract,
            format!("Contract {} terminated", due.id),
            "An inactive contract party could no longer perform the scheduled obligation."
                .to_owned(),
        );
        return;
    }
    let settlement = ContractSettlementState {
        buyer_owner_id: buyer.owner_dynasty_id(),
        seller_owner_id: seller.owner_dynasty_id(),
        seller_can_deliver: seller.inventory_quantity(due.good_id) >= due.quantity,
        buyer_can_pay: buyer.cash() >= payment,
    };
    let fulfilled = settlement.seller_can_deliver && settlement.buyer_can_pay;
    if fulfilled {
        settle_fulfilled_contract(state, due, payment, settlement);
    } else {
        settle_failed_contract(state, due, settlement);
    }
    let expired_active = state.contracts.get(&due.id).is_some_and(|contract| {
        contract.status == ContractStatus::Active && contract.next_due_day > due.end_day
    });
    if expired_active {
        state
            .contracts
            .get_mut(&due.id)
            .expect("contract must exist")
            .status = if fulfilled {
            ContractStatus::Fulfilled
        } else {
            ContractStatus::Breached
        };
        if !fulfilled {
            push_outbox(
                state,
                OutboxKind::Contract,
                format!("Contract {} expired in breach", due.id),
                "The final scheduled delivery was not completed before the contract ended."
                    .to_owned(),
            );
        }
    }
}

fn settle_fulfilled_contract(
    state: &mut AppState,
    due: DueContract,
    payment: Money,
    settlement: ContractSettlementState,
) {
    state
        .businesses
        .get_mut(due.seller_id)
        .expect("contract seller must exist")
        .remove_inventory(due.good_id, due.quantity);
    state
        .businesses
        .get_mut(due.buyer_id)
        .expect("contract buyer must exist")
        .add_inventory(due.good_id, due.quantity);
    transfer_contract_money(state, due.buyer_id, due.seller_id, payment);
    let contract = state
        .contracts
        .get_mut(&due.id)
        .expect("contract must exist");
    contract.fulfilled_deliveries = contract.fulfilled_deliveries.saturating_add(1);
    contract.next_due_day = contract.next_due_day.saturating_add(7);
    if settlement.buyer_owner_id != settlement.seller_owner_id {
        adjust_reliability_reputation(state, settlement.buyer_owner_id, 20);
        adjust_reliability_reputation(state, settlement.seller_owner_id, 20);
    }
}

fn settle_failed_contract(
    state: &mut AppState,
    due: DueContract,
    settlement: ContractSettlementState,
) {
    let penalty_parties = match (settlement.seller_can_deliver, settlement.buyer_can_pay) {
        (true, false) => Some((due.buyer_id, due.seller_id)),
        (false, true) => Some((due.seller_id, due.buyer_id)),
        (false, false) => None,
        (true, true) => unreachable!("fulfilled contracts do not enter failure settlement"),
    };
    if let Some((payer_id, recipient_id)) = penalty_parties {
        let available = state
            .businesses
            .get(payer_id)
            .expect("contract penalty payer must exist")
            .cash();
        transfer_contract_money(state, payer_id, recipient_id, due.penalty.min(available));
    }
    let breached = {
        let contract = state
            .contracts
            .get_mut(&due.id)
            .expect("contract must exist");
        contract.missed_deliveries = contract.missed_deliveries.saturating_add(1);
        contract.next_due_day = contract.next_due_day.saturating_add(7);
        if contract.missed_deliveries >= 3 {
            contract.status = ContractStatus::Breached;
        }
        contract.status == ContractStatus::Breached
    };
    if breached {
        push_outbox(
            state,
            OutboxKind::Contract,
            format!("Contract {} breached", due.id),
            format!(
                "Repeated nonperformance caused supply contract {} to terminate.",
                due.id
            ),
        );
    }
    if settlement.buyer_owner_id != settlement.seller_owner_id {
        if !settlement.seller_can_deliver {
            adjust_reliability_reputation(state, settlement.seller_owner_id, -120);
        }
        if !settlement.buyer_can_pay {
            adjust_reliability_reputation(state, settlement.buyer_owner_id, -120);
        }
    }
}

fn transfer_contract_money(
    state: &mut AppState,
    payer_id: BusinessId,
    recipient_id: BusinessId,
    amount: Money,
) {
    if amount == Money::ZERO {
        return;
    }
    {
        let payer = state
            .businesses
            .get_mut(payer_id)
            .expect("contract payer must exist");
        payer.finance.cash = payer.finance.cash.saturating_sub(amount);
        payer.finance.lifetime_costs = payer.finance.lifetime_costs.saturating_add(amount);
        payer.finance.version = payer.finance.version.saturating_add(1);
    }
    {
        let recipient = state
            .businesses
            .get_mut(recipient_id)
            .expect("contract recipient must exist");
        recipient.finance.cash = recipient.finance.cash.saturating_add(amount);
        recipient.finance.lifetime_revenue =
            recipient.finance.lifetime_revenue.saturating_add(amount);
        recipient.finance.version = recipient.finance.version.saturating_add(1);
    }
}

fn settle_loans(state: &mut AppState) {
    let day = state.clock.day();
    let interest_limit = active_interest_limit(state);
    let due: Vec<_> = state
        .loans
        .values()
        .filter(|loan| {
            matches!(
                loan.status,
                LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
            ) && loan.next_due_day <= day
        })
        .map(|loan| {
            (
                loan.id,
                loan.lender_dynasty_id,
                loan.borrower_dynasty_id,
                loan.weekly_payment,
                loan.balance,
                loan.interest_basis_points,
                loan.collateral_property_id,
            )
        })
        .collect();
    for (id, lender_id, borrower_id, weekly_payment, balance, interest, collateral) in due {
        let effective_interest = interest_limit.map_or(interest, |limit| interest.min(limit));
        let interest_due = Money::from_copper(
            balance
                .copper()
                .saturating_mul(i64::from(effective_interest))
                / 10_000
                / 52,
        );
        let accrued_balance = balance.saturating_add(interest_due);
        let amount_due = weekly_payment.min(accrued_balance);
        let borrower_treasury = state
            .dynasties
            .get(&borrower_id)
            .expect("loan borrower must exist")
            .treasury();
        state.loans.get_mut(&id).expect("loan must exist").balance = accrued_balance;
        if borrower_treasury >= amount_due {
            apply_loan_payment(state, id, amount_due);
            {
                let loan = state.loans.get_mut(&id).expect("loan must exist");
                loan.next_due_day = loan.next_due_day.saturating_add(7);
                loan.missed_payments = 0;
                if loan.status != LoanStatus::Repaid {
                    loan.status = LoanStatus::Current;
                }
            }
            adjust_reliability_reputation(state, borrower_id, 10);
        } else {
            let defaulted = {
                let loan = state.loans.get_mut(&id).expect("loan must exist");
                loan.missed_payments = loan.missed_payments.saturating_add(1);
                loan.next_due_day = loan.next_due_day.saturating_add(7);
                loan.status = if loan.missed_payments >= 3 {
                    LoanStatus::Defaulted
                } else {
                    LoanStatus::Delinquent
                };
                loan.status == LoanStatus::Defaulted
            };
            if defaulted {
                if let Some(property_id) = collateral {
                    let property = state
                        .properties
                        .get_mut(&property_id)
                        .expect("loan collateral must exist");
                    property.owner_dynasty_id = Some(lender_id);
                    property.collateral_loan_id = None;
                }
                push_outbox(
                    state,
                    OutboxKind::Finance,
                    format!("Loan {id} defaulted"),
                    format!(
                        "Dynasty {borrower_id} defaulted on its obligation to dynasty {lender_id}."
                    ),
                );
            }
            adjust_reliability_reputation(state, borrower_id, if defaulted { -400 } else { -60 });
        }
    }
}

fn active_interest_limit(state: &AppState) -> Option<u16> {
    active_law_value(state, LawKind::InterestLimit)
        .map(|value| u16::try_from(value.clamp(0, 10_000)).unwrap_or(10_000))
}

fn apply_loan_payment(state: &mut AppState, loan_id: crate::ids::LoanId, amount: Money) -> Money {
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
    if payment == Money::ZERO {
        return Money::ZERO;
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
    state
        .dynasties
        .get_mut(&borrower_id)
        .expect("loan borrower must exist")
        .resources
        .treasury = borrower_treasury.saturating_sub(payment);
    let lender = state
        .dynasties
        .get_mut(&lender_id)
        .expect("loan lender must exist");
    lender.resources.treasury = lender.resources.treasury.saturating_add(payment);
    let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
    loan.balance = loan.balance.saturating_sub(payment);
    if loan.balance == Money::ZERO {
        loan.status = LoanStatus::Repaid;
        loan.missed_payments = 0;
        if let Some(property_id) = collateral
            && let Some(property) = state.properties.get_mut(&property_id)
        {
            property.collateral_loan_id = None;
        }
    }
    payment
}

fn settle_property_rents(state: &mut AppState) {
    let annual_rent_limit =
        active_law_value(state, LawKind::RentRestriction).map(|value| value.clamp(0, 10_000));
    let rents: Vec<_> = state
        .properties
        .values()
        .filter_map(|property| {
            Some((
                property.owner_dynasty_id?,
                property.tenant_dynasty_id,
                property.occupant_business_id,
                property.weekly_rent,
                property.value,
            ))
        })
        .collect();
    for (owner_id, tenant_id, occupant_business_id, contractual_rent, property_value) in rents {
        let rent = annual_rent_limit.map_or(contractual_rent, |limit| {
            let annual_cap = Money::from_copper(
                property_value
                    .copper()
                    .saturating_mul(limit)
                    .saturating_div(10_000),
            );
            contractual_rent.min(Money::from_copper(annual_cap.copper() / 52))
        });
        if rent <= Money::ZERO {
            continue;
        }
        let paid = if let Some(tenant_id) = tenant_id {
            if owner_id == tenant_id {
                continue;
            }
            let tenant_cash = state
                .dynasties
                .get(&tenant_id)
                .expect("property tenant dynasty must exist")
                .treasury();
            let paid = rent.min(tenant_cash);
            state
                .dynasties
                .get_mut(&tenant_id)
                .expect("property tenant dynasty must exist")
                .resources
                .treasury = tenant_cash.saturating_sub(paid);
            paid
        } else if occupant_business_id.is_none() {
            state.market.clearing_account = state.market.clearing_account.saturating_sub(rent);
            rent
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
        owner.resources.treasury = owner.resources.treasury.saturating_add(paid);
    }
}

fn distribute_business_dividends(registry: &Registry, state: &mut AppState) {
    let dividends: Vec<_> = state
        .businesses
        .iter()
        .filter_map(|business| {
            if business.status() != BusinessStatus::Active
                || business.finance.lifetime_revenue <= business.finance.lifetime_costs
            {
                return None;
            }
            let recipe = registry
                .get_recipe(business.recipe_id())
                .expect("business recipe must exist");
            let operating_floor = business
                .policy
                .minimum_cash_reserve
                .saturating_add(recipe.daily_operating_cost().saturating_mul(21));
            let excess = business.cash().saturating_sub(operating_floor);
            let dividend = Money::from_copper(excess.copper() / 10).min(Money::from_copper(1_000));
            (dividend > Money::ZERO).then_some((
                business.id(),
                business.owner_dynasty_id(),
                dividend,
            ))
        })
        .collect();
    let mut total = Money::ZERO;
    for (business_id, owner_id, dividend) in dividends {
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("dividend business must exist");
        business.finance.cash = business.finance.cash.saturating_sub(dividend);
        business.finance.version = business.finance.version.saturating_add(1);
        let owner = state
            .dynasties
            .get_mut(&owner_id)
            .expect("dividend owner dynasty must exist");
        owner.resources.treasury = owner.resources.treasury.saturating_add(dividend);
        total = total.saturating_add(dividend);
    }
    if total > Money::ZERO {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::BusinessDividend,
            subject: "business-portfolio".to_owned(),
            detail: format!("dividends={}", total.copper()),
        });
    }
}

fn settle_employment(registry: &Registry, state: &mut AppState) {
    let agreements: Vec<_> = state
        .employment
        .values()
        .filter(|agreement| {
            matches!(
                agreement.status,
                EmploymentStatus::Active | EmploymentStatus::Disputed
            )
        })
        .map(|agreement| {
            (
                agreement.id,
                agreement.business_id,
                agreement.household_id,
                agreement.weekly_wage,
                agreement.status,
            )
        })
        .collect();
    for (id, business_id, household_id, wage, prior_status) in agreements {
        settle_employment_agreement(
            registry,
            state,
            id,
            business_id,
            household_id,
            wage,
            prior_status,
        );
    }
}

fn settle_employment_agreement(
    registry: &Registry,
    state: &mut AppState,
    employment_id: EmploymentId,
    business_id: BusinessId,
    household_id: HouseholdId,
    wage: Money,
    prior_status: EmploymentStatus,
) {
    let utilization_basis_points =
        business_labor_utilization_basis_points(registry, state, business_id);
    let scaled_wage = wage.saturating_mul(i64::from(utilization_basis_points));
    let wage_due = Money::from_copper(scaled_wage.copper() / 10_000);
    let paid = pay_employment_wage(registry, state, business_id, household_id, wage_due);
    let (recovered, became_disputed) = update_employment_after_payment(
        state,
        employment_id,
        prior_status,
        utilization_basis_points,
        paid,
        wage_due,
    );
    emit_employment_outcome(state, business_id, recovered, became_disputed);
}

fn pay_employment_wage(
    registry: &Registry,
    state: &mut AppState,
    business_id: BusinessId,
    household_id: HouseholdId,
    wage_due: Money,
) -> Money {
    let business = state
        .businesses
        .get(business_id)
        .expect("employment business must exist");
    let business_cash = business.cash();
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("employment business recipe must exist");
    let payroll_reserve = if business.status() == BusinessStatus::Distressed {
        recipe.daily_operating_cost()
    } else {
        business.policy.minimum_cash_reserve
    };
    let paid = wage_due.min(business_cash.saturating_sub(payroll_reserve));
    if paid <= Money::ZERO {
        return paid;
    }
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("employment business must exist");
    business.finance.cash = business_cash.saturating_sub(paid);
    business.finance.lifetime_costs = business.finance.lifetime_costs.saturating_add(paid);
    business.finance.version = business.finance.version.saturating_add(1);
    let household = state
        .households
        .get_mut(household_id)
        .expect("employment household must exist");
    household.cash = household.cash.saturating_add(paid);
    paid
}

fn update_employment_after_payment(
    state: &mut AppState,
    employment_id: EmploymentId,
    prior_status: EmploymentStatus,
    utilization_basis_points: u16,
    paid: Money,
    wage_due: Money,
) -> (bool, bool) {
    let agreement = state
        .employment
        .get_mut(&employment_id)
        .expect("employment must exist");
    if paid == wage_due {
        return update_fully_paid_employment(agreement, prior_status, utilization_basis_points);
    }
    let loyalty_loss = if prior_status == EmploymentStatus::Disputed {
        100
    } else {
        250
    };
    let condition_loss = if prior_status == EmploymentStatus::Disputed {
        50
    } else {
        100
    };
    agreement.loyalty_basis_points = agreement.loyalty_basis_points.saturating_sub(loyalty_loss);
    agreement.conditions_basis_points = agreement
        .conditions_basis_points
        .saturating_sub(condition_loss);
    let became_disputed =
        prior_status == EmploymentStatus::Active && agreement.loyalty_basis_points < 2_000;
    if became_disputed {
        agreement.status = EmploymentStatus::Disputed;
    }
    (false, became_disputed)
}

fn update_fully_paid_employment(
    agreement: &mut EmploymentAgreement,
    prior_status: EmploymentStatus,
    utilization_basis_points: u16,
) -> (bool, bool) {
    if utilization_basis_points != 10_000 {
        return (false, false);
    }
    agreement.loyalty_basis_points = agreement
        .loyalty_basis_points
        .saturating_add(if prior_status == EmploymentStatus::Disputed {
            180
        } else {
            30
        })
        .min(10_000);
    if prior_status != EmploymentStatus::Disputed {
        return (false, false);
    }
    agreement.conditions_basis_points = agreement
        .conditions_basis_points
        .saturating_add(60)
        .min(10_000);
    let recovered =
        agreement.loyalty_basis_points >= 3_000 && agreement.conditions_basis_points >= 3_000;
    if recovered {
        agreement.status = EmploymentStatus::Active;
    }
    (recovered, false)
}

fn emit_employment_outcome(
    state: &mut AppState,
    business_id: BusinessId,
    recovered: bool,
    became_disputed: bool,
) {
    if recovered {
        push_outbox(
            state,
            OutboxKind::District,
            format!("Labor dispute at business {business_id} settled"),
            "Sustained full wage payments restored a workable labor agreement.".to_owned(),
        );
    }
    if became_disputed {
        push_outbox(
            state,
            OutboxKind::District,
            format!("Labor dispute at business {business_id}"),
            "Repeated wage shortfalls have caused organized workplace resistance.".to_owned(),
        );
    }
}

fn business_labor_utilization_basis_points(
    registry: &Registry,
    state: &AppState,
    business_id: BusinessId,
) -> u16 {
    let business = state
        .businesses
        .get(business_id)
        .expect("employment business must exist");
    if matches!(
        business.status(),
        BusinessStatus::Closed | BusinessStatus::Insolvent
    ) {
        return 0;
    }
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("employment business recipe must exist");
    let output_good_id = recipe.output_good_id();
    let reserve_batches = i64::from(business.operations.capacity_batches_per_day)
        .saturating_mul(i64::from(business.policy.target_output_days));
    let policy_reserve = recipe
        .output_quantity()
        .saturating_mul_ratio(reserve_batches, 1);
    let contract_reserve = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active
                && contract.seller_business_id == business_id
                && contract.good_id == output_good_id
        })
        .fold(Quantity::ZERO, |total, contract| {
            total.saturating_add(contract.quantity_per_week)
        });
    let market_capacity =
        state
            .market
            .quotes
            .get(&output_good_id)
            .map_or(Quantity::ZERO, |quote| {
                quote
                    .target_stock
                    .saturating_mul_ratio(3, 2)
                    .saturating_sub(quote.stock)
                    .max(Quantity::ZERO)
            });
    let required_inventory = policy_reserve
        .saturating_add(contract_reserve)
        .saturating_add(market_capacity);
    if business.inventory_quantity(output_good_id) < required_inventory {
        10_000
    } else {
        2_500
    }
}

fn apply_public_work_completion(
    state: &mut AppState,
    district_id: DistrictId,
    kind: PublicWorkKind,
) {
    let Some(district) = state.districts.get_mut(&district_id) else {
        return;
    };
    match kind {
        PublicWorkKind::Drainage | PublicWorkKind::Hospital => {
            district.sanitation_basis_points = district
                .sanitation_basis_points
                .saturating_add(1_200)
                .min(10_000);
        }
        PublicWorkKind::WatchStation => {
            district.safety_basis_points = district
                .safety_basis_points
                .saturating_add(1_200)
                .min(10_000);
        }
        PublicWorkKind::Road
        | PublicWorkKind::Bridge
        | PublicWorkKind::Market
        | PublicWorkKind::Granary
        | PublicWorkKind::School => {
            district.employment_basis_points = district
                .employment_basis_points
                .saturating_add(600)
                .min(10_000);
        }
    }
    district.unrest_basis_points = district.unrest_basis_points.saturating_sub(500);
}

fn progress_public_works(registry: &Registry, state: &mut AppState) {
    let treasury_id = registry.get_institution_id("treasury");
    let ids: Vec<_> = state
        .public_works
        .values()
        .filter(|work| {
            matches!(
                work.status,
                PublicWorkStatus::Building | PublicWorkStatus::Suspended
            )
        })
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
        if weekly_spend > Money::ZERO
            && let Some(treasury_id) = treasury_id
        {
            let treasury = state
                .institutions
                .get_mut(&treasury_id)
                .expect("civic treasury runtime must exist");
            treasury.budget = treasury.budget.saturating_sub(weekly_spend);
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
                work.spent = work.spent.saturating_add(weekly_spend);
                let progress = work.spent.copper().saturating_mul(10_000) / work.budget.copper();
                work.progress_basis_points =
                    u16::try_from(progress.clamp(0, 10_000)).unwrap_or(10_000);
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
            push_outbox(
                state,
                OutboxKind::Politics,
                format!("Public work {id} suspended"),
                "Civic treasury funding is insufficient to continue construction.".to_owned(),
            );
        }
        if let Some((district_id, kind)) = completion {
            state
                .public_works
                .get_mut(&id)
                .expect("public work must exist")
                .status = PublicWorkStatus::Completed;
            apply_public_work_completion(state, district_id, kind);
            push_outbox(
                state,
                OutboxKind::Politics,
                format!("Public work {id} completed"),
                "A civic construction project has permanently changed district conditions."
                    .to_owned(),
            );
        }
    }
}

fn update_relationships_from_obligations(state: &mut AppState) {
    for relationship in state.relationships.values_mut() {
        if relationship.obligation > 0 {
            relationship.trust_basis_points = relationship
                .trust_basis_points
                .saturating_add(5)
                .min(10_000);
        } else if relationship.obligation < 0 {
            relationship.resentment_basis_points = relationship
                .resentment_basis_points
                .saturating_add(5)
                .min(10_000);
        }
    }
}

fn update_quality_reputations(state: &mut AppState) {
    let dynasty_ids: Vec<_> = state.dynasties.keys().copied().collect();
    for dynasty_id in dynasty_ids {
        let mut total_quality = 0_u64;
        let mut business_count = 0_u64;
        for business in state.businesses.iter().filter(|business| {
            business.owner_dynasty_id() == dynasty_id
                && business.status() != crate::core::BusinessStatus::Closed
        }) {
            total_quality =
                total_quality.saturating_add(u64::from(business.operations.quality_basis_points));
            business_count = business_count.saturating_add(1);
        }
        if business_count == 0 {
            continue;
        }
        let target = u16::try_from(total_quality / business_count).unwrap_or(10_000);
        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("reputation dynasty must exist");
        dynasty.resources.reputation_quality_basis_points = move_basis_points_toward(
            dynasty.resources.reputation_quality_basis_points,
            target,
            50,
        );
    }
}

fn move_basis_points_toward(current: u16, target: u16, maximum_step: u16) -> u16 {
    if current < target {
        current.saturating_add(target.saturating_sub(current).min(maximum_step))
    } else {
        current.saturating_sub(current.saturating_sub(target).min(maximum_step))
    }
}

fn adjust_reliability_reputation(state: &mut AppState, dynasty_id: DynastyId, delta: i16) {
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("reputation dynasty must exist");
    let adjusted = i32::from(dynasty.resources.reputation_reliability_basis_points)
        .saturating_add(i32::from(delta))
        .clamp(0, 10_000);
    dynasty.resources.reputation_reliability_basis_points =
        u16::try_from(adjusted).expect("clamped reputation must fit u16");
}

fn apply_law_economic_effects(registry: &Registry, state: &mut AppState) {
    let emergency_imports = active_law_value(state, LawKind::EmergencyImports)
        .map_or(Quantity::ZERO, |value| Quantity::from_units(value.max(0)));
    if emergency_imports > Quantity::ZERO
        && let Some(grain_id) = registry.get_good_id("grain")
    {
        let quote = state
            .market
            .quotes
            .get_mut(&grain_id)
            .expect("grain quote must exist");
        quote.stock = quote.stock.saturating_add(emergency_imports);
        quote.supply_today = quote.supply_today.saturating_add(emergency_imports);
    }
}

pub(crate) fn run_monthly_strategic_systems(registry: &Registry, state: &mut AppState) {
    update_district_conditions(state);
    resolve_institution_selections(state);
    apply_office_power_effects(registry, state);
    advance_ai_objectives(registry, state);
    update_information_reports(registry, state);
    resolve_legal_cases(state);
    update_external_route_risk(state);
    detect_and_advance_crises(registry, state);
    recover_external_routes(state);
}

fn recover_external_routes(state: &mut AppState) {
    for route in state.external_routes.values_mut() {
        route.disruption_basis_points = route.disruption_basis_points.saturating_sub(750);
    }
}

fn apply_office_power_effects(registry: &Registry, state: &mut AppState) {
    let offices: Vec<_> = state
        .institutions
        .values()
        .filter_map(|institution| {
            let holder = state.characters.get(institution.office_holder_id?)?;
            let district_id = registry
                .get_institution(institution.institution_id)?
                .district_id();
            Some((
                institution.institution_id,
                holder.dynasty_id(),
                district_id,
                institution.powers.iter().copied().collect::<Vec<_>>(),
            ))
        })
        .collect();
    for (institution_id, dynasty_id, district_id, powers) in offices {
        for power in powers {
            match power {
                OfficePower::Licenses => {
                    let dynasty = state
                        .dynasties
                        .get_mut(&dynasty_id)
                        .expect("officeholder dynasty must exist");
                    dynasty.resources.legitimacy_basis_points = dynasty
                        .resources
                        .legitimacy_basis_points
                        .saturating_add(15)
                        .min(10_000);
                }
                OfficePower::Inspections => {
                    let dynasty = state
                        .dynasties
                        .get_mut(&dynasty_id)
                        .expect("officeholder dynasty must exist");
                    dynasty.resources.reputation_quality_basis_points = dynasty
                        .resources
                        .reputation_quality_basis_points
                        .saturating_add(15)
                        .min(10_000);
                }
                OfficePower::MarketTolls | OfficePower::Taxation => {
                    let revenue = Money::from_copper(100);
                    state
                        .institutions
                        .get_mut(&institution_id)
                        .expect("office institution must exist")
                        .budget = state
                        .institutions
                        .get(&institution_id)
                        .expect("office institution must exist")
                        .budget
                        .saturating_add(revenue);
                    state.market.clearing_account =
                        state.market.clearing_account.saturating_sub(revenue);
                }
                OfficePower::DebtEnforcement => {
                    adjust_reliability_reputation(state, dynasty_id, 15);
                }
                OfficePower::CityContracts => {
                    award_city_contract(state, institution_id, dynasty_id);
                }
                OfficePower::PublicWorks => {
                    let district = state
                        .districts
                        .get_mut(&district_id)
                        .expect("office district must exist");
                    district.employment_basis_points = district
                        .employment_basis_points
                        .saturating_add(20)
                        .min(10_000);
                }
                OfficePower::WatchPriorities => {
                    let district = state
                        .districts
                        .get_mut(&district_id)
                        .expect("office district must exist");
                    district.safety_basis_points =
                        district.safety_basis_points.saturating_add(40).min(10_000);
                }
                OfficePower::EmergencyImports => {
                    if let Some(grain_id) = registry.get_good_id("grain") {
                        let quote = state
                            .market
                            .quotes
                            .get_mut(&grain_id)
                            .expect("grain quote must exist");
                        let quantity = Quantity::from_units(20);
                        quote.stock = quote.stock.saturating_add(quantity);
                        quote.supply_today = quote.supply_today.saturating_add(quantity);
                    }
                }
            }
        }
    }
}

fn award_city_contract(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
) {
    let business_id = state
        .businesses
        .ids_for_owner(dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|business_id| state.businesses.get(*business_id))
        .filter(|business| business.status() == BusinessStatus::Active)
        .min_by_key(|business| (business.cash(), business.id()))
        .map(crate::core::Business::id);
    let Some(business_id) = business_id else {
        return;
    };
    let institution_budget = state
        .institutions
        .get(&institution_id)
        .expect("city contract institution must exist")
        .budget;
    let award = Money::from_copper(250).min(institution_budget);
    if award == Money::ZERO {
        return;
    }
    state
        .institutions
        .get_mut(&institution_id)
        .expect("city contract institution must exist")
        .budget = institution_budget.saturating_sub(award);
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("city contract business must exist");
    business.finance.cash = business.finance.cash.saturating_add(award);
    business.finance.lifetime_revenue = business.finance.lifetime_revenue.saturating_add(award);
    business.finance.version = business.finance.version.saturating_add(1);
}

fn update_district_conditions(state: &mut AppState) {
    let district_ids: Vec<_> = state.districts.keys().copied().collect();
    for district_id in district_ids {
        let households: Vec<_> = state
            .households
            .ids_for_district(district_id)
            .into_iter()
            .flatten()
            .filter_map(|id| state.households.get(*id))
            .collect();
        let satisfaction = if households.is_empty() {
            5_000
        } else {
            let total: u64 = households
                .iter()
                .map(|household| u64::from(household.food_satisfaction_basis_points()))
                .sum();
            u16::try_from(total / households.len() as u64).unwrap_or(5_000)
        };
        let active_jobs: u32 = state
            .employment
            .values()
            .filter(|employment| {
                employment.status == EmploymentStatus::Active
                    && state
                        .businesses
                        .get(employment.business_id)
                        .is_some_and(|business| business.district_id() == district_id)
            })
            .map(|employment| u32::from(employment.workers))
            .sum();
        let district = state
            .districts
            .get_mut(&district_id)
            .expect("district runtime must exist");
        district.employment_basis_points =
            u16::try_from((active_jobs.saturating_mul(100)).min(10_000))
                .unwrap_or(10_000)
                .max(2_000);
        let hardship = 10_000_u16.saturating_sub(satisfaction);
        let unsafe_pressure = 10_000_u16.saturating_sub(district.safety_basis_points) / 3;
        district.unrest_basis_points = ((u32::from(district.unrest_basis_points) * 3
            + u32::from(hardship)
            + u32::from(unsafe_pressure))
            / 5)
        .min(10_000) as u16;
        let desirability = u32::from(district.safety_basis_points)
            .saturating_add(u32::from(district.sanitation_basis_points))
            / 2;
        district.rent_index_basis_points =
            u16::try_from(7_000_u32.saturating_add(desirability / 3).min(14_000)).unwrap_or(10_000);
    }
}

fn resolve_institution_selections(state: &mut AppState) {
    let day = state.clock.day();
    let due: Vec<_> = state
        .institutions
        .values()
        .filter(|institution| institution.next_selection_day <= day)
        .map(|institution| institution.institution_id)
        .collect();
    for institution_id in due {
        let candidates: Vec<_> = state
            .institutions
            .get(&institution_id)
            .expect("institution runtime must exist")
            .members
            .iter()
            .filter_map(|character_id| state.characters.get(*character_id))
            .filter(|character| character.status() == crate::core::CharacterStatus::Active)
            .map(|character| {
                let dynasty = state
                    .dynasties
                    .get(&character.dynasty_id())
                    .expect("candidate dynasty must exist");
                let nomination_subject =
                    super::commands::office_nomination_subject(institution_id, character.id());
                let campaign_bonus = state
                    .audit_log
                    .iter()
                    .rev()
                    .find(|record| {
                        record.kind() == AuditKind::OfficeNomination
                            && record.subject() == nomination_subject
                            && day.saturating_sub(record.day()) <= 180
                    })
                    .map_or(0_u32, |_| 4_000);
                let score = u32::from(character.capabilities.social)
                    .saturating_mul(100)
                    .saturating_add(u32::from(dynasty.resources.legitimacy_basis_points))
                    .saturating_add(campaign_bonus);
                (score, character.id())
            })
            .collect();
        let winner = candidates
            .into_iter()
            .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
            .map(|(_, character_id)| character_id);
        let term_number = {
            let institution = state
                .institutions
                .get_mut(&institution_id)
                .expect("institution runtime must exist");
            institution.office_holder_id = winner;
            institution.next_selection_day = day.saturating_add(360);
            institution.term_number = institution.term_number.saturating_add(1);
            institution.term_number
        };
        if let Some(winner) = winner {
            push_outbox(
                state,
                OutboxKind::Politics,
                format!("Institution {institution_id} selected a new officeholder"),
                format!("Character {winner} now holds the office for term {term_number}."),
            );
        }
    }
}

fn advance_ai_objectives(registry: &Registry, state: &mut AppState) {
    let objectives: Vec<_> = state
        .ai_objectives
        .values()
        .filter(|objective| objective.status == ObjectiveStatus::Pursuing)
        .map(|objective| (objective.id, objective.dynasty_id, objective.kind))
        .collect();
    for (objective_id, dynasty_id, kind) in objectives {
        let progress = match kind {
            ObjectiveKind::AcquireProperty => advance_ai_property_objective(state, dynasty_id),
            ObjectiveKind::WinOffice => advance_ai_office_objective(state, dynasty_id),
            ObjectiveKind::SecureSupply => advance_ai_supply_objective(registry, state, dynasty_id),
            ObjectiveKind::ReduceDebt => advance_ai_debt_objective(state, dynasty_id),
            ObjectiveKind::ImproveLegitimacy => advance_ai_legitimacy_objective(state, dynasty_id),
            ObjectiveKind::AccumulateCash => ObjectiveProgress::from_achieved(
                state
                    .dynasties
                    .get(&dynasty_id)
                    .is_some_and(|dynasty| dynasty.treasury() > Money::from_copper(120_000)),
            ),
            ObjectiveKind::ContainRival => advance_ai_rival_objective(state, dynasty_id),
        };
        if progress == ObjectiveProgress::Achieved {
            let objective = state
                .ai_objectives
                .get_mut(&objective_id)
                .expect("AI objective must exist");
            objective.status = ObjectiveStatus::Achieved;
            let new_id = state.next_ids.objective();
            state.ai_objectives.insert(
                new_id,
                AiObjective {
                    id: new_id,
                    dynasty_id,
                    kind: next_objective_kind(kind),
                    target_dynasty_id: Some(state.player_dynasty_id),
                    priority: 50,
                    created_day: state.clock.day(),
                    status: ObjectiveStatus::Pursuing,
                    rationale: "The prior objective was completed; the house selected the next strongest route to durable power.".to_owned(),
                },
            );
        }
    }
}

const fn next_objective_kind(kind: ObjectiveKind) -> ObjectiveKind {
    match kind {
        ObjectiveKind::AccumulateCash => ObjectiveKind::AcquireProperty,
        ObjectiveKind::AcquireProperty => ObjectiveKind::WinOffice,
        ObjectiveKind::WinOffice => ObjectiveKind::ImproveLegitimacy,
        ObjectiveKind::SecureSupply => ObjectiveKind::AccumulateCash,
        ObjectiveKind::ReduceDebt => ObjectiveKind::SecureSupply,
        ObjectiveKind::ImproveLegitimacy => ObjectiveKind::ContainRival,
        ObjectiveKind::ContainRival => ObjectiveKind::ReduceDebt,
    }
}

fn advance_ai_property_objective(state: &mut AppState, dynasty_id: DynastyId) -> ObjectiveProgress {
    let property_id = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id.is_none())
        .min_by_key(|property| (property.value, property.id))
        .map(|property| property.id);
    ObjectiveProgress::from_achieved(
        property_id.is_some_and(|property_id| {
            buy_unowned_property(state, dynasty_id, property_id).is_ok()
        }),
    )
}

fn advance_ai_office_objective(state: &mut AppState, dynasty_id: DynastyId) -> ObjectiveProgress {
    let holds_office = state.institutions.values().any(|institution| {
        institution.office_holder_id.is_some_and(|character_id| {
            state
                .characters
                .get(character_id)
                .is_some_and(|character| character.dynasty_id() == dynasty_id)
        })
    });
    if holds_office {
        return ObjectiveProgress::Achieved;
    }
    if let Some(dynasty) = state.dynasties.get_mut(&dynasty_id) {
        let spend = Money::from_copper(500).min(dynasty.resources.treasury);
        dynasty.resources.treasury = dynasty.resources.treasury.saturating_sub(spend);
        let legitimacy_gain = u16::try_from(spend.copper().saturating_mul(80) / 500)
            .unwrap_or(80)
            .min(80);
        dynasty.resources.legitimacy_basis_points = dynasty
            .resources
            .legitimacy_basis_points
            .saturating_add(legitimacy_gain)
            .min(10_000);
    }
    ObjectiveProgress::Pending
}

fn advance_ai_supply_objective(
    registry: &Registry,
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> ObjectiveProgress {
    let owner_businesses: Vec<_> = state
        .businesses
        .ids_for_owner(dynasty_id)
        .into_iter()
        .flatten()
        .copied()
        .collect();
    for buyer_id in owner_businesses {
        let buyer = state
            .businesses
            .get(buyer_id)
            .expect("indexed business must exist");
        let recipe = registry
            .get_recipe(buyer.recipe_id())
            .expect("business recipe must resolve");
        for input in recipe.inputs() {
            let already = state.contracts.values().any(|contract| {
                contract.status == ContractStatus::Active
                    && contract.buyer_business_id == buyer_id
                    && contract.good_id == input.good_id()
            });
            if already {
                return ObjectiveProgress::Achieved;
            }
            let seller_id = state.businesses.iter().find_map(|seller| {
                let seller_recipe = registry.get_recipe(seller.recipe_id())?;
                (seller.owner_dynasty_id() != dynasty_id
                    && !matches!(
                        seller.status(),
                        crate::core::BusinessStatus::Insolvent
                            | crate::core::BusinessStatus::Closed
                    )
                    && seller_recipe.output_good_id() == input.good_id())
                .then_some(seller.id())
            });
            let Some(seller_id) = seller_id else {
                continue;
            };
            let price = state
                .market
                .get_quote(input.good_id())
                .expect("market quote must exist")
                .price();
            let terms = SupplyContractTerms {
                buyer_business_id: buyer_id,
                seller_business_id: seller_id,
                good_id: input.good_id(),
                quantity_per_week: input.quantity().saturating_mul_ratio(4, 1),
                unit_price: price,
                penalty: Money::from_copper(500),
                duration_weeks: 26,
            };
            return ObjectiveProgress::from_achieved(
                sign_supply_contract(registry, state, terms).is_ok(),
            );
        }
    }
    ObjectiveProgress::Pending
}

fn advance_ai_debt_objective(state: &mut AppState, dynasty_id: DynastyId) -> ObjectiveProgress {
    let loan_id = state
        .loans
        .values()
        .find(|loan| {
            loan.borrower_dynasty_id == dynasty_id
                && matches!(
                    loan.status,
                    LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
                )
        })
        .map(|loan| loan.id);
    let Some(loan_id) = loan_id else {
        return ObjectiveProgress::Achieved;
    };
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .expect("AI dynasty must exist")
        .treasury();
    let balance = state
        .loans
        .get(&loan_id)
        .expect("AI loan must exist")
        .balance;
    let extra = Money::from_copper(1_000).min(treasury).min(balance);
    apply_loan_payment(state, loan_id, extra);
    ObjectiveProgress::from_achieved(
        state
            .loans
            .get(&loan_id)
            .is_some_and(|loan| loan.status == LoanStatus::Repaid),
    )
}

fn advance_ai_legitimacy_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> ObjectiveProgress {
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("AI dynasty must exist");
    if dynasty.resources.legitimacy_basis_points >= 7_500 {
        return ObjectiveProgress::Achieved;
    }
    let spend = Money::from_copper(750).min(dynasty.resources.treasury);
    dynasty.resources.treasury = dynasty.resources.treasury.saturating_sub(spend);
    let legitimacy_gain = u16::try_from(spend.copper().saturating_mul(120) / 750)
        .unwrap_or(120)
        .min(120);
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_add(legitimacy_gain)
        .min(10_000);
    ObjectiveProgress::Pending
}

fn advance_ai_rival_objective(state: &mut AppState, dynasty_id: DynastyId) -> ObjectiveProgress {
    if dynasty_id == state.player_dynasty_id {
        return ObjectiveProgress::Achieved;
    }
    let pair = DynastyPair::new(dynasty_id, state.player_dynasty_id);
    let Some(relationship) = state.relationships.get_mut(&pair) else {
        return ObjectiveProgress::Achieved;
    };
    relationship.fear_basis_points = relationship
        .fear_basis_points
        .saturating_add(100)
        .min(10_000);
    relationship.resentment_basis_points = relationship
        .resentment_basis_points
        .saturating_add(75)
        .min(10_000);
    ObjectiveProgress::from_achieved(relationship.fear_basis_points >= 5_000)
}

fn update_information_reports(registry: &Registry, state: &mut AppState) {
    let day = state.clock.day();
    state
        .information_reports
        .retain(|_, report| report.expires_day >= day);
    let most_changed = registry.goods().iter().filter_map(|good| {
        let quote = state.market.get_quote(good.id())?;
        let prior = quote.previous_price().copper().max(1);
        let change = (quote.price().copper() - prior).unsigned_abs();
        Some((
            change,
            good.name().to_owned(),
            quote.price(),
            quote.causes().to_vec(),
        ))
    });
    let Some((_, name, price, causes)) = most_changed.max_by_key(|item| item.0) else {
        return;
    };
    let id = state.next_ids.information_report();
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: state.player_dynasty_id,
            subject: format!("Monthly market report: {name}"),
            confidence: InformationConfidence::Confirmed,
            created_day: day,
            expires_day: day.saturating_add(120),
            source: "House ledgers, guild correspondence, and market inspection".to_owned(),
            summary: format!("{name} is priced at {price}; identified causes: {causes:?}."),
        },
    );
}

fn resolve_legal_cases(state: &mut AppState) {
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
                case.evidence_basis_points,
                case.public_attention_basis_points,
                case.damages,
            )
        })
        .collect();
    for (id, plaintiff_id, defendant_id, evidence, attention, damages) in due {
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
        if plaintiff_wins {
            let defendant_cash = state
                .dynasties
                .get(&defendant_id)
                .expect("legal defendant must exist")
                .treasury();
            let paid = damages.min(defendant_cash);
            state
                .dynasties
                .get_mut(&defendant_id)
                .expect("legal defendant must exist")
                .resources
                .treasury = defendant_cash.saturating_sub(paid);
            let plaintiff = state
                .dynasties
                .get_mut(&plaintiff_id)
                .expect("legal plaintiff must exist");
            plaintiff.resources.treasury = plaintiff.resources.treasury.saturating_add(paid);
        }
        state
            .legal_cases
            .get_mut(&id)
            .expect("legal case must exist")
            .status = if plaintiff_wins {
            LegalCaseStatus::DecidedForPlaintiff
        } else {
            LegalCaseStatus::DecidedForDefendant
        };
        push_outbox(
            state,
            OutboxKind::Legal,
            format!("Legal case {id} decided"),
            format!(
                "The court decided for dynasty {}.",
                if plaintiff_wins {
                    plaintiff_id
                } else {
                    defendant_id
                }
            ),
        );
    }
}

fn update_external_route_risk(state: &mut AppState) {
    for route in state.external_routes.values_mut() {
        let random_pressure = u16::try_from(state.rng.range_u32(500)).unwrap_or(0);
        if state.rng.is_chance_success(route.risk_basis_points / 12) {
            route.disruption_basis_points = route
                .disruption_basis_points
                .saturating_add(random_pressure)
                .min(9_500);
        } else {
            route.disruption_basis_points = route.disruption_basis_points.saturating_sub(150);
        }
    }
}

fn detect_and_advance_crises(registry: &Registry, state: &mut AppState) {
    let day = state.clock.day();
    let mut resolved = Vec::new();
    for crisis in state.crises.values_mut() {
        if !crisis.status.is_active() {
            continue;
        }
        crisis.severity_basis_points = crisis.severity_basis_points.saturating_sub(120);
        crisis.status = CrisisStatus::from_severity(crisis.severity_basis_points);
        if crisis.status == CrisisStatus::Resolved {
            resolved.push((crisis.id, crisis.kind));
        }
    }
    for (crisis_id, kind) in resolved {
        push_outbox(
            state,
            OutboxKind::Crisis,
            format!("Crisis {crisis_id} resolved"),
            format!("The {kind:?} crisis has subsided below an active threat level."),
        );
    }
    let has_grain_crisis = state
        .crises
        .values()
        .any(|crisis| crisis.kind == CrisisKind::GrainShortage && crisis.status.is_active());
    if !has_grain_crisis {
        let bread_stock_low = registry
            .get_good_id("bread")
            .and_then(|id| state.market.get_quote(id))
            .is_some_and(|quote| quote.stock() < Quantity::from_units(100));
        let average_satisfaction = if state.households.records().is_empty() {
            10_000
        } else {
            let total: u64 = state
                .households
                .iter()
                .map(|household| u64::from(household.food_satisfaction_basis_points()))
                .sum();
            u16::try_from(total / state.households.records().len() as u64).unwrap_or(10_000)
        };
        if bread_stock_low && average_satisfaction < 4_000 {
            insert_crisis(
                state,
                CrisisKind::GrainShortage,
                None,
                4_500,
                "Bread inventories and household food satisfaction fell below safe levels.",
            );
        }
    }
    let defaulted_loans = state
        .loans
        .values()
        .filter(|loan| loan.status == LoanStatus::Defaulted)
        .count();
    let has_panic = state
        .crises
        .values()
        .any(|crisis| crisis.kind == CrisisKind::BankingPanic && crisis.status.is_active());
    if defaulted_loans >= 2 && !has_panic {
        insert_crisis(
            state,
            CrisisKind::BankingPanic,
            None,
            3_800,
            "Multiple defaults damaged confidence in city credit.",
        );
    }
    detect_trade_disruption(state);
    if day > 0
        && day % 720 == 0
        && !has_active_crisis(state, CrisisKind::NobleDemand)
        && state.rng.is_chance_success(2_500)
    {
        let district_id = state.districts.keys().copied().next();
        insert_crisis(
            state,
            CrisisKind::NobleDemand,
            district_id,
            3_000,
            "The regional prince demanded an extraordinary payment from Rivergate.",
        );
    }
    detect_periodic_crises(state, day);
}

fn has_active_crisis(state: &AppState, kind: CrisisKind) -> bool {
    state
        .crises
        .values()
        .any(|crisis| crisis.kind == kind && crisis.status.is_active())
}

fn detect_periodic_crises(state: &mut AppState, day: i64) {
    if day <= 0 || day % 180 != 0 {
        return;
    }
    detect_urban_fire(state);
    detect_epidemic(state);
    detect_guild_revolt(state);
}

fn detect_urban_fire(state: &mut AppState) {
    if has_active_crisis(state, CrisisKind::UrbanFire) {
        return;
    }
    let Some((district_id, safety)) = state
        .districts
        .iter()
        .min_by_key(|(_, district)| district.safety_basis_points)
        .map(|(id, district)| (*id, district.safety_basis_points))
    else {
        return;
    };
    let fire_code = active_law_value(state, LawKind::FireCode)
        .unwrap_or(0)
        .clamp(0, 10_000);
    let chance = urban_fire_probability_basis_points(safety, fire_code);
    if state.rng.is_chance_success(chance) {
        insert_crisis(
            state,
            CrisisKind::UrbanFire,
            Some(district_id),
            urban_fire_severity_basis_points(safety, fire_code),
            "Unsafe buildings and weak fire prevention allowed an urban fire to spread.",
        );
    }
}

fn urban_fire_probability_basis_points(safety: u16, fire_code: i64) -> u16 {
    let deficiency = 10_000_u16.saturating_sub(safety);
    let chance = i64::from(deficiency)
        .saturating_div(4)
        .saturating_add(500)
        .saturating_sub(fire_code / 5)
        .clamp(0, 10_000);
    u16::try_from(chance).unwrap_or(0)
}

fn urban_fire_severity_basis_points(safety: u16, fire_code: i64) -> u16 {
    let deficiency = 10_000_u16.saturating_sub(safety);
    let severity = 4_000_i64
        .saturating_add(i64::from(deficiency) / 5)
        .saturating_sub(fire_code / 4)
        .clamp(1_000, 9_000);
    u16::try_from(severity).unwrap_or(9_000)
}

fn detect_epidemic(state: &mut AppState) {
    if has_active_crisis(state, CrisisKind::Epidemic) {
        return;
    }
    let Some((district_id, sanitation)) = state
        .districts
        .iter()
        .min_by_key(|(_, district)| district.sanitation_basis_points)
        .map(|(id, district)| (*id, district.sanitation_basis_points))
    else {
        return;
    };
    let deficiency = 10_000_u16.saturating_sub(sanitation);
    let chance = deficiency.saturating_div(4).saturating_add(250).min(10_000);
    if state.rng.is_chance_success(chance) {
        insert_crisis(
            state,
            CrisisKind::Epidemic,
            Some(district_id),
            3_000_u16.saturating_add(deficiency / 5).min(9_000),
            "Poor sanitation allowed an epidemic to take hold.",
        );
    }
}

fn detect_trade_disruption(state: &mut AppState) {
    if has_active_crisis(state, CrisisKind::TradeDisruption) {
        return;
    }
    let disruption = state
        .external_routes
        .values()
        .map(|route| route.disruption_basis_points)
        .max()
        .unwrap_or(0);
    if disruption >= 7_000 {
        insert_crisis(
            state,
            CrisisKind::TradeDisruption,
            None,
            disruption,
            "External trade routes became too disrupted to sustain normal commerce.",
        );
    }
}

fn detect_guild_revolt(state: &mut AppState) {
    if has_active_crisis(state, CrisisKind::GuildRevolt) {
        return;
    }
    let disputed_district = state.employment.values().find_map(|agreement| {
        (agreement.status == EmploymentStatus::Disputed)
            .then(|| state.businesses.get(agreement.business_id))
            .flatten()
            .map(crate::core::Business::district_id)
    });
    let disputed_count = state
        .employment
        .values()
        .filter(|agreement| agreement.status == EmploymentStatus::Disputed)
        .count();
    let restriction = active_law_value(state, LawKind::GuildEntryRestriction)
        .unwrap_or(0)
        .clamp(0, 10_000);
    let chance = guild_revolt_probability_basis_points(disputed_count, restriction);
    if disputed_count >= 2 || (chance > 0 && state.rng.is_chance_success(chance)) {
        let district_id = disputed_district.or_else(|| {
            state
                .districts
                .iter()
                .max_by_key(|(_, district)| district.unrest_basis_points)
                .map(|(id, _)| *id)
        });
        insert_crisis(
            state,
            CrisisKind::GuildRevolt,
            district_id,
            2_500_u16
                .saturating_add(
                    u16::try_from(disputed_count)
                        .unwrap_or(u16::MAX)
                        .saturating_mul(500),
                )
                .min(9_000),
            "Labor disputes and restrictive guild rules triggered organized resistance.",
        );
    }
}

fn guild_revolt_probability_basis_points(disputed_count: usize, restriction: i64) -> u16 {
    if disputed_count == 0 && restriction <= 0 {
        return 0;
    }
    let chance = 400_i64
        .saturating_add(restriction.clamp(0, 10_000) / 5)
        .saturating_add(
            i64::try_from(disputed_count)
                .unwrap_or(i64::MAX)
                .saturating_mul(800),
        )
        .clamp(0, 10_000);
    u16::try_from(chance).unwrap_or(10_000)
}

fn insert_crisis(
    state: &mut AppState,
    kind: CrisisKind,
    district_id: Option<DistrictId>,
    severity_basis_points: u16,
    cause: &str,
) -> crate::ids::CrisisId {
    let id = state.next_ids.crisis();
    state.crises.insert(
        id,
        Crisis {
            id,
            kind,
            district_id,
            started_day: state.clock.day(),
            severity_basis_points,
            status: CrisisStatus::Emerging,
            cause: cause.to_owned(),
        },
    );
    push_outbox(
        state,
        OutboxKind::Crisis,
        format!("Crisis emerged: {kind:?}"),
        cause.to_owned(),
    );
    id
}

pub(crate) fn run_annual_strategic_systems(state: &mut AppState) {
    educate_family_members(state);
    form_dynastic_marriage(state);
    update_family_councils(state);
}

fn educate_family_members(state: &mut AppState) {
    for character in state.characters.iter_mut() {
        if character.status() != crate::core::CharacterStatus::Active {
            continue;
        }
        match character.role() {
            CharacterRole::Heir | CharacterRole::Clerk => {
                character.capabilities.administration = character
                    .capabilities
                    .administration
                    .saturating_add(2)
                    .min(100);
                character.capabilities.commerce =
                    character.capabilities.commerce.saturating_add(1).min(100);
            }
            CharacterRole::HeadOfHouse
            | CharacterRole::BusinessManager
            | CharacterRole::GuildRepresentative => {}
        }
    }
}

fn form_dynastic_marriage(state: &mut AppState) {
    if state.clock.day() % 1_800 != 0 {
        return;
    }
    let heirs: Vec<_> = state
        .dynasties
        .values()
        .filter_map(|dynasty| Some((dynasty.id(), dynasty.heir_id()?)))
        .filter(|(_, heir_id)| {
            state
                .characters
                .get(*heir_id)
                .is_some_and(|character| character.status() == crate::core::CharacterStatus::Active)
        })
        .collect();
    let is_married = |character_id| {
        state.family_links.values().any(|link| {
            link.active
                && link.kind == FamilyLinkKind::Marriage
                && (link.first_character_id == character_id
                    || link.second_character_id == character_id)
        })
    };
    let selected_pair = heirs.iter().enumerate().find_map(|(index, left)| {
        if is_married(left.1) {
            return None;
        }
        heirs
            .iter()
            .skip(index + 1)
            .find(|right| !is_married(right.1))
            .map(|right| (*left, *right))
    });
    let Some(((left_dynasty, left_heir), (right_dynasty, right_heir))) = selected_pair else {
        return;
    };
    let id = state.next_ids.family_link();
    state.family_links.insert(
        id,
        FamilyLink {
            id,
            first_character_id: left_heir,
            second_character_id: right_heir,
            kind: FamilyLinkKind::Marriage,
            active: true,
            property_claim_basis_points: 2_500,
        },
    );
    let pair = DynastyPair::new(left_dynasty, right_dynasty);
    if let Some(relationship) = state.relationships.get_mut(&pair) {
        relationship.trust_basis_points = relationship
            .trust_basis_points
            .saturating_add(1_000)
            .min(10_000);
        relationship.obligation = relationship.obligation.saturating_add(2);
        relationship
            .memories
            .push("A dynastic marriage joined the two houses.".to_owned());
    }
    push_outbox(
        state,
        OutboxKind::Family,
        "Dynastic marriage concluded".to_owned(),
        format!(
            "The heirs of dynasties {left_dynasty} and {right_dynasty} entered a marriage compact."
        ),
    );
}

fn update_family_councils(state: &mut AppState) {
    let loyalty_adjustments: Vec<_> = state
        .family_councils
        .values()
        .map(|council| {
            let mut total_loyalty = 0_u64;
            let mut active_members = 0_u64;
            for character_id in &council.members {
                let character = state
                    .characters
                    .get(*character_id)
                    .expect("family council member must exist");
                if character.status() == crate::core::CharacterStatus::Active {
                    total_loyalty = total_loyalty
                        .saturating_add(u64::from(character.runtime.loyalty_basis_points));
                    active_members = active_members.saturating_add(1);
                }
            }
            let average_loyalty = total_loyalty
                .checked_div(active_members)
                .and_then(|average| u16::try_from(average).ok())
                .unwrap_or(5_000);
            let adjustment = (i32::from(average_loyalty) - 5_000) / 50;
            (council.dynasty_id, adjustment)
        })
        .collect();

    for (dynasty_id, loyalty_adjustment) in loyalty_adjustments {
        let council = state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("family council must exist");
        let members = u16::try_from(council.members.len()).unwrap_or(u16::MAX);
        let branch_pressure = i32::from(members.saturating_sub(2).saturating_mul(80));
        let governance_adjustment = match council.governance {
            HouseGovernance::HeadCommand => -200,
            HouseGovernance::Primogeniture => 50,
            HouseGovernance::FamilyPartnership => 250,
            HouseGovernance::BranchFederation => 120,
            HouseGovernance::ElectedHead => -50,
        };
        council.unity_basis_points = i32::from(council.unity_basis_points)
            .saturating_sub(branch_pressure)
            .saturating_add(50)
            .saturating_add(loyalty_adjustment)
            .saturating_add(governance_adjustment)
            .clamp(0, 10_000)
            .try_into()
            .expect("clamped family unity must fit u16");
        if council.unity_basis_points < 3_000
            && council.governance == HouseGovernance::Primogeniture
        {
            council.governance = HouseGovernance::FamilyPartnership;
            council.charter_version = council.charter_version.saturating_add(1);
        }
    }
}

pub(crate) fn push_outbox(state: &mut AppState, kind: OutboxKind, subject: String, body: String) {
    let id = state.next_ids.outbox();
    state.outbox.push(OutboxMessage {
        id,
        day: state.clock.day(),
        kind,
        subject,
        body,
        acknowledged: false,
    });
}

#[cfg(test)]
#[path = "strategic_tests.rs"]
mod tests;
