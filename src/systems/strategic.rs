//! Strategic initialization, periodic systems, and validated cross-record operations.

use super::SimulationError;
use super::transactions::{
    debit_market_clearing_account, next_business_finance_version, next_family_charter_version,
};
use crate::core::{
    AiObjective, AppState, AuditKind, AuditRecord, BusinessStatus, CharacterRole, CharacterStatus,
    ChronicleEntry, ChronicleKind, CivicDebtStatus, ContractStatus, Crisis, CrisisKind,
    CrisisStatus, DistrictRuntime, DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw,
    ExternalRoute, FamilyCouncilState, FamilyLink, FamilyLinkKind, HouseGovernance,
    InformationConfidence, InformationReport, InformationTarget, InstitutionRuntime, LawKind,
    LegalCase, LegalCaseKind, LegalCaseStatus, Loan, LoanStatus, ObjectiveKind, ObjectiveStatus,
    OfficePower, OutboxKind, OutboxMessage, Property, PropertyKind, PublicWork, PublicWorkKind,
    PublicWorkStatus, RelationshipState, SupplyContract,
};
use crate::ids::{
    BusinessId, CharacterId, CivicDebtId, DistrictId, DynastyId, EmploymentId, GoodId, HouseholdId,
    InstitutionId, PropertyId,
};
use crate::money::{Money, Quantity, checked_cost_for, cost_for, rounded_cost_copper_wide};
use crate::registry::{InstitutionKind, Registry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub(crate) const OFFICE_ADMINISTRATIVE_LOAD_PER_POWER: u16 = 10;
pub(crate) const OFFICE_DUTY_COST_PER_POWER: Money = Money::from_copper(100);
const OFFICE_DUTY_FAILURE_NOTIFICATION_INTERVAL_DAYS: i64 = 90;
const OFFICE_DUTY_FORFEITURE_WINDOW_DAYS: i64 = 90;
const OFFICE_DUTY_REELECTION_BAN_DAYS: i64 = 180;
const OFFICE_DUTY_FORFEITURE_THRESHOLD: usize = 3;
pub(crate) const DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS: i64 = 180;
pub(crate) const PROPERTY_LIQUIDATION_BASIS_POINTS: i64 = 5_000;
const PROPERTY_AUCTION_DISTRESS_TREASURY_LIMIT: Money = Money::from_copper(2_000);
const UNADDRESSED_CRISIS_MONTHLY_ESCALATION_BASIS_POINTS: u16 = 240;
const ADDRESSED_CRISIS_MONTHLY_RECOVERY_BASIS_POINTS: u16 = 360;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StrategicError {
    #[error(
        "state scenario {state_scenario:?} does not match registry scenario {registry_scenario:?}"
    )]
    RegistryMismatch {
        state_scenario: String,
        registry_scenario: String,
    },
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
    #[error(
        "loan {loan_id} already represents unsettled credit from dynasty {lender_dynasty_id} to dynasty {borrower_dynasty_id}"
    )]
    ExistingUnsettledLoan {
        lender_dynasty_id: DynastyId,
        borrower_dynasty_id: DynastyId,
        loan_id: crate::ids::LoanId,
    },
    #[error("defaulted loan {loan_id} cannot be restructured before day {available_day}")]
    DefaultedLoanRestructuringCooldown {
        loan_id: crate::ids::LoanId,
        available_day: i64,
    },
    #[error(
        "loan {loan_id} cannot add {incoming}; current balance {current} would exceed the supported money range"
    )]
    LoanBalanceOverflow {
        loan_id: crate::ids::LoanId,
        current: Money,
        incoming: Money,
    },
    #[error("amount must be positive")]
    NonPositiveAmount,
    #[error("quantity must be positive")]
    NonPositiveQuantity,
    #[error("contract duration must contain at least one week")]
    EmptyContractDuration,
    #[error(
        "contract payment for quantity {quantity} at unit price {unit_price} exceeds the supported money range"
    )]
    ContractPaymentOverflow {
        quantity: Quantity,
        unit_price: Money,
    },
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
    #[error(
        "dynasty {dynasty_id} cannot receive {incoming}; current treasury {current} would exceed the supported money range"
    )]
    DynastyTreasuryOverflow {
        dynasty_id: DynastyId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "business {business_id} cannot receive {incoming}; current cash {current} would exceed the supported money range"
    )]
    BusinessCashOverflow {
        business_id: BusinessId,
        current: Money,
        incoming: Money,
    },
    #[error("business {business_id} finance version is exhausted")]
    BusinessFinanceVersionExhausted { business_id: BusinessId },
    #[error(
        "business acquisition cost overflows the supported money range: price {purchase_price}, recapitalization {recapitalization}"
    )]
    AcquisitionCostOverflow {
        purchase_price: Money,
        recapitalization: Money,
    },
    #[error(
        "business {business_id} valuation exceeds the supported money range after applying the acquisition discount"
    )]
    BusinessValuationOverflow { business_id: BusinessId },
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
    #[error("property {property_id} is not owned by dynasty {seller_dynasty_id}")]
    PropertyNotOwnedBySeller {
        property_id: PropertyId,
        seller_dynasty_id: DynastyId,
    },
    #[error("property buyer and seller must differ")]
    SamePropertyParty,
    #[error("the civic treasury is not available for a property auction guarantee")]
    MissingCivicTreasury,
    #[error(
        "property auction has only {buyer_available} private and {civic_available} civic liquidity, requires {required}"
    )]
    InsufficientPropertyAuctionLiquidity {
        buyer_available: Money,
        civic_available: Money,
        required: Money,
    },
    #[error("property collateral references missing loan {loan_id}")]
    MissingCollateralLoan { loan_id: crate::ids::LoanId },
    #[error(
        "property {property_id} lien loan {loan_id} belongs to borrower {borrower_dynasty_id}, not seller {seller_dynasty_id}"
    )]
    PropertyLienBorrowerMismatch {
        property_id: PropertyId,
        loan_id: crate::ids::LoanId,
        borrower_dynasty_id: DynastyId,
        seller_dynasty_id: DynastyId,
    },
    #[error(
        "property {property_id} sale price {price} cannot settle lien loan {loan_id} balance {balance}"
    )]
    PropertySaleCannotSettleLien {
        property_id: PropertyId,
        loan_id: crate::ids::LoanId,
        price: Money,
        balance: Money,
    },
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

