//! Persistence round-trip, migration, and release-mode validation tests.

use super::*;
use crate::core::{AuditKind, CivicDebt, CivicDebtStatus, EnactedLaw, FamilyLinkKind, LawKind};
use crate::ids::{BusinessId, CharacterId, DynastyId, InstitutionId};
use crate::money::{Money, Quantity};
use crate::registry::Registry;
use crate::systems::{
    EducationFocus, OFFICE_NOMINATION_DELIVERY_REQUIREMENT, PlayerCommand, acquire_business,
    advance_days, apply_player_command, quote_business_acquisition,
};
use crate::test_support::{
    assert_state_eq, make_test_campaign, rivergate_registry_for_test, write_test_json_fixture,
};

#[track_caller]
fn assert_invalid_state(
    result: Result<AppState, PersistenceError>,
    expected_kind: StateValidationKind,
    expected_reason: &str,
) {
    match result {
        Err(PersistenceError::InvalidState { kind, reason, .. }) => {
            assert_eq!(
                kind, expected_kind,
                "unexpected validation category: {reason}"
            );
            assert!(
                reason.contains(expected_reason),
                "validation reason {reason:?} did not contain {expected_reason:?}"
            );
        }
        Err(error) => panic!("expected invalid-state error, got {error:?}"),
        Ok(_) => panic!("invalid save unexpectedly loaded"),
    }
}

#[track_caller]
fn assert_v1_migration(
    registry: &Registry,
    loaded: &AppState,
    dynasty_ids: &[DynastyId],
    character_ids: &[CharacterId],
    business_ids: &[BusinessId],
    officeholders: &BTreeMap<InstitutionId, Option<CharacterId>>,
) {
    assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
    assert_eq!(
        loaded.dynasties.keys().copied().collect::<Vec<_>>(),
        dynasty_ids
    );
    assert_eq!(
        loaded
            .characters
            .records()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        character_ids
    );
    assert_eq!(
        loaded
            .businesses
            .records()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        business_ids
    );
    assert_eq!(loaded.districts.len(), registry.districts().len());
    assert_eq!(loaded.institutions.len(), registry.institutions().len());
    assert_eq!(
        &loaded
            .institutions
            .iter()
            .map(|(institution_id, institution)| {
                (*institution_id, institution.office_holder_id)
            })
            .collect::<BTreeMap<_, _>>(),
        officeholders,
        "migration must preserve legacy officeholders"
    );
    assert!(
        !loaded.contracts.is_empty()
            && !loaded.loans.is_empty()
            && !loaded.properties.is_empty()
            && !loaded.employment.is_empty(),
        "migration must hydrate the connected strategic economy"
    );
    crate::systems::validate_invariants(registry, loaded);
}

mod round_trip {
    use super::*;

    #[test]
    fn preserves_deterministic_state() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        advance_days(registry, &mut state, 40).expect("simulation must advance");
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join("campaign.json");

        save_state(&path, &state).expect("state must save");
        let loaded = load_state(&path).expect("state must load");

        assert_state_eq(
            &state,
            &loaded,
            "save/load round-trip must preserve the complete deterministic state",
        );
    }

    #[test]
    fn preserves_authorized_civic_debt() {
        let mut state = make_test_campaign();
        let creditor_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain a non-player dynasty");
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::PublicDebtAuthorization,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                value: 10_000,
                active: false,
            },
        );
        let debt_id = state.next_ids.civic_debt();
        state.civic_debts.insert(
            debt_id,
            CivicDebt {
                id: debt_id,
                creditor_dynasty_id,
                authorizing_law_id: law_id,
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                principal: Money::from_copper(10_000),
                balance: Money::from_copper(9_500),
                weekly_payment: Money::from_copper(100),
                interest_basis_points: 600,
                issued_day: state.clock.day(),
                next_due_day: state.clock.day().saturating_add(7),
                missed_payments: 0,
                status: CivicDebtStatus::Current,
            },
        );
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join("civic-debt-campaign.json");

        save_state(&path, &state).expect("civic debt state must save");
        let loaded = load_state(&path).expect("civic debt state must load");

        assert_state_eq(
            &state,
            &loaded,
            "save/load must preserve municipal debt authorization and payment state",
        );
    }

    #[test]
    fn preserves_adopted_wards_and_family_education() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        {
            let player = state
                .dynasties
                .get_mut(&player_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(30_000);
            player.resources.legitimacy_basis_points = 5_000;
            player.resources.reputation_reliability_basis_points = 5_500;
        }
        let player_business_ids: BTreeSet<_> = state
            .businesses
            .iter()
            .filter(|business| business.owner_dynasty_id() == player_id)
            .map(crate::core::Business::id)
            .collect();
        let contract = state
            .contracts
            .values_mut()
            .find(|contract| {
                player_business_ids.contains(&contract.buyer_business_id)
                    || player_business_ids.contains(&contract.seller_business_id)
            })
            .expect("campaign must contain a player contract");
        let deliveries = u16::try_from(OFFICE_NOMINATION_DELIVERY_REQUIREMENT)
            .expect("delivery requirement must fit contract counters");
        contract.fulfilled_deliveries = deliveries;
        contract
            .fulfilled_deliveries_by_dynasty
            .insert(player_id, deliveries);

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AdoptWard {
                focus: EducationFocus::Administration,
            },
        )
        .expect("ward adoption must succeed");
        let ward_id = state
            .family_links
            .values()
            .find(|link| link.kind == FamilyLinkKind::Ward)
            .expect("ward link must exist")
            .second_character_id;
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EducateFamilyMember {
                character_id: ward_id,
                focus: EducationFocus::Social,
            },
        )
        .expect("ward education must succeed");
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join("family-campaign.json");

        save_state(&path, &state).expect("family state must save");
        let loaded = load_state(&path).expect("family state must load");

        assert_state_eq(
            &state,
            &loaded,
            "save/load must preserve ward records, family links, education, and histories",
        );
    }

    #[test]
    fn atomic_save_replaces_the_previous_campaign() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join("campaign.json");
        save_state(&path, &state).expect("initial state must save");
        advance_days(registry, &mut state, 7).expect("simulation must advance");

        save_state(&path, &state).expect("updated state must replace the original save");
        let loaded = load_state(&path).expect("replacement state must load");
        let files: Vec<_> = fs::read_dir(directory.path())
            .expect("save directory must be readable")
            .map(|entry| entry.expect("directory entry must be readable").file_name())
            .collect();

        assert_state_eq(
            &state,
            &loaded,
            "atomic replacement must persist the updated campaign",
        );
        assert_eq!(files, vec![path.file_name().expect("save name must exist")]);
    }
}

