//! Read-only causal projections and a self-contained HTML campaign dashboard.

use crate::core::{
    AppState, CampaignPhase, ContractStatus, CrisisKind, CrisisStatus, InformationConfidence,
    LawKind, LegalCaseStatus, LoanStatus, MarketCause, ObjectiveKind, ObjectiveStatus, OutboxKind,
    PublicWorkKind, PublicWorkStatus,
};
use crate::ids::{
    BusinessId, ContractId, CrisisId, DistrictId, DynastyId, InstitutionId, LawId, LegalCaseId,
    LoanId, PropertyId, PublicWorkId,
};
use crate::money::{Money, Quantity};
use crate::registry::Registry;
use serde::Serialize;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CampaignProjection {
    pub scenario: ScenarioProjection,
    pub player: DynastyProjection,
    pub dynasties: Vec<DynastyProjection>,
    pub districts: Vec<DistrictProjection>,
    pub market: Vec<MarketProjection>,
    pub contracts: Vec<ContractProjection>,
    pub loans: Vec<LoanProjection>,
    pub properties: Vec<PropertyProjection>,
    pub institutions: Vec<InstitutionProjection>,
    pub laws: Vec<LawProjection>,
    pub public_works: Vec<PublicWorkProjection>,
    pub legal_cases: Vec<LegalCaseProjection>,
    pub crises: Vec<CrisisProjection>,
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
    pub legitimacy_basis_points: u16,
    pub reputation_quality_basis_points: u16,
    pub reputation_reliability_basis_points: u16,
    pub administrative_capacity: u16,
    pub administrative_load: u16,
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
pub struct ContractProjection {
    pub id: ContractId,
    pub buyer_business_id: BusinessId,
    pub buyer_name: String,
    pub seller_business_id: BusinessId,
    pub seller_name: String,
    pub good: String,
    pub quantity_per_week: Quantity,
    pub unit_price: Money,
    pub next_due_day: i64,
    pub status: ContractStatus,
    pub fulfilled_deliveries: u16,
    pub missed_deliveries: u16,
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
pub struct PropertyProjection {
    pub id: PropertyId,
    pub name: String,
    pub district: String,
    pub owner: Option<String>,
    pub value: Money,
    pub weekly_rent: Money,
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
pub struct InformationProjection {
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

/// Builds the complete read-only campaign projection used by adapters.
///
/// # Panics
///
/// Panics when runtime references violate the invariants required by the projection.
#[must_use]
pub fn build_campaign_projection(registry: &Registry, state: &AppState) -> CampaignProjection {
    let summary = state.summary(registry);
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
        market: build_market_projections(registry, state),
        contracts: build_contract_projections(registry, state),
        loans: build_loan_projections(state),
        properties: build_property_projections(registry, state),
        institutions: build_institution_projections(registry, state),
        laws: build_law_projections(state),
        public_works: build_public_work_projections(registry, state),
        legal_cases: build_legal_case_projections(state),
        crises: build_crisis_projections(registry, state),
        information: state
            .information_reports
            .values()
            .filter(|report| report.owner_dynasty_id == state.player_dynasty_id)
            .map(|report| InformationProjection {
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
        .institution_runtime
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
    DynastyProjection {
        id: dynasty_id,
        name: dynasty.name().to_owned(),
        treasury: dynasty.treasury(),
        legitimacy_basis_points: dynasty.resources.legitimacy_basis_points,
        reputation_quality_basis_points: dynasty.resources.reputation_quality_basis_points,
        reputation_reliability_basis_points: dynasty.resources.reputation_reliability_basis_points,
        administrative_capacity: dynasty.administrative_capacity(),
        administrative_load: dynasty.administrative_load(),
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
                loan.borrower_dynasty_id == dynasty_id
                    && matches!(
                        loan.status,
                        LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
                    )
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
            let food = if households.is_empty() {
                0
            } else {
                let total: u64 = households
                    .iter()
                    .map(|household| u64::from(household.food_satisfaction_basis_points()))
                    .sum();
                u16::try_from(total / households.len() as u64).unwrap_or(0)
            };
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
            next_due_day: contract.next_due_day,
            status: contract.status,
            fulfilled_deliveries: contract.fulfilled_deliveries,
            missed_deliveries: contract.missed_deliveries,
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
        .institution_runtime
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
    let data = serde_json::to_string_pretty(&projection)?;
    let player = &projection.player;
    let district_rows = render_district_rows(&projection.districts);
    let market_rows = render_market_rows(&projection.market);
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
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.5 Georgia,serif}}header,main{{max-width:1200px;margin:auto;padding:24px}}header{{border-bottom:1px solid var(--line)}}h1,h2,h3{{margin:.2em 0}}small{{color:var(--muted)}}.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:16px}}section,article{{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:16px}}.metric{{font-size:1.6rem;color:var(--accent)}}table{{width:100%;border-collapse:collapse}}th,td{{padding:8px;border-bottom:1px solid var(--line);text-align:left;vertical-align:top}}.scroll{{overflow:auto}}pre{{white-space:pre-wrap;color:var(--muted)}}
</style>
</head>
<body>
<header><h1>{scenario}</h1><p>Year {year}, day {day} · simulation day {elapsed} · {phase:?}</p></header>
<main>
<div class="grid">
<section><small>Player dynasty</small><h2>House {player}</h2><div class="metric">{treasury}</div><p>Administrative load {load}/{capacity}</p></section>
<section><small>Commercial position</small><div class="metric">{businesses} businesses</div><p>{properties} properties · {loans} current borrowing relationships</p></section>
<section><small>Civic condition</small><div class="metric">{food:.1}% food satisfaction</div><p>{crises} active crises</p></section>
</div>
<h2>Districts</h2><section class="scroll"><table><thead><tr><th>District</th><th>Food</th><th>Employment</th><th>Sanitation</th><th>Unrest</th><th>Causes</th></tr></thead><tbody>{district_rows}</tbody></table></section>
<h2>Market</h2><section class="scroll"><table><thead><tr><th>Good</th><th>Price</th><th>Stock</th><th>Causes</th></tr></thead><tbody>{market_rows}</tbody></table></section>
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
        phase = projection.scenario.phase,
        player = escape_html(&player.name),
        treasury = player.treasury,
        load = player.administrative_load,
        capacity = player.administrative_capacity,
        businesses = player.businesses,
        properties = player.properties,
        loans = player.current_loans_as_borrower,
        food = f64::from(projection.scenario.average_food_satisfaction_basis_points) / 100.0,
        crises = projection.scenario.active_crises,
    ))
}

fn render_district_rows(districts: &[DistrictProjection]) -> String {
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
    let mut rows = String::new();
    for market in markets {
        write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:?}</td></tr>",
            escape_html(&market.good),
            market.price,
            market.stock,
            market.causes,
        )
        .expect("writing HTML into a String cannot fail");
    }
    rows
}