fn ensure_registry_matches(registry: &Registry, state: &AppState) -> Result<(), StrategicError> {
    if state.scenario_key() != registry.scenario().key() {
        return Err(StrategicError::RegistryMismatch {
            state_scenario: state.scenario_key().to_owned(),
            registry_scenario: registry.scenario().key().to_owned(),
        });
    }
    Ok(())
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
pub struct PropertyLiquidationQuote {
    pub price: Money,
    pub buyer_contribution: Money,
    pub civic_guarantee: Money,
    pub lien_payoff: Money,
    pub seller_proceeds: Money,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PropertyLienSettlement {
    loan_id: crate::ids::LoanId,
    lender_dynasty_id: DynastyId,
    payoff: Money,
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

const AI_OBJECTIVE_REVIEW_DAYS: i64 = 720;
pub(crate) const STANDARD_CONTRACT_BATCHES_PER_WEEK: i64 = 2;

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
struct ContractPartySettlementState {
    owner_id: DynastyId,
    can_perform: bool,
    can_receive: bool,
}

#[derive(Clone, Copy, Debug)]
struct ContractSettlementState {
    buyer: ContractPartySettlementState,
    seller: ContractPartySettlementState,
}

impl ContractSettlementState {
    const fn is_fulfilled(self) -> bool {
        self.buyer.can_perform
            && self.buyer.can_receive
            && self.seller.can_perform
            && self.seller.can_receive
    }

    const fn buyer_is_at_fault(self) -> bool {
        !self.buyer.can_perform || !self.buyer.can_receive
    }

    const fn seller_is_at_fault(self) -> bool {
        !self.seller.can_perform || !self.seller.can_receive
    }

    const fn has_attributable_nonperformance(self) -> bool {
        self.buyer_is_at_fault() || self.seller_is_at_fault()
    }

    const fn breaching_dynasty_id(self) -> Option<DynastyId> {
        match (self.buyer_is_at_fault(), self.seller_is_at_fault()) {
            (true, false) => Some(self.buyer.owner_id),
            (false, true) => Some(self.seller.owner_id),
            (true, true) | (false, false) => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DueLoan {
    id: crate::ids::LoanId,
    lender_id: DynastyId,
    borrower_id: DynastyId,
    weekly_payment: Money,
    balance: Money,
    interest_basis_points: u16,
    collateral_property_id: Option<PropertyId>,
}

#[derive(Clone, Copy, Debug)]
struct DueCivicDebt {
    id: CivicDebtId,
    creditor_dynasty_id: DynastyId,
    sponsor_dynasty_id: Option<DynastyId>,
    weekly_payment: Money,
    balance: Money,
    interest_basis_points: u16,
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
    let buyer_owner_id = state
        .businesses
        .get(buyer_business_id)
        .expect("validated contract buyer must exist")
        .owner_dynasty_id();
    let seller_owner_id = state
        .businesses
        .get(seller_business_id)
        .expect("validated contract seller must exist")
        .owner_dynasty_id();
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
            fulfilled_deliveries_by_dynasty: BTreeMap::default(),
            missed_deliveries: 0,
            breaching_dynasty_id: None,
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
    adjust_dynasty_relationship(
        state,
        buyer_owner_id,
        seller_owner_id,
        RelationshipDelta::new(40, 20, 0, -10, 1),
    );
    remember_dynasty_interaction(
        state,
        buyer_owner_id,
        seller_owner_id,
        &format!("Supply contract {id} was signed."),
    );
    record_counterparty_information(
        state,
        buyer_owner_id,
        seller_owner_id,
        "Contract negotiation and delivery records",
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
        let defaulted_loan_id = validate_loan_terms(state, &self.terms)?;
        Ok(commit_loan(state, &self.terms, defaulted_loan_id))
    }
}

fn commit_loan(
    state: &mut AppState,
    terms: &LoanTerms,
    defaulted_loan_id: Option<crate::ids::LoanId>,
) -> crate::ids::LoanId {
    let &LoanTerms {
        lender_dynasty_id,
        borrower_dynasty_id,
        principal,
        collateral_property_id,
        ..
    } = terms;
    let id = defaulted_loan_id.unwrap_or_else(|| state.next_ids.loan());
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
    commit_loan_record(state, terms, id, defaulted_loan_id);
    let restructured = defaulted_loan_id.is_some();
    push_outbox(
        state,
        OutboxKind::Finance,
        if restructured {
            format!("Loan {id} restructured")
        } else {
            format!("Loan {id} issued")
        },
        if restructured {
            format!(
                "Dynasty {lender_dynasty_id} restructured loan {id} and advanced {principal} to dynasty {borrower_dynasty_id}."
            )
        } else {
            format!(
                "Dynasty {lender_dynasty_id} lent {principal} to dynasty {borrower_dynasty_id}."
            )
        },
    );
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
            format!("Loan {id} was restructured with a {principal} advance.")
        } else {
            format!("Loan {id} was issued for {principal}.")
        },
    );
    record_counterparty_information(
        state,
        lender_dynasty_id,
        borrower_dynasty_id,
        "Credit underwriting and repayment records",
    );
    id
}

fn commit_loan_record(
    state: &mut AppState,
    terms: &LoanTerms,
    id: crate::ids::LoanId,
    defaulted_loan_id: Option<crate::ids::LoanId>,
) {
    if let Some(defaulted_loan_id) = defaulted_loan_id {
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
        loan.next_due_day = state.clock.day().saturating_add(7);
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
                next_due_day: state.clock.day().saturating_add(7),
                missed_payments: 0,
                collateral_property_id: terms.collateral_property_id,
                status: LoanStatus::Current,
            },
        );
    }
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
    ensure_registry_matches(registry, state)?;
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
    checked_cost_for(terms.quantity_per_week, terms.unit_price).ok_or(
        StrategicError::ContractPaymentOverflow {
            quantity: terms.quantity_per_week,
            unit_price: terms.unit_price,
        },
    )?;
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

fn validate_loan_terms(
    state: &AppState,
    terms: &LoanTerms,
) -> Result<Option<crate::ids::LoanId>, StrategicError> {
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
            && matches!(
                loan.status,
                LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
            )
    }) {
        return Err(StrategicError::ExistingUnsettledLoan {
            lender_dynasty_id: terms.lender_dynasty_id,
            borrower_dynasty_id: terms.borrower_dynasty_id,
            loan_id: existing.id,
        });
    }
    let defaulted_loan_id = validate_defaulted_loan_restructuring(state, terms)?;
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
    Ok(defaulted_loan_id)
}

fn validate_defaulted_loan_restructuring(
    state: &AppState,
    terms: &LoanTerms,
) -> Result<Option<crate::ids::LoanId>, StrategicError> {
    let defaulted_loan = state
        .loans
        .values()
        .filter(|loan| {
            loan.lender_dynasty_id == terms.lender_dynasty_id
                && loan.borrower_dynasty_id == terms.borrower_dynasty_id
                && loan.status == LoanStatus::Defaulted
        })
        .max_by_key(|loan| (loan.next_due_day, loan.id));
    let Some(defaulted_loan) = defaulted_loan else {
        return Ok(None);
    };
    let available_day = defaulted_loan
        .next_due_day
        .saturating_add(DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS);
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
        .treasury = buyer
        .treasury()
        .checked_sub(price)
        .expect("validated property buyer must cover the purchase price");
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

fn property_liquidation_lien(
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

fn property_auction_funding(
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

fn record_completed_loan_repayment(
    state: &mut AppState,
    lender_dynasty_id: DynastyId,
    borrower_dynasty_id: DynastyId,
    loan_id: crate::ids::LoanId,
) {
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
    record_counterparty_information(
        state,
        lender_dynasty_id,
        borrower_dynasty_id,
        "Completed loan repayment records",
    );
}

/// Sells an owned property to another dynasty at the canonical liquidation price.
///
/// Occupied premises remain occupied and become a tenancy when the buyer differs from the business
/// owner.
///
/// # Errors
///
/// Returns the same errors as [`quote_property_liquidation`].
///
/// # Panics
///
/// Panics only if validated dynasty or property records disappear during the synchronous commit.
pub fn sell_owned_property(
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
        );
    }
    let property = state
        .properties
        .get_mut(&property_id)
        .expect("validated property must exist");
    property.collateral_loan_id = None;
    property.owner_dynasty_id = Some(buyer_dynasty_id);
    property.tenant_dynasty_id = occupant_owner_id.filter(|owner_id| *owner_id != buyer_dynasty_id);
    push_outbox(
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
    );
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

fn resolve_business_purchase_price(
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
struct ValidatedBusinessAcquisition {
    quote: BusinessAcquisitionQuote,
    buyer_treasury: Money,
    total_required: Money,
    seller_treasury_after: Money,
    business_cash_after: Money,
    business_finance_version_after: u64,
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
    let business_finance_version_after = business
        .finance
        .version
        .checked_add(1)
        .ok_or(StrategicError::BusinessFinanceVersionExhausted { business_id })?;
    let recipe_id = business.recipe_id();
    let administrative_load = registry
        .get_recipe(recipe_id)
        .expect("business recipe references must be validated")
        .administrative_load();
    Ok(ValidatedBusinessAcquisition {
        quote,
        buyer_treasury,
        total_required,
        seller_treasury_after,
        business_cash_after,
        business_finance_version_after,
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
    super::synchronize_employment_for_business_status(state, business_id, BusinessStatus::Active);

    record_business_acquisition(state, buyer_dynasty_id, manager_id, recapitalization, quote);
}

fn synchronize_business_property_tenancy(
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

fn initialize_institutions(registry: &Registry, state: &mut AppState) {
    for definition in registry.institutions() {
        let mut members = BTreeSet::new();
        for dynasty in state.dynasties.values() {
            if dynasty.id() != state.player_dynasty_id {
                members.insert(dynasty.head_id());
            }
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
                powers: super::institution_powers_for(definition.kind()),
                budget: Money::from_copper(120_000),
                legitimacy_basis_points: 7_000,
                term_started_day: 0,
                next_selection_day: super::OFFICE_TERM_DAYS,
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
            let Some((seller_id, seller_owner, _)) = seller else {
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
                quantity_per_week: input
                    .quantity()
                    .saturating_mul_ratio(STANDARD_CONTRACT_BATCHES_PER_WEEK, 1),
                unit_price: price,
                penalty: cost_for(input.quantity(), price).saturating_mul(2),
                duration_weeks: if *buyer_owner == state.player_dynasty_id
                    || *seller_owner == state.player_dynasty_id
                {
                    26
                } else {
                    52
                },
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
    const INITIAL_OBJECTIVE_ROTATION: [ObjectiveKind; 5] = [
        ObjectiveKind::AcquireProperty,
        ObjectiveKind::WinOffice,
        ObjectiveKind::SecureSupply,
        ObjectiveKind::ImproveLegitimacy,
        ObjectiveKind::AccumulateCash,
    ];
    let dynasty_ids: Vec<_> = state
        .dynasties
        .keys()
        .copied()
        .filter(|id| *id != state.player_dynasty_id)
        .collect();
    for (index, dynasty_id) in dynasty_ids.into_iter().enumerate() {
        let kind = INITIAL_OBJECTIVE_ROTATION[index % INITIAL_OBJECTIVE_ROTATION.len()];
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
            target: None,
            subject: "Rivergate opening conditions".to_owned(),
            confidence: InformationConfidence::Confirmed,
            created_day: 0,
            expires_day: 90,
            source: "Household account books and market inspection".to_owned(),
            summary: "Food prices are politically sensitive, the southern district lacks sanitation, and the treasury remains strained after wall repairs.".to_owned(),
        },
    );
    push_outbox(
        state,
        OutboxKind::Information,
        "Rivergate briefing available".to_owned(),
        "The dynasty ledger now includes contracts, property, credit, institutional power, district conditions, and strategic reports.".to_owned(),
    );
}

pub(crate) fn run_daily_strategic_systems(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    apply_route_laws(state);
    apply_crisis_daily_effects(registry, state)?;
    apply_external_route_supply(state);
    Ok(())
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

fn apply_crisis_daily_effects(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
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
                apply_banking_panic_losses(state, severity)?;
            }
            CrisisKind::NobleDemand => {
                if let Some(treasury_id) = registry.get_institution_id("treasury")
                    && let Some(treasury) = state.institutions.get_mut(&treasury_id)
                {
                    let levy = Money::from_copper(i64::from(severity) / 20).min(treasury.budget);
                    treasury.budget = treasury
                        .budget
                        .checked_sub(levy)
                        .expect("bounded noble levy must not exceed civic treasury");
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
    Ok(())
}

fn apply_banking_panic_losses(state: &mut AppState, severity: u16) -> Result<(), SimulationError> {
    for business in state.businesses.iter_mut() {
        let loss = business
            .finance
            .cash
            .saturating_mul_ratio(i64::from(severity), 1_000_000);
        if loss > Money::ZERO {
            let resulting_cash = business
                .finance
                .cash
                .checked_sub(loss)
                .expect("banking-panic loss must not exceed business cash");
            let resulting_lifetime_costs =
                business.finance.lifetime_costs.checked_add(loss).ok_or(
                    SimulationError::BusinessLifetimeCostsOverflow {
                        business_id: business.id(),
                        current: business.finance.lifetime_costs,
                        incoming: loss,
                    },
                )?;
            let next_finance_version = next_business_finance_version(business)?;
            business.finance.cash = resulting_cash;
            business.finance.lifetime_costs = resulting_lifetime_costs;
            business.finance.version = next_finance_version;
        }
    }
    Ok(())
}

pub(crate) fn run_weekly_strategic_systems(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    settle_contracts(state)?;
    settle_loans(state)?;
    settle_civic_debts(registry, state)?;
    settle_property_rents(state)?;
    settle_employment(registry, state)?;
    distribute_business_dividends(registry, state)?;
    progress_public_works(registry, state);
    update_relationships_from_obligations(state);
    update_quality_reputations(state);
    apply_law_economic_effects(registry, state);
    Ok(())
}

fn settle_contracts(state: &mut AppState) -> Result<(), SimulationError> {
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
        settle_due_contract(state, due_contract)?;
    }
    Ok(())
}

fn settle_due_contract(state: &mut AppState, due: DueContract) -> Result<(), SimulationError> {
    let payment = cost_for(due.quantity, due.unit_price);
    let (seller_active, seller_owner_id, seller_can_deliver, seller_can_receive_payment) = {
        let seller = state
            .businesses
            .get(due.seller_id)
            .expect("contract seller must exist");
        (
            !matches!(
                seller.status(),
                BusinessStatus::Insolvent | BusinessStatus::Closed
            ),
            seller.owner_dynasty_id(),
            seller.inventory_quantity(due.good_id) >= due.quantity,
            seller.cash().max_nonnegative_addend() >= payment,
        )
    };
    let (buyer_active, buyer_owner_id, buyer_can_pay, buyer_can_receive_delivery) = {
        let buyer = state
            .businesses
            .get(due.buyer_id)
            .expect("contract buyer must exist");
        (
            !matches!(
                buyer.status(),
                BusinessStatus::Insolvent | BusinessStatus::Closed
            ),
            buyer.owner_dynasty_id(),
            buyer.cash() >= payment,
            buyer
                .inventory_quantity(due.good_id)
                .max_nonnegative_addend()
                >= due.quantity,
        )
    };
    if !seller_active || !buyer_active {
        terminate_inactive_contract(
            state,
            due.id,
            buyer_owner_id,
            seller_owner_id,
            buyer_active,
            seller_active,
        );
        return Ok(());
    }
    let settlement = ContractSettlementState {
        buyer: ContractPartySettlementState {
            owner_id: buyer_owner_id,
            can_perform: buyer_can_pay,
            can_receive: buyer_can_receive_delivery,
        },
        seller: ContractPartySettlementState {
            owner_id: seller_owner_id,
            can_perform: seller_can_deliver,
            can_receive: seller_can_receive_payment,
        },
    };
    let fulfilled = settlement.is_fulfilled();
    if fulfilled {
        settle_fulfilled_contract(state, due, payment, settlement)?;
    } else {
        settle_failed_contract(state, due, settlement)?;
    }
    finalize_expired_contract(state, due, settlement, fulfilled);
    Ok(())
}

fn terminate_inactive_contract(
    state: &mut AppState,
    contract_id: crate::ids::ContractId,
    buyer_owner_id: DynastyId,
    seller_owner_id: DynastyId,
    buyer_active: bool,
    seller_active: bool,
) {
    let contract = state
        .contracts
        .get_mut(&contract_id)
        .expect("contract must exist");
    contract.missed_deliveries = contract.missed_deliveries.saturating_add(1);
    contract.breaching_dynasty_id = match (buyer_active, seller_active) {
        (false, true) => Some(buyer_owner_id),
        (true, false) => Some(seller_owner_id),
        (false, false) | (true, true) => None,
    };
    contract.status = ContractStatus::Breached;
    if buyer_owner_id != seller_owner_id {
        if !seller_active {
            adjust_reliability_reputation(state, seller_owner_id, -120);
        }
        if !buyer_active {
            adjust_reliability_reputation(state, buyer_owner_id, -120);
        }
        adjust_dynasty_relationship(
            state,
            buyer_owner_id,
            seller_owner_id,
            RelationshipDelta::new(-100, -40, 0, 120, 0),
        );
        remember_dynasty_interaction(
            state,
            buyer_owner_id,
            seller_owner_id,
            &format!("Supply contract {contract_id} ended because a party became inactive."),
        );
        record_counterparty_information(
            state,
            buyer_owner_id,
            seller_owner_id,
            "Contract termination and business-status records",
        );
    }
    push_outbox(
        state,
        OutboxKind::Contract,
        format!("Contract {contract_id} terminated"),
        "An inactive contract party could no longer perform the scheduled obligation.".to_owned(),
    );
}

fn finalize_expired_contract(
    state: &mut AppState,
    due: DueContract,
    settlement: ContractSettlementState,
    fulfilled: bool,
) {
    let expired_active = state.contracts.get(&due.id).is_some_and(|contract| {
        contract.status == ContractStatus::Active && contract.next_due_day > due.end_day
    });
    if !expired_active {
        return;
    }
    let contract = state
        .contracts
        .get_mut(&due.id)
        .expect("contract must exist");
    contract.status = if fulfilled {
        ContractStatus::Fulfilled
    } else {
        ContractStatus::Breached
    };
    contract.breaching_dynasty_id = if fulfilled {
        None
    } else {
        settlement.breaching_dynasty_id()
    };
    if settlement.buyer.owner_id != settlement.seller.owner_id {
        let memory = if fulfilled {
            format!("Supply contract {} completed successfully.", due.id)
        } else {
            format!("Supply contract {} expired in breach.", due.id)
        };
        remember_dynasty_interaction(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            &memory,
        );
        record_counterparty_information(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            "Completed contract performance records",
        );
    }
    if !fulfilled {
        push_outbox(
            state,
            OutboxKind::Contract,
            format!("Contract {} expired in breach", due.id),
            "The final scheduled delivery was not completed before the contract ended.".to_owned(),
        );
    }
}

fn settle_fulfilled_contract(
    state: &mut AppState,
    due: DueContract,
    payment: Money,
    settlement: ContractSettlementState,
) -> Result<(), SimulationError> {
    let transferred = transfer_contract_money(state, due.buyer_id, due.seller_id, payment)?;
    debug_assert_eq!(
        transferred, payment,
        "prevalidated contract payment must transfer in full"
    );
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
    let contract = state
        .contracts
        .get_mut(&due.id)
        .expect("contract must exist");
    contract.fulfilled_deliveries = contract.fulfilled_deliveries.saturating_add(1);
    let buyer_deliveries = contract
        .fulfilled_deliveries_by_dynasty
        .entry(settlement.buyer.owner_id)
        .or_default();
    *buyer_deliveries = buyer_deliveries.saturating_add(1);
    if settlement.seller.owner_id != settlement.buyer.owner_id {
        let seller_deliveries = contract
            .fulfilled_deliveries_by_dynasty
            .entry(settlement.seller.owner_id)
            .or_default();
        *seller_deliveries = seller_deliveries.saturating_add(1);
    }
    contract.next_due_day = contract.next_due_day.saturating_add(7);
    if settlement.buyer.owner_id != settlement.seller.owner_id {
        adjust_reliability_reputation(state, settlement.buyer.owner_id, 20);
        adjust_reliability_reputation(state, settlement.seller.owner_id, 20);
        adjust_dynasty_relationship(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            RelationshipDelta::new(5, 3, 0, -2, 0),
        );
    }
    Ok(())
}

fn settle_failed_contract(
    state: &mut AppState,
    due: DueContract,
    settlement: ContractSettlementState,
) -> Result<(), SimulationError> {
    let penalty_parties = match (
        settlement.seller_is_at_fault(),
        settlement.buyer_is_at_fault(),
    ) {
        (false, true) => Some((due.buyer_id, due.seller_id)),
        (true, false) => Some((due.seller_id, due.buyer_id)),
        (false, false) | (true, true) => None,
    };
    if let Some((payer_id, recipient_id)) = penalty_parties {
        let available = state
            .businesses
            .get(payer_id)
            .expect("contract penalty payer must exist")
            .cash();
        transfer_contract_money(state, payer_id, recipient_id, due.penalty.min(available))?;
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
            contract.breaching_dynasty_id = settlement.breaching_dynasty_id();
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
    if settlement.has_attributable_nonperformance()
        && settlement.buyer.owner_id != settlement.seller.owner_id
    {
        if settlement.seller_is_at_fault() {
            adjust_reliability_reputation(state, settlement.seller.owner_id, -120);
        }
        if settlement.buyer_is_at_fault() {
            adjust_reliability_reputation(state, settlement.buyer.owner_id, -120);
        }
        adjust_dynasty_relationship(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            RelationshipDelta::new(-30, -10, 0, 40, 0),
        );
        if breached {
            remember_dynasty_interaction(
                state,
                settlement.buyer.owner_id,
                settlement.seller.owner_id,
                &format!(
                    "Supply contract {} was terminated for repeated nonperformance.",
                    due.id
                ),
            );
            record_counterparty_information(
                state,
                settlement.buyer.owner_id,
                settlement.seller.owner_id,
                "Contract breach and penalty records",
            );
        }
    }
    Ok(())
}

fn transfer_contract_money(
    state: &mut AppState,
    payer_id: BusinessId,
    recipient_id: BusinessId,
    amount: Money,
) -> Result<Money, SimulationError> {
    if amount <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let payer_cash = state
        .businesses
        .get(payer_id)
        .expect("contract payer must exist")
        .cash();
    let recipient_headroom = state
        .businesses
        .get(recipient_id)
        .expect("contract recipient must exist")
        .cash()
        .max_nonnegative_addend();
    let transferred = amount.min(payer_cash).min(recipient_headroom);
    if transferred <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let (payer_lifetime_costs, payer_finance_version) = {
        let payer = state
            .businesses
            .get(payer_id)
            .expect("contract payer must exist");
        (
            payer
                .finance
                .lifetime_costs
                .checked_add(transferred)
                .ok_or(SimulationError::BusinessLifetimeCostsOverflow {
                    business_id: payer_id,
                    current: payer.finance.lifetime_costs,
                    incoming: transferred,
                })?,
            next_business_finance_version(payer)?,
        )
    };
    let (recipient_lifetime_revenue, recipient_finance_version) = {
        let recipient = state
            .businesses
            .get(recipient_id)
            .expect("contract recipient must exist");
        (
            recipient
                .finance
                .lifetime_revenue
                .checked_add(transferred)
                .ok_or(SimulationError::BusinessLifetimeRevenueOverflow {
                    business_id: recipient_id,
                    current: recipient.finance.lifetime_revenue,
                    incoming: transferred,
                })?,
            next_business_finance_version(recipient)?,
        )
    };
    {
        let payer = state
            .businesses
            .get_mut(payer_id)
            .expect("contract payer must exist");
        payer.finance.cash = payer
            .finance
            .cash
            .checked_sub(transferred)
            .expect("bounded contract transfer must fit payer cash");
        payer.finance.lifetime_costs = payer_lifetime_costs;
        payer.finance.version = payer_finance_version;
    }
    {
        let recipient = state
            .businesses
            .get_mut(recipient_id)
            .expect("contract recipient must exist");
        recipient.finance.cash = recipient
            .finance
            .cash
            .checked_add(transferred)
            .expect("bounded contract transfer must fit recipient cash");
        recipient.finance.lifetime_revenue = recipient_lifetime_revenue;
        recipient.finance.version = recipient_finance_version;
    }
    Ok(transferred)
}

fn settle_loans(state: &mut AppState) -> Result<(), SimulationError> {
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
        .map(|loan| DueLoan {
            id: loan.id,
            lender_id: loan.lender_dynasty_id,
            borrower_id: loan.borrower_dynasty_id,
            weekly_payment: loan.weekly_payment,
            balance: loan.balance,
            interest_basis_points: loan.interest_basis_points,
            collateral_property_id: loan.collateral_property_id,
        })
        .collect();
    for due_loan in due {
        settle_due_loan(state, due_loan, interest_limit)?;
    }
    Ok(())
}

fn settle_civic_debts(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
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

fn settle_due_civic_debt(
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
    let creditor_headroom = state
        .dynasties
        .get(&due.creditor_dynasty_id)
        .expect("civic debt creditor must exist")
        .treasury()
        .max_nonnegative_addend();
    if creditor_headroom < amount_due {
        state
            .civic_debts
            .get_mut(&due.id)
            .expect("civic debt must exist")
            .next_due_day = state.clock.day().saturating_add(7);
        return Ok(());
    }
    state
        .civic_debts
        .get_mut(&due.id)
        .expect("civic debt must exist")
        .balance = accrued_balance;
    let treasury_budget = state
        .institutions
        .get(&treasury_id)
        .expect("civic treasury must exist")
        .budget;
    if treasury_budget >= amount_due {
        settle_successful_civic_debt_payment(state, treasury_id, due, amount_due);
    } else {
        settle_missed_civic_debt_payment(state, treasury_id, due);
    }
    Ok(())
}

fn settle_successful_civic_debt_payment(
    state: &mut AppState,
    treasury_id: InstitutionId,
    due: DueCivicDebt,
    payment: Money,
) {
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
        debt.balance = debt
            .balance
            .checked_sub(payment)
            .expect("validated civic debt payment must not exceed debt balance");
        debt.next_due_day = debt.next_due_day.saturating_add(7);
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
            record_counterparty_information(
                state,
                sponsor_dynasty_id,
                due.creditor_dynasty_id,
                "Completed municipal debt repayment records",
            );
        }
    }
    if repaid {
        push_outbox(
            state,
            OutboxKind::Finance,
            format!("Civic debt {} repaid", due.id),
            format!(
                "The city treasury repaid dynasty {} in full.",
                due.creditor_dynasty_id
            ),
        );
    }
}

fn settle_missed_civic_debt_payment(
    state: &mut AppState,
    treasury_id: InstitutionId,
    due: DueCivicDebt,
) {
    let defaulted = {
        let debt = state
            .civic_debts
            .get_mut(&due.id)
            .expect("civic debt must exist");
        debt.missed_payments = debt.missed_payments.saturating_add(1);
        debt.next_due_day = debt.next_due_day.saturating_add(7);
        debt.status = if debt.missed_payments >= 3 {
            CivicDebtStatus::Defaulted
        } else {
            CivicDebtStatus::Delinquent
        };
        debt.status == CivicDebtStatus::Defaulted
    };
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
            record_counterparty_information(
                state,
                sponsor_dynasty_id,
                due.creditor_dynasty_id,
                "Municipal debt default and civic treasury records",
            );
        }
    }
    if defaulted {
        push_outbox(
            state,
            OutboxKind::Finance,
            format!("Civic debt {} defaulted", due.id),
            format!(
                "The city treasury defaulted on its obligation to dynasty {}.",
                due.creditor_dynasty_id
            ),
        );
    }
}

fn settle_due_loan(
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
    let lender_headroom = state
        .dynasties
        .get(&due.lender_id)
        .expect("loan lender must exist")
        .treasury()
        .max_nonnegative_addend();
    if lender_headroom < amount_due {
        state
            .loans
            .get_mut(&due.id)
            .expect("loan must exist")
            .next_due_day = state.clock.day().saturating_add(7);
        return Ok(());
    }
    state
        .loans
        .get_mut(&due.id)
        .expect("loan must exist")
        .balance = accrued_balance;
    if borrower_treasury >= amount_due {
        settle_successful_loan_payment(state, due, amount_due);
    } else {
        settle_missed_loan_payment(state, due);
    }
    Ok(())
}

fn settle_successful_loan_payment(state: &mut AppState, due: DueLoan, amount_due: Money) {
    apply_loan_payment(state, due.id, amount_due);
    let loan = state.loans.get_mut(&due.id).expect("loan must exist");
    loan.next_due_day = loan.next_due_day.saturating_add(7);
    loan.missed_payments = 0;
    if loan.status != LoanStatus::Repaid {
        loan.status = LoanStatus::Current;
    }
    adjust_reliability_reputation(state, due.borrower_id, 10);
}

fn settle_missed_loan_payment(state: &mut AppState, due: DueLoan) {
    let defaulted = {
        let loan = state.loans.get_mut(&due.id).expect("loan must exist");
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
        seize_defaulted_collateral(state, due);
        push_outbox(
            state,
            OutboxKind::Finance,
            format!("Loan {} defaulted", due.id),
            format!(
                "Dynasty {} defaulted on its obligation to dynasty {}.",
                due.borrower_id, due.lender_id
            ),
        );
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
        record_counterparty_information(
            state,
            due.lender_id,
            due.borrower_id,
            "Loan default and collateral records",
        );
    }
}

fn seize_defaulted_collateral(state: &mut AppState, due: DueLoan) {
    if let Some(property_id) = due.collateral_property_id {
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
        let property = state
            .properties
            .get_mut(&property_id)
            .expect("loan collateral must exist");
        property.owner_dynasty_id = Some(due.lender_id);
        property.tenant_dynasty_id = occupant_owner_id
            .or(existing_tenant_id)
            .filter(|tenant_id| *tenant_id != due.lender_id);
        property.collateral_loan_id = None;
    }
}

fn active_interest_limit(state: &AppState) -> Option<u16> {
    active_law_value(state, LawKind::InterestLimit)
        .map(|value| u16::try_from(value.clamp(0, 10_000)).unwrap_or(10_000))
}

fn weekly_interest_due(balance: Money, annual_interest_basis_points: u16) -> Money {
    if balance <= Money::ZERO || annual_interest_basis_points == 0 {
        return Money::ZERO;
    }
    let annual_interest =
        balance.saturating_mul_ratio(i64::from(annual_interest_basis_points), 10_000);
    if annual_interest <= Money::ZERO {
        return Money::ZERO;
    }
    let weekly_interest = annual_interest.copper() / 52;
    Money::from_copper(
        weekly_interest.saturating_add(i64::from(annual_interest.copper() % 52 != 0)),
    )
}

fn apply_loan_payment(state: &mut AppState, loan_id: crate::ids::LoanId, amount: Money) -> Money {
    if amount <= Money::ZERO {
        return Money::ZERO;
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
        .treasury = borrower_treasury
        .checked_sub(payment)
        .expect("validated loan payment must not exceed borrower treasury");
    let lender = state
        .dynasties
        .get_mut(&lender_id)
        .expect("loan lender must exist");
    lender.resources.treasury = lender
        .resources
        .treasury
        .checked_add(payment)
        .expect("prevalidated loan payment must fit lender treasury");
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
        record_completed_loan_repayment(state, lender_id, borrower_id, loan_id);
    } else {
        adjust_dynasty_relationship(
            state,
            lender_id,
            borrower_id,
            RelationshipDelta::new(4, 2, 0, -1, 0),
        );
    }
    payment
}

fn settle_property_rents(state: &mut AppState) -> Result<(), SimulationError> {
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
            let annual_cap = property_value.saturating_mul_ratio(limit, 10_000);
            contractual_rent.min(Money::from_copper(annual_cap.copper() / 52))
        });
        let owner_headroom = state
            .dynasties
            .get(&owner_id)
            .expect("property owner dynasty must exist")
            .treasury()
            .max_nonnegative_addend();
        let receivable_rent = rent.min(owner_headroom);
        if receivable_rent <= Money::ZERO {
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
            let paid = receivable_rent.min(tenant_cash);
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
            debit_market_clearing_account(state, receivable_rent)?;
            receivable_rent
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

fn distribute_business_dividends(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
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
            let owner_headroom = state
                .dynasties
                .get(&business.owner_dynasty_id())
                .expect("dividend owner dynasty must exist")
                .treasury()
                .max_nonnegative_addend();
            let dividend = Money::from_copper(excess.copper() / 10)
                .min(Money::from_copper(1_000))
                .min(owner_headroom);
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
        let resulting_cash = business
            .finance
            .cash
            .checked_sub(dividend)
            .expect("planned dividend must fit business cash");
        let next_finance_version = next_business_finance_version(business)?;
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
    Ok(())
}

fn settle_employment(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
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
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct LaborEnvironment {
    utilization: u16,
    business_condition: u16,
    maintenance: u16,
}

fn settle_employment_agreement(
    registry: &Registry,
    state: &mut AppState,
    employment_id: EmploymentId,
    business_id: BusinessId,
    household_id: HouseholdId,
    wage: Money,
    prior_status: EmploymentStatus,
) -> Result<(), SimulationError> {
    let utilization_basis_points =
        business_labor_utilization_basis_points(registry, state, business_id);
    let labor_environment = {
        let business = state
            .businesses
            .get(business_id)
            .expect("employment business must exist");
        LaborEnvironment {
            utilization: utilization_basis_points,
            business_condition: business.operations.condition_basis_points,
            maintenance: business.policy.maintenance_basis_points,
        }
    };
    let wage_due = wage.saturating_mul_ratio(i64::from(utilization_basis_points), 10_000);
    let paid = pay_employment_wage(registry, state, business_id, household_id, wage_due)?;
    let (recovered, became_disputed) = update_employment_after_payment(
        state,
        employment_id,
        prior_status,
        labor_environment,
        paid,
        wage_due,
    );
    emit_employment_outcome(state, business_id, recovered, became_disputed);
    Ok(())
}

fn pay_employment_wage(
    registry: &Registry,
    state: &mut AppState,
    business_id: BusinessId,
    household_id: HouseholdId,
    wage_due: Money,
) -> Result<Money, SimulationError> {
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
    let spendable = business_cash.saturating_sub(payroll_reserve);
    if wage_due <= Money::ZERO || spendable <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let household_headroom = state
        .households
        .get(household_id)
        .expect("employment household must exist")
        .cash
        .max_nonnegative_addend();
    let paid = wage_due.min(spendable).min(household_headroom);
    if paid <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let (resulting_lifetime_costs, next_finance_version) = {
        let business = state
            .businesses
            .get(business_id)
            .expect("employment business must exist");
        (
            business.finance.lifetime_costs.checked_add(paid).ok_or(
                SimulationError::BusinessLifetimeCostsOverflow {
                    business_id,
                    current: business.finance.lifetime_costs,
                    incoming: paid,
                },
            )?,
            next_business_finance_version(business)?,
        )
    };
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("employment business must exist");
    business.finance.cash = business_cash
        .checked_sub(paid)
        .expect("bounded wage must fit business cash");
    business.finance.lifetime_costs = resulting_lifetime_costs;
    business.finance.version = next_finance_version;
    let household = state
        .households
        .get_mut(household_id)
        .expect("employment household must exist");
    household.cash = household
        .cash
        .checked_add(paid)
        .expect("bounded wage must fit household cash");
    Ok(paid)
}

fn update_employment_after_payment(
    state: &mut AppState,
    employment_id: EmploymentId,
    prior_status: EmploymentStatus,
    environment: LaborEnvironment,
    paid: Money,
    wage_due: Money,
) -> (bool, bool) {
    let agreement = state
        .employment
        .get_mut(&employment_id)
        .expect("employment must exist");
    if paid == wage_due {
        return update_fully_paid_employment(agreement, prior_status, environment);
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
    environment: LaborEnvironment,
) -> (bool, bool) {
    if prior_status != EmploymentStatus::Disputed {
        let strain = labor_strain_basis_points(environment);
        if strain > 0 {
            agreement.conditions_basis_points =
                agreement.conditions_basis_points.saturating_sub(strain);
            agreement.loyalty_basis_points = agreement
                .loyalty_basis_points
                .saturating_sub(strain.saturating_div(2));
            let became_disputed =
                agreement.conditions_basis_points < 3_000 || agreement.loyalty_basis_points < 2_000;
            if became_disputed {
                agreement.status = EmploymentStatus::Disputed;
            }
            return (false, became_disputed);
        }
        if environment.utilization == 10_000 {
            agreement.loyalty_basis_points = agreement
                .loyalty_basis_points
                .saturating_add(30)
                .min(10_000);
            agreement.conditions_basis_points = agreement
                .conditions_basis_points
                .saturating_add(10)
                .min(10_000);
        }
        return (false, false);
    }
    agreement.loyalty_basis_points = agreement
        .loyalty_basis_points
        .saturating_add(180)
        .min(10_000);
    agreement.conditions_basis_points = agreement
        .conditions_basis_points
        .saturating_add(60)
        .min(10_000);
    let recovered = agreement.loyalty_basis_points >= super::EMPLOYMENT_RECOVERY_BASIS_POINTS
        && agreement.conditions_basis_points >= super::EMPLOYMENT_RECOVERY_BASIS_POINTS;
    if recovered {
        agreement.status = EmploymentStatus::Active;
    }
    (recovered, false)
}

fn labor_strain_basis_points(environment: LaborEnvironment) -> u16 {
    if environment.utilization < 9_000 {
        return 0;
    }
    let maintenance_strain = 1_000_u16
        .saturating_sub(environment.maintenance)
        .saturating_div(5);
    let condition_strain = 7_000_u16
        .saturating_sub(environment.business_condition)
        .saturating_div(20);
    maintenance_strain.saturating_add(condition_strain).min(180)
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
            "Accumulated wage, workload, or workplace-condition pressure caused organized resistance."
                .to_owned(),
        );
    }
}

fn business_labor_utilization_basis_points(
    registry: &Registry,
    state: &AppState,
    business_id: BusinessId,
) -> u16 {
    const RETAINER_BASIS_POINTS: i64 = 2_500;
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
    let output_per_batch = recipe.output_quantity().milliunits();
    if output_per_batch <= 0 {
        return 0;
    }
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
    let reserve_shortfall = policy_reserve
        .saturating_sub(business.inventory_quantity(output_good_id))
        .max(Quantity::ZERO);
    let weekly_market_demand = state
        .market
        .quotes
        .get(&output_good_id)
        .map_or(Quantity::ZERO, |quote| {
            quote.demand_today.saturating_mul_ratio(7, 1)
        });
    let required_output = reserve_shortfall
        .saturating_add(contract_reserve)
        .saturating_add(weekly_market_demand);
    let required_batches = ceil_div_nonnegative_i64(required_output.milliunits(), output_per_batch);
    let weekly_capacity_batches =
        i64::from(business.operations.capacity_batches_per_day).saturating_mul(7);
    if weekly_capacity_batches <= 0 {
        return 0;
    }
    let utilization_numerator = required_batches.saturating_mul(10_000);
    let utilization = ceil_div_nonnegative_i64(utilization_numerator, weekly_capacity_batches)
        .clamp(RETAINER_BASIS_POINTS, 10_000);
    u16::try_from(utilization).expect("clamped utilization must fit u16")
}

fn ceil_div_nonnegative_i64(numerator: i64, denominator: i64) -> i64 {
    debug_assert!(numerator >= 0 && denominator > 0);
    let quotient = numerator / denominator;
    quotient.saturating_add(i64::from(numerator % denominator != 0))
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
            treasury.budget = treasury
                .budget
                .checked_sub(weekly_spend)
                .expect("bounded public-work spending must not exceed treasury budget");
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
                let progress = work
                    .spent
                    .saturating_mul_ratio(10_000, work.budget.copper())
                    .copper();
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
        let mut lifetime_revenue = Money::ZERO;
        let mut lifetime_costs = Money::ZERO;
        for business in state.businesses.iter().filter(|business| {
            business.owner_dynasty_id() == dynasty_id
                && business.status() != crate::core::BusinessStatus::Closed
        }) {
            total_quality =
                total_quality.saturating_add(u64::from(business.operations.quality_basis_points));
            business_count = business_count.saturating_add(1);
            lifetime_revenue = lifetime_revenue.saturating_add(business.finance.lifetime_revenue);
            lifetime_costs = lifetime_costs.saturating_add(business.finance.lifetime_costs);
        }
        if business_count == 0 {
            continue;
        }
        let target = u16::try_from(total_quality / business_count).unwrap_or(10_000);
        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("reputation dynasty must exist");
        let maximum_step = quality_reputation_step(
            dynasty.resources.reputation_quality_basis_points,
            target,
            lifetime_revenue,
            lifetime_costs,
        );
        dynasty.resources.reputation_quality_basis_points = move_basis_points_toward(
            dynasty.resources.reputation_quality_basis_points,
            target,
            maximum_step,
        );
    }
}

fn quality_reputation_step(
    current: u16,
    target: u16,
    lifetime_revenue: Money,
    lifetime_costs: Money,
) -> u16 {
    if current >= target {
        return 50;
    }
    let has_trade_history = lifetime_revenue > Money::ZERO || lifetime_costs > Money::ZERO;
    if has_trade_history && lifetime_revenue >= lifetime_costs {
        50
    } else {
        25
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct RelationshipDelta {
    trust: i16,
    respect: i16,
    fear: i16,
    resentment: i16,
    obligation: i32,
}

impl RelationshipDelta {
    pub(crate) const fn new(
        trust: i16,
        respect: i16,
        fear: i16,
        resentment: i16,
        obligation: i32,
    ) -> Self {
        Self {
            trust,
            respect,
            fear,
            resentment,
            obligation,
        }
    }
}

pub(crate) fn adjust_dynasty_relationship(
    state: &mut AppState,
    left_dynasty_id: DynastyId,
    right_dynasty_id: DynastyId,
    delta: RelationshipDelta,
) {
    if left_dynasty_id == right_dynasty_id {
        return;
    }
    let pair = DynastyPair::new(left_dynasty_id, right_dynasty_id);
    let day = state.clock.day();
    let relationship = state
        .relationships
        .get_mut(&pair)
        .expect("every dynasty pair must have a relationship record");
    relationship.trust_basis_points =
        adjust_basis_points(relationship.trust_basis_points, delta.trust);
    relationship.respect_basis_points =
        adjust_basis_points(relationship.respect_basis_points, delta.respect);
    relationship.fear_basis_points =
        adjust_basis_points(relationship.fear_basis_points, delta.fear);
    relationship.resentment_basis_points =
        adjust_basis_points(relationship.resentment_basis_points, delta.resentment);
    relationship.obligation = relationship.obligation.saturating_add(delta.obligation);
    relationship.last_interaction_day = day;
}

const MAX_RELATIONSHIP_MEMORIES: usize = 12;

pub(crate) fn remember_dynasty_interaction(
    state: &mut AppState,
    left_dynasty_id: DynastyId,
    right_dynasty_id: DynastyId,
    memory: &str,
) {
    if left_dynasty_id == right_dynasty_id {
        return;
    }
    let pair = DynastyPair::new(left_dynasty_id, right_dynasty_id);
    let day = state.clock.day();
    let relationship = state
        .relationships
        .get_mut(&pair)
        .expect("every dynasty pair must have a relationship record");
    if relationship.memories.len() >= MAX_RELATIONSHIP_MEMORIES {
        relationship.memories.remove(0);
    }
    relationship.memories.push(format!("Day {day}: {memory}"));
    relationship.last_interaction_day = day;
}

pub(crate) fn record_counterparty_information(
    state: &mut AppState,
    first_dynasty_id: DynastyId,
    second_dynasty_id: DynastyId,
    source: &str,
) {
    let player_dynasty_id = state.player_dynasty_id;
    let counterparty_id =
        if first_dynasty_id == player_dynasty_id && second_dynasty_id != player_dynasty_id {
            second_dynasty_id
        } else if second_dynasty_id == player_dynasty_id && first_dynasty_id != player_dynasty_id {
            first_dynasty_id
        } else {
            return;
        };
    let counterparty = state
        .dynasties
        .get(&counterparty_id)
        .expect("counterparty dynasty must exist");
    let target = InformationTarget::Counterparty {
        dynasty_id: counterparty_id,
    };
    let subject = format!("Counterparty report: House {}", counterparty.name());
    let reliability = counterparty.resources.reputation_reliability_basis_points;
    let pair = DynastyPair::new(player_dynasty_id, counterparty_id);
    let relationship = state
        .relationships
        .get(&pair)
        .expect("counterparty relationship must exist");
    let summary = format!(
        "Reliability {reliability} bp; trust {} bp; respect {} bp; resentment {} bp; obligation {}.",
        relationship.trust_basis_points,
        relationship.respect_basis_points,
        relationship.resentment_basis_points,
        relationship.obligation
    );
    state.information_reports.retain(|_, report| {
        report.owner_dynasty_id != player_dynasty_id || report.target != Some(target)
    });
    let id = state.next_ids.information_report();
    let day = state.clock.day();
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: player_dynasty_id,
            target: Some(target),
            subject,
            confidence: InformationConfidence::Probable,
            created_day: day,
            expires_day: day.saturating_add(180),
            source: source.to_owned(),
            summary,
        },
    );
}

fn adjust_basis_points(current: u16, delta: i16) -> u16 {
    u16::try_from(
        i32::from(current)
            .saturating_add(i32::from(delta))
            .clamp(0, 10_000),
    )
    .expect("clamped basis-point value must fit u16")
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

pub(crate) fn run_monthly_strategic_systems(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    update_district_conditions(state);
    resolve_institution_selections(registry, state)?;
    apply_office_duties(state)?;
    apply_office_power_effects(registry, state)?;
    advance_ai_objectives(registry, state);
    update_information_reports(registry, state);
    advance_legal_case_hearings(state);
    resolve_legal_cases(state);
    update_external_route_risk(state);
    detect_and_advance_crises(registry, state);
    recover_external_routes(state);
    Ok(())
}

pub(crate) fn dynasty_office_administrative_load(state: &AppState, dynasty_id: DynastyId) -> u16 {
    state
        .institutions
        .values()
        .filter(|institution| {
            institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == dynasty_id)
            })
        })
        .fold(0_u16, |load, institution| {
            let power_count = u16::try_from(institution.powers.len()).unwrap_or(u16::MAX);
            load.saturating_add(power_count.saturating_mul(OFFICE_ADMINISTRATIVE_LOAD_PER_POWER))
        })
}

fn apply_office_duties(state: &mut AppState) -> Result<(), SimulationError> {
    let duties: Vec<_> = state
        .institutions
        .values()
        .filter_map(|institution| {
            let holder_id = institution.office_holder_id?;
            let dynasty_id = state.characters.get(holder_id)?.dynasty_id();
            let power_count = u16::try_from(institution.powers.len()).unwrap_or(u16::MAX);
            Some((institution.institution_id, dynasty_id, power_count))
        })
        .collect();
    for (institution_id, dynasty_id, power_count) in duties {
        apply_office_duty(state, institution_id, dynasty_id, power_count)?;
    }
    Ok(())
}

fn apply_office_duty(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    power_count: u16,
) -> Result<(), SimulationError> {
    let required = OFFICE_DUTY_COST_PER_POWER.saturating_mul(i64::from(power_count));
    let institution_budget = state
        .institutions
        .get(&institution_id)
        .expect("office institution must exist")
        .budget;
    let collectible = required.min(institution_budget.max_nonnegative_addend());
    if collectible == Money::ZERO {
        return Ok(());
    }
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .expect("officeholder dynasty must exist")
        .treasury();
    let paid = collectible.min(treasury);
    transfer_office_duty_payment(
        state,
        institution_id,
        dynasty_id,
        institution_budget,
        treasury,
        paid,
    )?;
    if paid < collectible {
        record_office_duty_shortfall(
            state,
            institution_id,
            dynasty_id,
            required,
            paid,
            collectible.saturating_sub(paid),
        );
    }
    Ok(())
}

fn transfer_office_duty_payment(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    institution_budget: Money,
    treasury: Money,
    paid: Money,
) -> Result<(), SimulationError> {
    if paid == Money::ZERO {
        return Ok(());
    }
    let current_contributions = state
        .dynasties
        .get(&dynasty_id)
        .expect("officeholder dynasty must exist")
        .resources
        .civic_contributions;
    let next_contributions = current_contributions.checked_add(paid).ok_or(
        SimulationError::DynastyCivicContributionsOverflow {
            dynasty_id,
            current: current_contributions,
            incoming: paid,
        },
    )?;
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("officeholder dynasty must exist");
    dynasty.resources.treasury = treasury
        .checked_sub(paid)
        .expect("validated office-duty payment must not exceed dynasty treasury");
    dynasty.resources.civic_contributions = next_contributions;
    state
        .institutions
        .get_mut(&institution_id)
        .expect("office institution must exist")
        .budget = institution_budget
        .checked_add(paid)
        .expect("bounded civic duty contribution must fit institution budget");
    Ok(())
}

fn record_office_duty_shortfall(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    required: Money,
    paid: Money,
    shortfall: Money,
) {
    let subject = office_duty_subject(institution_id, dynasty_id);
    let recent_shortfalls = recent_office_duty_shortfalls(state, &subject);
    let should_notify = should_notify_office_duty_shortfall(state, &subject);
    penalize_office_duty_shortfall(state, institution_id, dynasty_id);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::OfficeDutyShortfall,
        subject: subject.clone(),
        detail: format!("required={required};paid={paid};shortfall={shortfall}"),
    });
    let forfeited = recent_shortfalls.saturating_add(1) >= OFFICE_DUTY_FORFEITURE_THRESHOLD;
    if forfeited {
        forfeit_office_for_unmet_duties(
            state,
            institution_id,
            &subject,
            recent_shortfalls.saturating_add(1),
        );
    }
    notify_player_office_duty_outcome(
        state,
        OfficeDutyOutcome {
            institution_id,
            dynasty_id,
            required,
            paid,
            shortfall,
            forfeited,
            should_notify,
        },
    );
}

fn recent_office_duty_shortfalls(state: &AppState, subject: &str) -> usize {
    state
        .audit_log
        .iter()
        .filter(|record| {
            record.kind() == AuditKind::OfficeDutyShortfall
                && record.subject() == subject
                && state.clock.day().saturating_sub(record.day())
                    <= OFFICE_DUTY_FORFEITURE_WINDOW_DAYS
        })
        .count()
}

fn should_notify_office_duty_shortfall(state: &AppState, subject: &str) -> bool {
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::OfficeDutyShortfall && record.subject() == subject
        })
        .is_none_or(|record| {
            state.clock.day()
                >= record
                    .day()
                    .saturating_add(OFFICE_DUTY_FAILURE_NOTIFICATION_INTERVAL_DAYS)
        })
}

fn penalize_office_duty_shortfall(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
) {
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("officeholder dynasty must exist");
    dynasty.resources.unmet_office_duties = dynasty.resources.unmet_office_duties.saturating_add(1);
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_sub(120);
    dynasty.resources.reputation_reliability_basis_points = dynasty
        .resources
        .reputation_reliability_basis_points
        .saturating_sub(80);
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("office institution must exist");
    institution.legitimacy_basis_points = institution.legitimacy_basis_points.saturating_sub(100);
}

fn forfeit_office_for_unmet_duties(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    subject: &str,
    recent_shortfalls: usize,
) {
    let day = state.clock.day();
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("office institution must exist");
    institution.office_holder_id = None;
    institution.next_selection_day = day.saturating_add(30);
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::OfficeDutyForfeiture,
        subject: subject.to_owned(),
        detail: format!("office forfeited after {recent_shortfalls} recent duty shortfalls"),
    });
}

