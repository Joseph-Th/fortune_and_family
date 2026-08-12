//! Strategic initialization, periodic systems, and validated cross-record operations.

use super::SimulationError;
use super::transactions::{
    TimelineError, add_market_supply, checked_future_day, checked_next_business_finance_version,
    debit_market_clearing_account, next_business_finance_version, next_family_charter_version,
};
use crate::core::{
    AiObjective, AppState, AuditKind, AuditRecord, BusinessStatus, CharacterRole, CharacterStatus,
    ChronicleEntry, ChronicleKind, CivicDebtStatus, ContractStatus, Crisis, CrisisKind,
    CrisisStatus, DistrictRuntime, DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw,
    ExternalRoute, FamilyCouncilState, FamilyLink, FamilyLinkKind, HouseGovernance,
    InformationConfidence, InformationReport, InformationTarget, InstitutionRuntime, LawKind,
    LegalCase, LegalCaseKind, LegalCaseStatus, LegalClaimSource, Loan, LoanStatus, ObjectiveKind,
    ObjectiveStatus, OfficePower, OutboxKind, OutboxMessage, Property, PropertyKind, PublicWork,
    PublicWorkKind, PublicWorkStatus, RelationshipState, SupplyContract,
};
use crate::ids::{
    BusinessId, CharacterId, CivicDebtId, DistrictId, DynastyId, EmploymentId, GoodId, HouseholdId,
    IdentifierAllocationError, InstitutionId, PropertyId,
};
use crate::money::{
    Money, Quantity, affordable_quantity, checked_cost_for, cost_for, rounded_cost_copper_wide,
};
use crate::registry::{InstitutionKind, Registry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub(crate) const OFFICE_ADMINISTRATIVE_LOAD_PER_POWER: u16 = 10;
pub(crate) const OFFICE_DUTY_COST_PER_POWER: Money = Money::from_copper(100);
pub(crate) const OFFICE_DUTY_PORTFOLIO_SURCHARGE_PER_ADDITIONAL_OFFICE: Money =
    Money::from_copper(50);
const OFFICE_DUTY_FAILURE_NOTIFICATION_INTERVAL_DAYS: i64 = 90;
const OFFICE_DUTY_FORFEITURE_WINDOW_DAYS: i64 = 90;
const OFFICE_DUTY_REELECTION_BAN_DAYS: i64 = 180;
const OFFICE_DUTY_FORFEITURE_THRESHOLD: usize = 3;
const OFFICE_NOMINATION_CAMPAIGN_BONUS: u32 = 2_000;
const OFFICE_CONCENTRATION_BACKLASH_PER_ADDITIONAL_OFFICE: i16 = 120;
const MAX_OFFICE_CONCENTRATION_BACKLASH: i16 = 600;
pub(crate) const DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS: i64 = 180;
pub(crate) const PROPERTY_LIQUIDATION_BASIS_POINTS: i64 = 5_000;
const PROPERTY_AUCTION_DISTRESS_TREASURY_LIMIT: Money = Money::from_copper(2_000);
const UNADDRESSED_CRISIS_MONTHLY_ESCALATION_BASIS_POINTS: u16 = 240;
const ADDRESSED_CRISIS_MONTHLY_RECOVERY_BASIS_POINTS: u16 = 360;
const EPIDEMIC_ONSET_WELFARE_DIVISOR: u16 = 7;
const EPIDEMIC_DAILY_WELFARE_DIVISOR: u16 = 60;
const DISTRICT_BACKGROUND_EMPLOYMENT_BASIS_POINTS: u16 = 4_500;
const DISTRICT_FORMAL_EMPLOYMENT_BASIS_POINTS_PER_WORKER: u32 = 100;
const DISTRICT_MAX_FORMAL_EMPLOYMENT_BONUS_BASIS_POINTS: u32 = 4_500;
const PUBLIC_WORK_TOOL_SHARE_BASIS_POINTS: i64 = 2_500;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StrategicError {
    #[error(transparent)]
    IdentifierAllocation(#[from] IdentifierAllocationError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
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
    #[error("business {business_id} is not owned by dynasty {dynasty_id}")]
    BusinessNotOwnedByDynasty {
        business_id: BusinessId,
        dynasty_id: DynastyId,
    },
    #[error("dynasty {dynasty_id} does not exist")]
    MissingDynasty { dynasty_id: DynastyId },
    #[error("property {property_id} does not exist")]
    MissingProperty { property_id: PropertyId },
    #[error("contract parties must be different businesses")]
    SameContractParty,
    #[error("contract businesses must belong to different dynasties, both belong to {dynasty_id}")]
    SameContractOwner { dynasty_id: DynastyId },
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
        "business {business_id} has only {available} distributable cash after preserving reserve {required_reserve}; requested {requested}"
    )]
    BusinessDistributionExceedsSurplus {
        business_id: BusinessId,
        available: Money,
        required_reserve: Money,
        requested: Money,
    },
    #[error(
        "dynasty {dynasty_id} cannot remove administrative load {outgoing}; current load is {current}"
    )]
    DynastyAdministrativeLoadUnderflow {
        dynasty_id: DynastyId,
        current: u16,
        outgoing: u16,
    },
    #[error(
        "dynasty {dynasty_id} cannot add administrative load {incoming}; current load {current} exceeds the supported range"
    )]
    DynastyAdministrativeLoadOverflow {
        dynasty_id: DynastyId,
        current: u16,
        incoming: u16,
    },
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
const AI_BUSINESS_RECOVERY_TREASURY_RESERVE: Money = Money::from_copper(20_000);
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
    due_day: i64,
    end_day: i64,
}

#[derive(Clone, Copy, Debug)]
struct ContractPartySettlementState {
    owner_id: DynastyId,
    can_perform: bool,
}

#[derive(Clone, Copy, Debug)]
struct ContractSettlementState {
    buyer: ContractPartySettlementState,
    seller: ContractPartySettlementState,
}

impl ContractSettlementState {
    const fn is_fulfilled(self) -> bool {
        self.buyer.can_perform && self.seller.can_perform
    }

    const fn buyer_is_at_fault(self) -> bool {
        !self.buyer.can_perform
    }

    const fn seller_is_at_fault(self) -> bool {
        !self.seller.can_perform
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

    const fn breach_victim_dynasty_id(self) -> Option<DynastyId> {
        match (self.buyer_is_at_fault(), self.seller_is_at_fault()) {
            (true, false) => Some(self.seller.owner_id),
            (false, true) => Some(self.buyer.owner_id),
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
    /// Returns the current validation error if state changed after the token was created, or an
    /// allocation or timeline error if durable contract feedback can no longer be recorded.
    pub fn commit(
        self,
        registry: &Registry,
        state: &mut AppState,
    ) -> Result<crate::ids::ContractId, StrategicError> {
        validate_supply_contract_terms(registry, state, &self.terms)?;
        let mut next_state = state.clone();
        let id = commit_supply_contract(&mut next_state, &self.terms)?;
        *state = next_state;
        Ok(id)
    }
}

fn commit_supply_contract(
    state: &mut AppState,
    terms: &SupplyContractTerms,
) -> Result<crate::ids::ContractId, StrategicError> {
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
    let id = state.next_ids.try_contract()?;
    let day = state.clock.day();
    let next_due_day = checked_future_day(day, 7)?;
    let end_day = checked_future_day(day, i64::from(duration_weeks) * 7)?;
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
            next_due_day,
            end_day,
            fulfilled_deliveries: 0,
            fulfilled_deliveries_by_dynasty: BTreeMap::default(),
            missed_deliveries: 0,
            breaching_dynasty_id: None,
            breach_victim_dynasty_id: None,
            unpaid_breach_penalty: Money::ZERO,
            status: ContractStatus::Active,
        },
    );
    try_push_outbox(
        state,
        OutboxKind::Contract,
        format!("Supply contract {id} signed"),
        format!(
            "Business {seller_business_id} will deliver {quantity_per_week} of good {good_id} to business {buyer_business_id} each week."
        ),
    )?;
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
    try_record_counterparty_information(
        state,
        buyer_owner_id,
        seller_owner_id,
        "Contract negotiation and delivery records",
    )?;
    Ok(id)
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
    /// Returns the current validation error if state changed after the token was created, or an
    /// allocation or timeline error if durable loan feedback can no longer be recorded.
    pub fn commit(self, state: &mut AppState) -> Result<crate::ids::LoanId, StrategicError> {
        let defaulted_loan_id = validate_loan_terms(state, &self.terms)?;
        let mut next_state = state.clone();
        let id = commit_loan(&mut next_state, &self.terms, defaulted_loan_id)?;
        *state = next_state;
        Ok(id)
    }
}

fn commit_loan(
    state: &mut AppState,
    terms: &LoanTerms,
    defaulted_loan_id: Option<crate::ids::LoanId>,
) -> Result<crate::ids::LoanId, StrategicError> {
    let &LoanTerms {
        lender_dynasty_id,
        borrower_dynasty_id,
        principal,
        collateral_property_id,
        ..
    } = terms;
    let id = match defaulted_loan_id {
        Some(id) => id,
        None => state.next_ids.try_loan()?,
    };
    let next_due_day = checked_future_day(state.clock.day(), 7)?;
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
    commit_loan_record(state, terms, id, defaulted_loan_id, next_due_day);
    let restructured = defaulted_loan_id.is_some();
    try_push_outbox(
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
    )?;
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
    try_record_counterparty_information(
        state,
        lender_dynasty_id,
        borrower_dynasty_id,
        "Credit underwriting and repayment records",
    )?;
    Ok(id)
}

fn commit_loan_record(
    state: &mut AppState,
    terms: &LoanTerms,
    id: crate::ids::LoanId,
    defaulted_loan_id: Option<crate::ids::LoanId>,
    next_due_day: i64,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SupplyContractCapacity {
    pub(crate) buyer: Quantity,
    pub(crate) seller: Quantity,
}

pub(crate) fn available_supply_contract_capacity(
    registry: &Registry,
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    good_id: GoodId,
) -> Option<SupplyContractCapacity> {
    let buyer = state.businesses.get(buyer_business_id)?;
    let seller = state.businesses.get(seller_business_id)?;
    let buyer_recipe = registry.get_recipe(buyer.recipe_id())?;
    let seller_recipe = registry.get_recipe(seller.recipe_id())?;
    if seller_recipe.output_good_id() != good_id {
        return None;
    }
    let input_per_batch = buyer_recipe
        .inputs()
        .iter()
        .find(|input| input.good_id() == good_id)?
        .quantity();
    let seller_capacity = seller_recipe.output_quantity().saturating_mul_ratio(
        i64::from(seller.operations.capacity_batches_per_day).saturating_mul(5),
        1,
    );
    let buyer_capacity = input_per_batch.saturating_mul_ratio(
        i64::from(buyer.operations.capacity_batches_per_day).saturating_mul(5),
        1,
    );
    let committed_outgoing = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active
                && contract.seller_business_id == seller_business_id
                && contract.good_id == good_id
        })
        .fold(Quantity::ZERO, |total, contract| {
            total.saturating_add(contract.quantity_per_week)
        });
    let committed_incoming = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active
                && contract.buyer_business_id == buyer_business_id
                && contract.good_id == good_id
        })
        .fold(Quantity::ZERO, |total, contract| {
            total.saturating_add(contract.quantity_per_week)
        });
    Some(SupplyContractCapacity {
        buyer: buyer_capacity.saturating_sub(committed_incoming),
        seller: seller_capacity.saturating_sub(committed_outgoing),
    })
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
    checked_future_day(state.clock.day(), 7)?;
    checked_future_day(state.clock.day(), i64::from(terms.duration_weeks) * 7)?;
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
    if buyer.owner_dynasty_id() == seller.owner_dynasty_id() {
        return Err(StrategicError::SameContractOwner {
            dynasty_id: buyer.owner_dynasty_id(),
        });
    }
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
/// Returns the same errors as [`validate_supply_contract`], plus allocation or timeline exhaustion
/// while committing the contract and its durable feedback.
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
/// Returns an error when the property or buyer is missing, the property is owned, funds are
/// insufficient, or durable feedback identifiers are exhausted.
pub fn buy_unowned_property(
    state: &mut AppState,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
) -> Result<(), StrategicError> {
    let mut next_state = state.clone();
    commit_unowned_property_purchase(&mut next_state, buyer_dynasty_id, property_id)?;
    *state = next_state;
    Ok(())
}