mod migrations {
    use super::*;

    #[test]
    fn v0_adds_audit_log() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let object = value.as_object_mut().expect("state JSON must be an object");
        object.insert("schema_version".to_owned(), Value::from(0));
        object.remove("audit_log");

        let migrated =
            migrate_to_current(value, Path::new("memory.json")).expect("version zero must migrate");

        assert_eq!(
            migrated["schema_version"],
            Value::from(CURRENT_SCHEMA_VERSION)
        );
        assert!(
            migrated["audit_log"].is_array(),
            "version-zero migration must add the audit log collection"
        );
    }

    #[test]
    fn v2_consolidates_institutions_and_removes_legacy_staffing() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let object = value.as_object_mut().expect("state JSON must be an object");
        object.insert("schema_version".to_owned(), Value::from(2));
        let institutions = object
            .remove("institutions")
            .expect("current state must contain institutions");
        object.insert("institution_runtime".to_owned(), institutions.clone());
        object.insert(
            "institutions".to_owned(),
            Value::Object(serde_json::Map::new()),
        );
        let business_records = object
            .get_mut("businesses")
            .and_then(Value::as_object_mut)
            .and_then(|businesses| businesses.get_mut("records"))
            .and_then(Value::as_object_mut)
            .expect("business records must be an object");
        for business in business_records.values_mut() {
            business["operations"]["employees"] = Value::from(8);
        }

        let migrated =
            migrate_to_current(value, Path::new("memory.json")).expect("version two must migrate");

        assert_eq!(migrated["institutions"], institutions);
        assert!(migrated.get("institution_runtime").is_none());
        assert!(
            migrated["businesses"]["records"]
                .as_object()
                .expect("business records must remain an object")
                .values()
                .all(|business| business["operations"].get("employees").is_none())
        );
    }

    #[test]
    fn v3_removes_the_unused_business_debt_aggregate() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let object = value.as_object_mut().expect("state JSON must be an object");
        object.insert("schema_version".to_owned(), Value::from(3));
        let business_records = object
            .get_mut("businesses")
            .and_then(Value::as_object_mut)
            .and_then(|businesses| businesses.get_mut("records"))
            .and_then(Value::as_object_mut)
            .expect("business records must be an object");
        for business in business_records.values_mut() {
            business["finance"]["debt"] = Value::from(12_345);
        }

        let migrated = migrate_to_current(value, Path::new("memory.json"))
            .expect("version three must migrate");

        assert_eq!(
            migrated["schema_version"],
            Value::from(CURRENT_SCHEMA_VERSION)
        );
        assert!(
            migrated["businesses"]["records"]
                .as_object()
                .expect("business records must remain an object")
                .values()
                .all(|business| business["finance"].get("debt").is_none()),
            "version-three migration must remove the unused debt aggregate"
        );
    }

    #[test]
    fn v4_resolves_duplicate_officeholders_deterministically() {
        let state = make_test_campaign();
        let holder_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let object = value.as_object_mut().expect("state JSON must be an object");
        object.insert("schema_version".to_owned(), Value::from(4));
        let institutions = object
            .get_mut("institutions")
            .and_then(Value::as_object_mut)
            .expect("institutions must be an object");
        let mut ordered: Vec<_> = institutions
            .iter()
            .map(|(key, institution)| {
                (
                    institution["institution_id"]
                        .as_u64()
                        .expect("institution ID must be numeric"),
                    key.clone(),
                )
            })
            .collect();
        ordered.sort_unstable();
        let selected = ordered
            .get(..2)
            .expect("fixture must contain at least two institutions");
        for (_, key) in selected {
            let institution = institutions
                .get_mut(key)
                .and_then(Value::as_object_mut)
                .expect("selected institution must remain present");
            institution.insert(
                "office_holder_id".to_owned(),
                Value::from(holder_id.value()),
            );
            let members = institution
                .get_mut("members")
                .and_then(Value::as_array_mut)
                .expect("institution members must be an array");
            if !members
                .iter()
                .any(|member| member == &Value::from(holder_id.value()))
            {
                members.push(Value::from(holder_id.value()));
            }
        }

        let migrated =
            migrate_to_current(value, Path::new("memory.json")).expect("version four must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        let retained: Vec<_> = loaded
            .institutions
            .iter()
            .filter_map(|(institution_id, institution)| {
                (institution.office_holder_id == Some(holder_id)).then_some(*institution_id)
            })
            .collect();
        let [retained_id] = retained.as_slice() else {
            panic!("migration must retain exactly one office: {retained:?}");
        };
        let expected_id = selected
            .first()
            .expect("selected institutions must not be empty")
            .0;
        assert_eq!(u64::from(retained_id.value()), expected_id);
        validate_state(&loaded).expect("migrated office ownership must be valid");
    }

    #[test]
    fn v5_restores_tenancy_for_separately_owned_business_premises() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() != state.player_dynasty_id)
            .expect("campaign must contain a non-player business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("selected business must exist");
            business.operations.status = crate::core::BusinessStatus::Distressed;
            business.finance.cash = Money::ZERO;
        }
        let buyer_id = state.player_dynasty_id;
        let manager_id = state
            .dynasties
            .get(&buyer_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an eligible heir");
        let quote = quote_business_acquisition(registry, &state, buyer_id, business_id)
            .expect("distressed business must be acquirable");
        state
            .dynasties
            .get_mut(&buyer_id)
            .expect("buyer dynasty must exist")
            .resources
            .treasury = quote
            .purchase_price
            .saturating_add(quote.minimum_recapitalization);
        acquire_business(
            registry,
            &mut state,
            buyer_id,
            business_id,
            manager_id,
            quote.minimum_recapitalization,
        )
        .expect("funded acquisition must succeed");
        let property_id = state
            .properties
            .values()
            .find(|property| property.occupant_business_id == Some(business_id))
            .expect("acquired business must occupy premises")
            .id;
        state
            .properties
            .get_mut(&property_id)
            .expect("business premises must exist")
            .tenant_dynasty_id = None;
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(5);

        let migrated =
            migrate_to_current(value, Path::new("memory.json")).expect("version five must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        assert_eq!(
            loaded
                .properties
                .get(&property_id)
                .expect("business premises must remain present")
                .tenant_dynasty_id,
            Some(buyer_id)
        );
        validate_state(&loaded).expect("migrated tenancy must satisfy release validation");
    }

    #[test]
    fn v6_advances_to_the_family_command_schema() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(6);

        let migrated =
            migrate_to_current(value, Path::new("memory.json")).expect("version six must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        validate_state(&loaded).expect("version-six campaign must remain valid");
    }

    #[test]
    fn v7_adds_stable_institution_term_start_dates() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(7);
        let institutions = value["institutions"]
            .as_object_mut()
            .expect("institutions must be an object");
        for institution in institutions.values_mut() {
            institution
                .as_object_mut()
                .expect("institution must be an object")
                .remove("term_started_day");
        }

        let migrated = migrate_to_current(value, Path::new("memory.json"))
            .expect("version seven must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        assert!(
            loaded.institutions.values().all(|institution| {
                institution.term_started_day
                    == institution
                        .next_selection_day
                        .saturating_sub(crate::systems::OFFICE_TERM_DAYS)
            }),
            "migration must preserve the term timing implied by version-seven saves"
        );
        validate_state(&loaded).expect("version-seven campaign must remain valid");
    }

    #[test]
    fn v8_adds_office_duty_history_fields() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(8);
        let dynasties = value["dynasties"]
            .as_object_mut()
            .expect("dynasties must be an object");
        for dynasty in dynasties.values_mut() {
            let resources = dynasty["resources"]
                .as_object_mut()
                .expect("dynasty resources must be an object");
            resources.remove("civic_contributions");
            resources.remove("unmet_office_duties");
        }

        let migrated = migrate_to_current(value, Path::new("memory.json"))
            .expect("version eight must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        assert!(loaded.dynasties.values().all(|dynasty| {
            dynasty.civic_contributions() == Money::ZERO && dynasty.unmet_office_duties() == 0
        }));
        validate_state(&loaded).expect("version-eight campaign must remain valid");
    }

    #[test]
    fn v9_adds_the_civic_debt_ledger_and_allocator() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(9);
        value
            .as_object_mut()
            .expect("state JSON must be an object")
            .remove("civic_debts");
        value["next_ids"]
            .as_object_mut()
            .expect("next IDs must be an object")
            .remove("civic_debt");
        for contract in value["contracts"]
            .as_object_mut()
            .expect("contracts must be an object")
            .values_mut()
        {
            contract
                .as_object_mut()
                .expect("contract must be an object")
                .remove("breaching_dynasty_id");
        }

        let migrated =
            migrate_to_current(value, Path::new("memory.json")).expect("version nine must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        assert!(loaded.civic_debts.is_empty());
        validate_state(&loaded).expect("version-nine campaign must remain valid");
    }

    #[test]
    fn v10_adds_contract_breach_attribution() {
        let mut state = make_test_campaign();
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::PublicDebtAuthorization,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                value: 10_000,
                active: true,
            },
        );
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(10);
        for contract in value["contracts"]
            .as_object_mut()
            .expect("contracts must be an object")
            .values_mut()
        {
            contract
                .as_object_mut()
                .expect("contract must be an object")
                .remove("breaching_dynasty_id");
        }

        let migrated =
            migrate_to_current(value, Path::new("memory.json")).expect("version ten must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        assert!(
            loaded
                .contracts
                .values()
                .all(|contract| contract.breaching_dynasty_id.is_none())
        );
        assert!(
            loaded
                .laws
                .values()
                .filter(|law| law.kind == LawKind::PublicDebtAuthorization)
                .all(|law| !law.active),
            "version-ten debt authorizations must migrate to consumed one-time laws"
        );
        validate_state(&loaded).expect("version-ten campaign must remain valid");
    }

    #[test]
    fn v11_attributes_historical_deliveries_to_current_contract_parties() {
        let mut state = make_test_campaign();
        let contract_id = *state
            .contracts
            .keys()
            .next()
            .expect("campaign must contain a contract");
        let (buyer_owner_id, seller_owner_id) = {
            let contract = state
                .contracts
                .get(&contract_id)
                .expect("contract must exist");
            (
                state
                    .businesses
                    .get(contract.buyer_business_id)
                    .expect("buyer must exist")
                    .owner_dynasty_id(),
                state
                    .businesses
                    .get(contract.seller_business_id)
                    .expect("seller must exist")
                    .owner_dynasty_id(),
            )
        };
        state
            .contracts
            .get_mut(&contract_id)
            .expect("contract must exist")
            .fulfilled_deliveries = 4;
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(11);
        for contract in value["contracts"]
            .as_object_mut()
            .expect("contracts must be an object")
            .values_mut()
        {
            contract
                .as_object_mut()
                .expect("contract must be an object")
                .remove("fulfilled_deliveries_by_dynasty");
        }

        let migrated = migrate_to_current(value, Path::new("memory.json"))
            .expect("version eleven must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        let contract = loaded
            .contracts
            .get(&contract_id)
            .expect("migrated contract must exist");
        assert_eq!(
            contract
                .fulfilled_deliveries_by_dynasty
                .get(&buyer_owner_id),
            Some(&4)
        );
        assert_eq!(
            contract
                .fulfilled_deliveries_by_dynasty
                .get(&seller_owner_id),
            Some(&4)
        );
        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        validate_state(&loaded).expect("version-eleven campaign must remain valid");
    }

    #[test]
    fn v12_removes_unearned_memberships_and_preserves_nominated_support() {
        let state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let character_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(12);
        let institutions = value["institutions"]
            .as_object_mut()
            .expect("institutions must be an object");
        let mut institution_ids: Vec<u64> = institutions
            .values()
            .map(|institution| {
                institution["institution_id"]
                    .as_u64()
                    .expect("institution ID must be numeric")
            })
            .collect();
        institution_ids.sort_unstable();
        let supported_institution_id = *institution_ids
            .first()
            .expect("campaign must contain institutions");
        for institution in institutions.values_mut() {
            institution["members"]
                .as_array_mut()
                .expect("members must be an array")
                .push(Value::from(character_id.value()));
        }
        value["audit_log"]
            .as_array_mut()
            .expect("audit log must be an array")
            .push(serde_json::json!({
                "day": 0,
                "kind": "OfficeNomination",
                "subject": format!(
                    "institution:{supported_institution_id}:character:{character_id}"
                ),
                "detail": "campaign_cost=300"
            }));

        let migrated = migrate_to_current(value, Path::new("memory.json"))
            .expect("version twelve must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        for (institution_id, institution) in &loaded.institutions {
            assert_eq!(
                institution.members.contains(&character_id),
                *institution_id
                    == InstitutionId::new(
                        u32::try_from(supported_institution_id)
                            .expect("test institution ID must fit u32")
                    ),
                "migration must retain only institution access backed by prior political activity"
            );
        }
        let support_subject =
            format!("institution:{supported_institution_id}:character:{character_id}");
        assert!(loaded.audit_log.iter().any(|record| {
            record.kind() == AuditKind::InstitutionPatronage && record.subject() == support_subject
        }));
        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        validate_state(&loaded).expect("version-twelve campaign must remain valid");
    }

    #[test]
    fn v13_adds_typed_information_targets_and_repairs_impossible_parentage() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let good_id = registry
            .goods()
            .first()
            .expect("registry must contain a good")
            .id();
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: crate::systems::InformationFocus::Market { good_id },
            },
        )
        .expect("market commission must succeed");
        let report_id = state
            .information_reports
            .values()
            .find(|report| report.subject.starts_with("Commissioned market brief:"))
            .expect("commissioned report must exist")
            .id();
        let parent_link = state
            .family_links
            .values()
            .find(|link| link.kind == FamilyLinkKind::ParentChild)
            .expect("campaign must contain a parent-child link")
            .clone();
        let parent_birth_day = state
            .characters
            .get(parent_link.first_character_id)
            .expect("parent must exist")
            .birth_day();
        state
            .characters
            .get_mut(parent_link.second_character_id)
            .expect("child must exist")
            .identity
            .birth_day = parent_birth_day.saturating_add(1);
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(13);
        for report in value["information_reports"]
            .as_object_mut()
            .expect("information reports must be an object")
            .values_mut()
        {
            report
                .as_object_mut()
                .expect("information report must be an object")
                .remove("target");
        }

        let migrated = migrate_to_current(value, Path::new("memory.json"))
            .expect("version thirteen must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(
            loaded
                .information_reports
                .get(&report_id)
                .expect("commissioned report must survive migration")
                .target,
            Some(crate::core::InformationTarget::Market { good_id })
        );
        assert_eq!(
            loaded
                .family_links
                .get(&parent_link.id)
                .expect("family link must survive migration")
                .kind,
            FamilyLinkKind::Sibling,
            "legacy impossible parentage must become a collateral-family relationship"
        );
        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        validate_state(&loaded).expect("version-thirteen campaign must remain valid");
    }

    #[test]
    fn v1_hydrates_strategic_state() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign();
        let dynasty_ids: Vec<_> = state.dynasties.keys().copied().collect();
        let character_ids: Vec<_> = state.characters.records().keys().copied().collect();
        let business_ids: Vec<_> = state.businesses.records().keys().copied().collect();
        let officeholders: BTreeMap<_, _> = state
            .institutions
            .iter()
            .map(|(institution_id, institution)| (*institution_id, institution.office_holder_id))
            .collect();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let object = value.as_object_mut().expect("state JSON must be an object");
        object.insert("schema_version".to_owned(), Value::from(1));
        for field in [
            "institution_runtime",
            "contracts",
            "loans",
            "properties",
            "employment",
            "family_links",
            "family_councils",
            "laws",
            "relationships",
            "information_reports",
            "ai_objectives",
            "districts",
            "public_works",
            "legal_cases",
            "external_routes",
            "crises",
            "outbox",
        ] {
            object.remove(field);
        }
        let next_ids = object
            .get_mut("next_ids")
            .and_then(Value::as_object_mut)
            .expect("next IDs must be an object");
        for field in [
            "contract",
            "property",
            "loan",
            "employment",
            "family_link",
            "law",
            "information_report",
            "objective",
            "public_work",
            "legal_case",
            "external_route",
            "crisis",
            "outbox",
        ] {
            next_ids.remove(field);
        }
        let (_directory, path) = write_test_json_fixture("version-one.json", &value);

        let loaded = load_state(&path).expect("version one save must load");

        assert_v1_migration(
            registry,
            &loaded,
            &dynasty_ids,
            &character_ids,
            &business_ids,
            &officeholders,
        );
    }
}