#[derive(Clone, Copy)]
struct OfficeDutyOutcome {
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    required: Money,
    paid: Money,
    shortfall: Money,
    forfeited: bool,
    should_notify: bool,
}

fn notify_player_office_duty_outcome(state: &mut AppState, outcome: OfficeDutyOutcome) {
    if outcome.dynasty_id != state.player_dynasty_id {
        return;
    }
    if outcome.forfeited {
        push_outbox(
            state,
            OutboxKind::Politics,
            format!("Office forfeited at institution {}", outcome.institution_id),
            "Repeatedly unmet civic duties forced the dynasty to surrender the office. The institution will select a replacement next month, and the dynasty cannot immediately return to the same office."
                .to_owned(),
        );
    } else if outcome.should_notify {
        push_outbox(
            state,
            OutboxKind::Politics,
            format!(
                "Office duty shortfall at institution {}",
                outcome.institution_id
            ),
            format!(
                "The dynasty funded {} of a {} monthly civic duty. The {} shortfall reduced institutional and dynastic standing.",
                outcome.paid, outcome.required, outcome.shortfall
            ),
        );
    }
}

fn office_duty_subject(institution_id: crate::ids::InstitutionId, dynasty_id: DynastyId) -> String {
    format!(
        "institution:{};dynasty:{}",
        institution_id.value(),
        dynasty_id.value()
    )
}