fn commit_unowned_property_purchase(
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
    try_push_outbox(
        state,
        OutboxKind::Property,
        format!("Property {property_id} acquired"),
        format!("Dynasty {buyer_dynasty_id} acquired the property for {price}."),
    )?;
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
        ),
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
    if business.status() != BusinessStatus::Active {
        return Err(StrategicError::BusinessInactive { business_id });
    }
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
        ),
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

/// Sells an owned property to another dynasty at the canonical liquidation price.
///
/// Occupied premises remain occupied and become a tenancy when the buyer differs from the business
/// owner.
///
/// # Errors
///
/// Returns the same errors as [`quote_property_liquidation`], plus allocation or timeline exhaustion
/// while recording repayment information or durable sale feedback.
///
/// # Panics
///
/// Panics only if synchronized dynasty, property, loan, or business records violate internal
/// invariants after successful validation.
pub fn sell_owned_property(
    registry: &Registry,
    state: &mut AppState,
    seller_dynasty_id: DynastyId,
    buyer_dynasty_id: DynastyId,
    property_id: PropertyId,
) -> Result<PropertyLiquidationQuote, StrategicError> {
    let mut next_state = state.clone();
    let quote = commit_owned_property_sale(
        registry,
        &mut next_state,
        seller_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )?;
    *state = next_state;
    Ok(quote)
}