mod validation {
    use super::*;

    #[test]
    fn rejects_zero_dynasty_generation() {
        let mut state = make_test_campaign();
        state
            .dynasties
            .values_mut()
            .next()
            .expect("campaign must contain a dynasty")
            .runtime
            .generation = 0;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("zero-generation.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid resource value",
        );
    }

    #[test]
    fn rejects_out_of_range_district_rent_index() {
        let mut state = make_test_campaign();
        state
            .districts
            .values_mut()
            .next()
            .expect("campaign must contain a district")
            .rent_index_basis_points = crate::systems::MAX_DISTRICT_RENT_INDEX_BASIS_POINTS + 1;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("invalid-rent-index.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid basis-point value",
        );
    }

    #[test]
    fn rejects_future_dated_institution_term_start() {
        let mut state = make_test_campaign();
        let future_day = state.clock.day().saturating_add(1);
        state
            .institutions
            .values_mut()
            .next()
            .expect("campaign must contain an institution")
            .term_started_day = future_day;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("future-office-term.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid budget or term timing",
        );
    }

    #[test]
    fn rejects_ward_links_that_cross_dynasties() {
        let mut state = make_test_campaign();
        let link_id = *state
            .family_links
            .keys()
            .next()
            .expect("campaign must contain a family link");
        let first_character_id = state
            .family_links
            .get(&link_id)
            .expect("selected family link must exist")
            .first_character_id;
        let first_dynasty_id = state
            .characters
            .get(first_character_id)
            .expect("family link character must exist")
            .dynasty_id();
        let foreign_character_id = state
            .characters
            .iter()
            .find(|character| character.dynasty_id() != first_dynasty_id)
            .expect("campaign must contain another dynasty")
            .id();
        let link = state
            .family_links
            .get_mut(&link_id)
            .expect("selected family link must exist");
        link.kind = FamilyLinkKind::Ward;
        link.second_character_id = foreign_character_id;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("cross-dynasty-ward.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "crosses dynasties",
        );
    }

