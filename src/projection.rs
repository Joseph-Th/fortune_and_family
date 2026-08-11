//! Read-only causal projections and a self-contained HTML campaign dashboard.

use crate::core::{
    AppState, BusinessStatus, CampaignPhase, CivicDebtStatus, ContractStatus, CrisisKind,
    CrisisStatus, InformationConfidence, InformationTarget, LawKind, LegalCaseStatus,
    LegalClaimSource, LoanStatus, MarketCause, ObjectiveKind, ObjectiveStatus, OutboxKind,
    PublicWorkKind, PublicWorkStatus,
};
use crate::ids::{
    BusinessId, CivicDebtId, ContractId, CrisisId, DistrictId, DynastyId, InstitutionId, LawId,
    LegalCaseId, LoanId, PropertyId, PublicWorkId,
};
use crate::money::{Money, Quantity};
use crate::registry::Registry;
use crate::systems::{
    dynasty_office_administrative_load, effective_property_weekly_rent, quote_business_acquisition,
};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSummary {
    pub scenario_name: String,
    pub year: i32,
    pub day_of_year: u16,
    pub elapsed_days: i64,
    pub dynasty_name: String,
    pub phase: CampaignPhase,
    pub dynasty_treasury: Money,
    pub business_cash: Money,
    pub businesses: usize,
    pub active_businesses: usize,
    pub population_groups: usize,
    pub average_food_satisfaction_basis_points: u16,
    pub chronicle_entries: usize,
    pub active_contracts: usize,
    pub current_loans: usize,
    pub outstanding_civic_debts: usize,
    pub civic_debt_balance: Money,
    pub properties: usize,
    pub active_crises: usize,
    pub unread_notifications: usize,
}

/// Builds the compact read-only summary used by user-interface adapters.
///
/// # Panics
///
/// Panics when the registry belongs to another scenario, the player dynasty reference is corrupt,
/// or a derived numeric invariant is corrupt.
#[must_use]
pub fn build_state_summary(registry: &Registry, state: &AppState) -> StateSummary {
    assert_eq!(
        state.scenario_key(),
        registry.scenario().key(),
        "state and registry scenarios must match before projection"
    );
    let dynasty = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let business_ids = state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .map_or(0, std::collections::BTreeSet::len);
    let active_businesses = state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .filter(|business| business.status() == BusinessStatus::Active)
        .count();
    let business_cash = state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .fold(Money::ZERO, |total, business| {
            total.saturating_add(business.cash())
        });
    let average_food_satisfaction_basis_points =
        crate::core::population_weighted_food_satisfaction_basis_points(state.households.iter())
            .unwrap_or(0);

    StateSummary {
        scenario_name: registry.scenario().name().to_owned(),
        year: state.clock.year(registry.scenario().start_year()),
        day_of_year: state.clock.day_of_year(),
        elapsed_days: state.clock.day(),
        dynasty_name: dynasty.name().to_owned(),
        phase: dynasty.phase(),
        dynasty_treasury: dynasty.treasury(),
        business_cash,
        businesses: business_ids,
        active_businesses,
        population_groups: state.households.records().len(),
        average_food_satisfaction_basis_points,
        chronicle_entries: state.chronicle.len(),
        active_contracts: state
            .contracts
            .values()
            .filter(|contract| contract.status() == ContractStatus::Active)
            .count(),
        current_loans: state
            .loans
            .values()
            .filter(|loan| loan.status().is_repayment_active())
            .count(),
        outstanding_civic_debts: state
            .civic_debts
            .values()
            .filter(|debt| debt.status != CivicDebtStatus::Repaid)
            .count(),
        civic_debt_balance: state
            .civic_debts
            .values()
            .filter(|debt| debt.status != CivicDebtStatus::Repaid)
            .fold(Money::ZERO, |total, debt| {
                total.saturating_add(debt.balance)
            }),
        properties: state.properties.len(),
        active_crises: state
            .crises
            .values()
            .filter(|crisis| crisis.status.is_active())
            .count(),
        unread_notifications: state
            .outbox
            .iter()
            .filter(|message| !message.acknowledged)
            .count(),
    }
}

