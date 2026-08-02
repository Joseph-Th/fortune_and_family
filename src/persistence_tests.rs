use super::*;
use crate::ids::{BusinessId, CharacterId, DynastyId, InstitutionId};
use crate::registry::Registry;
use crate::systems::advance_days;
use crate::test_support::{
    assert_state_eq, make_test_campaign, rivergate_registry_for_test, write_test_json_fixture,
};

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
}