fn recover_external_routes(state: &mut AppState) {
    for route in state.external_routes.values_mut() {
        route.disruption_basis_points = route.disruption_basis_points.saturating_sub(750);
    }
}

fn apply_office_power_effects(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
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
            apply_office_power(
                registry,
                state,
                institution_id,
                dynasty_id,
                district_id,
                power,
            )?;
        }
    }
    Ok(())
}

fn apply_office_power(
    registry: &Registry,
    state: &mut AppState,
    institution_id: InstitutionId,
    dynasty_id: DynastyId,
    district_id: DistrictId,
    power: OfficePower,
) -> Result<(), SimulationError> {
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
            let institution_budget = state
                .institutions
                .get(&institution_id)
                .expect("office institution must exist")
                .budget;
            let revenue = Money::from_copper(100).min(institution_budget.max_nonnegative_addend());
            if revenue > Money::ZERO {
                state
                    .institutions
                    .get_mut(&institution_id)
                    .expect("office institution must exist")
                    .budget = institution_budget
                    .checked_add(revenue)
                    .expect("bounded office revenue must fit institution budget");
                debit_market_clearing_account(state, revenue)?;
            }
        }
        OfficePower::DebtEnforcement => adjust_reliability_reputation(state, dynasty_id, 15),
        OfficePower::CityContracts => award_city_contract(state, institution_id, dynasty_id)?,
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
    Ok(())
}