    #[test]
    fn rejects_exhausted_identifier_allocator() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["next_ids"]["business"] = Value::from(u32::MAX);
        let (_directory, path) = write_test_json_fixture("exhausted-business-id.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::IdentifierAllocation,
            "exhausted the supported identifier space",
        );
    }

    #[test]
    fn rejects_public_work_progress_inconsistent_with_spending() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let work = value["public_works"]
            .as_object_mut()
            .and_then(|works| works.values_mut().next())
            .expect("serialized state must contain a public work");
        work["progress_basis_points"] = Value::from(9_999);
        let (_directory, path) = write_test_json_fixture("invalid-work-progress.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "progress does not match its spending or lifecycle",
        );
    }

    #[test]
    fn save_rejects_invalid_state_before_creating_files() {
        let mut state = make_test_campaign();
        state
            .businesses
            .iter_mut()
            .next()
            .expect("campaign must contain a business")
            .finance
            .cash = Money::from_copper(-1);
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let parent = directory.path().join("nested");
        let path = parent.join("invalid.json");

        let result = save_state(&path, &state);

        match result {
            Err(PersistenceError::InvalidState { kind, reason, .. }) => {
                assert_eq!(kind, StateValidationKind::NumericRanges);
                assert!(reason.contains("invalid economic value"));
            }
            Err(error) => panic!("expected invalid-state error, got {error:?}"),
            Ok(()) => panic!("invalid in-memory state unexpectedly saved"),
        }
        assert!(
            !parent.exists(),
            "validation must run before persistence creates directories or files"
        );
    }

    #[test]
    fn rejects_missing_player_dynasty_reference() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["player_dynasty_id"] = Value::from(u32::MAX);
        let (_directory, path) = write_test_json_fixture("invalid-player.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::PrimaryRecords,
            "player dynasty does not exist",
        );
    }

    #[test]
    fn rejects_stale_business_indexes() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["businesses"]["by_owner"] = Value::Object(serde_json::Map::new());
        let (_directory, path) = write_test_json_fixture("stale-index.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::PrimaryRecords,
            "business ownership or district index is stale or incomplete",
        );
    }

    #[test]
    fn rejects_stale_next_id_allocators() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["next_ids"]["business"] = Value::from(0);
        let (_directory, path) = write_test_json_fixture("stale-next-id.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::IdentifierAllocation,
            "next business ID",
        );
    }

    #[test]
    fn rejects_negative_business_cash() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let business = value["businesses"]["records"]
            .as_object_mut()
            .and_then(|records| records.values_mut().next())
            .expect("serialized state must contain a business");
        business["finance"]["cash"] = Value::from(-1);
        let (_directory, path) = write_test_json_fixture("negative-business-cash.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid economic value",
        );
    }

    #[test]
    fn rejects_negative_business_lifetime_totals() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let business = value["businesses"]["records"]
            .as_object_mut()
            .and_then(|records| records.values_mut().next())
            .expect("serialized state must contain a business");
        business["finance"]["lifetime_revenue"] = Value::from(-1);
        let (_directory, path) = write_test_json_fixture("negative-business-lifetime.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid economic value",
        );
    }

    #[test]
    fn rejects_business_policy_outside_command_ranges() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let business = value["businesses"]["records"]
            .as_object_mut()
            .and_then(|records| records.values_mut().next())
            .expect("serialized state must contain a business");
        business["policy"]["target_output_days"] = Value::from(31);
        let (_directory, path) = write_test_json_fixture("invalid-business-policy.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid economic value",
        );
    }

    #[test]
    fn rejects_contracts_with_unrepresentable_weekly_invoices() {
        let mut state = make_test_campaign();
        let contract = state
            .contracts
            .values_mut()
            .next()
            .expect("campaign must contain a supply contract");
        contract.quantity_per_week = Quantity::from_milliunits(i64::MAX);
        contract.unit_price = Money::from_copper(i64::MAX);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("overflowing-contract-invoice.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "supply contract",
        );
    }

    #[test]
    fn rejects_inactive_dynasty_head() {
        let state = make_test_campaign();
        let head_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let character = value["characters"]["records"]
            .as_object_mut()
            .and_then(|records| {
                records.values_mut().find(|character| {
                    character["identity"]["id"].as_u64() == Some(u64::from(head_id.value()))
                })
            })
            .expect("serialized state must contain the player head");
        character["runtime"]["status"] = Value::String("Deceased".to_owned());
        let (_directory, path) = write_test_json_fixture("inactive-head.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::PrimaryRecords,
            "inactive head or heir",
        );
    }

    #[test]
    fn rejects_same_character_as_dynasty_head_and_heir() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let dynasty = value["dynasties"]
            .as_object_mut()
            .and_then(|dynasties| dynasties.values_mut().next())
            .expect("serialized state must contain a dynasty");
        let head_id = dynasty["relationships"]["head_id"].clone();
        dynasty["relationships"]["heir_id"] = head_id;
        let (_directory, path) = write_test_json_fixture("same-head-and-heir.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::PrimaryRecords,
            "same character as head and heir",
        );
    }

    #[test]
    fn rejects_dynasty_head_with_wrong_role() {
        let state = make_test_campaign();
        let head_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let character = value["characters"]["records"]
            .as_object_mut()
            .and_then(|records| {
                records.values_mut().find(|character| {
                    character["identity"]["id"].as_u64() == Some(u64::from(head_id.value()))
                })
            })
            .expect("serialized state must contain the player head");
        character["runtime"]["role"] = Value::String("Heir".to_owned());
        let (_directory, path) = write_test_json_fixture("wrong-head-role.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::PrimaryRecords,
            "wrong role",
        );
    }

    #[test]
    fn rejects_stale_administrative_load() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let dynasty = value["dynasties"]
            .as_object_mut()
            .and_then(|dynasties| dynasties.values_mut().next())
            .expect("serialized state must contain a dynasty");
        let current = dynasty["resources"]["administrative_load"]
            .as_u64()
            .expect("administrative load must be numeric");
        dynasty["resources"]["administrative_load"] = Value::from(current + 1);
        let (_directory, path) = write_test_json_fixture("stale-administrative-load.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::PrimaryRecords,
            "does not match derived load",
        );
    }

    #[test]
    fn rejects_negative_civic_contributions() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let dynasty = value["dynasties"]
            .as_object_mut()
            .and_then(|dynasties| dynasties.values_mut().next())
            .expect("serialized state must contain a dynasty");
        dynasty["resources"]["civic_contributions"] = Value::from(-1);
        let (_directory, path) = write_test_json_fixture("negative-civic-duty.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid resource value",
        );
    }

    #[test]
    fn rejects_unsettled_zero_balance_loan() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let loan = value["loans"]
            .as_object_mut()
            .and_then(|loans| loans.values_mut().next())
            .expect("serialized state must contain a loan");
        loan["balance"] = Value::from(0);
        loan["status"] = Value::String("Current".to_owned());
        let (_directory, path) = write_test_json_fixture("zero-balance-current-loan.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "unsettled loan",
        );
    }

    #[test]
    fn rejects_operational_employment_for_an_insolvent_business() {
        let state = make_test_campaign();
        let business_id = state
            .employment
            .values()
            .find(|agreement| agreement.status == crate::core::EmploymentStatus::Active)
            .expect("campaign must contain active employment")
            .business_id;
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let business = value["businesses"]["records"]
            .as_object_mut()
            .and_then(|records| {
                records.values_mut().find(|business| {
                    business["identity"]["id"].as_u64() == Some(u64::from(business_id.value()))
                })
            })
            .expect("serialized state must contain the employment business");
        business["operations"]["status"] = Value::String("Insolvent".to_owned());
        let (_directory, path) =
            write_test_json_fixture("active-insolvent-employment.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "incompatible with its business lifecycle",
        );
    }

    #[test]
    fn rejects_defaulted_loan_with_unseized_collateral() {
        let mut state = make_test_campaign();
        let loan = state
            .loans
            .values_mut()
            .find(|loan| loan.collateral_property_id.is_some())
            .expect("campaign must contain a collateralized loan");
        loan.status = crate::core::LoanStatus::Defaulted;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("unseized-default-collateral.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "invalid collateral relationship",
        );
    }

    #[test]
    fn rejects_active_contract_due_after_its_term() {
        let mut state = make_test_campaign();
        let contract = state
            .contracts
            .values_mut()
            .find(|contract| contract.status == crate::core::ContractStatus::Active)
            .expect("campaign must contain an active contract");
        contract.next_due_day = contract.end_day.saturating_add(1);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("late-active-contract.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "incompatible with its parties or term",
        );
    }

    #[test]
    fn rejects_contract_breach_attribution_on_an_active_contract() {
        let mut state = make_test_campaign();
        let contract_id = state
            .contracts
            .values()
            .find(|contract| contract.status == crate::core::ContractStatus::Active)
            .expect("campaign must contain an active contract")
            .id;
        let seller_business_id = state
            .contracts
            .get(&contract_id)
            .expect("contract must exist")
            .seller_business_id;
        let seller_dynasty_id = state
            .businesses
            .get(seller_business_id)
            .expect("contract seller must exist")
            .owner_dynasty_id();
        state
            .contracts
            .get_mut(&contract_id)
            .expect("contract must exist")
            .breaching_dynasty_id = Some(seller_dynasty_id);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("active-contract-breacher.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "incompatible with its parties or term",
        );
    }

    #[test]
    fn rejects_unattributed_contract_fulfillment() {
        let mut state = make_test_campaign();
        let contract = state
            .contracts
            .values_mut()
            .next()
            .expect("campaign must contain a contract");
        contract.fulfilled_deliveries = 1;
        contract.fulfilled_deliveries_by_dynasty.clear();
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("unattributed-contract-delivery.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "incompatible with its parties or term",
        );
    }

    #[test]
    fn rejects_contract_delivery_credit_for_a_missing_dynasty() {
        let mut state = make_test_campaign();
        let contract = state
            .contracts
            .values_mut()
            .next()
            .expect("campaign must contain a contract");
        contract.fulfilled_deliveries = 1;
        contract.fulfilled_deliveries_by_dynasty.clear();
        contract
            .fulfilled_deliveries_by_dynasty
            .insert(DynastyId::new(u32::MAX), 1);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("missing-contract-credit-dynasty.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "incompatible with its parties or term",
        );
    }

    #[test]
    fn rejects_civic_debt_with_missing_creditor() {
        let mut state = make_test_campaign();
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::PublicDebtAuthorization,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                value: 10_000,
                active: false,
            },
        );
        let debt_id = state.next_ids.civic_debt();
        state.civic_debts.insert(
            debt_id,
            CivicDebt {
                id: debt_id,
                creditor_dynasty_id: DynastyId::new(u32::MAX),
                authorizing_law_id: law_id,
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                principal: Money::from_copper(10_000),
                balance: Money::from_copper(10_000),
                weekly_payment: Money::from_copper(100),
                interest_basis_points: 600,
                issued_day: state.clock.day(),
                next_due_day: state.clock.day().saturating_add(7),
                missed_payments: 0,
                status: CivicDebtStatus::Current,
            },
        );
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("missing-civic-creditor.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "invalid identity or authorization reference",
        );
    }

    #[test]
    fn rejects_civic_debt_funded_by_its_own_sponsor() {
        let mut state = make_test_campaign();
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::PublicDebtAuthorization,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                value: 10_000,
                active: false,
            },
        );
        let debt_id = state.next_ids.civic_debt();
        state.civic_debts.insert(
            debt_id,
            CivicDebt {
                id: debt_id,
                creditor_dynasty_id: state.player_dynasty_id,
                authorizing_law_id: law_id,
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                principal: Money::from_copper(10_000),
                balance: Money::from_copper(10_000),
                weekly_payment: Money::from_copper(100),
                interest_basis_points: 600,
                issued_day: state.clock.day(),
                next_due_day: state.clock.day().saturating_add(7),
                missed_payments: 0,
                status: CivicDebtStatus::Current,
            },
        );
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("self-funded-civic-debt.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "invalid identity or authorization reference",
        );
    }

    #[test]
    fn rejects_active_consumed_public_debt_authorization() {
        let mut state = make_test_campaign();
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::PublicDebtAuthorization,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                value: 10_000,
                active: true,
            },
        );
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("active-consumed-debt-authorization.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "consumed one-time authorization",
        );
    }

    #[test]
    fn rejects_future_dated_outbox_message() {
        let state = make_test_campaign();
        let future_day = state.clock.day().saturating_add(1);
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let message = value["outbox"]
            .as_array_mut()
            .and_then(|messages| messages.first_mut())
            .expect("serialized state must contain an outbox message");
        message["day"] = Value::from(future_day);
        let (_directory, path) = write_test_json_fixture("future-outbox.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "not chronologically valid",
        );
    }

    #[test]
    fn rejects_household_labor_overallocation() {
        let state = make_test_campaign();
        let agreement = state
            .employment
            .values()
            .next()
            .expect("campaign must contain employment");
        let household_id = agreement.household_id;
        let members = agreement.workers.saturating_sub(1).max(1);
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let household = value["households"]["records"]
            .as_object_mut()
            .and_then(|records| {
                records.values_mut().find(|household| {
                    household["id"].as_u64() == Some(u64::from(household_id.value()))
                })
            })
            .expect("serialized state must contain the employed household");
        household["members"] = Value::from(members);
        let (_directory, path) = write_test_json_fixture("overallocated-labor.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "exceeds household labor capacity",
        );
    }

    #[test]
    fn rejects_duplicate_unresolved_legal_cases() {
        let mut state = make_test_campaign();
        let mut duplicate = state
            .legal_cases
            .values()
            .next()
            .expect("campaign must contain a legal case")
            .clone();
        let duplicate_id = state.next_ids.legal_case();
        duplicate.id = duplicate_id;
        state.legal_cases.insert(duplicate_id, duplicate);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("duplicate-legal-case.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "duplicates an unresolved case",
        );
    }

    #[test]
    fn rejects_out_of_order_audit_history() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        advance_days(registry, &mut state, 2).expect("simulation must advance");
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["audit_log"]
            .as_array_mut()
            .expect("audit log must be an array")
            .reverse();
        let (_directory, path) = write_test_json_fixture("unordered-audit.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "audit log is not chronologically valid",
        );
    }

    #[test]
    fn rejects_duplicate_property_occupants() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let properties = value["properties"]
            .as_object_mut()
            .expect("properties must be an object");
        let occupant = properties
            .values()
            .filter_map(|property| property.get("occupant_business_id").cloned())
            .find(|occupant| !occupant.is_null())
            .expect("bootstrap must create an occupied property");
        let unoccupied = properties
            .values_mut()
            .find(|property| property["occupant_business_id"].is_null())
            .expect("bootstrap must create an unoccupied property");
        unoccupied["occupant_business_id"] = occupant;
        let (_directory, path) = write_test_json_fixture("duplicate-occupant.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "invalid or duplicate occupant",
        );
    }

    #[test]
    fn rejects_unowned_occupied_property() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let property = value["properties"]
            .as_object_mut()
            .and_then(|properties| {
                properties
                    .values_mut()
                    .find(|property| !property["occupant_business_id"].is_null())
            })
            .expect("bootstrap must create an occupied property");
        property["owner_dynasty_id"] = Value::Null;
        property["tenant_dynasty_id"] = Value::Null;
        let (_directory, path) = write_test_json_fixture("unowned-occupied-property.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "invalid or duplicate occupant",
        );
    }
}
