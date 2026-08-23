//! Read-only causal projections and a self-contained HTML campaign dashboard.

use crate::core::{
    AppState, BusinessStatus, CampaignPhase, CivicDebtStatus, ContractStatus, CrisisKind,
    CrisisStatus, EmploymentStatus, HouseGovernance, InformationConfidence, InformationTarget,
    LawKind, LegalCaseKind, LegalCaseStatus, LegalClaimSource, LoanStatus, MarketCause,
    ObjectiveKind, ObjectiveStatus, OfficePower, OutboxKind, PublicWorkKind, PublicWorkStatus,
};
use crate::ids::{
    BusinessId, CivicDebtId, ContractId, CrisisId, DistrictId, DynastyId, EmploymentId,
    InformationReportId, InstitutionId, LawId, LegalCaseId, LoanId, OutboxMessageId, PropertyId,
    PublicWorkId,
};
use crate::money::{Money, Quantity};
use crate::registry::Registry;
use crate::systems::{
    dynasty_office_administrative_load, effective_property_weekly_rent, quote_business_acquisition,
    quote_player_legal_settlement,
};
use serde::Serialize;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

// `build_state_summary` is the single read-model entry point; no convenience
// wrapper duplicates it.

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CampaignProjection {
    pub scenario: ScenarioProjection,
    pub player: DynastyProjection,
    pub family: FamilyProjection,
    pub dynasties: Vec<DynastyProjection>,
    pub districts: Vec<DistrictProjection>,
    pub businesses: Vec<BusinessProjection>,
    pub employment: Vec<EmploymentProjection>,
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
    /// Total unread outbox messages, counted across the whole history rather
    /// than only the recent notification window surfaced above.
    pub unread_notifications: usize,
    /// Conditions that need the player's attention. The projection layer owns
    /// this classification once so every read view (CLI summary, dashboard)
    /// reports the same set of flags instead of re-deriving its own rules.
    pub attention: Vec<AttentionItem>,
}