fn award_city_contract(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
) -> Result<(), SimulationError> {
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
        return Ok(());
    };
    let institution_budget = state
        .institutions
        .get(&institution_id)
        .expect("city contract institution must exist")
        .budget;
    let business_headroom = state
        .businesses
        .get(business_id)
        .expect("city contract business must exist")
        .cash()
        .max_nonnegative_addend();
    let award = Money::from_copper(250)
        .min(institution_budget)
        .min(business_headroom);
    if award == Money::ZERO {
        return Ok(());
    }
    let (resulting_lifetime_revenue, next_finance_version) = {
        let business = state
            .businesses
            .get(business_id)
            .expect("city contract business must exist");
        (
            business.finance.lifetime_revenue.checked_add(award).ok_or(
                SimulationError::BusinessLifetimeRevenueOverflow {
                    business_id,
                    current: business.finance.lifetime_revenue,
                    incoming: award,
                },
            )?,
            next_business_finance_version(business)?,
        )
    };
    state
        .institutions
        .get_mut(&institution_id)
        .expect("city contract institution must exist")
        .budget = institution_budget
        .checked_sub(award)
        .expect("bounded city-contract award must not exceed institution budget");
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("city contract business must exist");
    business.finance.cash = business
        .finance
        .cash
        .checked_add(award)
        .expect("bounded city-contract award must fit business cash");
    business.finance.lifetime_revenue = resulting_lifetime_revenue;
    business.finance.version = next_finance_version;
    Ok(())
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
        let satisfaction = crate::core::population_weighted_food_satisfaction_basis_points(
            households.iter().copied(),
        )
        .unwrap_or(5_000);
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
            .saturating_add(u32::from(district.sanitation_basis_points));
        district.rent_index_basis_points = u16::try_from(
            u32::from(super::MIN_DISTRICT_RENT_INDEX_BASIS_POINTS)
                .saturating_add(desirability / 3)
                .min(u32::from(super::MAX_DISTRICT_RENT_INDEX_BASIS_POINTS)),
        )
        .expect("bounded district rent index must fit u16");
    }
}

