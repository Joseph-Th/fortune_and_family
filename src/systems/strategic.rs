//! Strategic initialization, periodic systems, and validated cross-record operations.

use crate::core::{
    AiObjective, AppState, CharacterRole, ContractStatus, Crisis, CrisisKind, CrisisStatus,
    DistrictRuntime, DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw, ExternalRoute,
    FamilyCouncilState, FamilyLink, FamilyLinkKind, HouseGovernance, InformationConfidence,
    InformationReport, InstitutionRuntime, LawKind, LegalCase, LegalCaseKind, LegalCaseStatus,
    Loan, LoanStatus, ObjectiveKind, ObjectiveStatus, OfficePower, OutboxKind, OutboxMessage,
    Property, PropertyKind, PublicWork, PublicWorkKind, PublicWorkStatus, RelationshipState,
    SupplyContract,
};
use crate::ids::{BusinessId, DistrictId, DynastyId, GoodId, PropertyId};
use crate::money::{Money, Quantity, cost_for};
use crate::registry::{InstitutionKind, Registry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StrategicError {
    #[error("business {business_id} does not exist")]
    MissingBusiness { business_id: BusinessId },
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
    #[error("property {property_id} is already owned")]
    PropertyAlreadyOwned { property_id: PropertyId },
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

#[derive(Debug)]
pub struct ValidatedSupplyContract {
    terms: SupplyContractTerms,
}

impl ValidatedSupplyContract {
    pub fn commit(self, state: &mut AppState) -> crate::ids::ContractId {
        let id = state.next_ids.contract();
        let day = state.clock.day();
        let end_day = day.saturating_add(i64::from(self.terms.duration_weeks).saturating_mul(7));
        state.contracts.insert(
            id,
            SupplyContract {
                id,
                buyer_business_id: self.terms.buyer_business_id,
                seller_business_id: self.terms.seller_business_id,
                good_id: self.terms.good_id,
                quantity_per_week: self.terms.quantity_per_week,
                unit_price: self.terms.unit_price,
                penalty: self.terms.penalty,
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
                "Business {} will deliver {} of good {} to business {} each week.",
                self.terms.seller_business_id,
                self.terms.quantity_per_week,
                self.terms.good_id,
                self.terms.buyer_business_id
            ),
        );
        id
    }
}

#[derive(Debug)]
pub struct ValidatedLoan {
    terms: LoanTerms,
}

impl ValidatedLoan {
    /// Commits a previously validated loan atomically.
    ///
    /// # Panics
    ///
    /// Panics if the validated parties or collateral were removed before commit.
    pub fn commit(self, state: &mut AppState) -> crate::ids::LoanId {
        let id = state.next_ids.loan();
        let lender = state
            .dynasties
            .get_mut(&self.terms.lender_dynasty_id)
            .expect("validated lender must exist");
        lender.resources.treasury = lender
            .resources
            .treasury
            .saturating_sub(self.terms.principal);
        let borrower = state
            .dynasties
            .get_mut(&self.terms.borrower_dynasty_id)
            .expect("validated borrower must exist");
        borrower.resources.treasury = borrower
            .resources
            .treasury
            .saturating_add(self.terms.principal);
        if let Some(property_id) = self.terms.collateral_property_id {
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
                lender_dynasty_id: self.terms.lender_dynasty_id,
                borrower_dynasty_id: self.terms.borrower_dynasty_id,
                principal: self.terms.principal,
                balance: self.terms.principal,
                weekly_payment: self.terms.weekly_payment,
                interest_basis_points: self.terms.interest_basis_points.min(10_000),
                next_due_day: state.clock.day().saturating_add(7),
                missed_payments: 0,
                collateral_property_id: self.terms.collateral_property_id,
                status: LoanStatus::Current,
            },
        );
        push_outbox(
            state,
            OutboxKind::Finance,
            format!("Loan {id} issued"),
            format!(
                "Dynasty {} lent {} to dynasty {}.",
                self.terms.lender_dynasty_id, self.terms.principal, self.terms.borrower_dynasty_id
            ),
        );
        id
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
    if terms.buyer_business_id == terms.seller_business_id {
        return Err(StrategicError::SameContractParty);
    }
    if terms.quantity_per_week <= Quantity::ZERO {
        return Err(StrategicError::NonPositiveQuantity);
    }
    if terms.unit_price <= Money::ZERO || terms.penalty < Money::ZERO {
        return Err(StrategicError::NonPositiveAmount);
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
    Ok(ValidatedSupplyContract { terms })
}

/// Validates and creates a supply contract through its canonical commit token.
///
/// # Errors
///
/// Returns the same errors as [`validate_supply_contract`].
pub fn create_supply_contract(
    registry: &Registry,
    state: &mut AppState,
    terms: SupplyContractTerms,
) -> Result<crate::ids::ContractId, StrategicError> {
    Ok(validate_supply_contract(registry, state, terms)?.commit(state))
}

/// Validates a loan without mutating state.
///
/// # Errors
///
/// Returns an error for missing parties, invalid terms, insufficient lender funds, or invalid collateral.
pub fn validate_loan(state: &AppState, terms: LoanTerms) -> Result<ValidatedLoan, StrategicError> {
    if terms.lender_dynasty_id == terms.borrower_dynasty_id {
        return Err(StrategicError::SameLoanParty);
    }
    if terms.principal <= Money::ZERO || terms.weekly_payment <= Money::ZERO {
        return Err(StrategicError::NonPositiveAmount);
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
            return Err(StrategicError::MissingProperty { property_id });
        }
    }
    Ok(ValidatedLoan { terms })
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
    Ok(validate_loan(state, terms)?.commit(state))
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
        let legacy = state
            .institutions
            .get(&definition.id())
            .expect("legacy institution state must be initialized");
        let mut members = BTreeSet::new();
        for dynasty in state.dynasties.values() {
            members.insert(dynasty.head_id());
        }
        state.institution_runtime.insert(
            definition.id(),
            InstitutionRuntime {
                institution_id: definition.id(),
                members,
                office_holder_id: legacy.office_holder_id(),
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
                business.operations.employees,
            )
        })
        .collect();
    for (business_id, district_id, workers) in businesses {
        let Some(household_id) = state
            .households
            .ids_for_district(district_id)
            .and_then(|ids| ids.iter().next())
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
                weekly_wage: Money::from_copper(i64::from(workers).saturating_mul(95)),
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
        let Some(good_id) = registry.get_good_id(good_key) else {
            continue;
        };
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
                token.commit(state);
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
        if let Ok(token) = validate_loan(state, terms) {
            token.commit(state);
        }
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
        .unwrap_or(registry.districts()[0].id());
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
    apply_external_route_supply(state);
    apply_law_price_controls(registry, state);
    apply_crisis_daily_effects(registry, state);
}

fn apply_external_route_supply(state: &mut AppState) {
    let routes: Vec<_> = state
        .external_routes
        .values()
        .filter(|route| route.active)
        .map(|route| {
            let available_basis_points = 10_000_u16.saturating_sub(route.disruption_basis_points);
            (
                route.good_id,
                route
                    .daily_capacity
                    .saturating_mul_ratio(i64::from(available_basis_points), 10_000),
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

fn apply_law_price_controls(registry: &Registry, state: &mut AppState) {
    let ceiling = state
        .laws
        .values()
        .find(|law| law.active && law.kind == LawKind::BreadPriceCeiling)
        .map(|law| law.value);
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
        .filter(|crisis| matches!(crisis.status, CrisisStatus::Emerging | CrisisStatus::Active))
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
                    quote.target_stock = quote.target_stock.saturating_mul_ratio(
                        10_000_i64.saturating_add(i64::from(severity) / 4),
                        10_000,
                    );
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
            CrisisKind::BankingPanic | CrisisKind::GuildRevolt | CrisisKind::NobleDemand => {}
        }
    }
}

pub(crate) fn run_weekly_strategic_systems(registry: &Registry, state: &mut AppState) {
    settle_contracts(state);
    settle_loans(state);
    settle_property_rents(state);
    settle_employment(state);
    progress_public_works(state);
    update_relationships_from_obligations(state);
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
        .map(|contract| {
            (
                contract.id,
                contract.buyer_business_id,
                contract.seller_business_id,
                contract.good_id,
                contract.quantity_per_week,
                contract.unit_price,
                contract.penalty,
                contract.end_day,
            )
        })
        .collect();
    for (id, buyer_id, seller_id, good_id, quantity, unit_price, penalty, end_day) in due {
        let payment = cost_for(quantity, unit_price);
        let seller_inventory = state
            .businesses
            .get(seller_id)
            .expect("contract seller must exist")
            .inventory_quantity(good_id);
        let buyer_cash = state
            .businesses
            .get(buyer_id)
            .expect("contract buyer must exist")
            .cash();
        let fulfilled = seller_inventory >= quantity && buyer_cash >= payment;
        if fulfilled {
            {
                let seller = state
                    .businesses
                    .get_mut(seller_id)
                    .expect("contract seller must exist");
                seller.remove_inventory(good_id, quantity);
                seller.finance.cash = seller.finance.cash.saturating_add(payment);
                seller.finance.lifetime_revenue =
                    seller.finance.lifetime_revenue.saturating_add(payment);
            }
            {
                let buyer = state
                    .businesses
                    .get_mut(buyer_id)
                    .expect("contract buyer must exist");
                buyer.add_inventory(good_id, quantity);
                buyer.finance.cash = buyer.finance.cash.saturating_sub(payment);
                buyer.finance.lifetime_costs = buyer.finance.lifetime_costs.saturating_add(payment);
            }
            let contract = state.contracts.get_mut(&id).expect("contract must exist");
            contract.fulfilled_deliveries = contract.fulfilled_deliveries.saturating_add(1);
            contract.next_due_day = contract.next_due_day.saturating_add(7);
        } else {
            let available = state
                .businesses
                .get(seller_id)
                .expect("contract seller must exist")
                .cash();
            let paid_penalty = penalty.min(available);
            state
                .businesses
                .get_mut(seller_id)
                .expect("contract seller must exist")
                .finance
                .cash = available.saturating_sub(paid_penalty);
            let buyer = state
                .businesses
                .get_mut(buyer_id)
                .expect("contract buyer must exist");
            buyer.finance.cash = buyer.finance.cash.saturating_add(paid_penalty);
            let contract = state.contracts.get_mut(&id).expect("contract must exist");
            contract.missed_deliveries = contract.missed_deliveries.saturating_add(1);
            contract.next_due_day = contract.next_due_day.saturating_add(7);
            if contract.missed_deliveries >= 3 {
                contract.status = ContractStatus::Breached;
                push_outbox(
                    state,
                    OutboxKind::Contract,
                    format!("Contract {id} breached"),
                    format!("Repeated nonperformance caused supply contract {id} to terminate."),
                );
            }
        }
        let contract = state.contracts.get_mut(&id).expect("contract must exist");
        if contract.status == ContractStatus::Active && contract.next_due_day > end_day {
            contract.status = ContractStatus::Fulfilled;
        }
    }
}

fn settle_loans(state: &mut AppState) {
    let day = state.clock.day();
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
        let interest_due =
            Money::from_copper(balance.copper().saturating_mul(i64::from(interest)) / 10_000 / 52);
        let amount_due = weekly_payment.min(balance.saturating_add(interest_due));
        let borrower_treasury = state
            .dynasties
            .get(&borrower_id)
            .expect("loan borrower must exist")
            .treasury();
        if borrower_treasury >= amount_due {
            state
                .dynasties
                .get_mut(&borrower_id)
                .expect("loan borrower must exist")
                .resources
                .treasury = borrower_treasury.saturating_sub(amount_due);
            let lender = state
                .dynasties
                .get_mut(&lender_id)
                .expect("loan lender must exist");
            lender.resources.treasury = lender.resources.treasury.saturating_add(amount_due);
            let loan = state.loans.get_mut(&id).expect("loan must exist");
            loan.balance = loan
                .balance
                .saturating_add(interest_due)
                .saturating_sub(amount_due);
            loan.next_due_day = loan.next_due_day.saturating_add(7);
            loan.missed_payments = 0;
            loan.status = if loan.balance <= Money::ZERO {
                LoanStatus::Repaid
            } else {
                LoanStatus::Current
            };
            if loan.status == LoanStatus::Repaid
                && let Some(property_id) = collateral
                && let Some(property) = state.properties.get_mut(&property_id)
            {
                property.collateral_loan_id = None;
            }
        } else {
            let loan = state.loans.get_mut(&id).expect("loan must exist");
            loan.missed_payments = loan.missed_payments.saturating_add(1);
            loan.next_due_day = loan.next_due_day.saturating_add(7);
            loan.status = if loan.missed_payments >= 3 {
                LoanStatus::Defaulted
            } else {
                LoanStatus::Delinquent
            };
            if loan.status == LoanStatus::Defaulted {
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
        }
    }
}

fn settle_property_rents(state: &mut AppState) {
    let rents: Vec<_> = state
        .properties
        .values()
        .filter_map(|property| {
            Some((
                property.owner_dynasty_id?,
                property.tenant_dynasty_id?,
                property.weekly_rent,
            ))
        })
        .collect();
    for (owner_id, tenant_id, rent) in rents {
        if owner_id == tenant_id || rent <= Money::ZERO {
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
        let owner = state
            .dynasties
            .get_mut(&owner_id)
            .expect("property owner dynasty must exist");
        owner.resources.treasury = owner.resources.treasury.saturating_add(paid);
    }
}

fn settle_employment(state: &mut AppState) {
    let agreements: Vec<_> = state
        .employment
        .values()
        .filter(|agreement| agreement.status == EmploymentStatus::Active)
        .map(|agreement| {
            (
                agreement.id,
                agreement.business_id,
                agreement.household_id,
                agreement.weekly_wage,
            )
        })
        .collect();
    for (id, business_id, household_id, wage) in agreements {
        let business_cash = state
            .businesses
            .get(business_id)
            .expect("employment business must exist")
            .cash();
        let paid = wage.min(business_cash);
        state
            .businesses
            .get_mut(business_id)
            .expect("employment business must exist")
            .finance
            .cash = business_cash.saturating_sub(paid);
        let household = state
            .households
            .get_mut(household_id)
            .expect("employment household must exist");
        household.cash = household.cash.saturating_add(paid);
        let agreement = state
            .employment
            .get_mut(&id)
            .expect("employment must exist");
        if paid == wage {
            agreement.loyalty_basis_points = agreement
                .loyalty_basis_points
                .saturating_add(30)
                .min(10_000);
        } else {
            agreement.loyalty_basis_points = agreement.loyalty_basis_points.saturating_sub(250);
            agreement.conditions_basis_points =
                agreement.conditions_basis_points.saturating_sub(100);
            if agreement.loyalty_basis_points < 2_000 {
                agreement.status = EmploymentStatus::Disputed;
                push_outbox(
                    state,
                    OutboxKind::District,
                    format!("Labor dispute at business {business_id}"),
                    "Repeated wage shortfalls have caused organized workplace resistance."
                        .to_owned(),
                );
            }
        }
    }
}

fn progress_public_works(state: &mut AppState) {
    let ids: Vec<_> = state
        .public_works
        .values()
        .filter(|work| work.status == PublicWorkStatus::Building)
        .map(|work| work.id)
        .collect();
    for id in ids {
        let work = state
            .public_works
            .get_mut(&id)
            .expect("public work must exist");
        let remaining = work.budget.saturating_sub(work.spent);
        let weekly_spend = Money::from_copper(1_500).min(remaining);
        work.spent = work.spent.saturating_add(weekly_spend);
        let progress = if work.budget.copper() <= 0 {
            10_000
        } else {
            work.spent.copper().saturating_mul(10_000) / work.budget.copper()
        };
        work.progress_basis_points = u16::try_from(progress.clamp(0, 10_000)).unwrap_or(10_000);
        if work.progress_basis_points >= 10_000 {
            work.status = PublicWorkStatus::Completed;
            if let Some(district) = state.districts.get_mut(&work.district_id) {
                match work.kind {
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

fn apply_law_economic_effects(registry: &Registry, state: &mut AppState) {
    let emergency_imports = state
        .laws
        .values()
        .any(|law| law.active && law.kind == LawKind::EmergencyImports);
    if emergency_imports && let Some(grain_id) = registry.get_good_id("grain") {
        let quote = state
            .market
            .quotes
            .get_mut(&grain_id)
            .expect("grain quote must exist");
        quote.stock = quote.stock.saturating_add(Quantity::from_units(150));
    }
    let interest_limit = state
        .laws
        .values()
        .find(|law| law.active && law.kind == LawKind::InterestLimit)
        .map(|law| u16::try_from(law.value.clamp(0, 10_000)).unwrap_or(10_000));
    if let Some(limit) = interest_limit {
        for loan in state.loans.values_mut() {
            loan.interest_basis_points = loan.interest_basis_points.min(limit);
        }
    }
}

pub(crate) fn run_monthly_strategic_systems(registry: &Registry, state: &mut AppState) {
    update_district_conditions(state);
    resolve_institution_selections(state);
    advance_ai_objectives(registry, state);
    update_information_reports(registry, state);
    resolve_legal_cases(state);
    update_external_route_risk(state);
    detect_and_advance_crises(registry, state);
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
        .institution_runtime
        .values()
        .filter(|institution| institution.next_selection_day <= day)
        .map(|institution| institution.institution_id)
        .collect();
    for institution_id in due {
        let candidates: Vec<_> = state
            .institution_runtime
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
                let score = u32::from(character.capabilities.social)
                    .saturating_mul(100)
                    .saturating_add(u32::from(dynasty.resources.legitimacy_basis_points));
                (score, character.id())
            })
            .collect();
        let winner = candidates
            .into_iter()
            .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
            .map(|(_, character_id)| character_id);
        let term_number = {
            let institution = state
                .institution_runtime
                .get_mut(&institution_id)
                .expect("institution runtime must exist");
            institution.office_holder_id = winner;
            institution.next_selection_day = day.saturating_add(360);
            institution.term_number = institution.term_number.saturating_add(1);
            institution.term_number
        };
        if let Some(legacy) = state.institutions.get_mut(&institution_id) {
            legacy.office_holder_id = winner;
            legacy.policy_version = legacy.policy_version.saturating_add(1);
        }
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
        let achieved = match kind {
            ObjectiveKind::AcquireProperty => ai_acquire_property(state, dynasty_id),
            ObjectiveKind::WinOffice => ai_pursue_office(state, dynasty_id),
            ObjectiveKind::SecureSupply => ai_secure_supply(registry, state, dynasty_id),
            ObjectiveKind::ReduceDebt => ai_reduce_debt(state, dynasty_id),
            ObjectiveKind::ImproveLegitimacy => ai_improve_legitimacy(state, dynasty_id),
            ObjectiveKind::AccumulateCash => state
                .dynasties
                .get(&dynasty_id)
                .is_some_and(|dynasty| dynasty.treasury() > Money::from_copper(120_000)),
            ObjectiveKind::ContainRival => ai_contain_rival(state, dynasty_id),
        };
        if achieved {
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

fn ai_acquire_property(state: &mut AppState, dynasty_id: DynastyId) -> bool {
    let property_id = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id.is_none())
        .min_by_key(|property| (property.value, property.id))
        .map(|property| property.id);
    property_id
        .is_some_and(|property_id| buy_unowned_property(state, dynasty_id, property_id).is_ok())
}

fn ai_pursue_office(state: &mut AppState, dynasty_id: DynastyId) -> bool {
    let holds_office = state.institution_runtime.values().any(|institution| {
        institution.office_holder_id.is_some_and(|character_id| {
            state
                .characters
                .get(character_id)
                .is_some_and(|character| character.dynasty_id() == dynasty_id)
        })
    });
    if holds_office {
        return true;
    }
    if let Some(dynasty) = state.dynasties.get_mut(&dynasty_id) {
        let spend = Money::from_copper(500).min(dynasty.resources.treasury);
        dynasty.resources.treasury = dynasty.resources.treasury.saturating_sub(spend);
        dynasty.resources.legitimacy_basis_points = dynasty
            .resources
            .legitimacy_basis_points
            .saturating_add(80)
            .min(10_000);
    }
    false
}

fn ai_secure_supply(registry: &Registry, state: &mut AppState, dynasty_id: DynastyId) -> bool {
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
                return true;
            }
            let seller_id = state.businesses.iter().find_map(|seller| {
                let seller_recipe = registry.get_recipe(seller.recipe_id())?;
                (seller.owner_dynasty_id() != dynasty_id
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
            return create_supply_contract(registry, state, terms).is_ok();
        }
    }
    false
}

fn ai_reduce_debt(state: &mut AppState, dynasty_id: DynastyId) -> bool {
    let loan_id = state
        .loans
        .values()
        .find(|loan| {
            loan.borrower_dynasty_id == dynasty_id
                && matches!(loan.status, LoanStatus::Current | LoanStatus::Delinquent)
        })
        .map(|loan| loan.id);
    let Some(loan_id) = loan_id else {
        return true;
    };
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .expect("AI dynasty must exist")
        .treasury();
    let extra = Money::from_copper(1_000).min(treasury);
    state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("AI dynasty must exist")
        .resources
        .treasury = treasury.saturating_sub(extra);
    let loan = state.loans.get_mut(&loan_id).expect("AI loan must exist");
    loan.balance = loan.balance.saturating_sub(extra);
    loan.balance <= Money::ZERO
}

fn ai_improve_legitimacy(state: &mut AppState, dynasty_id: DynastyId) -> bool {
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("AI dynasty must exist");
    if dynasty.resources.legitimacy_basis_points >= 7_500 {
        return true;
    }
    let spend = Money::from_copper(750).min(dynasty.resources.treasury);
    dynasty.resources.treasury = dynasty.resources.treasury.saturating_sub(spend);
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_add(120)
        .min(10_000);
    false
}

fn ai_contain_rival(state: &mut AppState, dynasty_id: DynastyId) -> bool {
    let pair = DynastyPair::new(dynasty_id, state.player_dynasty_id);
    let Some(relationship) = state.relationships.get_mut(&pair) else {
        return true;
    };
    relationship.fear_basis_points = relationship
        .fear_basis_points
        .saturating_add(100)
        .min(10_000);
    relationship.resentment_basis_points = relationship
        .resentment_basis_points
        .saturating_add(75)
        .min(10_000);
    relationship.fear_basis_points >= 5_000
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
        if state.rng.chance_basis_points(route.risk_basis_points / 12) {
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
    for crisis in state.crises.values_mut() {
        if matches!(crisis.status, CrisisStatus::Resolved) {
            continue;
        }
        crisis.status = CrisisStatus::Active;
        crisis.severity_basis_points = crisis.severity_basis_points.saturating_sub(120);
        if crisis.severity_basis_points < 500 {
            crisis.status = CrisisStatus::Resolved;
        }
    }
    let has_grain_crisis = state.crises.values().any(|crisis| {
        crisis.kind == CrisisKind::GrainShortage
            && matches!(crisis.status, CrisisStatus::Emerging | CrisisStatus::Active)
    });
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
            create_crisis(
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
    let has_panic = state.crises.values().any(|crisis| {
        crisis.kind == CrisisKind::BankingPanic
            && matches!(crisis.status, CrisisStatus::Emerging | CrisisStatus::Active)
    });
    if defaulted_loans >= 2 && !has_panic {
        create_crisis(
            state,
            CrisisKind::BankingPanic,
            None,
            3_800,
            "Multiple defaults damaged confidence in city credit.",
        );
    }
    if day > 0 && day % 720 == 0 && state.rng.chance_basis_points(2_500) {
        let district_id = state.districts.keys().copied().next();
        create_crisis(
            state,
            CrisisKind::NobleDemand,
            district_id,
            3_000,
            "The regional prince demanded an extraordinary payment from Rivergate.",
        );
    }
}

fn create_crisis(
    state: &mut AppState,
    kind: CrisisKind,
    district_id: Option<DistrictId>,
    severity_basis_points: u16,
    cause: &str,
) {
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
        .collect();
    let Some((left_dynasty, left_heir)) = heirs.first().copied() else {
        return;
    };
    let Some((right_dynasty, right_heir)) = heirs
        .iter()
        .copied()
        .find(|(dynasty_id, _)| *dynasty_id != left_dynasty)
    else {
        return;
    };
    let already_linked = state.family_links.values().any(|link| {
        link.active
            && link.kind == FamilyLinkKind::Marriage
            && ((link.first_character_id == left_heir && link.second_character_id == right_heir)
                || (link.first_character_id == right_heir && link.second_character_id == left_heir))
    });
    if already_linked {
        return;
    }
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
    for council in state.family_councils.values_mut() {
        let members = u16::try_from(council.members.len()).unwrap_or(u16::MAX);
        let branch_pressure = members.saturating_sub(2).saturating_mul(80);
        council.unity_basis_points = council
            .unity_basis_points
            .saturating_sub(branch_pressure)
            .saturating_add(50)
            .min(10_000);
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
mod tests {
    use super::*;
    use crate::core::NewGameConfig;
    use crate::registry::build_rivergate_registry;
    use crate::systems::{advance_days, build_new_game, validate_invariants};

    #[test]
    fn strategic_bootstrap_creates_connected_records() {
        let registry = build_rivergate_registry();
        let state = build_new_game(&registry, NewGameConfig::default());

        assert!(!state.properties.is_empty());
        assert!(!state.contracts.is_empty());
        assert!(!state.employment.is_empty());
        assert_eq!(state.districts.len(), registry.districts().len());
        assert_eq!(
            state.institution_runtime.len(),
            registry.institutions().len()
        );
    }

    #[test]
    fn strategic_systems_remain_valid_across_two_generations() {
        let registry = build_rivergate_registry();
        let mut state = build_new_game(&registry, NewGameConfig::default());

        advance_days(&registry, &mut state, 7_200).expect("strategic simulation must advance");
        validate_invariants(&registry, &state);

        assert!(state.clock.day() >= 7_200);
        assert!(!state.information_reports.is_empty());
        assert!(!state.ai_objectives.is_empty());
    }

    #[test]
    fn invalid_supply_contract_does_not_mutate_state() {
        let registry = build_rivergate_registry();
        let state = build_new_game(&registry, NewGameConfig::default());
        let before = state.clone();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("business must exist")
            .id();
        let good_id = registry.goods()[0].id();

        let result = validate_supply_contract(
            &registry,
            &state,
            SupplyContractTerms {
                buyer_business_id: business_id,
                seller_business_id: business_id,
                good_id,
                quantity_per_week: Quantity::ONE,
                unit_price: Money::from_copper(1),
                penalty: Money::ZERO,
                duration_weeks: 1,
            },
        );

        assert!(matches!(result, Err(StrategicError::SameContractParty)));
        assert_eq!(state, before);
    }
}
