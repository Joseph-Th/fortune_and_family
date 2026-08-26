//! Deterministic bootstrap initialization of the strategic runtime state.

#[allow(clippy::wildcard_imports)]
use super::*;

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

pub(crate) fn initialize_districts(registry: &Registry, state: &mut AppState) {
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
            },
        );
    }
}

pub(crate) fn initialize_district_employment(state: &mut AppState) {
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

pub(crate) fn initialize_institutions(registry: &Registry, state: &mut AppState) {
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
                powers: crate::systems::institution_powers_for(definition.kind()),
                budget: Money::from_copper(120_000),
                legitimacy_basis_points: 7_000,
                term_started_day: 0,
                next_selection_day: crate::systems::OFFICE_TERM_DAYS,
                term_number: 1,
                active_directive: None,
            },
        );
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "each authored premises block stays visible beside its neighbors"
)]
pub(crate) fn initialize_properties(registry: &Registry, state: &mut AppState) {
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
                anchor_value: Money::from_copper(28_000),
                value: Money::from_copper(28_000),
                weekly_rent: Money::from_copper(340),
                condition_basis_points: 8_000,
                collateral_loan_id: None,
            },
        );
        // The premises back-pointer lets a firm evicted during insolvency
        // re-occupy its purpose-built workshop once it trades again.
        state
            .businesses
            .get_mut(business_id)
            .expect("seeded business must exist")
            .premises_property_id = Some(property_id);
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
                anchor_value: Money::from_copper(45_000),
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
                anchor_value: Money::from_copper(55_000),
                value: Money::from_copper(55_000),
                weekly_rent: Money::from_copper(140),
                condition_basis_points: 6_500,
                collateral_loan_id: None,
            },
        );
        // A modest workshop gives a rising house an attainable first step on
        // the property ladder: rental income within reach of an established
        // business, while the warehouses stay a mid-game aspiration.
        let property_id = state.next_ids.property();
        state.properties.insert(
            property_id,
            Property {
                id: property_id,
                name: format!("Vacant {} Workshop", district.name()),
                kind: PropertyKind::Workshop,
                district_id: district.id(),
                owner_dynasty_id: None,
                occupant_business_id: None,
                tenant_dynasty_id: None,
                anchor_value: Money::from_copper(24_000),
                value: Money::from_copper(24_000),
                weekly_rent: Money::from_copper(60),
                condition_basis_points: 7_000,
                collateral_loan_id: None,
            },
        );
    }
}

pub(crate) fn initialize_employment(state: &mut AppState) {
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
                    .saturating_mul(crate::systems::WORKERS_PER_BATCH),
            )
        })
        .collect();
    for (business_id, district_id, workers) in businesses {
        let Some(household_id) = state
            .households
            .ids_for_district(district_id)
            .and_then(|ids| {
                ids.iter().find(|id| {
                    crate::systems::available_household_workers(state, **id) >= u32::from(workers)
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

pub(crate) fn initialize_family_governance(state: &mut AppState) {
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

pub(crate) fn initialize_relationships(state: &mut AppState) {
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

pub(crate) fn initialize_laws(state: &mut AppState) {
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

pub(crate) fn initialize_routes(registry: &Registry, state: &mut AppState) {
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

pub(crate) fn initialize_contracts(registry: &Registry, state: &mut AppState) {
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

pub(crate) fn initialize_loans(state: &mut AppState) {
    // Opening credit is flavor between NPC houses only: player lending is a
    // deliberate player command, never an autonomous counterparty position
    // created before the player has acted.
    //
    // Independent lender/borrower pairs, not a chain: a chained arrangement
    // would have one house borrowing in the same pass it lends, funding its
    // own creditor position out of freshly borrowed principal.
    let dynasty_ids: Vec<_> = state
        .dynasties
        .keys()
        .copied()
        .filter(|id| *id != state.player_dynasty_id)
        .collect();
    for pair in dynasty_ids.chunks(2).take(2) {
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
                .find(|property| {
                    property.owner_dynasty_id == Some(*borrower)
                        && property.collateral_loan_id.is_none()
                })
                .map(|property| property.id),
        };
        let token = validate_loan(state, terms)
            .expect("authored bootstrap loan must satisfy strategic validation");
        token
            .commit(state)
            .expect("validated bootstrap loan must commit without intervening mutation");
    }
}

pub(crate) fn initialize_objectives(state: &mut AppState) {
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
                priority: 60 + u16::try_from(index).unwrap_or(0),
                created_day: 0,
                status: ObjectiveStatus::Pursuing,
                rationale: format!("House strategy selected from current assets and institutional access: {kind:?}."),
            },
        );
    }
}

pub(crate) fn initialize_public_works(registry: &Registry, state: &mut AppState) {
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

pub(crate) fn initialize_information(state: &mut AppState) {
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