/// Severity of an [`AttentionItem`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum AttentionTone {
    Urgent,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AttentionItem {
    pub tone: AttentionTone,
    pub category: String,
    pub title: String,
    pub detail: String,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FamilyProjection {
    pub governance: HouseGovernance,
    pub unity_basis_points: u16,
    pub head_id: crate::ids::CharacterId,
    pub head: String,
    pub heir_id: Option<crate::ids::CharacterId>,
    pub heir: Option<String>,
    pub members: usize,
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
    pub owner_dynasty_id: DynastyId,
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
pub struct EmploymentProjection {
    pub id: EmploymentId,
    pub business_id: BusinessId,
    pub business: String,
    pub owner_dynasty_id: DynastyId,
    pub workers: u16,
    pub weekly_wage: Money,
    pub loyalty_basis_points: u16,
    pub conditions_basis_points: u16,
    pub status: EmploymentStatus,
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
    pub lender_dynasty_id: DynastyId,
    pub lender: String,
    pub borrower_dynasty_id: DynastyId,
    pub borrower: String,
    pub principal: Money,
    pub balance: Money,
    pub weekly_payment: Money,
    pub interest_basis_points: u16,
    pub next_due_day: i64,
    pub missed_payments: u16,
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
    pub owner_dynasty_id: Option<DynastyId>,
    pub owner: Option<String>,
    pub value: Money,
    pub weekly_rent: Money,
    pub effective_weekly_rent: Money,
    pub district_rent_index_basis_points: u16,
    pub condition_basis_points: u16,
    pub occupied_business_id: Option<BusinessId>,
    pub tenant_dynasty_id: Option<DynastyId>,
    pub tenant: Option<String>,
    pub collateral_loan_id: Option<LoanId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InstitutionProjection {
    pub id: InstitutionId,
    pub name: String,
    pub officeholder: Option<String>,
    pub officeholder_dynasty_id: Option<DynastyId>,
    pub officeholder_dynasty: Option<String>,
    pub budget: Money,
    pub legitimacy_basis_points: u16,
    pub term_started_day: i64,
    pub next_selection_day: i64,
    pub powers: Vec<OfficePower>,
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
    pub sponsor_dynasty_id: Option<DynastyId>,
    pub sponsor: Option<String>,
    pub budget: Money,
    pub spent: Money,
    pub progress_basis_points: u16,
    pub status: PublicWorkStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LegalCaseProjection {
    pub id: LegalCaseId,
    pub plaintiff_dynasty_id: DynastyId,
    pub plaintiff: String,
    pub defendant_dynasty_id: DynastyId,
    pub defendant: String,
    pub kind: LegalCaseKind,
    pub claim_source: Option<LegalClaimSource>,
    pub evidence_basis_points: u16,
    pub hearing_day: i64,
    pub damages: Money,
    pub status: LegalCaseStatus,
    pub settlement_amount: Option<Money>,
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
    pub id: InformationReportId,
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
    pub id: OutboxMessageId,
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
    let mut projection = CampaignProjection {
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
        family: build_family_projection(state),
        dynasties,
        districts: build_district_projections(registry, state),
        businesses: build_business_projections(registry, state),
        employment: build_employment_projections(state),
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
                id: report.id,
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
                id: message.id,
                day: message.day,
                kind: message.kind,
                subject: message.subject.clone(),
                body: message.body.clone(),
                acknowledged: message.acknowledged,
            })
            .collect(),
        unread_notifications: state
            .outbox
            .iter()
            .filter(|message| !message.acknowledged)
            .count(),
        attention: Vec::new(),
    };
    projection.attention = build_attention_items(&projection);
    projection
}

fn build_family_projection(state: &AppState) -> FamilyProjection {
    let dynasty = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let council = state
        .family_councils
        .get(&state.player_dynasty_id)
        .expect("player family council must exist");
    let head_id = dynasty.head_id();
    let head = state
        .characters
        .get(head_id)
        .expect("player dynasty head must exist")
        .name()
        .to_owned();
    let heir_id = dynasty.heir_id();
    let heir = heir_id.map(|character_id| {
        state
            .characters
            .get(character_id)
            .expect("player dynasty heir must exist")
            .name()
            .to_owned()
    });
    FamilyProjection {
        governance: council.governance,
        unity_basis_points: council.unity_basis_points,
        head_id,
        head,
        heir_id,
        heir,
        members: council.members.len(),
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
                owner_dynasty_id: business.owner_dynasty_id(),
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

fn build_employment_projections(state: &AppState) -> Vec<EmploymentProjection> {
    state
        .employment
        .values()
        .map(|agreement| {
            let business = state
                .businesses
                .get(agreement.business_id)
                .expect("employment business must exist");
            EmploymentProjection {
                id: agreement.id,
                business_id: agreement.business_id,
                business: business.name().to_owned(),
                owner_dynasty_id: business.owner_dynasty_id(),
                workers: agreement.workers,
                weekly_wage: agreement.weekly_wage,
                loyalty_basis_points: agreement.loyalty_basis_points,
                conditions_basis_points: agreement.conditions_basis_points,
                status: agreement.status,
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
            lender_dynasty_id: loan.lender_dynasty_id,
            lender: state
                .dynasties
                .get(&loan.lender_dynasty_id)
                .expect("loan lender must exist")
                .name()
                .to_owned(),
            borrower_dynasty_id: loan.borrower_dynasty_id,
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
            next_due_day: loan.next_due_day,
            missed_payments: loan.missed_payments,
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
            owner_dynasty_id: property.owner_dynasty_id,
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
            tenant_dynasty_id: property.tenant_dynasty_id,
            tenant: property.tenant_dynasty_id.map(|dynasty_id| {
                state
                    .dynasties
                    .get(&dynasty_id)
                    .expect("property tenant must exist")
                    .name()
                    .to_owned()
            }),
            collateral_loan_id: property.collateral_loan_id,
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
                officeholder_dynasty_id: holder.map(crate::core::Character::dynasty_id),
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
                powers: institution.powers.iter().copied().collect(),
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
            sponsor_dynasty_id: work.sponsor_dynasty_id,
            sponsor: work.sponsor_dynasty_id.map(|dynasty_id| {
                state
                    .dynasties
                    .get(&dynasty_id)
                    .expect("public work sponsor must exist")
                    .name()
                    .to_owned()
            }),
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
            plaintiff_dynasty_id: case.plaintiff_dynasty_id,
            plaintiff: state
                .dynasties
                .get(&case.plaintiff_dynasty_id)
                .expect("legal plaintiff must exist")
                .name()
                .to_owned(),
            defendant_dynasty_id: case.defendant_dynasty_id,
            defendant: state
                .dynasties
                .get(&case.defendant_dynasty_id)
                .expect("legal defendant must exist")
                .name()
                .to_owned(),
            kind: case.kind,
            claim_source: case.claim_source,
            evidence_basis_points: case.evidence_basis_points,
            hearing_day: case.hearing_day,
            damages: case.damages,
            status: case.status,
            settlement_amount: quote_player_legal_settlement(state, case.id)
                .ok()
                .map(|quote| quote.amount),
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

struct DashboardFragments {
    attention: String,
    district_rows: String,
    business_rows: String,
    acquisition_rows: String,
    contract_rows: String,
    employment_rows: String,
    loan_rows: String,
    property_rows: String,
    office_rows: String,
    public_work_rows: String,
    legal_rows: String,
    crisis_cards: String,
    information_cards: String,
    law_rows: String,
    institution_rows: String,
    market_rows: String,
    civic_debt_rows: String,
    relationship_rows: String,
    alerts: String,
    unread_notices: usize,
    player_cases: usize,
    offices: usize,
    business_cash: Money,
}

fn filtered_clones<T: Clone>(items: &[T], predicate: impl Fn(&T) -> bool) -> Vec<T> {
    items
        .iter()
        .filter(|item| predicate(item))
        .cloned()
        .collect()
}

fn build_dashboard_fragments(projection: &CampaignProjection) -> DashboardFragments {
    let player_id = projection.player.id;
    let player_business_ids = projection
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id == player_id)
        .map(|business| business.id)
        .collect::<std::collections::BTreeSet<_>>();
    let businesses = filtered_clones(&projection.businesses, |business| {
        business.owner_dynasty_id == player_id
    });
    let acquisitions = filtered_clones(&projection.businesses, |business| {
        business.owner_dynasty_id != player_id && business.acquisition.is_some()
    });
    let contracts = filtered_clones(&projection.contracts, |contract| {
        player_business_ids.contains(&contract.buyer_business_id)
            || player_business_ids.contains(&contract.seller_business_id)
    });
    let employment = filtered_clones(&projection.employment, |agreement| {
        agreement.owner_dynasty_id == player_id
    });
    let loans = filtered_clones(&projection.loans, |loan| {
        loan.lender_dynasty_id == player_id || loan.borrower_dynasty_id == player_id
    });
    let properties = filtered_clones(&projection.properties, |property| {
        property.owner_dynasty_id == Some(player_id)
    });
    let offices = filtered_clones(&projection.institutions, |institution| {
        institution.officeholder_dynasty_id == Some(player_id)
    });
    let works = filtered_clones(&projection.public_works, |work| {
        work.sponsor_dynasty_id == Some(player_id)
    });
    let legal_cases = filtered_clones(&projection.legal_cases, |case| {
        case.plaintiff_dynasty_id == player_id || case.defendant_dynasty_id == player_id
    });
    let crises = filtered_clones(&projection.crises, |crisis| crisis.status.is_active());
    let laws = filtered_clones(&projection.laws, |law| law.active);
    DashboardFragments {
        attention: render_attention_items(projection),
        district_rows: render_district_rows(&projection.districts),
        business_rows: render_business_rows(&businesses),
        acquisition_rows: render_acquisition_rows(&acquisitions),
        contract_rows: render_contract_rows(&contracts),
        employment_rows: render_employment_rows(&employment),
        loan_rows: render_loan_rows(&loans, player_id),
        property_rows: render_property_rows(&properties),
        office_rows: render_institution_rows(&offices, player_id),
        public_work_rows: render_public_work_rows(&works),
        legal_rows: render_legal_case_rows(&legal_cases, player_id),
        crisis_cards: render_crisis_cards(&crises),
        information_cards: render_information_cards(&projection.information),
        law_rows: render_law_rows(&laws),
        institution_rows: render_institution_rows(&projection.institutions, player_id),
        market_rows: render_market_rows(&projection.market),
        civic_debt_rows: render_civic_debt_rows(&projection.civic_debts),
        relationship_rows: render_relationship_rows(&projection.relationships),
        alerts: render_notifications(&projection.notifications),
        unread_notices: projection.unread_notifications,
        player_cases: legal_cases
            .iter()
            .filter(|case| {
                matches!(
                    case.status,
                    LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                )
            })
            .count(),
        offices: offices.len(),
        business_cash: businesses.iter().fold(Money::ZERO, |total, business| {
            total.saturating_add(business.cash)
        }),
    }
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
    let serialized = serde_json::to_string_pretty(&projection)?;
    let data = escape_json_for_html_script(&serialized);
    let data_display = escape_html(&serialized);
    let DashboardFragments {
        attention,
        district_rows,
        business_rows,
        acquisition_rows,
        contract_rows,
        employment_rows,
        loan_rows,
        property_rows,
        office_rows,
        public_work_rows,
        legal_rows,
        crisis_cards,
        information_cards,
        law_rows,
        institution_rows,
        market_rows,
        civic_debt_rows,
        relationship_rows,
        alerts,
        unread_notices,
        player_cases,
        offices,
        business_cash,
    } = build_dashboard_fragments(&projection);
    let player = &projection.player;
    let family = &projection.family;
    let heir = family.heir.as_deref().unwrap_or("No designated heir");
    Ok(format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Civic Dynasty · {scenario}</title><style>
:root{{color-scheme:dark;--bg:#100f0d;--panel:#1c1915;--panel-2:#252019;--line:#453a2e;--text:#f2eadf;--muted:#b8aa99;--accent:#d8ad67;--urgent:#e48a7a;--warning:#e2bd6d;--good:#91bf8d;--info:#8db7c8}}
*{{box-sizing:border-box}}html{{scroll-behavior:smooth}}body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}header,main{{max-width:1240px;margin:auto;padding:24px}}header{{border-bottom:1px solid var(--line);padding-bottom:18px}}h1,h2,h3{{font-family:Georgia,serif;margin:.2em 0}}h2{{margin-top:1.6em}}p{{margin:.45em 0}}small,.muted{{color:var(--muted)}}a{{color:var(--accent)}}nav{{display:flex;flex-wrap:wrap;gap:8px;margin-top:18px}}nav a{{text-decoration:none;border:1px solid var(--line);border-radius:999px;padding:5px 10px;color:var(--text);background:var(--panel)}}.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:14px}}section,article{{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:16px}}.metric{{font:1.65rem/1.2 Georgia,serif;color:var(--accent);margin:.15em 0 .3em}}.kicker{{text-transform:uppercase;letter-spacing:.08em;font-size:.72rem;color:var(--muted)}}.badge{{display:inline-block;border:1px solid var(--line);border-radius:999px;padding:2px 8px;font-size:.78rem;white-space:nowrap}}.badge.urgent{{border-color:var(--urgent);color:var(--urgent)}}.badge.warning{{border-color:var(--warning);color:var(--warning)}}.badge.good{{border-color:var(--good);color:var(--good)}}.badge.info{{border-color:var(--info);color:var(--info)}}.attention article{{border-left:4px solid var(--info)}}.attention article.urgent{{border-left-color:var(--urgent)}}.attention article.warning{{border-left-color:var(--warning)}}.attention .action{{margin-top:10px;color:var(--accent);font-weight:600}}table{{width:100%;border-collapse:collapse}}th,td{{padding:9px 8px;border-bottom:1px solid var(--line);text-align:left;vertical-align:top}}th{{color:var(--muted);font-size:.8rem;text-transform:uppercase;letter-spacing:.04em}}tbody tr:last-child td{{border-bottom:0}}.scroll{{overflow:auto}}details{{margin:18px 0;background:var(--panel);border:1px solid var(--line);border-radius:10px}}summary{{cursor:pointer;padding:14px 16px;font-weight:700}}details>div,details>section,details>pre{{margin:0 16px 16px}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;color:var(--muted);font-size:.82rem}}.split{{display:grid;grid-template-columns:minmax(0,1.35fr) minmax(260px,.65fr);gap:16px}}.notice.unread{{border-color:var(--accent)}}.empty{{color:var(--muted)}}@media(max-width:760px){{header,main{{padding:16px}}.split{{grid-template-columns:1fr}}th,td{{padding:8px 6px}}}}
.sr-only{{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}}
</style></head><body><header><div class="kicker">Civic Dynasty · {phase}</div><h1>House {player} in {scenario}</h1><p>Year {year}, day {day} · simulation day {elapsed}</p><nav aria-label="Dashboard sections"><a href="#attention">Needs attention</a><a href="#house">House</a><a href="#operations">Operations</a><a href="#obligations">Obligations</a><a href="#civic">Civic & legal</a><a href="#intelligence">Intelligence</a><a href="#city">City context</a></nav></header><main><h2 id="house">House overview</h2><div class="grid">
<section><div class="kicker">Available treasury</div><div class="metric">{treasury}</div><p>{business_cash} held across player businesses</p></section>
<section><div class="kicker">Administrative capacity</div><div class="metric">{load} / {capacity}</div><p>{office_load} load comes from offices · {unmet_duties} unmet office duties</p></section>
<section><div class="kicker">Family continuity</div><div class="metric">{family_unity:.1}% unity</div><p>{governance} · head {head} · heir {heir} · {family_members} council members</p></section>
<section><div class="kicker">Current pressure</div><div class="metric">{unread_notices} unread notices</div><p>{crises} active crises · {player_cases} player legal cases</p></section>
</div><h2 id="attention">Needs attention</h2><div class="grid attention">{attention}</div><h2 id="operations">Your operations</h2>
<section class="scroll"><table><caption class="sr-only">Player business operations</caption><thead><tr><th scope="col">Business</th><th scope="col">Status</th><th scope="col">Cash</th><th scope="col">Condition</th><th scope="col">Policy</th><th scope="col">Manager</th></tr></thead><tbody>{business_rows}</tbody></table></section>
<div class="split">
<section class="scroll"><h3>Labor</h3><table><caption class="sr-only">Player labor agreements</caption><thead><tr><th scope="col">Agreement</th><th scope="col">Business</th><th scope="col">Workers</th><th scope="col">Weekly wage</th><th scope="col">Conditions</th><th scope="col">Loyalty</th><th scope="col">Status</th></tr></thead><tbody>{employment_rows}</tbody></table></section>
<section><h3>Family & office</h3><p><strong>{governance}</strong> governance with {family_unity:.1}% family unity.</p><p>Head: {head} <small>#{head_id}</small><br>Heir: {heir}{heir_id}</p><p>{offices} offices currently held.</p></section>
</div><h2 id="obligations">Contracts, finance & property</h2>
<section class="scroll"><h3>Supply obligations involving your businesses</h3><table><caption class="sr-only">Player supply contract obligations and performance</caption><thead><tr><th scope="col">Contract</th><th scope="col">Parties</th><th scope="col">Good</th><th scope="col">Terms</th><th scope="col">Status</th><th scope="col">Performance</th><th scope="col">Breach</th></tr></thead><tbody>{contract_rows}</tbody></table></section>
<div class="split">
<section class="scroll"><h3>Private finance</h3><table><caption class="sr-only">Private loans involving the player dynasty</caption><thead><tr><th scope="col">Loan</th><th scope="col">Role & counterparty</th><th scope="col">Balance</th><th scope="col">Weekly payment</th><th scope="col">Rate</th><th scope="col">Status / due</th></tr></thead><tbody>{loan_rows}</tbody></table></section>
<section class="scroll"><h3>Your property</h3><table><caption class="sr-only">Player property holdings</caption><thead><tr><th scope="col">Property</th><th scope="col">Value</th><th scope="col">Rent</th><th scope="col">Condition</th><th scope="col">Use / lien</th></tr></thead><tbody>{property_rows}</tbody></table></section>
</div><h2 id="civic">Civic power, projects & legal pressure</h2>
<section class="scroll"><h3>Your offices</h3><table><caption class="sr-only">Offices held by the player dynasty</caption><thead><tr><th scope="col">Institution</th><th scope="col">Officeholder</th><th scope="col">Next selection</th><th scope="col">Institution standing</th><th scope="col">Powers</th></tr></thead><tbody>{office_rows}</tbody></table></section>
<div class="split">
<section class="scroll"><h3>Sponsored public works</h3><table><caption class="sr-only">Player-sponsored public works</caption><thead><tr><th scope="col">Project</th><th scope="col">District</th><th scope="col">Funding</th><th scope="col">Progress</th><th scope="col">Status</th></tr></thead><tbody>{public_work_rows}</tbody></table></section>
<section><h3>Active crises</h3><div class="grid">{crisis_cards}</div></section>
</div>
<section class="scroll"><h3>Legal cases involving House {player}</h3><table><caption class="sr-only">Legal cases involving the player dynasty</caption><thead><tr><th scope="col">Case</th><th scope="col">Role & counterparty</th><th scope="col">Claim</th><th scope="col">Evidence</th><th scope="col">Hearing</th><th scope="col">Exposure</th><th scope="col">Status</th></tr></thead><tbody>{legal_rows}</tbody></table></section><h2 id="intelligence">Intelligence & notices</h2>
<div class="split"><section><h3>Current reports</h3><div class="grid">{information_cards}</div></section><section><h3>Recent notices</h3><div class="grid">{alerts}</div></section></div><h2 id="city">City context</h2>
<details open><summary>Market and district conditions</summary><div><h3>Market</h3><section class="scroll"><table><caption class="sr-only">Market prices, movement, stocks, and flows</caption><thead><tr><th scope="col">Good</th><th scope="col">Price</th><th scope="col">Movement</th><th scope="col">Stock / target</th><th scope="col">Demand / supply</th><th scope="col">Drivers</th></tr></thead><tbody>{market_rows}</tbody></table></section><h3>Districts</h3><section class="scroll"><table><caption class="sr-only">District conditions</caption><thead><tr><th scope="col">District</th><th scope="col">Food</th><th scope="col">Employment</th><th scope="col">Sanitation</th><th scope="col">Safety</th><th scope="col">Unrest</th><th scope="col">Drivers</th></tr></thead><tbody>{district_rows}</tbody></table></section></div></details>
<details><summary>Institutions, active laws, municipal debt & relationships</summary><div><h3>Institutions</h3><section class="scroll"><table><caption class="sr-only">City institutions</caption><thead><tr><th scope="col">Institution</th><th scope="col">Officeholder</th><th scope="col">Next selection</th><th scope="col">Institution standing</th><th scope="col">Powers</th></tr></thead><tbody>{institution_rows}</tbody></table></section><h3>Active laws</h3><section class="scroll"><table><caption class="sr-only">Active laws</caption><thead><tr><th scope="col">Law</th><th scope="col">Value</th><th scope="col">Sponsor</th><th scope="col">Enacted</th></tr></thead><tbody>{law_rows}</tbody></table></section><h3>Municipal debt</h3><section class="scroll"><table><caption class="sr-only">Municipal debt obligations</caption><thead><tr><th scope="col">Creditor</th><th scope="col">Principal</th><th scope="col">Balance</th><th scope="col">Weekly payment</th><th scope="col">Interest</th><th scope="col">Status</th><th scope="col">Next due</th></tr></thead><tbody>{civic_debt_rows}</tbody></table></section><h3>Dynasty relationships</h3><section class="scroll"><table><caption class="sr-only">Dynasty relationship measures</caption><thead><tr><th scope="col">House</th><th scope="col">Trust</th><th scope="col">Respect</th><th scope="col">Fear</th><th scope="col">Resentment</th><th scope="col">Obligation</th><th scope="col">Last interaction</th></tr></thead><tbody>{relationship_rows}</tbody></table></section></div></details>
<details><summary>Acquisition opportunities</summary><section class="scroll"><table><caption class="sr-only">Businesses currently available for acquisition</caption><thead><tr><th scope="col">Business</th><th scope="col">Owner</th><th scope="col">Status</th><th scope="col">Condition</th><th scope="col">Purchase</th><th scope="col">Required working capital</th></tr></thead><tbody>{acquisition_rows}</tbody></table></section></details>
<details><summary>Complete projection data</summary><pre>{data_display}</pre></details>
</main><script type="application/json" id="campaign-data">{data}</script></body></html>"##,
        scenario = escape_html(&projection.scenario.name),
        year = projection.scenario.year,
        day = projection.scenario.day_of_year,
        elapsed = projection.scenario.elapsed_days,
        phase = projection.scenario.phase.label(),
        player = escape_html(&player.name),
        treasury = player.treasury,
        business_cash = business_cash,
        load = player.effective_administrative_load,
        office_load = player.office_administrative_load,
        capacity = player.administrative_capacity,
        unmet_duties = player.unmet_office_duties,
        family_unity = f64::from(family.unity_basis_points) / 100.0,
        governance = escape_html(&humanize_debug(&family.governance)),
        head = escape_html(&family.head),
        head_id = family.head_id,
        heir = escape_html(heir),
        heir_id = family
            .heir_id
            .map_or_else(String::new, |id| format!(" <small>#{id}</small>")),
        family_members = family.members,
        unread_notices = unread_notices,
        crises = projection.scenario.active_crises,
        player_cases = player_cases,
        offices = offices,
    ))
}

/// Classifies every condition that needs the player's attention. This is the
/// single canonical classification, consumed by both the CLI summary and the
/// dashboard.
fn build_attention_items(projection: &CampaignProjection) -> Vec<AttentionItem> {
    let mut items = Vec::new();
    append_house_attention(projection, &mut items);
    append_operational_attention(projection, &mut items);
    append_finance_attention(projection, &mut items);
    append_legal_and_crisis_attention(projection, &mut items);
    let player_business_ids = projection
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id == projection.player.id)
        .map(|business| business.id)
        .collect::<std::collections::BTreeSet<_>>();
    append_contract_attention(projection, &player_business_ids, &mut items);
    append_notice_attention(projection, &mut items);
    items
}

fn attention(
    tone: AttentionTone,
    category: &str,
    title: String,
    detail: String,
    action: String,
) -> AttentionItem {
    AttentionItem {
        tone,
        category: category.to_owned(),
        title,
        detail,
        action,
    }
}

fn render_attention_items(projection: &CampaignProjection) -> String {
    if projection.attention.is_empty() {
        return "<article class=\"good\"><span class=\"badge good\">Stable</span><h3>No immediate intervention is flagged</h3><p>Current records show no overdue office duties, player labor disputes, distressed player businesses, adverse debt states, active crises, or unresolved claims against the house.</p></article>".to_owned();
    }
    projection
        .attention
        .iter()
        .take(12)
        .map(render_attention_card)
        .collect()
}

fn render_attention_card(item: &AttentionItem) -> String {
    let tone = match item.tone {
        AttentionTone::Urgent => "urgent",
        AttentionTone::Warning => "warning",
        AttentionTone::Info => "info",
    };
    format!(
        "<article class=\"{tone}\"><span class=\"badge {tone}\">{}</span><h3>{}</h3><p>{}</p><p class=\"action\">{}</p></article>",
        escape_html(&item.category),
        escape_html(&item.title),
        escape_html(&item.detail),
        escape_html(&item.action),
    )
}

fn append_house_attention(projection: &CampaignProjection, items: &mut Vec<AttentionItem>) {
    if projection.player.effective_administrative_load > projection.player.administrative_capacity {
        items.push(attention(
            AttentionTone::Urgent,
            "Administrative capacity",
            "House administration is overloaded".to_owned(),
            format!(
                "Current load is {} against {} capacity.",
                projection.player.effective_administrative_load,
                projection.player.administrative_capacity
            ),
            "Reduce administrative commitments before expanding businesses or offices.".to_owned(),
        ));
    }
    if projection.player.unmet_office_duties > 0 {
        items.push(attention(
            AttentionTone::Urgent,
            "Office duties",
            "Recurring office duties are unmet".to_owned(),
            format!(
                "{} office-duty obligations are currently outstanding.",
                projection.player.unmet_office_duties
            ),
            "Protect treasury and administrative capacity for the next duty settlement.".to_owned(),
        ));
    }
}

fn append_operational_attention(projection: &CampaignProjection, items: &mut Vec<AttentionItem>) {
    let player_id = projection.player.id;
    for agreement in projection.employment.iter().filter(|agreement| {
        agreement.owner_dynasty_id == player_id && agreement.status == EmploymentStatus::Disputed
    }) {
        items.push(attention(
            AttentionTone::Urgent,
            "Labor",
            format!("Labor dispute at {}", agreement.business),
            format!(
                "Agreement #{} covers {} workers; conditions are {:.1}% and loyalty is {:.1}%.",
                agreement.id,
                agreement.workers,
                f64::from(agreement.conditions_basis_points) / 100.0,
                f64::from(agreement.loyalty_basis_points) / 100.0
            ),
            format!(
                "Resolve labor dispute #{} before operations deteriorate.",
                agreement.id
            ),
        ));
    }
    for business in projection.businesses.iter().filter(|business| {
        business.owner_dynasty_id == player_id
            && matches!(
                business.status,
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            )
    }) {
        items.push(attention(
            AttentionTone::Urgent,
            "Business",
            format!("{} is {}", business.name, business.status.label()),
            format!(
                "Business #{} has {} cash and {:.1}% condition.",
                business.id,
                business.cash,
                f64::from(business.condition_basis_points) / 100.0
            ),
            format!(
                "Review capitalization and policy for business #{}.",
                business.id
            ),
        ));
    }
}

fn append_finance_attention(projection: &CampaignProjection, items: &mut Vec<AttentionItem>) {
    let player_id = projection.player.id;
    for loan in projection.loans.iter().filter(|loan| {
        loan.borrower_dynasty_id == player_id
            && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
    }) {
        let due = if loan.status.is_repayment_active() {
            format!(" Next scheduled payment is day {}.", loan.next_due_day)
        } else {
            String::new()
        };
        items.push(attention(
            AttentionTone::Urgent,
            "Private finance",
            format!("Loan #{} is {}", loan.id, loan.status.label()),
            format!(
                "{} remains outstanding with {} missed payments.{}",
                loan.balance, loan.missed_payments, due
            ),
            "Preserve liquidity and review collateral exposure before taking on new commitments."
                .to_owned(),
        ));
    }
}

fn append_legal_and_crisis_attention(
    projection: &CampaignProjection,
    items: &mut Vec<AttentionItem>,
) {
    let player_id = projection.player.id;
    for case in projection.legal_cases.iter().filter(|case| {
        case.defendant_dynasty_id == player_id
            && matches!(
                case.status,
                LegalCaseStatus::Filed | LegalCaseStatus::Hearing
            )
    }) {
        let action = case.settlement_amount.map_or_else(
            || format!("Review case #{} before its hearing.", case.id),
            |amount| format!("Case #{} can currently be settled for {amount}.", case.id),
        );
        items.push(attention(
            AttentionTone::Urgent,
            "Legal",
            format!("{} claim by {}", humanize_debug(&case.kind), case.plaintiff),
            format!(
                "Case #{} seeks {} in damages; hearing day {} and evidence strength {:.1}%.",
                case.id,
                case.damages,
                case.hearing_day,
                f64::from(case.evidence_basis_points) / 100.0
            ),
            action,
        ));
    }
    for crisis in projection
        .crises
        .iter()
        .filter(|crisis| crisis.status.is_active())
    {
        let district = crisis
            .district
            .as_deref()
            .map_or_else(|| "citywide".to_owned(), |name| format!("in {name}"));
        items.push(attention(
            AttentionTone::Warning,
            "Crisis",
            format!("{} {district}", crisis.kind.label()),
            format!(
                "Crisis #{} is {} at {:.1}% severity. {}",
                crisis.id,
                crisis_status_label(crisis.status),
                f64::from(crisis.severity_basis_points) / 100.0,
                crisis.cause
            ),
            format!("Review response options for crisis #{}.", crisis.id),
        ));
    }
}

fn append_contract_attention(
    projection: &CampaignProjection,
    player_business_ids: &std::collections::BTreeSet<BusinessId>,
    items: &mut Vec<AttentionItem>,
) {
    let player_id = projection.player.id;
    for contract in projection.contracts.iter().filter(|contract| {
        player_business_ids.contains(&contract.buyer_business_id)
            || player_business_ids.contains(&contract.seller_business_id)
    }) {
        if contract.breaching_dynasty_id == Some(player_id) {
            items.push(attention(
                AttentionTone::Urgent,
                "Contract",
                format!(
                    "House {} breached contract #{}",
                    projection.player.name, contract.id
                ),
                format!(
                    "{} remains as an unpaid breach penalty on the {} contract.",
                    contract.unpaid_breach_penalty, contract.good
                ),
                "Review legal and liquidity exposure from the breached obligation.".to_owned(),
            ));
        } else if contract.breach_victim_dynasty_id == Some(player_id) {
            items.push(attention(
                AttentionTone::Warning,
                "Contract",
                format!("Contract #{} was breached against your house", contract.id),
                format!(
                    "{} is the unpaid breach penalty on the {} contract.",
                    contract.unpaid_breach_penalty, contract.good
                ),
                "Review whether the attributed breach supports a legal claim.".to_owned(),
            ));
        } else if contract.status == ContractStatus::Active && contract.missed_deliveries > 0 {
            items.push(attention(
                AttentionTone::Warning,
                "Contract",
                format!("Contract #{} has missed deliveries", contract.id),
                format!(
                    "{} deliveries have been missed; the next obligation is day {}.",
                    contract.missed_deliveries, contract.next_due_day
                ),
                "Review inventory, cash, and counterparty performance before the next settlement."
                    .to_owned(),
            ));
        }
    }
}

fn append_notice_attention(projection: &CampaignProjection, items: &mut Vec<AttentionItem>) {
    let unread = projection
        .notifications
        .iter()
        .filter(|notification| !notification.acknowledged)
        .count();
    if unread > 0
        && let Some(latest) = projection
            .notifications
            .iter()
            .rev()
            .find(|notification| !notification.acknowledged)
    {
        items.push(attention(
            AttentionTone::Info,
            "Notices",
            format!("{unread} unread notices"),
            format!("Latest: {}", latest.subject),
            format!("Review and acknowledge notice #{} when handled.", latest.id),
        ));
    }
}

fn render_employment_rows(agreements: &[EmploymentProjection]) -> String {
    if agreements.is_empty() {
        return "<tr><td colspan=\"7\" class=\"empty\">No labor agreements are attached to your businesses.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for agreement in agreements {
        write!(
            rows,
            "<tr><td>#{}</td><td>{}<br><small>business #{}</small></td><td>{}</td><td>{}</td><td>{:.1}%</td><td>{:.1}%</td><td>{}</td></tr>",
            agreement.id,
            escape_html(&agreement.business),
            agreement.business_id,
            agreement.workers,
            agreement.weekly_wage,
            f64::from(agreement.conditions_basis_points) / 100.0,
            f64::from(agreement.loyalty_basis_points) / 100.0,
            employment_status_label(agreement.status),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_loan_rows(loans: &[LoanProjection], player_id: DynastyId) -> String {
    if loans.is_empty() {
        return "<tr><td colspan=\"6\" class=\"empty\">No private loans involve your dynasty.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for loan in loans {
        let role = if loan.borrower_dynasty_id == player_id {
            format!("Borrower from {}", escape_html(&loan.lender))
        } else {
            format!("Lender to {}", escape_html(&loan.borrower))
        };
        let due = if loan.status.is_repayment_active() {
            format!("day {}", loan.next_due_day)
        } else {
            "no scheduled payment".to_owned()
        };
        let arrears = if loan.missed_payments > 0 {
            format!(" · {} missed", loan.missed_payments)
        } else {
            String::new()
        };
        write!(
            rows,
            "<tr><td>#{}</td><td>{}</td><td>{}<br><small>original {}</small></td><td>{}</td><td>{:.1}%</td><td>{}<br><small>{due}{arrears}</small></td></tr>",
            loan.id,
            role,
            loan.balance,
            loan.principal,
            loan.weekly_payment,
            f64::from(loan.interest_basis_points) / 100.0,
            loan.status.label(),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_property_rows(properties: &[PropertyProjection]) -> String {
    if properties.is_empty() {
        return "<tr><td colspan=\"5\" class=\"empty\">Your dynasty owns no property.</td></tr>"
            .to_owned();
    }
    let mut rows = String::new();
    for property in properties {
        let use_text = property.occupied_business_id.map_or_else(
            || {
                property.tenant.as_deref().map_or_else(
                    || "Vacant".to_owned(),
                    |tenant| format!("Tenant: {}", escape_html(tenant)),
                )
            },
            |business_id| format!("Occupied by business #{business_id}"),
        );
        let lien = property
            .collateral_loan_id
            .map_or_else(String::new, |loan_id| {
                format!("<br><small>Collateral for loan #{loan_id}</small>")
            });
        write!(
            rows,
            "<tr><td>{}<br><small>#{} · {}</small></td><td>{}</td><td>{}<br><small>base {}</small></td><td>{:.1}%</td><td>{}{}</td></tr>",
            escape_html(&property.name),
            property.id,
            escape_html(&property.district),
            property.value,
            property.effective_weekly_rent,
            property.weekly_rent,
            f64::from(property.condition_basis_points) / 100.0,
            use_text,
            lien,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_institution_rows(institutions: &[InstitutionProjection], player_id: DynastyId) -> String {
    if institutions.is_empty() {
        return "<tr><td colspan=\"5\" class=\"empty\">No matching offices or institutions.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for institution in institutions {
        let officeholder = institution.officeholder.as_deref().map_or_else(
            || "Vacant".to_owned(),
            |holder| {
                let dynasty = institution
                    .officeholder_dynasty
                    .as_deref()
                    .unwrap_or("Unknown house");
                let your_office = if institution.officeholder_dynasty_id == Some(player_id) {
                    " <span class=\"badge good\">Your office</span>"
                } else {
                    ""
                };
                format!(
                    "{}<br><small>{}</small>{your_office}",
                    escape_html(holder),
                    escape_html(dynasty)
                )
            },
        );
        let powers = if institution.powers.is_empty() {
            "None".to_owned()
        } else {
            institution
                .powers
                .iter()
                .map(humanize_debug)
                .collect::<Vec<_>>()
                .join(", ")
        };
        write!(
            rows,
            "<tr><td>{}<br><small>#{}</small></td><td>{}</td><td>day {}</td><td>{} budget<br><small>{:.1}% legitimacy</small></td><td>{}</td></tr>",
            escape_html(&institution.name),
            institution.id,
            officeholder,
            institution.next_selection_day,
            institution.budget,
            f64::from(institution.legitimacy_basis_points) / 100.0,
            escape_html(&powers),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_public_work_rows(works: &[PublicWorkProjection]) -> String {
    if works.is_empty() {
        return "<tr><td colspan=\"5\" class=\"empty\">Your dynasty is not sponsoring a public work.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for work in works {
        write!(
            rows,
            "<tr><td>{}<br><small>#{}</small></td><td>{}</td><td>{} / {}</td><td>{:.1}%</td><td>{}</td></tr>",
            escape_html(&humanize_debug(&work.kind)),
            work.id,
            escape_html(&work.district),
            work.spent,
            work.budget,
            f64::from(work.progress_basis_points) / 100.0,
            public_work_status_label(work.status),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_legal_case_rows(cases: &[LegalCaseProjection], player_id: DynastyId) -> String {
    if cases.is_empty() {
        return "<tr><td colspan=\"7\" class=\"empty\">No legal cases involve your dynasty.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for case in cases {
        let role = if case.defendant_dynasty_id == player_id {
            format!("Defendant vs. {}", escape_html(&case.plaintiff))
        } else {
            format!("Plaintiff vs. {}", escape_html(&case.defendant))
        };
        let source = case.claim_source.map_or_else(
            || "No source record".to_owned(),
            |source| match source {
                LegalClaimSource::Loan { loan_id } => format!("Loan #{loan_id}"),
                LegalClaimSource::Contract { contract_id } => format!("Contract #{contract_id}"),
            },
        );
        let settlement = case.settlement_amount.map_or_else(String::new, |amount| {
            format!("<br><small>Current settlement: {amount}</small>")
        });
        write!(
            rows,
            "<tr><td>#{}<br><small>{}</small></td><td>{}</td><td>{}<br><small>{}</small></td><td>{:.1}%</td><td>day {}</td><td>{}{}</td><td>{}</td></tr>",
            case.id,
            escape_html(&humanize_debug(&case.kind)),
            role,
            escape_html(&humanize_debug(&case.kind)),
            escape_html(&source),
            f64::from(case.evidence_basis_points) / 100.0,
            case.hearing_day,
            case.damages,
            settlement,
            legal_case_status_label(case.status),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_crisis_cards(crises: &[CrisisProjection]) -> String {
    if crises.is_empty() {
        return "<article class=\"empty\"><p>No active crises.</p></article>".to_owned();
    }
    let mut cards = String::new();
    for crisis in crises {
        let district = crisis.district.as_deref().unwrap_or("Citywide");
        write!(
            cards,
            "<article><span class=\"badge warning\">{}</span><h3>{}</h3><p>{:.1}% severity · {}</p><p>{}</p><p class=\"action\">Crisis #{}</p></article>",
            crisis_status_label(crisis.status),
            escape_html(crisis.kind.label()),
            f64::from(crisis.severity_basis_points) / 100.0,
            escape_html(district),
            escape_html(&crisis.cause),
            crisis.id,
        )
        .expect("writing HTML into a String cannot fail");
    }
    cards
}

fn render_information_cards(reports: &[InformationProjection]) -> String {
    if reports.is_empty() {
        return "<article class=\"empty\"><p>No current player intelligence reports.</p></article>"
            .to_owned();
    }
    let mut cards = String::new();
    for report in reports.iter().rev().take(12) {
        write!(
            cards,
            "<article><span class=\"badge info\">{}</span><h3>{}</h3><p>{}</p><p><small>Report #{} · {} · created day {} · expires day {}</small></p><p class=\"action\">Reference report #{} for intelligence actions.</p></article>",
            report.confidence.label(),
            escape_html(&report.subject),
            escape_html(&report.summary),
            report.id,
            escape_html(&report.source),
            report.created_day,
            report.expires_day,
            report.id,
        )
        .expect("writing HTML into a String cannot fail");
    }
    cards
}

fn render_law_rows(laws: &[LawProjection]) -> String {
    if laws.is_empty() {
        return "<tr><td colspan=\"4\" class=\"empty\">No active laws.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for law in laws {
        write!(
            rows,
            "<tr><td>{}<br><small>#{}</small><br><small>{}</small></td><td>{}</td><td>{}</td><td>day {}</td></tr>",
            escape_html(&humanize_debug(&law.kind)),
            law.id,
            escape_html(&law_effect_summary(law.kind, law.value)),
            law.value,
            law.sponsor.as_deref().map_or("—".to_owned(), escape_html),
            law.enacted_day,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

/// One-line player-facing summary of what an enacted kind actually does in
/// the simulation, so a raw law row answers "what does this change?" without
/// forcing the reader through the systems code.
fn law_effect_summary(kind: LawKind, value: i64) -> String {
    match kind {
        LawKind::BreadPriceCeiling => {
            format!("Caps bread at {} copper while active.", value.max(0))
        }
        LawKind::ForeignMerchantToll => {
            "Tolls every regional trade route, discouraging imports.".to_owned()
        }
        LawKind::InterestLimit => {
            format!(
                "Caps private loan interest at {} basis points.",
                value.clamp(0, 10_000)
            )
        }
        LawKind::FireCode => "Lowers the chance and severity of urban fires.".to_owned(),
        LawKind::RentRestriction => {
            "Caps weekly rents below their district-indexed level.".to_owned()
        }
        LawKind::GuildEntryRestriction => {
            "Reserves craft-market access for chartered guild members, raises the cost of \
             joining a guild, and breeds revolt pressure."
                .to_owned()
        }
        LawKind::EmergencyImports => {
            format!(
                "Adds {} units of grain to the market each day.",
                value.max(0)
            )
        }
        LawKind::PublicDebtAuthorization => {
            "Authorizes the civic treasury to borrow from dynasty creditors.".to_owned()
        }
    }
}

fn render_acquisition_rows(businesses: &[BusinessProjection]) -> String {
    if businesses.is_empty() {
        return "<tr><td colspan=\"6\" class=\"empty\">No businesses currently meet acquisition conditions.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for business in businesses {
        let quote = business
            .acquisition
            .expect("acquisition rows require quoted businesses");
        write!(
            rows,
            "<tr><td>{}<br><small>#{} · {} · {}</small></td><td>{}</td><td>{}</td><td>{:.1}%</td><td>{}</td><td>{}</td></tr>",
            escape_html(&business.name),
            business.id,
            escape_html(&business.district),
            escape_html(&business.recipe),
            escape_html(&business.owner),
            business.status.label(),
            f64::from(business.condition_basis_points) / 100.0,
            quote.purchase_price,
            quote.minimum_recapitalization,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
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
        return "<tr><td colspan=\"6\" class=\"empty\">Your dynasty owns no businesses.</td></tr>"
            .to_owned();
    }
    let mut rows = String::new();
    for business in businesses {
        write!(
            rows,
            "<tr><td>{}<br><small>#{} · {} · {}</small></td><td>{}</td><td>{}</td><td>{:.1}%<br><small>quality {:.1}%</small></td><td>inputs {}d · outputs {}d<br><small>reserve {} · maintenance {:.1}% · quality target {:.1}%</small></td><td>{}</td></tr>",
            escape_html(&business.name),
            business.id,
            escape_html(&business.district),
            escape_html(&business.recipe),
            business.status.label(),
            business.cash,
            f64::from(business.condition_basis_points) / 100.0,
            f64::from(business.quality_basis_points) / 100.0,
            business.target_input_days,
            business.target_output_days,
            business.minimum_cash_reserve,
            f64::from(business.maintenance_basis_points) / 100.0,
            f64::from(business.quality_target_basis_points) / 100.0,
            escape_html(&business.manager),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_contract_rows(contracts: &[ContractProjection]) -> String {
    if contracts.is_empty() {
        return "<tr><td colspan=\"7\" class=\"empty\">No supply contracts involve your businesses.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for contract in contracts {
        let next_due = if contract.status == ContractStatus::Active {
            format!("next due day {}", contract.next_due_day)
        } else {
            "no further delivery due".to_owned()
        };
        let delivery_credits = if contract.delivery_credits.is_empty() {
            String::new()
        } else {
            format!(
                "<br><small>Credit: {}</small>",
                contract
                    .delivery_credits
                    .iter()
                    .map(|credit| {
                        format!("{}: {}", escape_html(&credit.dynasty), credit.deliveries)
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            )
        };
        let breach = match (
            contract.breaching_dynasty.as_deref(),
            contract.breach_victim_dynasty.as_deref(),
        ) {
            (Some(breacher), Some(victim)) => format!(
                "{} → {}<br><small>unpaid penalty {}</small>",
                escape_html(breacher),
                escape_html(victim),
                contract.unpaid_breach_penalty
            ),
            _ => "—".to_owned(),
        };
        write!(
            rows,
            "<tr><td>#{}</td><td>Buyer: {}<br>Seller: {}</td><td>{}</td><td>{} weekly at {} each<br><small>through day {} · penalty {} · {}</small></td><td>{}</td><td>{} fulfilled · {} missed{}</td><td>{}</td></tr>",
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
        return "<tr><td colspan=\"7\" class=\"empty\">No district records are available.</td></tr>".to_owned();
    }
    let mut rows = String::new();
    for district in districts {
        write!(
            rows,
            "<tr><td>{}<br><small>{} residents</small></td><td>{:.1}%</td><td>{:.1}%</td><td>{:.1}%</td><td>{:.1}%</td><td>{:.1}%</td><td>{}</td></tr>",
            escape_html(&district.name),
            district.population,
            f64::from(district.food_satisfaction_basis_points) / 100.0,
            f64::from(district.employment_basis_points) / 100.0,
            f64::from(district.sanitation_basis_points) / 100.0,
            f64::from(district.safety_basis_points) / 100.0,
            f64::from(district.unrest_basis_points) / 100.0,
            escape_html(&district.causes.join("; ")),
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_market_rows(markets: &[MarketProjection]) -> String {
    if markets.is_empty() {
        return "<tr><td colspan=\"6\" class=\"empty\">No market quotes are available.</td></tr>"
            .to_owned();
    }
    let mut rows = String::new();
    for market in markets {
        let causes = market
            .causes
            .iter()
            .map(market_cause_label)
            .collect::<Vec<_>>()
            .join(", ");
        let movement = market_price_movement(market.price, market.previous_price);
        write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{} / {}</td><td>{} / {}</td><td>{}</td></tr>",
            escape_html(&market.good),
            market.price,
            movement,
            market.stock,
            market.target_stock,
            market.demand_today,
            market.supply_today,
            causes,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_notifications(notifications: &[NotificationProjection]) -> String {
    if notifications.is_empty() {
        return "<article class=\"empty\"><p>No recent notices.</p></article>".to_owned();
    }
    let mut alerts = String::new();
    let visible = notifications
        .iter()
        .rev()
        .filter(|notification| !notification.acknowledged)
        .take(8)
        .chain(
            notifications
                .iter()
                .rev()
                .filter(|notification| notification.acknowledged)
                .take(4),
        );
    for notification in visible {
        let state = if notification.acknowledged {
            "Read"
        } else {
            "Unread"
        };
        let class = if notification.acknowledged {
            "notice"
        } else {
            "notice unread"
        };
        write!(
            alerts,
            "<article class=\"{class}\"><span class=\"badge info\">{state}</span><small> Day {} · {} · notice #{}</small><h3>{}</h3><p>{}</p></article>",
            notification.day,
            outbox_kind_label(notification.kind),
            notification.id,
            escape_html(&notification.subject),
            escape_html(&notification.body),
        )
        .expect("writing HTML into a String cannot fail");
    }
    alerts
}

const fn employment_status_label(status: EmploymentStatus) -> &'static str {
    match status {
        EmploymentStatus::Active => "Active",
        EmploymentStatus::Disputed => "Disputed",
        EmploymentStatus::Suspended => "Suspended",
        EmploymentStatus::Ended => "Ended",
    }
}

const fn public_work_status_label(status: PublicWorkStatus) -> &'static str {
    match status {
        PublicWorkStatus::Planned => "Planned",
        PublicWorkStatus::Building => "Building",
        PublicWorkStatus::Completed => "Completed",
        PublicWorkStatus::Suspended => "Suspended",
    }
}

const fn legal_case_status_label(status: LegalCaseStatus) -> &'static str {
    match status {
        LegalCaseStatus::Filed => "Filed",
        LegalCaseStatus::Hearing => "Hearing",
        LegalCaseStatus::DecidedForPlaintiff => "Decided for plaintiff",
        LegalCaseStatus::DecidedForDefendant => "Decided for defendant",
        LegalCaseStatus::Settled => "Settled",
    }
}

const fn crisis_status_label(status: CrisisStatus) -> &'static str {
    match status {
        CrisisStatus::Emerging => "Emerging",
        CrisisStatus::Active => "Active",
        CrisisStatus::Resolved => "Resolved",
        CrisisStatus::Escalated => "Escalated",
    }
}

fn market_price_movement(price: Money, previous_price: Money) -> String {
    match price.cmp(&previous_price) {
        std::cmp::Ordering::Greater => format!("↑ from {previous_price}"),
        std::cmp::Ordering::Less => format!("↓ from {previous_price}"),
        std::cmp::Ordering::Equal => "unchanged".to_owned(),
    }
}

fn humanize_debug<T: std::fmt::Debug>(value: &T) -> String {
    humanize_identifier(&format!("{value:?}"))
}

fn humanize_identifier(value: &str) -> String {
    let mut output = String::with_capacity(value.len().saturating_add(8));
    let mut previous_was_lower_or_digit = false;
    for character in value.chars() {
        if character == '_' {
            output.push(' ');
            previous_was_lower_or_digit = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_was_lower_or_digit {
            output.push(' ');
        }
        output.push(character);
        previous_was_lower_or_digit = character.is_ascii_lowercase() || character.is_ascii_digit();
    }
    output
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
