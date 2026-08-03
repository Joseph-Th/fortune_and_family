//! Persistence round-trip, migration, and release-mode validation tests.

use super::*;
use crate::ids::{BusinessId, CharacterId, DynastyId, InstitutionId};
use crate::registry::Registry;
use crate::systems::{acquire_business, advance_days, quote_business_acquisition};
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
        let selected = &ordered[..2];
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
        assert_eq!(retained.len(), 1);
        assert_eq!(u64::from(retained[0].value()), selected[0].0);
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
    fn rejects_active_unimplemented_law() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let law = value["laws"]
            .as_object_mut()
            .and_then(|laws| laws.values_mut().next())
            .expect("serialized state must contain a law");
        law["kind"] = Value::String("PublicDebtAuthorization".to_owned());
        law["value"] = Value::from(1);
        law["active"] = Value::Bool(true);
        let (_directory, path) = write_test_json_fixture("unsupported-active-law.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "is not implemented",
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