fn resolve_institution_selections(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let due: Vec<_> = state
        .institutions
        .values()
        .filter(|institution| institution.next_selection_day <= day)
        .map(|institution| institution.institution_id)
        .collect();
    let mut selections = Vec::new();
    let mut planned_office_holders = BTreeSet::new();
    for institution_id in due {
        let institution_kind = registry
            .get_institution(institution_id)
            .expect("runtime institution must have a registry definition")
            .kind();
        let institution = state
            .institutions
            .get(&institution_id)
            .expect("institution runtime must exist");
        let incumbent_id = institution.office_holder_id;
        let member_ids: Vec<_> = institution.members.iter().copied().collect();
        let candidates: Vec<_> = member_ids
            .iter()
            .filter_map(|character_id| state.characters.get(*character_id))
            .filter(|character| character.status() == crate::core::CharacterStatus::Active)
            .filter(|character| !planned_office_holders.contains(&character.id()))
            .filter(|character| {
                !state.institutions.values().any(|other| {
                    other.institution_id != institution_id
                        && other.office_holder_id == Some(character.id())
                })
            })
            .filter(|character| {
                !has_recent_office_duty_forfeiture(
                    state,
                    institution_id,
                    character.dynasty_id(),
                    day,
                ) && (character.dynasty_id() != state.player_dynasty_id
                    || incumbent_id == Some(character.id())
                    || has_recent_office_nomination(state, institution_id, character.id(), day))
            })
            .map(|character| {
                let dynasty = state
                    .dynasties
                    .get(&character.dynasty_id())
                    .expect("candidate dynasty must exist");
                let campaign_bonus =
                    if has_recent_office_nomination(state, institution_id, character.id(), day) {
                        4_000
                    } else {
                        0
                    };
                let relationship_support =
                    institution_relationship_support(state, institution_id, character.dynasty_id());
                let score = institution_capability_score(character, institution_kind)
                    .saturating_add(u32::from(dynasty.resources.legitimacy_basis_points))
                    .saturating_add(campaign_bonus)
                    .saturating_add(relationship_support);
                (score, character.id())
            })
            .collect();
        let winner = candidates
            .into_iter()
            .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
            .map(|(_, character_id)| character_id);
        let term_number = institution
            .term_number
            .checked_add(1)
            .ok_or(SimulationError::InstitutionTermNumberExhausted { institution_id })?;
        if let Some(winner) = winner {
            planned_office_holders.insert(winner);
        }
        selections.push((institution_id, winner, term_number));
    }

    for (institution_id, winner, term_number) in selections {
        let institution = state
            .institutions
            .get_mut(&institution_id)
            .expect("institution runtime must exist");
        institution.office_holder_id = winner;
        institution.term_started_day = day;
        institution.next_selection_day = day.saturating_add(super::OFFICE_TERM_DAYS);
        institution.term_number = term_number;
        if let Some(winner) = winner {
            push_outbox(
                state,
                OutboxKind::Politics,
                format!("Institution {institution_id} selected a new officeholder"),
                format!("Character {winner} now holds the office for term {term_number}."),
            );
        }
    }
    Ok(())
}