fn render_notifications(notifications: &[NotificationProjection]) -> String {
    let mut alerts = String::new();
    for notification in notifications.iter().rev().take(12) {
        write!(
            alerts,
            "<article><small>Day {} · {:?}</small><h3>{}</h3><p>{}</p></article>",
            notification.day,
            notification.kind,
            escape_html(&notification.subject),
            escape_html(&notification.body),
        )
        .expect("writing HTML into a String cannot fail");
    }
    alerts
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NewGameConfig;
    use crate::registry::build_rivergate_registry;
    use crate::systems::build_new_game;

    #[test]
    fn campaign_projection_contains_every_primary_view() {
        let registry = build_rivergate_registry();
        let state = build_new_game(&registry, NewGameConfig::default());
        let projection = build_campaign_projection(&registry, &state);

        assert_eq!(projection.districts.len(), registry.districts().len());
        assert_eq!(projection.market.len(), registry.goods().len());
        assert_eq!(projection.dynasties.len(), state.dynasties.len());
        assert!(!projection.contracts.is_empty());
        assert!(!projection.institutions.is_empty());
    }

    #[test]
    fn html_dashboard_embeds_projection_data() {
        let registry = build_rivergate_registry();
        let state = build_new_game(&registry, NewGameConfig::default());

        let html = render_campaign_html(&registry, &state).expect("dashboard must render");

        assert!(html.contains("Civic Dynasty"));
        assert!(html.contains("campaign-data"));
        assert!(html.contains("House Valeri"));
    }
}