fn commit_owned_property_sale(
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

fn validate_acquisition_administrative_load(
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

fn commit_business_acquisition(
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
    super::synchronize_employment_for_business_status(state, business_id, BusinessStatus::Active);
    cancel_internalized_contracts(state, business_id, buyer_dynasty_id)?;

    record_business_acquisition(state, buyer_dynasty_id, manager_id, recapitalization, quote)?;
    Ok(())
}

fn cancel_internalized_contracts(
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
        ),
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

pub(crate) fn initialize_strategic_state(registry: &Registry, state: &mut AppState) {
    initialize_districts(registry, state);
    initialize_institutions(registry, state);
    initialize_properties(registry, state);
    initialize_employment(state);
    initialize_district_employment(state);
    initialize_family_governance(state);
    initialize_relationships(state);
    initialize_laws(state);
    initialize_routes(registry, state);
    initialize_contracts(registry, state);
    initialize_loans(state);
    initialize_objectives(state);
    initialize_public_works(registry, state);
    initialize_information(state);
}

fn initialize_districts(registry: &Registry, state: &mut AppState) {
    for district in registry.districts() {
        state.districts.insert(
            district.id(),
            DistrictRuntime {
                district_id: district.id(),
                rent_index_basis_points: 10_000,
                employment_basis_points: DISTRICT_BACKGROUND_EMPLOYMENT_BASIS_POINTS,
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

fn initialize_district_employment(state: &mut AppState) {
    let district_ids: Vec<_> = state.districts.keys().copied().collect();
    for district_id in district_ids {
        let employment = district_employment_basis_points(state, district_id);
        state
            .districts
            .get_mut(&district_id)
            .expect("district runtime must exist")
            .employment_basis_points = employment;
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
                active_directive: None,
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
                value: Money::from_copper(55_000),
                weekly_rent: Money::from_copper(140),
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
    recover_ai_businesses(registry, state);
    apply_external_route_supply(state)?;
    Ok(())
}

pub(crate) fn expire_time_limited_state(state: &mut AppState) {
    let day = state.clock.day();
    state
        .information_reports
        .retain(|_, report| report.expires_day >= day);
    for institution in state.institutions.values_mut() {
        if institution
            .active_directive
            .is_some_and(|directive| directive.expires_day < day)
        {
            institution.active_directive = None;
        }
    }
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

fn apply_external_route_supply(state: &mut AppState) -> Result<(), SimulationError> {
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
        add_market_supply(state, good_id, quantity)?;
    }
    Ok(())
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
                    quote.demand_today = quote.demand_today.checked_add(crisis_demand).ok_or(
                        SimulationError::MarketDemandOverflow {
                            good_id: bread_id,
                            current: quote.demand_today,
                            incoming: crisis_demand,
                        },
                    )?;
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
                apply_epidemic_household_pressure(
                    state,
                    district_id,
                    (severity / EPIDEMIC_DAILY_WELFARE_DIVISOR).max(1),
                );
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
    progress_public_works(registry, state)?;
    update_relationships_from_obligations(state);
    update_quality_reputations(state);
    apply_law_economic_effects(registry, state)?;
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
            due_day: contract.next_due_day,
            end_day: contract.end_day,
        })
        .collect();
    for due_contract in due {
        settle_due_contract(state, due_contract)?;
    }
    Ok(())
}

fn settle_due_contract(state: &mut AppState, due: DueContract) -> Result<(), SimulationError> {
    let final_delivery = is_final_contract_delivery(due);
    let payment = cost_for(due.quantity, due.unit_price);
    let (seller_active, seller_owner_id, seller_can_deliver) = {
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
        )
    };
    let (buyer_active, buyer_owner_id, buyer_can_pay) = {
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
        )?;
        return Ok(());
    }
    let settlement = ContractSettlementState {
        buyer: ContractPartySettlementState {
            owner_id: buyer_owner_id,
            can_perform: buyer_can_pay,
        },
        seller: ContractPartySettlementState {
            owner_id: seller_owner_id,
            can_perform: seller_can_deliver,
        },
    };
    let fulfilled = settlement.is_fulfilled();
    let terminates_for_misses = !fulfilled
        && state
            .contracts
            .get(&due.id)
            .expect("contract must exist")
            .missed_deliveries
            >= 2;
    let next_due_day = if final_delivery || terminates_for_misses {
        None
    } else {
        Some(checked_future_day(due.due_day, 7)?)
    };
    if fulfilled {
        let seller_cash = state
            .businesses
            .get(due.seller_id)
            .expect("contract seller must exist")
            .cash();
        seller_cash
            .checked_add(payment)
            .ok_or(SimulationError::BusinessCashOverflow {
                business_id: due.seller_id,
                current: seller_cash,
                incoming: payment,
            })?;
        let buyer_inventory = state
            .businesses
            .get(due.buyer_id)
            .expect("contract buyer must exist")
            .inventory_quantity(due.good_id);
        buyer_inventory.checked_add(due.quantity).ok_or(
            SimulationError::BusinessInventoryOverflow {
                business_id: due.buyer_id,
                good_id: due.good_id,
                current: buyer_inventory,
                incoming: due.quantity,
            },
        )?;
        settle_fulfilled_contract(state, due, payment, settlement, next_due_day)?;
    } else {
        let terminal_breach = final_delivery || terminates_for_misses;
        settle_failed_contract(state, due, settlement, next_due_day, terminal_breach)?;
    }
    finalize_expired_contract(state, due, settlement, fulfilled, final_delivery)?;
    Ok(())
}

fn is_final_contract_delivery(due: DueContract) -> bool {
    due.due_day >= due.end_day
        || due
            .end_day
            .checked_sub(due.due_day)
            .is_some_and(|remaining_days| remaining_days < 7)
}

fn terminate_inactive_contract(
    state: &mut AppState,
    contract_id: crate::ids::ContractId,
    buyer_owner_id: DynastyId,
    seller_owner_id: DynastyId,
    buyer_active: bool,
    seller_active: bool,
) -> Result<(), SimulationError> {
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
    contract.breach_victim_dynasty_id = match (buyer_active, seller_active) {
        (false, true) => Some(seller_owner_id),
        (true, false) => Some(buyer_owner_id),
        (false, false) | (true, true) => None,
    };
    contract.unpaid_breach_penalty = if contract.breach_victim_dynasty_id.is_some() {
        contract.penalty
    } else {
        Money::ZERO
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
        try_record_counterparty_information(
            state,
            buyer_owner_id,
            seller_owner_id,
            "Contract termination and business-status records",
        )?;
    }
    try_push_outbox(
        state,
        OutboxKind::Contract,
        format!("Contract {contract_id} terminated"),
        "An inactive contract party could no longer perform the scheduled obligation.".to_owned(),
    )?;
    Ok(())
}

fn finalize_expired_contract(
    state: &mut AppState,
    due: DueContract,
    settlement: ContractSettlementState,
    fulfilled: bool,
    final_delivery: bool,
) -> Result<(), SimulationError> {
    let expired_active = final_delivery
        && state
            .contracts
            .get(&due.id)
            .is_some_and(|contract| contract.status == ContractStatus::Active);
    if !expired_active {
        return Ok(());
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
    contract.breach_victim_dynasty_id = if fulfilled {
        None
    } else {
        settlement.breach_victim_dynasty_id()
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
        try_record_counterparty_information(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            "Completed contract performance records",
        )?;
    }
    if !fulfilled {
        try_push_outbox(
            state,
            OutboxKind::Contract,
            format!("Contract {} expired in breach", due.id),
            "The final scheduled delivery was not completed before the contract ended.".to_owned(),
        )?;
    }
    Ok(())
}

fn settle_fulfilled_contract(
    state: &mut AppState,
    due: DueContract,
    payment: Money,
    settlement: ContractSettlementState,
    next_due_day: Option<i64>,
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
    if let Some(next_due_day) = next_due_day {
        contract.next_due_day = next_due_day;
    }
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
    next_due_day: Option<i64>,
    terminal_breach: bool,
) -> Result<(), SimulationError> {
    let penalty_parties = match (
        settlement.seller_is_at_fault(),
        settlement.buyer_is_at_fault(),
    ) {
        (false, true) => Some((due.buyer_id, due.seller_id)),
        (true, false) => Some((due.seller_id, due.buyer_id)),
        (false, false) | (true, true) => None,
    };
    let unpaid_terminal_penalty = if let Some((payer_id, recipient_id)) = penalty_parties {
        let available = state
            .businesses
            .get(payer_id)
            .expect("contract penalty payer must exist")
            .cash();
        let transferred =
            transfer_contract_money(state, payer_id, recipient_id, due.penalty.min(available))?;
        due.penalty
            .checked_sub(transferred)
            .expect("bounded contract penalty transfer cannot exceed the contractual penalty")
    } else {
        Money::ZERO
    };
    let breached = {
        let contract = state
            .contracts
            .get_mut(&due.id)
            .expect("contract must exist");
        contract.missed_deliveries = contract.missed_deliveries.saturating_add(1);
        if let Some(next_due_day) = next_due_day {
            contract.next_due_day = next_due_day;
        }
        if contract.missed_deliveries >= 3 {
            contract.status = ContractStatus::Breached;
            contract.breaching_dynasty_id = settlement.breaching_dynasty_id();
            contract.breach_victim_dynasty_id = settlement.breach_victim_dynasty_id();
        }
        if terminal_breach && settlement.has_attributable_nonperformance() {
            contract.unpaid_breach_penalty = unpaid_terminal_penalty;
        }
        contract.status == ContractStatus::Breached
    };
    if breached {
        try_push_outbox(
            state,
            OutboxKind::Contract,
            format!("Contract {} breached", due.id),
            format!(
                "Repeated nonperformance caused supply contract {} to terminate.",
                due.id
            ),
        )?;
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
            try_record_counterparty_information(
                state,
                settlement.buyer.owner_id,
                settlement.seller.owner_id,
                "Contract breach and penalty records",
            )?;
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
    let recipient_cash = state
        .businesses
        .get(recipient_id)
        .expect("contract recipient must exist")
        .cash();
    let transferred = amount.min(payer_cash);
    if transferred <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    recipient_cash
        .checked_add(transferred)
        .ok_or(SimulationError::BusinessCashOverflow {
            business_id: recipient_id,
            current: recipient_cash,
            incoming: transferred,
        })?;
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
        .filter(|loan| loan.status.is_repayment_active() && loan.next_due_day <= day)
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
    let treasury_budget = state
        .institutions
        .get(&treasury_id)
        .expect("civic treasury must exist")
        .budget;
    if treasury_budget >= amount_due {
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
        if treasury_budget >= amount_due {
            let remaining_balance = accrued_balance
                .checked_sub(amount_due)
                .expect("civic debt payment cannot exceed accrued balance");
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
    if treasury_budget >= amount_due {
        settle_successful_civic_debt_payment(state, treasury_id, due, amount_due, next_due_day)?;
    } else {
        settle_missed_civic_debt_payment(state, treasury_id, due, next_due_day)?;
    }
    Ok(())
}

fn settle_successful_civic_debt_payment(
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

fn settle_missed_civic_debt_payment(
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
    if borrower_treasury >= amount_due {
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
        if borrower_treasury >= amount_due {
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
    if borrower_treasury >= amount_due {
        settle_successful_loan_payment(state, due, amount_due, next_due_day)?;
    } else {
        settle_missed_loan_payment(state, due, next_due_day)?;
    }
    Ok(())
}

fn settle_successful_loan_payment(
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
    if loan.status != LoanStatus::Repaid {
        loan.status = LoanStatus::Current;
    }
    adjust_reliability_reputation(state, due.borrower_id, 10);
    Ok(())
}

fn settle_missed_loan_payment(
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
        let collateral_recovery = seize_defaulted_collateral(state, due);
        let remaining_balance = state
            .loans
            .get(&due.id)
            .expect("defaulted loan must exist")
            .balance;
        try_push_outbox(
            state,
            OutboxKind::Finance,
            format!("Loan {} defaulted", due.id),
            format!(
                "Dynasty {} defaulted on its obligation to dynasty {}. Collateral recovered {}; remaining balance {}.",
                due.borrower_id, due.lender_id, collateral_recovery, remaining_balance
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

fn seize_defaulted_collateral(state: &mut AppState, due: DueLoan) -> Money {
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
        apply_defaulted_collateral_recovery(state, due.id)
    } else {
        Money::ZERO
    }
}

fn apply_defaulted_collateral_recovery(state: &mut AppState, loan_id: crate::ids::LoanId) -> Money {
    let recovery = {
        let loan = state
            .loans
            .get(&loan_id)
            .expect("defaulted loan must exist");
        if loan.status != LoanStatus::Defaulted {
            return Money::ZERO;
        }
        let Some(property_id) = loan.collateral_property_id else {
            return Money::ZERO;
        };
        state
            .properties
            .get(&property_id)
            .expect("defaulted loan collateral must exist")
            .value
            .saturating_mul_ratio(PROPERTY_LIQUIDATION_BASIS_POINTS, 10_000)
            .min(loan.balance)
    };
    if recovery <= Money::ZERO {
        return Money::ZERO;
    }
    let loan = state
        .loans
        .get_mut(&loan_id)
        .expect("defaulted loan must exist");
    loan.balance = loan
        .balance
        .checked_sub(recovery)
        .expect("collateral recovery must not exceed the defaulted balance");
    if loan.balance == Money::ZERO {
        loan.status = LoanStatus::Repaid;
        loan.missed_payments = 0;
    }
    recovery
}

pub(crate) fn rebuild_defaulted_collateral_recoveries(state: &mut AppState) {
    let loan_ids = state
        .loans
        .values()
        .filter(|loan| {
            loan.status == LoanStatus::Defaulted && loan.collateral_property_id.is_some()
        })
        .map(|loan| loan.id)
        .collect::<Vec<_>>();
    for loan_id in loan_ids {
        apply_defaulted_collateral_recovery(state, loan_id);
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

fn apply_loan_payment(
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
fn settle_property_rents(state: &mut AppState) -> Result<(), SimulationError> {
    let rents: Vec<_> = state
        .properties
        .values()
        .filter_map(|property| {
            Some((
                property.owner_dynasty_id?,
                property.tenant_dynasty_id,
                property.occupant_business_id,
                effective_property_weekly_rent(state, property),
            ))
        })
        .collect();
    for (owner_id, tenant_id, occupant_business_id, rent) in rents {
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
            let owner_treasury = state
                .dynasties
                .get(&owner_id)
                .expect("property owner dynasty must exist")
                .treasury();
            owner_treasury
                .checked_add(rent)
                .ok_or(SimulationError::DynastyTreasuryOverflow {
                    dynasty_id: owner_id,
                    current: owner_treasury,
                    incoming: rent,
                })?;
            debit_market_clearing_account(state, rent)?;
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
        owner.resources.treasury = owner
            .resources
            .treasury
            .checked_add(paid)
            .expect("bounded rent must fit owner treasury");
    }
    Ok(())
}

pub(crate) fn effective_property_weekly_rent(state: &AppState, property: &Property) -> Money {
    let indexed_rent =
        if property.tenant_dynasty_id.is_none() && property.occupant_business_id.is_none() {
            let rent_index = state
                .districts
                .get(&property.district_id)
                .expect("property district runtime must exist")
                .rent_index_basis_points;
            property
                .weekly_rent
                .saturating_mul_ratio(i64::from(rent_index), 10_000)
        } else {
            property.weekly_rent
        };
    active_law_value(state, LawKind::RentRestriction).map_or(indexed_rent, |limit| {
        let annual_cap = property
            .value
            .saturating_mul_ratio(limit.clamp(0, 10_000), 10_000);
        indexed_rent.min(Money::from_copper(annual_cap.copper() / 52))
    })
}

fn distribute_business_dividends(
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
            detail: format!("dividends={total_copper}"),
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
    emit_employment_outcome(state, business_id, recovered, became_disputed)?;
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
    let household_cash = state
        .households
        .get(household_id)
        .expect("employment household must exist")
        .cash;
    let paid = wage_due.min(spendable);
    if paid <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    household_cash
        .checked_add(paid)
        .ok_or(SimulationError::HouseholdCashOverflow {
            household_id,
            current: household_cash,
            incoming: paid,
        })?;
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
        let strain = labor_strain_basis_points(agreement, environment);
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

fn labor_strain_basis_points(
    agreement: &EmploymentAgreement,
    environment: LaborEnvironment,
) -> u16 {
    if environment.utilization < 9_000 {
        return 0;
    }
    let maintenance_strain = 1_000_u16
        .saturating_sub(environment.maintenance)
        .saturating_div(5);
    let condition_strain = 7_000_u16
        .saturating_sub(environment.business_condition)
        .saturating_div(20);
    let raw_strain = maintenance_strain.saturating_add(condition_strain).min(180);

    // A workforce with accumulated loyalty and decent conditions can absorb ordinary periods of
    // high utilization without turning every growth policy into a predictable dispute timer.
    // Extreme under-maintenance still erodes that buffer and eventually creates resistance, while
    // missed payroll continues to bypass this path and directly damages the relationship.
    let social_resilience = 68_u16.saturating_add(
        agreement
            .loyalty_basis_points
            .min(agreement.conditions_basis_points)
            .saturating_div(200),
    );
    raw_strain.saturating_sub(social_resilience)
}

fn emit_employment_outcome(
    state: &mut AppState,
    business_id: BusinessId,
    recovered: bool,
    became_disputed: bool,
) -> Result<(), SimulationError> {
    if recovered {
        try_push_outbox(
            state,
            OutboxKind::District,
            format!("Labor dispute at business {business_id} settled"),
            "Sustained full wage payments restored a workable labor agreement.".to_owned(),
        )?;
    }
    if became_disputed {
        try_push_outbox(
            state,
            OutboxKind::District,
            format!("Labor dispute at business {business_id}"),
            "Accumulated wage, workload, or workplace-condition pressure caused organized resistance."
                .to_owned(),
        )?;
    }
    Ok(())
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

const fn public_work_employment_bonus_basis_points(kind: PublicWorkKind) -> u16 {
    match kind {
        PublicWorkKind::Market => 800,
        PublicWorkKind::Road | PublicWorkKind::Bridge => 500,
        PublicWorkKind::Granary | PublicWorkKind::School => 300,
        PublicWorkKind::Drainage | PublicWorkKind::WatchStation | PublicWorkKind::Hospital => 0,
    }
}

fn completed_public_work_employment_bonus_basis_points(
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

fn progress_public_works(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
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
struct PublicWorkToolPurchase {
    tools_id: GoodId,
    market_stock_after: Quantity,
    market_demand_after: Quantity,
    clearing_after: Money,
}

fn plan_public_work_tool_purchase(
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
    let market_demand_after =
        quote
            .demand_today
            .checked_add(quantity)
            .ok_or(SimulationError::MarketDemandOverflow {
                good_id: tools_id,
                current: quote.demand_today,
                incoming: quantity,
            })?;
    let clearing_after = state.market.clearing_account.checked_add(cost).ok_or(
        SimulationError::MarketClearingAccountOverflow {
            current: state.market.clearing_account,
            change: cost,
        },
    )?;
    Ok(Some(PublicWorkToolPurchase {
        tools_id,
        market_stock_after,
        market_demand_after,
        clearing_after,
    }))
}

fn apply_public_work_tool_purchase(state: &mut AppState, purchase: PublicWorkToolPurchase) {
    let quote = state
        .market
        .quotes
        .get_mut(&purchase.tools_id)
        .expect("planned public-work tools quote must exist");
    quote.stock = purchase.market_stock_after;
    quote.demand_today = purchase.market_demand_after;
    state.market.clearing_account = purchase.clearing_after;
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
        let mut lifetime_revenue_copper = 0_i128;
        let mut lifetime_costs_copper = 0_i128;
        for business in state.businesses.iter().filter(|business| {
            business.owner_dynasty_id() == dynasty_id
                && business.status() != crate::core::BusinessStatus::Closed
        }) {
            total_quality =
                total_quality.saturating_add(u64::from(business.operations.quality_basis_points));
            business_count = business_count.saturating_add(1);
            lifetime_revenue_copper += i128::from(business.finance.lifetime_revenue.copper());
            lifetime_costs_copper += i128::from(business.finance.lifetime_costs.copper());
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
            lifetime_revenue_copper,
            lifetime_costs_copper,
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
    lifetime_revenue_copper: i128,
    lifetime_costs_copper: i128,
) -> u16 {
    if current >= target {
        return 50;
    }
    let has_trade_history = lifetime_revenue_copper > 0 || lifetime_costs_copper > 0;
    if has_trade_history && lifetime_revenue_copper >= lifetime_costs_copper {
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

pub(crate) const MAX_RELATIONSHIP_MEMORIES: usize = 12;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(crate) enum DurableFeedbackError {
    #[error(transparent)]
    IdentifierAllocation(#[from] IdentifierAllocationError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
}

impl From<DurableFeedbackError> for StrategicError {
    fn from(error: DurableFeedbackError) -> Self {
        match error {
            DurableFeedbackError::IdentifierAllocation(error) => Self::IdentifierAllocation(error),
            DurableFeedbackError::Timeline(error) => Self::Timeline(error),
        }
    }
}

impl From<DurableFeedbackError> for SimulationError {
    fn from(error: DurableFeedbackError) -> Self {
        match error {
            DurableFeedbackError::IdentifierAllocation(error) => Self::IdentifierAllocation(error),
            DurableFeedbackError::Timeline(error) => Self::Timeline(error),
        }
    }
}

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

pub(crate) fn try_record_counterparty_information(
    state: &mut AppState,
    first_dynasty_id: DynastyId,
    second_dynasty_id: DynastyId,
    source: &str,
) -> Result<(), DurableFeedbackError> {
    let player_dynasty_id = state.player_dynasty_id;
    let counterparty_id =
        if first_dynasty_id == player_dynasty_id && second_dynasty_id != player_dynasty_id {
            second_dynasty_id
        } else if second_dynasty_id == player_dynasty_id && first_dynasty_id != player_dynasty_id {
            first_dynasty_id
        } else {
            return Ok(());
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
    let day = state.clock.day();
    let expires_day = checked_future_day(day, 180)?;
    let id = state.next_ids.try_information_report()?;
    state.information_reports.retain(|_, report| {
        report.owner_dynasty_id != player_dynasty_id || report.target != Some(target)
    });
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: player_dynasty_id,
            target: Some(target),
            subject,
            confidence: InformationConfidence::Probable,
            created_day: day,
            expires_day,
            source: source.to_owned(),
            summary,
        },
    );
    Ok(())
}

fn adjust_basis_points(current: u16, delta: i16) -> u16 {
    u16::try_from(
        i32::from(current)
            .saturating_add(i32::from(delta))
            .clamp(0, 10_000),
    )
    .expect("clamped basis-point value must fit u16")
}

fn apply_law_economic_effects(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let emergency_imports = active_law_value(state, LawKind::EmergencyImports)
        .map_or(Quantity::ZERO, |value| Quantity::from_units(value.max(0)));
    if emergency_imports > Quantity::ZERO
        && let Some(grain_id) = registry.get_good_id("grain")
    {
        add_market_supply(state, grain_id, emergency_imports)?;
    }
    Ok(())
}

pub(crate) fn run_monthly_strategic_systems(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    update_district_conditions(state);
    resolve_institution_selections(registry, state)?;
    apply_office_duties(state)?;
    apply_office_power_effects(registry, state)?;
    apply_active_office_directives(registry, state)?;
    advance_ai_objectives(registry, state)?;
    update_information_reports(registry, state)?;
    file_grounded_ai_legal_cases(state)?;
    advance_legal_case_hearings(state)?;
    resolve_legal_cases(state)?;
    update_external_route_risk(state);
    detect_and_advance_crises(registry, state)?;
    recover_external_routes(state);
    Ok(())
}

fn recover_ai_businesses(registry: &Registry, state: &mut AppState) {
    let mut business_ids: Vec<_> = state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() != state.player_dynasty_id)
        .filter(|business| {
            matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            )
        })
        .map(crate::core::Business::id)
        .collect();
    business_ids.sort_unstable();

    for business_id in business_ids {
        let business = state
            .businesses
            .get(business_id)
            .expect("indexed AI business must exist");
        if checked_next_business_finance_version(business).is_none() {
            continue;
        }
        let owner_dynasty_id = business.owner_dynasty_id();
        let target_cash = business_recapitalization_target(registry, state, business);
        let shortfall = Money::from_copper(
            target_cash
                .copper()
                .saturating_sub(business.cash().copper())
                .max(0),
        );
        if shortfall == Money::ZERO {
            continue;
        }
        let treasury = state
            .dynasties
            .get(&owner_dynasty_id)
            .expect("AI business owner dynasty must exist")
            .treasury();
        let available = Money::from_copper(
            treasury
                .copper()
                .saturating_sub(AI_BUSINESS_RECOVERY_TREASURY_RESERVE.copper())
                .max(0),
        );
        let amount = shortfall.min(available);
        if amount == Money::ZERO {
            continue;
        }
        capitalize_owned_business(state, owner_dynasty_id, business_id, amount)
            .expect("prevalidated AI business capitalization must commit");
    }
}

fn apply_active_office_directives(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let directives: Vec<_> = state
        .institutions
        .values_mut()
        .filter_map(|institution| {
            let directive = institution.active_directive?;
            if day > directive.expires_day {
                institution.active_directive = None;
                return None;
            }
            Some((institution.institution_id, directive.power))
        })
        .collect();
    for (institution_id, power) in directives {
        let district_id = registry
            .get_institution(institution_id)
            .expect("active office directive institution must remain registered")
            .district_id();
        apply_office_directive_momentum(registry, state, institution_id, district_id, power)?;
    }
    Ok(())
}

fn apply_office_directive_momentum(
    registry: &Registry,
    state: &mut AppState,
    institution_id: InstitutionId,
    district_id: DistrictId,
    power: OfficePower,
) -> Result<(), SimulationError> {
    match power {
        OfficePower::Licenses => adjust_directive_businesses(state, district_id, 10, 10),
        OfficePower::Inspections => adjust_directive_businesses(state, district_id, 15, 25),
        OfficePower::MarketTolls => adjust_directive_household_welfare(state, district_id, -15),
        OfficePower::DebtEnforcement => {
            for (pair, relationship) in &mut state.relationships {
                if pair.first == state.player_dynasty_id || pair.second == state.player_dynasty_id {
                    relationship.respect_basis_points = relationship
                        .respect_basis_points
                        .saturating_add(10)
                        .min(10_000);
                    relationship.fear_basis_points =
                        relationship.fear_basis_points.saturating_add(5).min(10_000);
                }
            }
        }
        OfficePower::CityContracts => adjust_directive_businesses(state, district_id, 20, 10),
        OfficePower::PublicWorks => adjust_directive_businesses(state, district_id, 20, 5),
        OfficePower::WatchPriorities => {
            adjust_directive_household_welfare(state, district_id, 10);
            for crisis in state.crises.values_mut().filter(|crisis| {
                crisis.district_id == Some(district_id) && crisis.status.is_active()
            }) {
                crisis.severity_basis_points = crisis.severity_basis_points.saturating_sub(60);
            }
        }
        OfficePower::Taxation => adjust_directive_household_welfare(state, district_id, -20),
        OfficePower::EmergencyImports => {
            adjust_directive_household_welfare(state, district_id, 50);
            if let Some(grain_id) = registry.get_good_id("grain") {
                add_market_supply(state, grain_id, Quantity::from_units(5))?;
            }
        }
    }
    if matches!(power, OfficePower::MarketTolls | OfficePower::Taxation)
        && let Some(institution) = state.institutions.get_mut(&institution_id)
    {
        institution.legitimacy_basis_points = institution
            .legitimacy_basis_points
            .saturating_add(10)
            .min(10_000);
    }
    Ok(())
}

fn adjust_directive_businesses(
    state: &mut AppState,
    district_id: DistrictId,
    condition: u16,
    quality: u16,
) {
    for business in state.businesses.iter_mut().filter(|business| {
        business.district_id() == district_id
            && matches!(
                business.status(),
                BusinessStatus::Active | BusinessStatus::Distressed
            )
    }) {
        business.operations.condition_basis_points = business
            .operations
            .condition_basis_points
            .saturating_add(condition)
            .min(10_000);
        business.operations.quality_basis_points = business
            .operations
            .quality_basis_points
            .saturating_add(quality)
            .min(10_000);
    }
}

fn adjust_directive_household_welfare(state: &mut AppState, district_id: DistrictId, delta: i16) {
    for household in state
        .households
        .iter_mut()
        .filter(|household| household.district_id() == district_id)
    {
        household.food_satisfaction_basis_points = if delta >= 0 {
            household
                .food_satisfaction_basis_points
                .saturating_add(delta.unsigned_abs())
                .min(10_000)
        } else {
            household
                .food_satisfaction_basis_points
                .saturating_sub(delta.unsigned_abs())
        };
    }
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

fn office_duty_required(power_count: usize, office_count: usize) -> Money {
    if power_count == 0 || office_count == 0 {
        return Money::ZERO;
    }
    let power_count = i64::try_from(power_count).unwrap_or(i64::MAX);
    let additional_offices = i64::try_from(office_count.saturating_sub(1)).unwrap_or(i64::MAX);
    OFFICE_DUTY_COST_PER_POWER
        .saturating_mul(power_count)
        .saturating_add(
            OFFICE_DUTY_PORTFOLIO_SURCHARGE_PER_ADDITIONAL_OFFICE
                .saturating_mul(additional_offices),
        )
}

pub(crate) fn projected_dynasty_monthly_office_duty(
    state: &AppState,
    dynasty_id: DynastyId,
    additional_office_power_count: usize,
) -> Money {
    let additional_offices = (additional_office_power_count > 0)
        .then_some(additional_office_power_count)
        .into_iter()
        .collect::<Vec<_>>();
    projected_dynasty_monthly_office_duty_with_additional_offices(
        state,
        dynasty_id,
        &additional_offices,
    )
}

pub(crate) fn projected_dynasty_monthly_office_duty_with_additional_offices(
    state: &AppState,
    dynasty_id: DynastyId,
    additional_office_power_counts: &[usize],
) -> Money {
    let held_power_counts: Vec<_> = state
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
        .map(|institution| institution.powers.len())
        .collect();
    let office_count = held_power_counts
        .len()
        .saturating_add(additional_office_power_counts.len());
    held_power_counts
        .into_iter()
        .chain(additional_office_power_counts.iter().copied())
        .fold(Money::ZERO, |total, power_count| {
            total.saturating_add(office_duty_required(power_count, office_count))
        })
}

#[derive(Clone, Copy)]
struct OfficeDutyPlan {
    institution_id: InstitutionId,
    dynasty_id: DynastyId,
    power_count: usize,
    office_count: usize,
}

fn apply_office_duties(state: &mut AppState) -> Result<(), SimulationError> {
    let office_counts = state
        .institutions
        .values()
        .filter_map(|institution| {
            let holder_id = institution.office_holder_id?;
            state
                .characters
                .get(holder_id)
                .map(crate::core::Character::dynasty_id)
        })
        .fold(
            BTreeMap::<DynastyId, usize>::new(),
            |mut counts, dynasty_id| {
                *counts.entry(dynasty_id).or_default() += 1;
                counts
            },
        );
    let duties: Vec<_> = state
        .institutions
        .values()
        .filter_map(|institution| {
            let holder_id = institution.office_holder_id?;
            let dynasty_id = state.characters.get(holder_id)?.dynasty_id();
            let office_count = office_counts.get(&dynasty_id).copied().unwrap_or(1);
            Some(OfficeDutyPlan {
                institution_id: institution.institution_id,
                dynasty_id,
                power_count: institution.powers.len(),
                office_count,
            })
        })
        .collect();
    preflight_office_duty_contributions(state, &duties)?;
    for duty in duties {
        apply_office_duty(
            state,
            duty.institution_id,
            duty.dynasty_id,
            duty.power_count,
            duty.office_count,
        )?;
    }
    Ok(())
}

fn preflight_office_duty_contributions(
    state: &AppState,
    duties: &[OfficeDutyPlan],
) -> Result<(), SimulationError> {
    let mut projected_treasuries = BTreeMap::new();
    let mut projected_institution_budgets = BTreeMap::new();
    let mut projected_contributions = BTreeMap::new();
    for duty in duties {
        let required = office_duty_required(duty.power_count, duty.office_count);
        let treasury = projected_treasuries
            .entry(duty.dynasty_id)
            .or_insert_with(|| {
                state
                    .dynasties
                    .get(&duty.dynasty_id)
                    .expect("officeholder dynasty must exist")
                    .treasury()
            });
        let paid = required.min(*treasury);
        *treasury = treasury
            .checked_sub(paid)
            .expect("projected office-duty payment must not exceed treasury");
        if paid == Money::ZERO {
            continue;
        }
        let institution_budget = projected_institution_budgets
            .entry(duty.institution_id)
            .or_insert_with(|| {
                state
                    .institutions
                    .get(&duty.institution_id)
                    .expect("office institution must exist")
                    .budget
            });
        *institution_budget = institution_budget.checked_add(paid).ok_or(
            SimulationError::InstitutionBudgetOverflow {
                institution_id: duty.institution_id,
                current: *institution_budget,
                incoming: paid,
            },
        )?;
        let contributions = projected_contributions
            .entry(duty.dynasty_id)
            .or_insert_with(|| {
                state
                    .dynasties
                    .get(&duty.dynasty_id)
                    .expect("officeholder dynasty must exist")
                    .resources
                    .civic_contributions
            });
        *contributions = contributions.checked_add(paid).ok_or(
            SimulationError::DynastyCivicContributionsOverflow {
                dynasty_id: duty.dynasty_id,
                current: *contributions,
                incoming: paid,
            },
        )?;
    }
    Ok(())
}

fn apply_office_duty(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    power_count: usize,
    office_count: usize,
) -> Result<(), SimulationError> {
    let required = office_duty_required(power_count, office_count);
    let institution_budget = state
        .institutions
        .get(&institution_id)
        .expect("office institution must exist")
        .budget;
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .expect("officeholder dynasty must exist")
        .treasury();
    let paid = required.min(treasury);
    transfer_office_duty_payment(
        state,
        institution_id,
        dynasty_id,
        institution_budget,
        treasury,
        paid,
    )?;
    if paid < required {
        record_office_duty_shortfall(
            state,
            institution_id,
            dynasty_id,
            required,
            paid,
            required.saturating_sub(paid),
        )?;
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
    let next_institution_budget =
        institution_budget
            .checked_add(paid)
            .ok_or(SimulationError::InstitutionBudgetOverflow {
                institution_id,
                current: institution_budget,
                incoming: paid,
            })?;
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
        .budget = next_institution_budget;
    Ok(())
}

fn record_office_duty_shortfall(
    state: &mut AppState,
    institution_id: crate::ids::InstitutionId,
    dynasty_id: DynastyId,
    required: Money,
    paid: Money,
    shortfall: Money,
) -> Result<(), SimulationError> {
    let subject = office_duty_subject(institution_id, dynasty_id);
    let recent_shortfalls = recent_office_duty_shortfalls(state, &subject);
    let should_notify = should_notify_office_duty_shortfall(state, &subject);
    penalize_office_duty_shortfall(state, institution_id, dynasty_id);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::OfficeDutyShortfall,
        subject: subject.clone().into(),
        detail: format!("required={required};paid={paid};shortfall={shortfall}"),
    });
    let forfeited = recent_shortfalls.saturating_add(1) >= OFFICE_DUTY_FORFEITURE_THRESHOLD;
    if forfeited {
        forfeit_office_for_unmet_duties(
            state,
            institution_id,
            &subject,
            recent_shortfalls.saturating_add(1),
        )?;
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
    )?;
    Ok(())
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
            checked_future_day(record.day(), OFFICE_DUTY_FAILURE_NOTIFICATION_INTERVAL_DAYS)
                .is_ok_and(|next_notification_day| state.clock.day() >= next_notification_day)
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
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let next_selection_day = checked_future_day(day, 30)?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("office institution must exist");
    institution.office_holder_id = None;
    institution.next_selection_day = next_selection_day;
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::OfficeDutyForfeiture,
        subject: subject.into(),
        detail: format!("office forfeited after {recent_shortfalls} recent duty shortfalls"),
    });
    Ok(())
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

fn notify_player_office_duty_outcome(
    state: &mut AppState,
    outcome: OfficeDutyOutcome,
) -> Result<(), SimulationError> {
    if outcome.dynasty_id != state.player_dynasty_id {
        return Ok(());
    }
    if outcome.forfeited {
        try_push_outbox(
            state,
            OutboxKind::Politics,
            format!("Office forfeited at institution {}", outcome.institution_id),
            "Repeatedly unmet civic duties forced the dynasty to surrender the office. The institution will select a replacement next month, and the dynasty cannot immediately return to the same office."
                .to_owned(),
        )?;
    } else if outcome.should_notify {
        try_push_outbox(
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
        )?;
    }
    Ok(())
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
            let revenue = Money::from_copper(100);
            let next_budget = institution_budget.checked_add(revenue).ok_or(
                SimulationError::InstitutionBudgetOverflow {
                    institution_id,
                    current: institution_budget,
                    incoming: revenue,
                },
            )?;
            debit_market_clearing_account(state, revenue)?;
            state
                .institutions
                .get_mut(&institution_id)
                .expect("office institution must exist")
                .budget = next_budget;
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
                let quantity = Quantity::from_units(20);
                add_market_supply(state, grain_id, quantity)?;
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
    let award = Money::from_copper(250).min(institution_budget);
    if award == Money::ZERO {
        return Ok(());
    }
    let (resulting_cash, resulting_lifetime_revenue, next_finance_version) = {
        let business = state
            .businesses
            .get(business_id)
            .expect("city contract business must exist");
        (
            business
                .cash()
                .checked_add(award)
                .ok_or(SimulationError::BusinessCashOverflow {
                    business_id,
                    current: business.cash(),
                    incoming: award,
                })?,
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
    business.finance.cash = resulting_cash;
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
        let employment = district_employment_basis_points(state, district_id);
        let district = state
            .districts
            .get_mut(&district_id)
            .expect("district runtime must exist");
        district.employment_basis_points = employment;
        district.unrest_basis_points = district_unrest_next_basis_points(district, satisfaction);
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

fn district_employment_basis_points(state: &AppState, district_id: DistrictId) -> u16 {
    let active_jobs = super::saturating_worker_count(
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
    let formal_employment_bonus = active_jobs
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

fn district_unrest_next_basis_points(district: &DistrictRuntime, food_satisfaction: u16) -> u16 {
    let food_pressure = 10_000_u16.saturating_sub(food_satisfaction);
    let safety_pressure = 10_000_u16.saturating_sub(district.safety_basis_points) / 3;
    let employment_pressure = 6_000_u16.saturating_sub(district.employment_basis_points);
    let sanitation_pressure = 7_000_u16.saturating_sub(district.sanitation_basis_points) / 2;
    let rent_pressure = district.rent_index_basis_points.saturating_sub(11_000) / 2;
    let pressure = u32::from(food_pressure)
        .saturating_add(u32::from(safety_pressure))
        .saturating_add(u32::from(employment_pressure))
        .saturating_add(u32::from(sanitation_pressure))
        .saturating_add(u32::from(rent_pressure));
    u16::try_from(
        (u32::from(district.unrest_basis_points)
            .saturating_mul(3)
            .saturating_add(pressure)
            / 5)
        .min(10_000),
    )
    .expect("bounded district unrest must fit u16")
}

fn resolve_institution_selections(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let due = due_institution_selections(state, day);
    if due.is_empty() {
        return Ok(());
    }
    let next_selection_day = checked_future_day(day, super::OFFICE_TERM_DAYS)?;
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
                        OFFICE_NOMINATION_CAMPAIGN_BONUS
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
            .filter(|next| *next < u32::MAX)
            .ok_or(SimulationError::InstitutionTermNumberExhausted { institution_id })?;
        if let Some(winner) = winner {
            planned_office_holders.insert(winner);
        }
        selections.push((institution_id, winner, term_number));
    }

    for (institution_id, winner, term_number) in &selections {
        let institution = state
            .institutions
            .get_mut(institution_id)
            .expect("institution runtime must exist");
        institution.office_holder_id = *winner;
        institution.term_started_day = day;
        institution.next_selection_day = next_selection_day;
        institution.term_number = *term_number;
    }
    for (institution_id, winner, term_number) in selections {
        if let Some(winner) = winner {
            apply_office_concentration_backlash(state, institution_id, winner);
            try_push_outbox(
                state,
                OutboxKind::Politics,
                format!("Institution {institution_id} selected a new officeholder"),
                format!("Character {winner} now holds the office for term {term_number}."),
            )?;
        }
    }
    Ok(())
}

fn due_institution_selections(state: &AppState, day: i64) -> Vec<InstitutionId> {
    state
        .institutions
        .values()
        .filter(|institution| institution.next_selection_day <= day)
        .map(|institution| institution.institution_id)
        .collect()
}

fn apply_office_concentration_backlash(
    state: &mut AppState,
    institution_id: InstitutionId,
    winner_id: CharacterId,
) {
    let winner_dynasty_id = state
        .characters
        .get(winner_id)
        .expect("selected officeholder must exist")
        .dynasty_id();
    let office_count = state
        .institutions
        .values()
        .filter(|institution| {
            institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == winner_dynasty_id)
            })
        })
        .count();
    let additional_offices = office_count.saturating_sub(1);
    if additional_offices == 0 {
        return;
    }
    let backlash = i16::try_from(additional_offices)
        .unwrap_or(i16::MAX)
        .saturating_mul(OFFICE_CONCENTRATION_BACKLASH_PER_ADDITIONAL_OFFICE)
        .min(MAX_OFFICE_CONCENTRATION_BACKLASH);
    let member_dynasties: BTreeSet<_> = state
        .institutions
        .get(&institution_id)
        .expect("selected institution must exist")
        .members
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .map(crate::core::Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != winner_dynasty_id)
        .collect();
    for member_dynasty_id in member_dynasties {
        adjust_dynasty_relationship(
            state,
            winner_dynasty_id,
            member_dynasty_id,
            RelationshipDelta::new(-(backlash / 2), 30, backlash / 3, backlash, 0),
        );
        remember_dynasty_interaction(
            state,
            winner_dynasty_id,
            member_dynasty_id,
            &format!(
                "house {winner_dynasty_id} consolidated {office_count} offices after winning institution {institution_id}, increasing coalition resistance"
            ),
        );
    }
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

fn ai_strategic_attempt<T>(result: &Result<T, StrategicError>) -> Result<bool, SimulationError> {
    match result {
        Ok(_) => Ok(true),
        Err(StrategicError::IdentifierAllocation(error)) => Err((*error).into()),
        Err(_) => Ok(false),
    }
}

fn advance_ai_objectives(registry: &Registry, state: &mut AppState) -> Result<(), SimulationError> {
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
            ObjectiveKind::AcquireProperty => advance_ai_property_objective(state, dynasty_id)?,
            ObjectiveKind::WinOffice => advance_ai_office_objective(state, dynasty_id),
            ObjectiveKind::SecureSupply => {
                advance_ai_supply_objective(registry, state, dynasty_id)?
            }
            ObjectiveKind::ReduceDebt => advance_ai_debt_objective(state, dynasty_id)?,
            ObjectiveKind::ImproveLegitimacy => advance_ai_legitimacy_objective(state, dynasty_id),
            ObjectiveKind::AccumulateCash => ObjectiveProgress::from_achieved(
                ai_net_liquid_position(state, dynasty_id) > i128::from(120_000),
            ),
            ObjectiveKind::ContainRival => advance_ai_rival_objective(state, dynasty_id)?,
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
            let new_id = state.next_ids.try_objective()?;
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
    Ok(())
}

fn ai_net_liquid_position(state: &AppState, dynasty_id: DynastyId) -> i128 {
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .map_or(0_i128, |dynasty| i128::from(dynasty.treasury().copper()));
    let outstanding_debt = state
        .loans
        .values()
        .filter(|loan| loan.borrower_dynasty_id == dynasty_id && loan.status != LoanStatus::Repaid)
        .map(|loan| i128::from(loan.balance.copper()))
        .sum::<i128>();
    treasury - outstanding_debt
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

fn advance_ai_property_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    let property_id = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id.is_none())
        .min_by_key(|property| (property.value, property.id))
        .map(|property| property.id);
    let achieved = match property_id {
        Some(property_id) => {
            ai_strategic_attempt(&buy_unowned_property(state, dynasty_id, property_id))?
        }
        None => false,
    };
    Ok(ObjectiveProgress::from_achieved(achieved))
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
) -> Result<ObjectiveProgress, SimulationError> {
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
                return Ok(ObjectiveProgress::Achieved);
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
            if ai_strategic_attempt(&sign_supply_contract(registry, state, terms))? {
                return Ok(ObjectiveProgress::Achieved);
            }
        }
    }
    Ok(ObjectiveProgress::Pending)
}

fn advance_ai_debt_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    let loan_id = state
        .loans
        .values()
        .find(|loan| loan.borrower_dynasty_id == dynasty_id && loan.status != LoanStatus::Repaid)
        .map(|loan| loan.id);
    let Some(loan_id) = loan_id else {
        return Ok(ObjectiveProgress::Achieved);
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
    apply_loan_payment(state, loan_id, extra)?;
    Ok(ObjectiveProgress::from_achieved(
        state
            .loans
            .get(&loan_id)
            .is_some_and(|loan| loan.status == LoanStatus::Repaid),
    ))
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

fn advance_ai_rival_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    if dynasty_id == state.player_dynasty_id {
        return Ok(ObjectiveProgress::Achieved);
    }
    let pair = DynastyPair::new(dynasty_id, state.player_dynasty_id);
    let Some(relationship) = state.relationships.get_mut(&pair) else {
        return Ok(ObjectiveProgress::Achieved);
    };
    relationship.trust_basis_points = relationship.trust_basis_points.saturating_sub(75);
    relationship.fear_basis_points = relationship
        .fear_basis_points
        .saturating_add(100)
        .min(10_000);
    relationship.resentment_basis_points = relationship
        .resentment_basis_points
        .saturating_add(100)
        .min(10_000);
    let achieved = relationship.fear_basis_points >= 5_000;
    if achieved {
        let rival_name = state.dynasties.get(&dynasty_id).map_or_else(
            || dynasty_id.to_string(),
            |dynasty| dynasty.name().to_owned(),
        );
        remember_dynasty_interaction(
            state,
            dynasty_id,
            state.player_dynasty_id,
            &format!(
                "House {rival_name} completed a containment campaign that hardened bilateral commercial relations."
            ),
        );
        try_record_counterparty_information(
            state,
            dynasty_id,
            state.player_dynasty_id,
            "Rival patronage, guild correspondence, and commercial refusals",
        )?;
        try_push_outbox(
            state,
            OutboxKind::Information,
            format!("House {rival_name} is containing the dynasty"),
            "A sustained rival campaign has reduced trust and increased resentment. Future contracts with that house may require a premium or discount until relations improve."
                .to_owned(),
        )?;
    }
    Ok(ObjectiveProgress::from_achieved(achieved))
}

fn update_information_reports(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
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
        state
            .information_reports
            .retain(|_, report| report.expires_day >= day);
        return Ok(());
    };
    let expires_day = checked_future_day(day, 120)?;
    let id = state.next_ids.try_information_report()?;
    state
        .information_reports
        .retain(|_, report| report.expires_day >= day);
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: state.player_dynasty_id,
            target: Some(InformationTarget::Market { good_id }),
            subject: format!("Monthly market report: {name}"),
            confidence: InformationConfidence::Confirmed,
            created_day: day,
            expires_day,
            source: "House ledgers, guild correspondence, and market inspection".to_owned(),
            summary: format!("{name} is priced at {price}; identified causes: {causes:?}."),
        },
    );
    Ok(())
}

fn file_grounded_ai_legal_cases(state: &mut AppState) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let player_id = state.player_dynasty_id;
    let plaintiff_ids: Vec<_> = state
        .dynasties
        .keys()
        .copied()
        .filter(|dynasty_id| *dynasty_id != player_id)
        .collect();
    for plaintiff_id in plaintiff_ids {
        let can_fund_filing = state
            .dynasties
            .get(&plaintiff_id)
            .is_some_and(|dynasty| dynasty.treasury() >= super::LEGAL_CASE_FILING_COST);
        if !can_fund_filing || !legal_filing_interval_available(state, plaintiff_id, day) {
            continue;
        }
        let Some(claim) = next_grounded_ai_legal_claim(state, plaintiff_id) else {
            continue;
        };
        let hearing_day = checked_future_day(day, 60)?;
        let legal_case_id = state.next_ids.try_legal_case()?;
        let plaintiff_treasury = state
            .dynasties
            .get(&plaintiff_id)
            .expect("legal plaintiff dynasty must exist")
            .treasury();
        state
            .dynasties
            .get_mut(&plaintiff_id)
            .expect("legal plaintiff dynasty must exist")
            .resources
            .treasury = plaintiff_treasury
            .checked_sub(super::LEGAL_CASE_FILING_COST)
            .expect("prevalidated legal filing cost must fit plaintiff treasury");
        state.legal_cases.insert(
            legal_case_id,
            LegalCase {
                id: legal_case_id,
                plaintiff_dynasty_id: plaintiff_id,
                defendant_dynasty_id: claim.defendant_dynasty_id,
                kind: claim.kind,
                claim_source: Some(claim.claim_source),
                evidence_basis_points: claim.evidence_basis_points,
                public_attention_basis_points: 1_500,
                filed_day: day,
                hearing_day,
                damages: claim.maximum_damages,
                status: LegalCaseStatus::Filed,
            },
        );
        adjust_dynasty_relationship(
            state,
            plaintiff_id,
            claim.defendant_dynasty_id,
            RelationshipDelta::new(-100, -30, 0, 150, 0),
        );
        remember_dynasty_interaction(
            state,
            plaintiff_id,
            claim.defendant_dynasty_id,
            &format!(
                "House {} filed a {:?} case against house {} over {}.",
                plaintiff_id, claim.kind, claim.defendant_dynasty_id, claim.description
            ),
        );
        if claim.defendant_dynasty_id == player_id {
            try_push_outbox(
                state,
                OutboxKind::Legal,
                format!("Legal case {legal_case_id} filed against the dynasty"),
                format!(
                    "Dynasty {plaintiff_id} filed a {:?} claim for up to {}. The hearing is scheduled for day {hearing_day}: {}.",
                    claim.kind, claim.maximum_damages, claim.description
                ),
            )?;
        }
    }
    Ok(())
}

fn legal_filing_interval_available(state: &AppState, plaintiff_id: DynastyId, day: i64) -> bool {
    state
        .legal_cases
        .values()
        .filter(|legal_case| legal_case.plaintiff_dynasty_id == plaintiff_id)
        .map(|legal_case| legal_case.filed_day)
        .max()
        .is_none_or(|last_filing_day| {
            last_filing_day
                .checked_add(super::LEGAL_CASE_FILING_INTERVAL_DAYS)
                .is_some_and(|next_filing_day| day >= next_filing_day)
        })
}

fn next_grounded_ai_legal_claim(
    state: &AppState,
    plaintiff_id: DynastyId,
) -> Option<super::LegalClaimQuote> {
    state
        .dynasties
        .keys()
        .copied()
        .filter(|defendant_id| *defendant_id != plaintiff_id)
        .flat_map(|defendant_id| {
            [LegalCaseKind::Debt, LegalCaseKind::ContractBreach]
                .into_iter()
                .filter_map(move |kind| {
                    super::quote_grounded_legal_claim(state, plaintiff_id, defendant_id, kind)
                })
        })
        .filter(|claim| {
            !state.legal_cases.values().any(|legal_case| {
                legal_case.plaintiff_dynasty_id == plaintiff_id
                    && legal_case.defendant_dynasty_id == claim.defendant_dynasty_id
                    && legal_case.kind == claim.kind
                    && matches!(
                        legal_case.status,
                        LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                    )
            })
        })
        .max_by_key(|claim| {
            (
                claim.evidence_basis_points,
                claim.maximum_damages,
                std::cmp::Reverse(claim.defendant_dynasty_id),
                std::cmp::Reverse(claim.kind),
            )
        })
}

fn advance_legal_case_hearings(state: &mut AppState) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let entering_hearing: Vec<_> = state
        .legal_cases
        .values()
        .filter(|legal_case| {
            legal_case.status == LegalCaseStatus::Filed
                && legal_case.hearing_day > day
                && legal_case.hearing_day.saturating_sub(day) <= 30
        })
        .map(|legal_case| {
            (
                legal_case.id,
                legal_case.plaintiff_dynasty_id == state.player_dynasty_id
                    || legal_case.defendant_dynasty_id == state.player_dynasty_id,
            )
        })
        .collect();
    for (legal_case_id, player_is_party) in entering_hearing {
        state
            .legal_cases
            .get_mut(&legal_case_id)
            .expect("legal case must exist")
            .status = LegalCaseStatus::Hearing;
        if player_is_party {
            try_push_outbox(
                state,
                OutboxKind::Legal,
                format!("Legal case {legal_case_id} entered hearing"),
                "The court began formal proceedings ahead of judgment.".to_owned(),
            )?;
        }
    }
    Ok(())
}

fn resolve_legal_cases(state: &mut AppState) -> Result<(), SimulationError> {
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
                case.kind,
                case.claim_source,
                case.evidence_basis_points,
                case.public_attention_basis_points,
                case.damages,
            )
        })
        .collect();
    for (id, plaintiff_id, defendant_id, kind, claim_source, evidence, attention, damages) in due {
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
        let (awarded, paid) = if plaintiff_wins {
            let awarded = recoverable_legal_damages(state, claim_source, damages);
            let paid = settle_legal_damages(state, plaintiff_id, defendant_id, awarded)?;
            settle_legal_claim_source(state, claim_source, plaintiff_id, defendant_id);
            (awarded, paid)
        } else {
            (Money::ZERO, Money::ZERO)
        };
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
        if plaintiff_id == state.player_dynasty_id || defendant_id == state.player_dynasty_id {
            try_push_outbox(
                state,
                OutboxKind::Legal,
                format!("Legal case {id} decided"),
                if plaintiff_wins {
                    let settlement_note = if claim_source.is_some() {
                        " The grounded source obligation is settled by the judgment."
                    } else {
                        ""
                    };
                    format!(
                        "The court decided the {kind:?} claim for dynasty {plaintiff_id}, awarded {awarded}, and recovered {paid} immediately.{settlement_note}"
                    )
                } else {
                    format!(
                        "The court decided the {kind:?} claim for dynasty {defendant_id}; no damages were awarded."
                    )
                },
            )?;
        }
    }
    Ok(())
}

fn settle_legal_damages(
    state: &mut AppState,
    plaintiff_id: DynastyId,
    defendant_id: DynastyId,
    damages: Money,
) -> Result<Money, SimulationError> {
    let defendant_cash = state
        .dynasties
        .get(&defendant_id)
        .expect("legal defendant must exist")
        .treasury();
    let plaintiff_treasury = state
        .dynasties
        .get(&plaintiff_id)
        .expect("legal plaintiff must exist")
        .treasury();
    let paid = damages.min(defendant_cash);
    plaintiff_treasury
        .checked_add(paid)
        .ok_or(SimulationError::DynastyTreasuryOverflow {
            dynasty_id: plaintiff_id,
            current: plaintiff_treasury,
            incoming: paid,
        })?;
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
        .expect("prevalidated damages must fit plaintiff treasury");
    Ok(paid)
}

pub(crate) fn recoverable_legal_damages(
    state: &AppState,
    claim_source: Option<LegalClaimSource>,
    requested: Money,
) -> Money {
    match claim_source {
        Some(LegalClaimSource::Loan { loan_id }) => state
            .loans
            .get(&loan_id)
            .map_or(Money::ZERO, |loan| requested.min(loan.balance)),
        Some(LegalClaimSource::Contract { contract_id }) => state
            .contracts
            .get(&contract_id)
            .map_or(Money::ZERO, |contract| {
                requested.min(contract.unpaid_breach_penalty)
            }),
        None => requested,
    }
}

pub(crate) fn settle_legal_claim_source(
    state: &mut AppState,
    claim_source: Option<LegalClaimSource>,
    plaintiff_id: DynastyId,
    defendant_id: DynastyId,
) {
    match claim_source {
        Some(LegalClaimSource::Loan { loan_id }) => {
            let collateral_property_id = {
                let Some(loan) = state.loans.get_mut(&loan_id) else {
                    return;
                };
                if loan.lender_dynasty_id != plaintiff_id
                    || loan.borrower_dynasty_id != defendant_id
                {
                    return;
                }
                loan.balance = Money::ZERO;
                loan.status = LoanStatus::Repaid;
                loan.missed_payments = 0;
                loan.collateral_property_id
            };
            if let Some(property_id) = collateral_property_id
                && let Some(property) = state.properties.get_mut(&property_id)
                && property.collateral_loan_id == Some(loan_id)
            {
                property.collateral_loan_id = None;
            }
        }
        Some(LegalClaimSource::Contract { contract_id }) => {
            let Some(contract) = state.contracts.get_mut(&contract_id) else {
                return;
            };
            if contract.breaching_dynasty_id != Some(defendant_id)
                || contract.breach_victim_dynasty_id != Some(plaintiff_id)
            {
                return;
            }
            contract.unpaid_breach_penalty = Money::ZERO;
        }
        None => {}
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

fn detect_and_advance_crises(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    advance_existing_crises(state)?;
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
            )?;
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
        )?;
    }
    detect_trade_disruption(state)?;
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
        )?;
    }
    detect_periodic_crises(state, day)?;
    Ok(())
}

fn advance_existing_crises(state: &mut AppState) -> Result<(), SimulationError> {
    let mut resolved = Vec::new();
    let mut escalated = Vec::new();
    let addressed_subjects: BTreeSet<_> = state
        .audit_log
        .iter()
        .filter(|record| crisis_response_contains_crisis(record))
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
        try_push_outbox(
            state,
            OutboxKind::Crisis,
            format!("Crisis {crisis_id} escalated"),
            format!(
                "The {kind:?} crisis intensified because no effective response had contained it."
            ),
        )?;
    }
    for (crisis_id, kind) in resolved {
        try_push_outbox(
            state,
            OutboxKind::Crisis,
            format!("Crisis {crisis_id} resolved"),
            format!("The {kind:?} crisis has subsided below an active threat level."),
        )?;
    }
    Ok(())
}

pub(crate) fn crisis_response_contains_crisis(record: &AuditRecord) -> bool {
    record.kind() == AuditKind::CrisisResponse
        && matches!(
            record.detail(),
            "response=Relief" | "response=Reform" | "response=Suppress"
        )
}

fn has_active_crisis(state: &AppState, kind: CrisisKind) -> bool {
    state
        .crises
        .values()
        .any(|crisis| crisis.kind == kind && crisis.status.is_active())
}

fn detect_periodic_crises(state: &mut AppState, day: i64) -> Result<(), SimulationError> {
    if day <= 0 || day % 180 != 0 {
        return Ok(());
    }
    detect_urban_fire(state)?;
    detect_epidemic(state)?;
    detect_guild_revolt(state)?;
    Ok(())
}

fn detect_urban_fire(state: &mut AppState) -> Result<(), SimulationError> {
    if has_active_crisis(state, CrisisKind::UrbanFire) {
        return Ok(());
    }
    let Some((district_id, safety)) = state
        .districts
        .iter()
        .min_by_key(|(_, district)| district.safety_basis_points)
        .map(|(id, district)| (*id, district.safety_basis_points))
    else {
        return Ok(());
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
        )?;
    }
    Ok(())
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

fn detect_epidemic(state: &mut AppState) -> Result<(), SimulationError> {
    if has_active_crisis(state, CrisisKind::Epidemic) {
        return Ok(());
    }
    let Some((district_id, sanitation)) = state
        .districts
        .iter()
        .min_by_key(|(_, district)| district.sanitation_basis_points)
        .map(|(id, district)| (*id, district.sanitation_basis_points))
    else {
        return Ok(());
    };
    let deficiency = 10_000_u16.saturating_sub(sanitation);
    let chance = deficiency.saturating_div(4).saturating_add(250).min(10_000);
    if state.rng.is_chance_success(chance) {
        let severity = 3_000_u16.saturating_add(deficiency / 5).min(9_000);
        insert_crisis(
            state,
            CrisisKind::Epidemic,
            Some(district_id),
            severity,
            "Poor sanitation allowed an epidemic to take hold.",
        )?;
        apply_epidemic_household_pressure(
            state,
            Some(district_id),
            (severity / EPIDEMIC_ONSET_WELFARE_DIVISOR).max(1),
        );
    }
    Ok(())
}

fn apply_epidemic_household_pressure(
    state: &mut AppState,
    district_id: Option<DistrictId>,
    welfare_loss: u16,
) {
    for household in state.households.iter_mut().filter(|household| {
        district_id.is_none_or(|district_id| household.district_id() == district_id)
    }) {
        household.food_satisfaction_basis_points = household
            .food_satisfaction_basis_points
            .saturating_sub(welfare_loss);
    }
}

fn detect_trade_disruption(state: &mut AppState) -> Result<(), SimulationError> {
    if has_active_crisis(state, CrisisKind::TradeDisruption) {
        return Ok(());
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
        )?;
    }
    Ok(())
}

fn detect_guild_revolt(state: &mut AppState) -> Result<(), SimulationError> {
    if has_active_crisis(state, CrisisKind::GuildRevolt) {
        return Ok(());
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
        )?;
    }
    Ok(())
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
) -> Result<crate::ids::CrisisId, SimulationError> {
    let id = state.next_ids.try_crisis()?;
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
    try_push_outbox(
        state,
        OutboxKind::Crisis,
        format!("Crisis emerged: {kind:?}"),
        cause.to_owned(),
    )?;
    Ok(id)
}

pub(crate) fn run_annual_strategic_systems(state: &mut AppState) -> Result<(), SimulationError> {
    educate_family_members(state);
    form_dynastic_marriage(state)?;
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

fn form_dynastic_marriage(state: &mut AppState) -> Result<(), SimulationError> {
    if state.clock.day() % 1_800 != 0 {
        return Ok(());
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
        return Ok(());
    };
    let id = state.next_ids.try_family_link()?;
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
    }
    remember_dynasty_interaction(
        state,
        left_dynasty,
        right_dynasty,
        "A dynastic marriage joined the two houses.",
    );
    try_push_outbox(
        state,
        OutboxKind::Family,
        "Dynastic marriage concluded".to_owned(),
        format!(
            "The heirs of dynasties {left_dynasty} and {right_dynasty} entered a marriage compact."
        ),
    )?;
    Ok(())
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
            subject: format!("dynasty:{dynasty_id}").into(),
            detail: format!(
                "automatic=true;from={prior:?};governance={governance:?};reason=low_unity"
            ),
        });
        try_push_outbox(
            state,
            OutboxKind::Family,
            format!("House {dynasty_id} charter changed under pressure"),
            format!(
                "Low family unity forced a transition from {prior:?} to {governance:?} governance."
            ),
        )?;
    }
    Ok(())
}

pub(crate) fn try_push_outbox(
    state: &mut AppState,
    kind: OutboxKind,
    subject: String,
    body: String,
) -> Result<(), IdentifierAllocationError> {
    let id = state.next_ids.try_outbox()?;
    state.outbox.push(OutboxMessage {
        id,
        day: state.clock.day(),
        kind,
        subject,
        body,
        acknowledged: false,
    });
    Ok(())
}

pub(crate) fn push_outbox(state: &mut AppState, kind: OutboxKind, subject: String, body: String) {
    try_push_outbox(state, kind, subject, body)
        .expect("bootstrap identifier space must be available");
}

#[cfg(test)]
#[path = "strategic_tests.rs"]
mod tests;