pub(crate) fn institution_capability_score(
    character: &crate::core::Character,
    institution_kind: InstitutionKind,
) -> u32 {
    let capabilities = &character.capabilities;
    let (primary, secondary) = match institution_kind {
        InstitutionKind::CraftGuild => (capabilities.craft, capabilities.commerce),
        InstitutionKind::MerchantGuild | InstitutionKind::MarketOffice => {
            (capabilities.commerce, capabilities.administration)
        }
        InstitutionKind::Council | InstitutionKind::Charity => {
            (capabilities.social, capabilities.administration)
        }
        InstitutionKind::Court | InstitutionKind::Watch => {
            (capabilities.administration, capabilities.social)
        }
        InstitutionKind::Treasury => (capabilities.administration, capabilities.commerce),
    };
    u32::from(primary)
        .saturating_mul(100)
        .saturating_add(u32::from(secondary).saturating_mul(30))
}

fn has_recent_office_nomination(
    state: &AppState,
    institution_id: crate::ids::InstitutionId,
    character_id: CharacterId,
    day: i64,
) -> bool {
    let nomination_subject =
        super::commands::office_nomination_subject(institution_id, character_id);
    state.audit_log.iter().rev().any(|record| {
        record.kind() == AuditKind::OfficeNomination
            && record.subject() == nomination_subject
            && day.saturating_sub(record.day()) <= 180
    })
}

fn has_recent_office_duty_forfeiture(
    state: &AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    day: i64,
) -> bool {
    let subject = office_duty_subject(institution_id, dynasty_id);
    state.audit_log.iter().rev().any(|record| {
        record.kind() == AuditKind::OfficeDutyForfeiture
            && record.subject() == subject
            && day.saturating_sub(record.day()) <= OFFICE_DUTY_REELECTION_BAN_DAYS
    })
}

fn institution_relationship_support(
    state: &AppState,
    institution_id: crate::ids::InstitutionId,
    candidate_dynasty_id: DynastyId,
) -> u32 {
    let member_dynasties: BTreeSet<_> = state
        .institutions
        .get(&institution_id)
        .expect("institution runtime must exist")
        .members
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .map(crate::core::Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != candidate_dynasty_id)
        .collect();
    let mut total = 0_u32;
    let mut count = 0_u32;
    for dynasty_id in member_dynasties {
        let relationship = state
            .relationships
            .get(&DynastyPair::new(candidate_dynasty_id, dynasty_id))
            .expect("every dynasty pair must have a relationship record");
        let positive = u32::from(relationship.trust_basis_points)
            .saturating_add(u32::from(relationship.respect_basis_points))
            .saturating_add(u32::from(relationship.fear_basis_points) / 2);
        total = total.saturating_add(
            positive.saturating_sub(u32::from(relationship.resentment_basis_points)),
        );
        count = count.saturating_add(1);
    }
    total
        .checked_div(count)
        .map_or(0, |average| (average / 4).min(3_000))
}