impl AppState {
    /// Builds the compact read-only summary used by user-interface adapters.
    #[must_use]
    pub fn summary(&self, registry: &Registry) -> StateSummary {
        build_state_summary(registry, self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CampaignProjection {
    pub scenario: ScenarioProjection,
    pub player: DynastyProjection,
    pub dynasties: Vec<DynastyProjection>,
    pub districts: Vec<DistrictProjection>,
    pub businesses: Vec<BusinessProjection>,
    pub market: Vec<MarketProjection>,
    pub contracts: Vec<ContractProjection>,
    pub loans: Vec<LoanProjection>,
    pub civic_debts: Vec<CivicDebtProjection>,
    pub properties: Vec<PropertyProjection>,
    pub institutions: Vec<InstitutionProjection>,
    pub laws: Vec<LawProjection>,
    pub public_works: Vec<PublicWorkProjection>,
    pub legal_cases: Vec<LegalCaseProjection>,
    pub crises: Vec<CrisisProjection>,
    pub relationships: Vec<RelationshipProjection>,
    pub information: Vec<InformationProjection>,
    pub notifications: Vec<NotificationProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScenarioProjection {
    pub name: String,
    pub year: i32,
    pub day_of_year: u16,
    pub elapsed_days: i64,
    pub phase: CampaignPhase,
    pub average_food_satisfaction_basis_points: u16,
    pub active_crises: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DynastyProjection {
    pub id: DynastyId,
    pub name: String,
    pub treasury: Money,
    pub civic_contributions: Money,
    pub unmet_office_duties: u32,
    pub legitimacy_basis_points: u16,
    pub reputation_quality_basis_points: u16,
    pub reputation_reliability_basis_points: u16,
    pub administrative_capacity: u16,
    pub administrative_load: u16,
    pub office_administrative_load: u16,
    pub effective_administrative_load: u16,
    pub generation: u16,
    pub properties: usize,
    pub businesses: usize,
    pub current_loans_as_borrower: usize,
    pub offices: Vec<String>,
    pub active_objective: Option<ObjectiveProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ObjectiveProjection {
    pub kind: ObjectiveKind,
    pub status: ObjectiveStatus,
    pub priority: u16,
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DistrictProjection {
    pub id: DistrictId,
    pub name: String,
    pub population: u32,
    pub rent_index_basis_points: u16,
    pub employment_basis_points: u16,
    pub sanitation_basis_points: u16,
    pub safety_basis_points: u16,
    pub unrest_basis_points: u16,
    pub food_satisfaction_basis_points: u16,
    pub businesses: usize,
    pub properties: usize,
    pub causes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MarketProjection {
    pub good: String,
    pub price: Money,
    pub previous_price: Money,
    pub stock: Quantity,
    pub target_stock: Quantity,
    pub demand_today: Quantity,
    pub supply_today: Quantity,
    pub causes: Vec<MarketCause>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BusinessProjection {
    pub id: BusinessId,
    pub name: String,
    pub owner: String,
    pub district: String,
    pub recipe: String,
    pub manager: String,
    pub status: BusinessStatus,
    pub cash: Money,
    pub capacity_batches_per_day: u16,
    pub condition_basis_points: u16,
    pub quality_basis_points: u16,
    pub target_input_days: u16,
    pub target_output_days: u16,
    pub minimum_cash_reserve: Money,
    pub maintenance_basis_points: u16,
    pub quality_target_basis_points: u16,
    pub inventory: Vec<BusinessInventoryProjection>,
    pub acquisition: Option<BusinessAcquisitionProjection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BusinessInventoryProjection {
    pub good: String,
    pub quantity: Quantity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BusinessAcquisitionProjection {
    pub purchase_price: Money,
    pub minimum_recapitalization: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContractProjection {
    pub id: ContractId,
    pub buyer_business_id: BusinessId,
    pub buyer_name: String,
    pub seller_business_id: BusinessId,
    pub seller_name: String,
    pub good: String,
    pub quantity_per_week: Quantity,
    pub unit_price: Money,
    pub penalty: Money,
    pub next_due_day: i64,
    pub end_day: i64,
    pub status: ContractStatus,
    pub fulfilled_deliveries: u16,
    pub delivery_credits: Vec<ContractDeliveryCreditProjection>,
    pub missed_deliveries: u16,
    pub breaching_dynasty_id: Option<DynastyId>,
    pub breaching_dynasty: Option<String>,
    pub breach_victim_dynasty_id: Option<DynastyId>,
    pub breach_victim_dynasty: Option<String>,
    pub unpaid_breach_penalty: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContractDeliveryCreditProjection {
    pub dynasty_id: DynastyId,
    pub dynasty: String,
    pub deliveries: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LoanProjection {
    pub id: LoanId,
    pub lender: String,
    pub borrower: String,
    pub principal: Money,
    pub balance: Money,
    pub weekly_payment: Money,
    pub interest_basis_points: u16,
    pub status: LoanStatus,
    pub collateral_property_id: Option<PropertyId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CivicDebtProjection {
    pub id: CivicDebtId,
    pub creditor_dynasty_id: DynastyId,
    pub creditor: String,
    pub sponsor: Option<String>,
    pub authorizing_law_id: LawId,
    pub principal: Money,
    pub balance: Money,
    pub weekly_payment: Money,
    pub interest_basis_points: u16,
    pub issued_day: i64,
    pub next_due_day: i64,
    pub missed_payments: u8,
    pub status: CivicDebtStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PropertyProjection {
    pub id: PropertyId,
    pub name: String,
    pub district: String,
    pub owner: Option<String>,
    pub value: Money,
    pub weekly_rent: Money,
    pub effective_weekly_rent: Money,
    pub district_rent_index_basis_points: u16,
    pub condition_basis_points: u16,
    pub occupied_business_id: Option<BusinessId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstitutionProjection {
    pub id: InstitutionId,
    pub name: String,
    pub officeholder: Option<String>,
    pub officeholder_dynasty: Option<String>,
    pub budget: Money,
    pub legitimacy_basis_points: u16,
    pub term_started_day: i64,
    pub next_selection_day: i64,
    pub powers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LawProjection {
    pub id: LawId,
    pub kind: LawKind,
    pub value: i64,
    pub enacted_day: i64,
    pub sponsor: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicWorkProjection {
    pub id: PublicWorkId,
    pub district: String,
    pub kind: PublicWorkKind,
    pub budget: Money,
    pub spent: Money,
    pub progress_basis_points: u16,
    pub status: PublicWorkStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LegalCaseProjection {
    pub id: LegalCaseId,
    pub plaintiff: String,
    pub defendant: String,
    pub kind: String,
    pub claim_source: Option<LegalClaimSource>,
    pub evidence_basis_points: u16,
    pub hearing_day: i64,
    pub damages: Money,
    pub status: LegalCaseStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CrisisProjection {
    pub id: CrisisId,
    pub kind: CrisisKind,
    pub district: Option<String>,
    pub started_day: i64,
    pub severity_basis_points: u16,
    pub status: CrisisStatus,
    pub cause: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RelationshipProjection {
    pub dynasty_id: DynastyId,
    pub dynasty_name: String,
    pub trust_basis_points: u16,
    pub respect_basis_points: u16,
    pub fear_basis_points: u16,
    pub resentment_basis_points: u16,
    pub obligation: i32,
    pub last_interaction_day: i64,
    pub memories: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InformationProjection {
    pub target: Option<InformationTarget>,
    pub subject: String,
    pub confidence: InformationConfidence,
    pub source: String,
    pub summary: String,
    pub created_day: i64,
    pub expires_day: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NotificationProjection {
    pub day: i64,
    pub kind: OutboxKind,
    pub subject: String,
    pub body: String,
    pub acknowledged: bool,
}

/// Builds the read-only campaign projection used by adapters.
///
/// # Panics
///
/// Panics when runtime references violate the invariants required by the projection.
#[must_use]
pub fn build_campaign_projection(registry: &Registry, state: &AppState) -> CampaignProjection {
    let summary = build_state_summary(registry, state);
    let dynasties = state
        .dynasties
        .values()
        .map(|dynasty| build_dynasty_projection(registry, state, dynasty.id()))
        .collect::<Vec<_>>();
    let player = dynasties
        .iter()
        .find(|dynasty| dynasty.id == state.player_dynasty_id)
        .cloned()
        .expect("player dynasty projection must exist");
    CampaignProjection {
        scenario: ScenarioProjection {
            name: summary.scenario_name,
            year: summary.year,
            day_of_year: summary.day_of_year,
            elapsed_days: summary.elapsed_days,
            phase: summary.phase,
            average_food_satisfaction_basis_points: summary.average_food_satisfaction_basis_points,
            active_crises: summary.active_crises,
        },
        player,
        dynasties,
        districts: build_district_projections(registry, state),
        businesses: build_business_projections(registry, state),
        market: build_market_projections(registry, state),
        contracts: build_contract_projections(registry, state),
        loans: build_loan_projections(state),
        civic_debts: build_civic_debt_projections(state),
        properties: build_property_projections(registry, state),
        institutions: build_institution_projections(registry, state),
        laws: build_law_projections(state),
        public_works: build_public_work_projections(registry, state),
        legal_cases: build_legal_case_projections(state),
        crises: build_crisis_projections(registry, state),
        relationships: build_relationship_projections(state),
        information: state
            .information_reports
            .values()
            .filter(|report| report.owner_dynasty_id == state.player_dynasty_id)
            .map(|report| InformationProjection {
                target: report.target,
                subject: report.subject.clone(),
                confidence: report.confidence,
                source: report.source.clone(),
                summary: report.summary.clone(),
                created_day: report.created_day,
                expires_day: report.expires_day,
            })
            .collect(),
        notifications: state
            .outbox
            .iter()
            .rev()
            .take(50)
            .rev()
            .map(|message| NotificationProjection {
                day: message.day,
                kind: message.kind,
                subject: message.subject.clone(),
                body: message.body.clone(),
                acknowledged: message.acknowledged,
            })
            .collect(),
    }
}

fn build_relationship_projections(state: &AppState) -> Vec<RelationshipProjection> {
    state
        .relationships
        .values()
        .filter_map(|relationship| {
            let other_dynasty_id = if relationship.pair.first == state.player_dynasty_id {
                relationship.pair.second
            } else if relationship.pair.second == state.player_dynasty_id {
                relationship.pair.first
            } else {
                return None;
            };
            let dynasty = state
                .dynasties
                .get(&other_dynasty_id)
                .expect("relationship dynasty must exist");
            Some(RelationshipProjection {
                dynasty_id: other_dynasty_id,
                dynasty_name: dynasty.name().to_owned(),
                trust_basis_points: relationship.trust_basis_points,
                respect_basis_points: relationship.respect_basis_points,
                fear_basis_points: relationship.fear_basis_points,
                resentment_basis_points: relationship.resentment_basis_points,
                obligation: relationship.obligation,
                last_interaction_day: relationship.last_interaction_day,
                memories: relationship.memories.clone(),
            })
        })
        .collect()
}

fn build_dynasty_projection(
    registry: &Registry,
    state: &AppState,
    dynasty_id: DynastyId,
) -> DynastyProjection {
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .expect("projection dynasty must exist");
    let offices = state
        .institutions
        .values()
        .filter_map(|institution| {
            let holder = state.characters.get(institution.office_holder_id?)?;
            if holder.dynasty_id() != dynasty_id {
                return None;
            }
            registry
                .get_institution(institution.institution_id)
                .map(|definition| definition.name().to_owned())
        })
        .collect();
    let active_objective = state
        .ai_objectives
        .values()
        .filter(|objective| {
            objective.dynasty_id == dynasty_id && objective.status == ObjectiveStatus::Pursuing
        })
        .max_by_key(|objective| (objective.priority, std::cmp::Reverse(objective.id)))
        .map(|objective| ObjectiveProjection {
            kind: objective.kind,
            status: objective.status,
            priority: objective.priority,
            rationale: objective.rationale.clone(),
        });
    let office_administrative_load = dynasty_office_administrative_load(state, dynasty_id);
    DynastyProjection {
        id: dynasty_id,
        name: dynasty.name().to_owned(),
        treasury: dynasty.treasury(),
        civic_contributions: dynasty.civic_contributions(),
        unmet_office_duties: dynasty.unmet_office_duties(),
        legitimacy_basis_points: dynasty.resources.legitimacy_basis_points,
        reputation_quality_basis_points: dynasty.resources.reputation_quality_basis_points,
        reputation_reliability_basis_points: dynasty.resources.reputation_reliability_basis_points,
        administrative_capacity: dynasty.administrative_capacity(),
        administrative_load: dynasty.administrative_load(),
        office_administrative_load,
        effective_administrative_load: dynasty
            .administrative_load()
            .saturating_add(office_administrative_load),
        generation: dynasty.runtime.generation,
        properties: state
            .properties
            .values()
            .filter(|property| property.owner_dynasty_id == Some(dynasty_id))
            .count(),
        businesses: state
            .businesses
            .ids_for_owner(dynasty_id)
            .map_or(0, std::collections::BTreeSet::len),
        current_loans_as_borrower: state
            .loans
            .values()
            .filter(|loan| {
                loan.borrower_dynasty_id == dynasty_id && loan.status.is_repayment_active()
            })
            .count(),
        offices,
        active_objective,
    }
}

fn build_district_projections(registry: &Registry, state: &AppState) -> Vec<DistrictProjection> {
    registry
        .districts()
        .iter()
        .map(|definition| {
            let runtime = state
                .districts
                .get(&definition.id())
                .expect("district runtime must exist");
            let households = state
                .households
                .ids_for_district(definition.id())
                .into_iter()
                .flatten()
                .filter_map(|id| state.households.get(*id))
                .collect::<Vec<_>>();
            let food = crate::core::population_weighted_food_satisfaction_basis_points(
                households.iter().copied(),
            )
            .unwrap_or(0);
            DistrictProjection {
                id: definition.id(),
                name: definition.name().to_owned(),
                population: definition.population(),
                rent_index_basis_points: runtime.rent_index_basis_points,
                employment_basis_points: runtime.employment_basis_points,
                sanitation_basis_points: runtime.sanitation_basis_points,
                safety_basis_points: runtime.safety_basis_points,
                unrest_basis_points: runtime.unrest_basis_points,
                food_satisfaction_basis_points: food,
                businesses: state
                    .businesses
                    .ids_for_district(definition.id())
                    .map_or(0, std::collections::BTreeSet::len),
                properties: state
                    .properties
                    .values()
                    .filter(|property| property.district_id == definition.id())
                    .count(),
                causes: district_causes(runtime, food),
            }
        })
        .collect()
}

fn district_causes(runtime: &crate::core::DistrictRuntime, food: u16) -> Vec<String> {
    let mut causes = Vec::new();
    if food < 4_000 {
        causes.push("Household food access is poor".to_owned());
    }
    if runtime.sanitation_basis_points < 5_000 {
        causes.push("Sanitation infrastructure is inadequate".to_owned());
    }
    if runtime.safety_basis_points < 5_000 {
        causes.push("Public safety is weak".to_owned());
    }
    if runtime.employment_basis_points < 4_000 {
        causes.push("Formal employment is scarce".to_owned());
    }
    if runtime.unrest_basis_points > 5_000 {
        causes.push("Material hardship and organization are driving unrest".to_owned());
    }
    if causes.is_empty() {
        causes.push("Conditions are broadly stable".to_owned());
    }
    causes
}

fn build_business_projections(registry: &Registry, state: &AppState) -> Vec<BusinessProjection> {
    state
        .businesses
        .iter()
        .map(|business| {
            let owner = state
                .dynasties
                .get(&business.owner_dynasty_id())
                .expect("business owner must exist");
            let district = registry
                .get_district(business.district_id())
                .expect("business district must exist");
            let recipe = registry
                .get_recipe(business.recipe_id())
                .expect("business recipe must exist");
            let manager = state
                .characters
                .get(business.manager_id())
                .expect("business manager must exist");
            let inventory = business
                .inventory()
                .iter()
                .map(|(good_id, quantity)| BusinessInventoryProjection {
                    good: registry
                        .get_good(*good_id)
                        .expect("business inventory good must exist")
                        .name()
                        .to_owned(),
                    quantity: *quantity,
                })
                .collect();
            let acquisition =
                quote_business_acquisition(registry, state, state.player_dynasty_id, business.id())
                    .ok()
                    .map(|quote| BusinessAcquisitionProjection {
                        purchase_price: quote.purchase_price,
                        minimum_recapitalization: quote.minimum_recapitalization,
                    });
            BusinessProjection {
                id: business.id(),
                name: business.name().to_owned(),
                owner: owner.name().to_owned(),
                district: district.name().to_owned(),
                recipe: recipe.name().to_owned(),
                manager: manager.name().to_owned(),
                status: business.status(),
                cash: business.cash(),
                capacity_batches_per_day: business.operations.capacity_batches_per_day,
                condition_basis_points: business.operations.condition_basis_points,
                quality_basis_points: business.operations.quality_basis_points,
                target_input_days: business.policy.target_input_days,
                target_output_days: business.policy.target_output_days,
                minimum_cash_reserve: business.policy.minimum_cash_reserve,
                maintenance_basis_points: business.policy.maintenance_basis_points,
                quality_target_basis_points: business.policy.quality_target_basis_points,
                inventory,
                acquisition,
            }
        })
        .collect()
}

fn build_market_projections(registry: &Registry, state: &AppState) -> Vec<MarketProjection> {
    registry
        .goods()
        .iter()
        .map(|good| {
            let quote = state
                .market
                .get_quote(good.id())
                .expect("market quote must exist");
            MarketProjection {
                good: good.name().to_owned(),
                price: quote.price,
                previous_price: quote.previous_price,
                stock: quote.stock,
                target_stock: quote.target_stock,
                demand_today: quote.demand_today,
                supply_today: quote.supply_today,
                causes: quote.causes.clone(),
            }
        })
        .collect()
}

fn build_contract_projections(registry: &Registry, state: &AppState) -> Vec<ContractProjection> {
    state
        .contracts
        .values()
        .map(|contract| ContractProjection {
            id: contract.id,
            buyer_business_id: contract.buyer_business_id,
            buyer_name: state
                .businesses
                .get(contract.buyer_business_id)
                .expect("contract buyer must exist")
                .name()
                .to_owned(),
            seller_business_id: contract.seller_business_id,
            seller_name: state
                .businesses
                .get(contract.seller_business_id)
                .expect("contract seller must exist")
                .name()
                .to_owned(),
            good: registry
                .get_good(contract.good_id)
                .expect("contract good must exist")
                .name()
                .to_owned(),
            quantity_per_week: contract.quantity_per_week,
            unit_price: contract.unit_price,
            penalty: contract.penalty,
            next_due_day: contract.next_due_day,
            end_day: contract.end_day,
            status: contract.status,
            fulfilled_deliveries: contract.fulfilled_deliveries,
            delivery_credits: contract
                .fulfilled_deliveries_by_dynasty
                .iter()
                .map(
                    |(dynasty_id, deliveries)| ContractDeliveryCreditProjection {
                        dynasty_id: *dynasty_id,
                        dynasty: state
                            .dynasties
                            .get(dynasty_id)
                            .expect("credited contract dynasty must exist")
                            .name()
                            .to_owned(),
                        deliveries: *deliveries,
                    },
                )
                .collect(),
            missed_deliveries: contract.missed_deliveries,
            breaching_dynasty_id: contract.breaching_dynasty_id,
            breaching_dynasty: contract.breaching_dynasty_id.map(|dynasty_id| {
                state
                    .dynasties
                    .get(&dynasty_id)
                    .expect("contract breaching dynasty must exist")
                    .name()
                    .to_owned()
            }),
            breach_victim_dynasty_id: contract.breach_victim_dynasty_id,
            breach_victim_dynasty: contract.breach_victim_dynasty_id.map(|dynasty_id| {
                state
                    .dynasties
                    .get(&dynasty_id)
                    .expect("contract breach victim dynasty must exist")
                    .name()
                    .to_owned()
            }),
            unpaid_breach_penalty: contract.unpaid_breach_penalty,
        })
        .collect()
}

fn build_loan_projections(state: &AppState) -> Vec<LoanProjection> {
    state
        .loans
        .values()
        .map(|loan| LoanProjection {
            id: loan.id,
            lender: state
                .dynasties
                .get(&loan.lender_dynasty_id)
                .expect("loan lender must exist")
                .name()
                .to_owned(),
            borrower: state
                .dynasties
                .get(&loan.borrower_dynasty_id)
                .expect("loan borrower must exist")
                .name()
                .to_owned(),
            principal: loan.principal,
            balance: loan.balance,
            weekly_payment: loan.weekly_payment,
            interest_basis_points: loan.interest_basis_points,
            status: loan.status,
            collateral_property_id: loan.collateral_property_id,
        })
        .collect()
}

fn build_civic_debt_projections(state: &AppState) -> Vec<CivicDebtProjection> {
    state
        .civic_debts
        .values()
        .map(|debt| CivicDebtProjection {
            id: debt.id,
            creditor_dynasty_id: debt.creditor_dynasty_id,
            creditor: state
                .dynasties
                .get(&debt.creditor_dynasty_id)
                .expect("civic debt creditor must exist")
                .name()
                .to_owned(),
            sponsor: debt.sponsor_dynasty_id.map(|dynasty_id| {
                state
                    .dynasties
                    .get(&dynasty_id)
                    .expect("civic debt sponsor must exist")
                    .name()
                    .to_owned()
            }),
            authorizing_law_id: debt.authorizing_law_id,
            principal: debt.principal,
            balance: debt.balance,
            weekly_payment: debt.weekly_payment,
            interest_basis_points: debt.interest_basis_points,
            issued_day: debt.issued_day,
            next_due_day: debt.next_due_day,
            missed_payments: debt.missed_payments,
            status: debt.status,
        })
        .collect()
}

fn build_property_projections(registry: &Registry, state: &AppState) -> Vec<PropertyProjection> {
    state
        .properties
        .values()
        .map(|property| PropertyProjection {
            id: property.id,
            name: property.name.clone(),
            district: registry
                .get_district(property.district_id)
                .expect("property district must exist")
                .name()
                .to_owned(),
            owner: property.owner_dynasty_id.map(|dynasty_id| {
                state
                    .dynasties
                    .get(&dynasty_id)
                    .expect("property owner must exist")
                    .name()
                    .to_owned()
            }),
            value: property.value,
            weekly_rent: property.weekly_rent,
            effective_weekly_rent: effective_property_weekly_rent(state, property),
            district_rent_index_basis_points: state
                .districts
                .get(&property.district_id)
                .expect("property district runtime must exist")
                .rent_index_basis_points,
            condition_basis_points: property.condition_basis_points,
            occupied_business_id: property.occupant_business_id,
        })
        .collect()
}

fn build_institution_projections(
    registry: &Registry,
    state: &AppState,
) -> Vec<InstitutionProjection> {
    state
        .institutions
        .values()
        .map(|institution| {
            let holder = institution
                .office_holder_id
                .and_then(|id| state.characters.get(id));
            InstitutionProjection {
                id: institution.institution_id,
                name: registry
                    .get_institution(institution.institution_id)
                    .expect("institution definition must exist")
                    .name()
                    .to_owned(),
                officeholder: holder.map(|character| character.name().to_owned()),
                officeholder_dynasty: holder.map(|character| {
                    state
                        .dynasties
                        .get(&character.dynasty_id())
                        .expect("officeholder dynasty must exist")
                        .name()
                        .to_owned()
                }),
                budget: institution.budget,
                legitimacy_basis_points: institution.legitimacy_basis_points,
                term_started_day: institution.term_started_day,
                next_selection_day: institution.next_selection_day,
                powers: institution
                    .powers
                    .iter()
                    .map(|power| format!("{power:?}"))
                    .collect(),
            }
        })
        .collect()
}

fn build_law_projections(state: &AppState) -> Vec<LawProjection> {
    state
        .laws
        .values()
        .map(|law| LawProjection {
            id: law.id,
            kind: law.kind,
            value: law.value,
            enacted_day: law.enacted_day,
            sponsor: law.sponsor_dynasty_id.map(|dynasty_id| {
                state
                    .dynasties
                    .get(&dynasty_id)
                    .expect("law sponsor must exist")
                    .name()
                    .to_owned()
            }),
            active: law.active,
        })
        .collect()
}

fn build_public_work_projections(
    registry: &Registry,
    state: &AppState,
) -> Vec<PublicWorkProjection> {
    state
        .public_works
        .values()
        .map(|work| PublicWorkProjection {
            id: work.id,
            district: registry
                .get_district(work.district_id)
                .expect("public work district must exist")
                .name()
                .to_owned(),
            kind: work.kind,
            budget: work.budget,
            spent: work.spent,
            progress_basis_points: work.progress_basis_points,
            status: work.status,
        })
        .collect()
}

fn build_legal_case_projections(state: &AppState) -> Vec<LegalCaseProjection> {
    state
        .legal_cases
        .values()
        .map(|case| LegalCaseProjection {
            id: case.id,
            plaintiff: state
                .dynasties
                .get(&case.plaintiff_dynasty_id)
                .expect("legal plaintiff must exist")
                .name()
                .to_owned(),
            defendant: state
                .dynasties
                .get(&case.defendant_dynasty_id)
                .expect("legal defendant must exist")
                .name()
                .to_owned(),
            kind: format!("{:?}", case.kind),
            claim_source: case.claim_source,
            evidence_basis_points: case.evidence_basis_points,
            hearing_day: case.hearing_day,
            damages: case.damages,
            status: case.status,
        })
        .collect()
}

fn build_crisis_projections(registry: &Registry, state: &AppState) -> Vec<CrisisProjection> {
    state
        .crises
        .values()
        .map(|crisis| CrisisProjection {
            id: crisis.id,
            kind: crisis.kind,
            district: crisis.district_id.map(|district_id| {
                registry
                    .get_district(district_id)
                    .expect("crisis district must exist")
                    .name()
                    .to_owned()
            }),
            started_day: crisis.started_day,
            severity_basis_points: crisis.severity_basis_points,
            status: crisis.status,
            cause: crisis.cause.clone(),
        })
        .collect()
}

/// Renders a self-contained campaign dashboard with no external assets or scripts.
///
/// # Errors
///
/// Returns a serialization error if the projection cannot be encoded for the embedded data block.
///
/// # Panics
///
/// Panics when runtime references violate the invariants required by the projection.
pub fn render_campaign_html(
    registry: &Registry,
    state: &AppState,
) -> Result<String, serde_json::Error> {
    let projection = build_campaign_projection(registry, state);
    let data = escape_json_for_html_script(&serde_json::to_string_pretty(&projection)?);
    let player = &projection.player;
    let district_rows = render_district_rows(&projection.districts);
    let business_rows = render_business_rows(&projection.businesses);
    let contract_rows = render_contract_rows(&projection.contracts);
    let market_rows = render_market_rows(&projection.market);
    let civic_debt_rows = render_civic_debt_rows(&projection.civic_debts);
    let relationship_rows = render_relationship_rows(&projection.relationships);
    let alerts = render_notifications(&projection.notifications);
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Civic Dynasty · {scenario}</title>
<style>
:root{{color-scheme:dark;--bg:#12100d;--panel:#211c17;--line:#493c30;--text:#eee6da;--muted:#b8aa99;--accent:#d4a75e}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 Georgia,serif}}header,main{{max-width:1200px;margin:auto;padding:24px}}header{{border-bottom:1px solid var(--line)}}h1,h2,h3{{margin:.2em 0}}small{{color:var(--muted)}}.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:16px}}section,article{{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:16px}}.metric{{font-size:1.6rem;color:var(--accent)}}table{{width:100%;border-collapse:collapse}}th,td{{padding:8px;border-bottom:1px solid var(--line);text-align:left;vertical-align:top}}.scroll{{overflow:auto}}pre{{white-space:pre-wrap;color:var(--muted)}}.sr-only{{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}}
</style>
</head>
<body>
<header><h1>{scenario}</h1><p>Year {year}, day {day} · simulation day {elapsed} · {phase}</p></header>
<main>
<div class="grid">
<section><small>Player dynasty</small><h2>House {player}</h2><div class="metric">{treasury}</div><p>Administrative load {load}/{capacity}, including {office_load} from offices</p><p>{contributions} in civic duties · {unmet_duties} unmet duties</p></section>
<section><small>Commercial position</small><div class="metric">{businesses} businesses</div><p>{properties} properties · {loans} current borrowing relationships</p></section>
<section><small>Municipal finance</small><div class="metric">{civic_debt_balance}</div><p>{civic_debts} outstanding civic obligations</p></section>
<section><small>Civic condition</small><div class="metric">{food:.1}% food satisfaction</div><p>{crises} active crises</p></section>
</div>
<h2>Businesses</h2><section class="scroll"><table><caption class="sr-only">Business operations</caption><thead><tr><th scope="col">Business</th><th scope="col">Owner</th><th scope="col">Status</th><th scope="col">Cash</th><th scope="col">Condition</th><th scope="col">Policy</th><th scope="col">Manager</th><th scope="col">Acquisition</th></tr></thead><tbody>{business_rows}</tbody></table></section>
<h2>Supply contracts</h2><section class="scroll"><table><caption class="sr-only">Supply contract obligations and performance</caption><thead><tr><th scope="col">Contract</th><th scope="col">Buyer</th><th scope="col">Seller</th><th scope="col">Good</th><th scope="col">Terms</th><th scope="col">Status</th><th scope="col">Performance</th><th scope="col">Delivery credit</th><th scope="col">Breach</th></tr></thead><tbody>{contract_rows}</tbody></table></section>
<h2>Districts</h2><section class="scroll"><table><caption class="sr-only">District conditions</caption><thead><tr><th scope="col">District</th><th scope="col">Food</th><th scope="col">Employment</th><th scope="col">Sanitation</th><th scope="col">Unrest</th><th scope="col">Causes</th></tr></thead><tbody>{district_rows}</tbody></table></section>
<h2>Market</h2><section class="scroll"><table><caption class="sr-only">Market prices and stocks</caption><thead><tr><th scope="col">Good</th><th scope="col">Price</th><th scope="col">Stock</th><th scope="col">Causes</th></tr></thead><tbody>{market_rows}</tbody></table></section>
<h2>Municipal debt</h2><section class="scroll"><table><caption class="sr-only">Municipal debt obligations</caption><thead><tr><th scope="col">Creditor</th><th scope="col">Principal</th><th scope="col">Balance</th><th scope="col">Weekly payment</th><th scope="col">Interest</th><th scope="col">Status</th><th scope="col">Next due</th></tr></thead><tbody>{civic_debt_rows}</tbody></table></section>
<h2>Dynasty relationships</h2><section class="scroll"><table><caption class="sr-only">Dynasty relationship measures</caption><thead><tr><th scope="col">House</th><th scope="col">Trust</th><th scope="col">Respect</th><th scope="col">Fear</th><th scope="col">Resentment</th><th scope="col">Obligation</th><th scope="col">Last interaction</th></tr></thead><tbody>{relationship_rows}</tbody></table></section>
<h2>Recent notices</h2><div class="grid">{alerts}</div>
<h2>Embedded projection</h2><section><pre id="data"></pre></section>
</main>
<script type="application/json" id="campaign-data">{data}</script>
<script>document.getElementById('data').textContent=document.getElementById('campaign-data').textContent;</script>
</body></html>"#,
        scenario = escape_html(&projection.scenario.name),
        year = projection.scenario.year,
        day = projection.scenario.day_of_year,
        elapsed = projection.scenario.elapsed_days,
        phase = projection.scenario.phase.label(),
        player = escape_html(&player.name),
        treasury = player.treasury,
        load = player.effective_administrative_load,
        office_load = player.office_administrative_load,
        capacity = player.administrative_capacity,
        contributions = player.civic_contributions,
        unmet_duties = player.unmet_office_duties,
        businesses = player.businesses,
        properties = player.properties,
        loans = player.current_loans_as_borrower,
        civic_debts = projection
            .civic_debts
            .iter()
            .filter(|debt| debt.status != CivicDebtStatus::Repaid)
            .count(),
        civic_debt_balance = projection
            .civic_debts
            .iter()
            .filter(|debt| debt.status != CivicDebtStatus::Repaid)
            .fold(Money::ZERO, |total, debt| total
                .saturating_add(debt.balance)),
        food = f64::from(projection.scenario.average_food_satisfaction_basis_points) / 100.0,
        crises = projection.scenario.active_crises,
    ))
}

fn render_civic_debt_rows(debts: &[CivicDebtProjection]) -> String {
    if debts.is_empty() {
        return "<tr><td colspan=\"7\">No municipal debt has been issued.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for debt in debts {
        let next_due = match debt.status {
            CivicDebtStatus::Current | CivicDebtStatus::Delinquent => {
                format!("day {}", debt.next_due_day)
            }
            CivicDebtStatus::Defaulted | CivicDebtStatus::Repaid => "none".to_owned(),
        };
        write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.1}%</td><td>{}</td><td>{}</td></tr>",
            escape_html(&debt.creditor),
            debt.principal,
            debt.balance,
            debt.weekly_payment,
            f64::from(debt.interest_basis_points) / 100.0,
            civic_debt_status_label(debt.status),
            next_due,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_relationship_rows(relationships: &[RelationshipProjection]) -> String {
    if relationships.is_empty() {
        return "<tr><td colspan=\"7\">No dynasty relationships are available.</td></tr>"
            .to_owned();
    }
    let mut rows = String::new();
    for relationship in relationships {
        write!(
            rows,
            "<tr><td>{}</td><td>{:.1}%</td><td>{:.1}%</td><td>{:.1}%</td><td>{:.1}%</td><td>{}</td><td>day {}</td></tr>",
            escape_html(&relationship.dynasty_name),
            f64::from(relationship.trust_basis_points) / 100.0,
            f64::from(relationship.respect_basis_points) / 100.0,
            f64::from(relationship.fear_basis_points) / 100.0,
            f64::from(relationship.resentment_basis_points) / 100.0,
            relationship.obligation,
            relationship.last_interaction_day,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_business_rows(businesses: &[BusinessProjection]) -> String {
    if businesses.is_empty() {
        return "<tr><td colspan=\"8\">No businesses are operating.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for business in businesses {
        let acquisition = business.acquisition.map_or_else(
            || "—".to_owned(),
            |quote| {
                format!(
                    "{} + {} working capital",
                    quote.purchase_price, quote.minimum_recapitalization
                )
            },
        );
        write!(
            rows,
            "<tr><td>{}<br><small>{} · {}</small></td><td>{}</td><td>{}</td><td>{}</td><td>{:.1}%</td><td>inputs {}d · outputs {}d · reserve {} · maintenance {:.1}% · quality {:.1}%</td><td>{}</td><td>{}</td></tr>",
            escape_html(&business.name),
            escape_html(&business.district),
            escape_html(&business.recipe),
            escape_html(&business.owner),
            business_status_label(business.status),
            business.cash,
            f64::from(business.condition_basis_points) / 100.0,
            business.target_input_days,
            business.target_output_days,
            business.minimum_cash_reserve,
            f64::from(business.maintenance_basis_points) / 100.0,
            f64::from(business.quality_target_basis_points) / 100.0,
            escape_html(&business.manager),
            escape_html(&acquisition),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_contract_rows(contracts: &[ContractProjection]) -> String {
    if contracts.is_empty() {
        return "<tr><td colspan=\"9\">No supply contracts are recorded.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for contract in contracts {
        let next_due = if contract.status == ContractStatus::Active {
            format!("next due day {}", contract.next_due_day)
        } else {
            "no further delivery due".to_owned()
        };
        let delivery_credits = if contract.delivery_credits.is_empty() {
            "None yet".to_owned()
        } else {
            contract
                .delivery_credits
                .iter()
                .map(|credit| format!("{}: {}", escape_html(&credit.dynasty), credit.deliveries))
                .collect::<Vec<_>>()
                .join("<br>")
        };
        let breach = contract
            .breaching_dynasty
            .as_deref()
            .map_or_else(|| "—".to_owned(), escape_html);
        write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{} weekly at {} each<br><small>through day {} · penalty {} · {}</small></td><td>{}</td><td>{} fulfilled · {} missed</td><td>{}</td><td>{}</td></tr>",
            contract.id,
            escape_html(&contract.buyer_name),
            escape_html(&contract.seller_name),
            escape_html(&contract.good),
            contract.quantity_per_week,
            contract.unit_price,
            contract.end_day,
            contract.penalty,
            next_due,
            contract_status_label(contract.status),
            contract.fulfilled_deliveries,
            contract.missed_deliveries,
            delivery_credits,
            breach,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_district_rows(districts: &[DistrictProjection]) -> String {
    if districts.is_empty() {
        return "<tr><td colspan=\"6\">No district records are available.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for district in districts {
        write!(
            rows,
            "<tr><td>{}</td><td>{:.1}%</td><td>{:.1}%</td><td>{:.1}%</td><td>{:.1}%</td><td>{}</td></tr>",
            escape_html(&district.name),
            f64::from(district.food_satisfaction_basis_points) / 100.0,
            f64::from(district.employment_basis_points) / 100.0,
            f64::from(district.sanitation_basis_points) / 100.0,
            f64::from(district.unrest_basis_points) / 100.0,
            escape_html(&district.causes.join("; ")),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_market_rows(markets: &[MarketProjection]) -> String {
    if markets.is_empty() {
        return "<tr><td colspan=\"4\">No market quotes are available.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for market in markets {
        let causes = market
            .causes
            .iter()
            .map(market_cause_label)
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(&market.good),
            market.price,
            market.stock,
            causes,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_notifications(notifications: &[NotificationProjection]) -> String {
    if notifications.is_empty() {
        return "<article><p>No recent notices.</p></article>".to_owned();
    }
    let mut alerts = String::new();
    for notification in notifications.iter().rev().take(12) {
        write!(
            alerts,
            "<article><small>Day {} · {}</small><h3>{}</h3><p>{}</p></article>",
            notification.day,
            outbox_kind_label(notification.kind),
            escape_html(&notification.subject),
            escape_html(&notification.body),
        )
        .expect("writing HTML into a String cannot fail");
    }
    alerts
}

const fn business_status_label(status: BusinessStatus) -> &'static str {
    match status {
        BusinessStatus::Active => "Active",
        BusinessStatus::Distressed => "Distressed",
        BusinessStatus::Insolvent => "Insolvent",
        BusinessStatus::Closed => "Closed",
    }
}

const fn civic_debt_status_label(status: CivicDebtStatus) -> &'static str {
    match status {
        CivicDebtStatus::Current => "Current",
        CivicDebtStatus::Delinquent => "Delinquent",
        CivicDebtStatus::Defaulted => "Defaulted",
        CivicDebtStatus::Repaid => "Repaid",
    }
}

const fn contract_status_label(status: ContractStatus) -> &'static str {
    match status {
        ContractStatus::Active => "Active",
        ContractStatus::Fulfilled => "Fulfilled",
        ContractStatus::Breached => "Breached",
        ContractStatus::Renegotiated => "Renegotiated",
        ContractStatus::Cancelled => "Cancelled",
    }
}

const fn market_cause_label(cause: &MarketCause) -> &'static str {
    match cause {
        MarketCause::StockBelowTarget => "Stock below target",
        MarketCause::StockAboveTarget => "Stock above target",
        MarketCause::DemandExceededSupply => "Demand exceeded supply",
        MarketCause::SupplyExceededDemand => "Supply exceeded demand",
        MarketCause::SeasonalPressure => "Seasonal pressure",
        MarketCause::StableConditions => "Stable conditions",
    }
}

const fn outbox_kind_label(kind: OutboxKind) -> &'static str {
    match kind {
        OutboxKind::Contract => "Contract",
        OutboxKind::Finance => "Finance",
        OutboxKind::Property => "Property",
        OutboxKind::Family => "Family",
        OutboxKind::Politics => "Politics",
        OutboxKind::Law => "Law",
        OutboxKind::District => "District",
        OutboxKind::Legal => "Legal",
        OutboxKind::Crisis => "Crisis",
        OutboxKind::Information => "Information",
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn escape_json_for_html_script(value: &str) -> String {
    value
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