fn advance_ai_objectives(registry: &Registry, state: &mut AppState) {
    let day = state.clock.day();
    let objectives: Vec<_> = state
        .ai_objectives
        .values()
        .filter(|objective| objective.status == ObjectiveStatus::Pursuing)
        .map(|objective| {
            (
                objective.id,
                objective.dynasty_id,
                objective.kind,
                objective.created_day,
            )
        })
        .collect();
    for (objective_id, dynasty_id, kind, created_day) in objectives {
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
        let terminal = match progress {
            ObjectiveProgress::Achieved => Some((
                ObjectiveStatus::Achieved,
                "The prior objective was completed; the house selected the next strongest route to durable power.",
            )),
            ObjectiveProgress::Pending
                if day.saturating_sub(created_day) >= AI_OBJECTIVE_REVIEW_DAYS =>
            {
                Some((
                    ObjectiveStatus::Abandoned,
                    "The prior objective stalled; the house redirected resources toward a more viable route to durable power.",
                ))
            }
            ObjectiveProgress::Pending => None,
        };
        if let Some((terminal_status, rationale)) = terminal {
            let objective = state
                .ai_objectives
                .get_mut(&objective_id)
                .expect("AI objective must exist");
            objective.status = terminal_status;
            if terminal_status == ObjectiveStatus::Abandoned {
                objective.rationale.push_str(
                    " The house abandoned this route after two years without decisive progress.",
                );
            }
            let new_id = state.next_ids.objective();
            state.ai_objectives.insert(
                new_id,
                AiObjective {
                    id: new_id,
                    dynasty_id,
                    kind: next_objective_kind(kind),
                    target_dynasty_id: Some(state.player_dynasty_id),
                    priority: 50,
                    created_day: day,
                    status: ObjectiveStatus::Pursuing,
                    rationale: rationale.to_owned(),
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
        dynasty.resources.treasury = dynasty
            .resources
            .treasury
            .checked_sub(spend)
            .expect("bounded AI office spending must not exceed treasury");
        let legitimacy_gain = u16::try_from(spend.saturating_mul_ratio(80, 500).copper())
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
        .filter_map(|business_id| {
            state
                .businesses
                .get(*business_id)
                .filter(|business| {
                    !matches!(
                        business.status(),
                        BusinessStatus::Insolvent | BusinessStatus::Closed
                    )
                })
                .map(crate::core::Business::id)
        })
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
            if sign_supply_contract(registry, state, terms).is_ok() {
                return ObjectiveProgress::Achieved;
            }
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
    dynasty.resources.treasury = dynasty
        .resources
        .treasury
        .checked_sub(spend)
        .expect("bounded AI legitimacy spending must not exceed treasury");
    let legitimacy_gain = u16::try_from(spend.saturating_mul_ratio(120, 750).copper())
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
            good.id(),
            good.name().to_owned(),
            quote.price(),
            quote.causes().to_vec(),
        ))
    });
    let Some((_, good_id, name, price, causes)) = most_changed.max_by_key(|item| item.0) else {
        return;
    };
    let id = state.next_ids.information_report();
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: state.player_dynasty_id,
            target: Some(InformationTarget::Market { good_id }),
            subject: format!("Monthly market report: {name}"),
            confidence: InformationConfidence::Confirmed,
            created_day: day,
            expires_day: day.saturating_add(120),
            source: "House ledgers, guild correspondence, and market inspection".to_owned(),
            summary: format!("{name} is priced at {price}; identified causes: {causes:?}."),
        },
    );
}

fn advance_legal_case_hearings(state: &mut AppState) {
    let day = state.clock.day();
    let entering_hearing: Vec<_> = state
        .legal_cases
        .values()
        .filter(|legal_case| {
            legal_case.status == LegalCaseStatus::Filed
                && legal_case.hearing_day > day
                && legal_case.hearing_day.saturating_sub(day) <= 30
        })
        .map(|legal_case| legal_case.id)
        .collect();
    for legal_case_id in entering_hearing {
        state
            .legal_cases
            .get_mut(&legal_case_id)
            .expect("legal case must exist")
            .status = LegalCaseStatus::Hearing;
        push_outbox(
            state,
            OutboxKind::Legal,
            format!("Legal case {legal_case_id} entered hearing"),
            "The court began formal proceedings ahead of judgment.".to_owned(),
        );
    }
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
            settle_legal_damages(state, plaintiff_id, defendant_id, damages);
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
        adjust_dynasty_relationship(
            state,
            plaintiff_id,
            defendant_id,
            RelationshipDelta::new(-60, 20, 50, 120, 0),
        );
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

fn settle_legal_damages(
    state: &mut AppState,
    plaintiff_id: DynastyId,
    defendant_id: DynastyId,
    damages: Money,
) {
    let defendant_cash = state
        .dynasties
        .get(&defendant_id)
        .expect("legal defendant must exist")
        .treasury();
    let plaintiff_headroom = state
        .dynasties
        .get(&plaintiff_id)
        .expect("legal plaintiff must exist")
        .treasury()
        .max_nonnegative_addend();
    let paid = damages.min(defendant_cash).min(plaintiff_headroom);
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
        .expect("bounded damages must fit plaintiff treasury");
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
    advance_existing_crises(state);
    let has_grain_crisis = state
        .crises
        .values()
        .any(|crisis| crisis.kind == CrisisKind::GrainShortage && crisis.status.is_active());
    if !has_grain_crisis {
        let bread_stock_low = registry
            .get_good_id("bread")
            .and_then(|id| state.market.get_quote(id))
            .is_some_and(|quote| quote.stock() < Quantity::from_units(100));
        let average_satisfaction = crate::core::population_weighted_food_satisfaction_basis_points(
            state.households.iter(),
        )
        .unwrap_or(10_000);
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
        .count()
        .saturating_add(
            state
                .civic_debts
                .values()
                .filter(|debt| debt.status == CivicDebtStatus::Defaulted)
                .count(),
        );
    let active_panic = state
        .crises
        .values()
        .any(|crisis| crisis.kind == CrisisKind::BankingPanic && crisis.status.is_active());
    let prior_panics = state
        .crises
        .values()
        .filter(|crisis| crisis.kind == CrisisKind::BankingPanic)
        .count();
    let next_panic_threshold = prior_panics.saturating_add(1).saturating_mul(2);
    if defaulted_loans >= next_panic_threshold && !active_panic {
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

fn advance_existing_crises(state: &mut AppState) {
    let mut resolved = Vec::new();
    let mut escalated = Vec::new();
    let addressed_subjects: BTreeSet<_> = state
        .audit_log
        .iter()
        .filter(|record| record.kind() == AuditKind::CrisisResponse)
        .map(|record| record.subject().to_owned())
        .collect();
    for crisis in state.crises.values_mut() {
        if !crisis.status.is_active() {
            continue;
        }
        let previous_status = crisis.status;
        let subject = format!("crisis:{}", crisis.id);
        crisis.severity_basis_points = if addressed_subjects.contains(&subject) {
            crisis
                .severity_basis_points
                .saturating_sub(ADDRESSED_CRISIS_MONTHLY_RECOVERY_BASIS_POINTS)
        } else {
            crisis
                .severity_basis_points
                .saturating_add(UNADDRESSED_CRISIS_MONTHLY_ESCALATION_BASIS_POINTS)
                .min(10_000)
        };
        crisis.status = CrisisStatus::from_severity(crisis.severity_basis_points);
        if crisis.status == CrisisStatus::Resolved {
            resolved.push((crisis.id, crisis.kind));
        } else if previous_status != CrisisStatus::Escalated
            && crisis.status == CrisisStatus::Escalated
        {
            escalated.push((crisis.id, crisis.kind));
        }
    }
    for (crisis_id, kind) in escalated {
        push_outbox(
            state,
            OutboxKind::Crisis,
            format!("Crisis {crisis_id} escalated"),
            format!(
                "The {kind:?} crisis intensified because no effective response had contained it."
            ),
        );
    }
    for (crisis_id, kind) in resolved {
        push_outbox(
            state,
            OutboxKind::Crisis,
            format!("Crisis {crisis_id} resolved"),
            format!("The {kind:?} crisis has subsided below an active threat level."),
        );
    }
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

pub(crate) fn run_annual_strategic_systems(state: &mut AppState) -> Result<(), SimulationError> {
    educate_family_members(state);
    form_dynastic_marriage(state);
    update_family_councils(state)?;
    Ok(())
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

fn update_family_councils(state: &mut AppState) -> Result<(), SimulationError> {
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

    let mut updates = Vec::new();
    for (dynasty_id, loyalty_adjustment) in loyalty_adjustments {
        let council = state
            .family_councils
            .get(&dynasty_id)
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
        let unity_basis_points = i32::from(council.unity_basis_points)
            .saturating_sub(branch_pressure)
            .saturating_add(50)
            .saturating_add(loyalty_adjustment)
            .saturating_add(governance_adjustment)
            .clamp(0, 10_000)
            .try_into()
            .expect("clamped family unity must fit u16");
        let governance_change =
            if unity_basis_points < 3_000 && council.governance == HouseGovernance::Primogeniture {
                Some((
                    council.governance,
                    HouseGovernance::FamilyPartnership,
                    next_family_charter_version(dynasty_id, council.charter_version)?,
                ))
            } else {
                None
            };
        updates.push((dynasty_id, unity_basis_points, governance_change));
    }

    let mut governance_changes = Vec::new();
    for (dynasty_id, unity_basis_points, governance_change) in updates {
        let council = state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("family council must exist");
        council.unity_basis_points = unity_basis_points;
        if let Some((prior, governance, next_charter_version)) = governance_change {
            council.governance = governance;
            council.charter_version = next_charter_version;
            governance_changes.push((dynasty_id, prior, governance));
        }
    }
    for (dynasty_id, prior, governance) in governance_changes {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::HouseGovernanceChange,
            subject: format!("dynasty:{dynasty_id}"),
            detail: format!(
                "automatic=true;from={prior:?};governance={governance:?};reason=low_unity"
            ),
        });
        push_outbox(
            state,
            OutboxKind::Family,
            format!("House {dynasty_id} charter changed under pressure"),
            format!(
                "Low family unity forced a transition from {prior:?} to {governance:?} governance."
            ),
        );
    }
    Ok(())
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
