//! Persistence round-trip, migration, and release-mode validation tests.

use super::*;
use crate::core::{
    AuditKind, AuditRecord, CampaignPhase, CivicDebt, CivicDebtStatus, Crisis, CrisisKind,
    CrisisStatus, EnactedLaw, FamilyLinkKind, LawKind, LegalCase, LegalCaseKind, LegalCaseStatus,
    LegalClaimSource, OfficeDirectiveState, OfficePower,
};
use crate::ids::{BusinessId, CharacterId, DynastyId, InstitutionId};
use crate::money::{Money, Quantity};
use crate::registry::Registry;
use crate::systems::{
    EducationFocus, OFFICE_NOMINATION_DELIVERY_REQUIREMENT,
    OFFICE_NOMINATION_REPUTATION_REQUIREMENT, PlayerCommand, acquire_business, advance_days,
    apply_player_command, quote_business_acquisition,
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
        crate::systems::rebuild_campaign_phases(&mut state);
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
    fn preserves_active_office_directive() {
        let mut state = make_test_campaign();
        let institution = state
            .institutions
            .values_mut()
            .find(|institution| institution.powers.contains(&OfficePower::Inspections))
            .expect("campaign must contain an institution with inspection power");
        institution.active_directive = Some(OfficeDirectiveState {
            power: OfficePower::Inspections,
            expires_day: 180,
        });
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join("office-directive-campaign.json");

        save_state(&path, &state).expect("office directive state must save");
        let loaded = load_state(&path).expect("office directive state must load");

        assert_state_eq(
            &state,
            &loaded,
            "save/load must preserve active office directive momentum",
        );
    }

    #[test]
    fn preserves_grounded_legal_claim_source() {
        let mut state = make_test_campaign();
        let loan = state
            .loans
            .values()
            .next()
            .expect("campaign must contain a loan")
            .clone();
        let case_id = state.next_ids.legal_case();
        state.legal_cases.insert(
            case_id,
            LegalCase {
                id: case_id,
                plaintiff_dynasty_id: loan.lender_dynasty_id,
                defendant_dynasty_id: loan.borrower_dynasty_id,
                kind: LegalCaseKind::Debt,
                claim_source: Some(LegalClaimSource::Loan { loan_id: loan.id }),
                evidence_basis_points: 7_500,
                public_attention_basis_points: 1_500,
                filed_day: state.clock.day(),
                hearing_day: state.clock.day().saturating_add(60),
                damages: loan.balance,
                status: LegalCaseStatus::Filed,
            },
        );
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join("grounded-legal-claim.json");

        save_state(&path, &state).expect("grounded legal claim state must save");
        let loaded = load_state(&path).expect("grounded legal claim state must load");

        assert_state_eq(
            &state,
            &loaded,
            "save/load must preserve the exact obligation underlying a legal claim",
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
    fn load_repairs_active_ward_with_inactive_guardian() {
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let ward_id = state
            .dynasties
            .get(&dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let guardian_id = state.next_ids.character();
        let mut guardian = state
            .characters
            .get(ward_id)
            .expect("player heir must exist")
            .clone();
        guardian.identity.id = guardian_id;
        guardian.identity.name = "Inactive Guardian".to_owned();
        guardian.runtime.status = crate::core::CharacterStatus::Incapacitated;
        guardian.runtime.health_basis_points = 0;
        guardian.runtime.role = crate::core::CharacterRole::Clerk;
        state.characters.insert(guardian);
        let link_id = state.next_ids.family_link();
        state.family_links.insert(
            link_id,
            crate::core::FamilyLink {
                id: link_id,
                first_character_id: guardian_id,
                second_character_id: ward_id,
                kind: FamilyLinkKind::Ward,
                active: true,
                property_claim_basis_points: 1_500,
            },
        );
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("stale-active-ward.json", &value);

        let loaded = load_state(&path).expect("stale ward lifecycle must be normalized on load");

        assert!(
            !loaded
                .family_links
                .get(&link_id)
                .expect("ward link must remain recorded")
                .active,
            "load normalization must deactivate guardianship with an inactive guardian"
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
    fn v1_resets_strategic_allocators_that_did_not_exist_in_that_schema() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(1);
        let next_ids = value["next_ids"]
            .as_object_mut()
            .expect("next IDs must be an object");
        let introduced_fields = [
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
        ];
        for field in introduced_fields {
            next_ids.insert(field.to_owned(), Value::from(u32::MAX - 1));
        }

        let migrated = migrate_v1_to_v2(value).expect("version one must migrate");

        for field in introduced_fields {
            assert_eq!(
                migrated["next_ids"][field],
                Value::from(0),
                "schema-one saves must not inject allocator state for future namespace {field}"
            );
        }
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
    fn v14_adds_explicit_office_directive_state() {
        let state = make_test_campaign();
        let institution_id = state
            .institutions
            .values()
            .find(|institution| institution.powers.contains(&OfficePower::Inspections))
            .expect("campaign must contain an institution with inspection power")
            .institution_id;
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(14);
        for institution in value["institutions"]
            .as_object_mut()
            .expect("institutions must be an object")
            .values_mut()
        {
            institution
                .as_object_mut()
                .expect("institution must be an object")
                .remove("active_directive");
        }
        value["audit_log"]
            .as_array_mut()
            .expect("audit log must be an array")
            .push(serde_json::json!({
                "day": 0,
                "kind": "OfficeDirective",
                "subject": format!("institution:{institution_id}"),
                "detail": "district=1;power=Inspections;legitimacy_cost=100"
            }));

        let migrated = migrate_to_current(value, Path::new("memory.json"))
            .expect("version fourteen must migrate");
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");

        assert_eq!(
            loaded
                .institutions
                .get(&institution_id)
                .expect("migrated institution must exist")
                .active_directive,
            Some(OfficeDirectiveState {
                power: OfficePower::Inspections,
                expires_day: 180,
            }),
            "recent legacy directives must retain their remaining policy duration"
        );
        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        validate_state(&loaded).expect("version-fourteen campaign must remain valid");
    }

    #[test]
    fn v15_adds_optional_legal_claim_source() {
        let mut state = make_test_campaign();
        let mut dynasty_ids = state.dynasties.keys().copied();
        let plaintiff_dynasty_id = dynasty_ids.next().expect("campaign must contain a dynasty");
        let defendant_dynasty_id = dynasty_ids
            .next()
            .expect("campaign must contain a second dynasty");
        let case_id = state.next_ids.legal_case();
        state.legal_cases.insert(
            case_id,
            LegalCase {
                id: case_id,
                plaintiff_dynasty_id,
                defendant_dynasty_id,
                kind: LegalCaseKind::Fraud,
                claim_source: None,
                evidence_basis_points: 6_500,
                public_attention_basis_points: 2_000,
                filed_day: state.clock.day(),
                hearing_day: state.clock.day().saturating_add(60),
                damages: Money::from_copper(2_500),
                status: LegalCaseStatus::Filed,
            },
        );
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(15);
        for legal_case in value["legal_cases"]
            .as_object_mut()
            .expect("legal cases must be an object")
            .values_mut()
        {
            legal_case
                .as_object_mut()
                .expect("legal case must be an object")
                .remove("claim_source");
        }
        for contract in value["contracts"]
            .as_object_mut()
            .expect("contracts must be an object")
            .values_mut()
        {
            let contract = contract
                .as_object_mut()
                .expect("contract must be an object");
            contract.remove("breach_victim_dynasty_id");
            contract.remove("unpaid_breach_penalty");
        }

        let migrated = migrate_v15_to_v16(value).expect("version fifteen must migrate");

        assert_eq!(migrated["schema_version"], Value::from(16));
        assert_eq!(
            migrated["legal_cases"][case_id.value().to_string()]["claim_source"],
            Value::Null,
            "legacy cases must remain valid but explicitly record that no source obligation was persisted"
        );
        assert!(
            migrated["contracts"]
                .as_object()
                .expect("migrated contracts must be an object")
                .values()
                .all(|contract| contract["breach_victim_dynasty_id"] == Value::Null),
            "legacy contracts must explicitly record that no historical breach victim was persisted"
        );
        assert!(
            migrated["contracts"]
                .as_object()
                .expect("migrated contracts must be an object")
                .values()
                .all(|contract| contract["unpaid_breach_penalty"] == 0),
            "legacy contracts must not invent an unpaid breach obligation that was never persisted"
        );
        let loaded: AppState =
            serde_json::from_value(migrated).expect("migrated state must deserialize");
        assert_eq!(
            loaded
                .legal_cases
                .get(&case_id)
                .expect("migrated legal case must exist")
                .claim_source,
            None
        );
        assert_eq!(
            loaded.schema_version(),
            16,
            "the focused migration must stop at its declared target schema"
        );
    }

    #[test]
    fn v16_normalizes_progression_and_expired_time_limited_state() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        crate::systems::advance_days(&crate::registry::build_rivergate_registry(), &mut state, 1)
            .expect("migration fixture must advance through the canonical simulation path");
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .runtime
            .phase = CampaignPhase::Dominion;
        assert!(
            !state.information_reports.is_empty(),
            "campaign must contain information reports after one simulated day"
        );
        for report in state.information_reports.values_mut() {
            report.expires_day = 0;
        }
        let institution_id = state
            .institutions
            .iter()
            .find(|(_, institution)| !institution.powers.is_empty())
            .map(|(institution_id, _)| *institution_id)
            .expect("campaign must contain an institution with office powers");
        let power = state
            .institutions
            .get(&institution_id)
            .and_then(|institution| institution.powers.iter().next().copied())
            .expect("institution must expose an office power");
        state
            .institutions
            .get_mut(&institution_id)
            .expect("institution must exist")
            .active_directive = Some(OfficeDirectiveState {
            power,
            expires_day: 0,
        });
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(16);
        let (_directory, path) = write_test_json_fixture("v16-normalization.json", &value);

        let loaded = load_state(&path).expect("version-sixteen campaign must migrate");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        assert_eq!(
            loaded
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Foundation,
            "calendar-age phase state must be rebuilt from durable campaign progression"
        );
        assert!(
            loaded.information_reports.is_empty(),
            "expired reports must not survive migration into the stricter lifecycle"
        );
        assert!(
            loaded
                .institutions
                .get(&institution_id)
                .expect("institution must exist")
                .active_directive
                .is_none(),
            "expired directives must not survive migration into the stricter lifecycle"
        );
    }

    #[test]
    fn v17_preserves_existing_history_when_endowment_audit_kind_is_introduced() {
        let mut state = make_test_campaign();
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CashTransfer,
            subject: format!("dynasty:{}", state.player_dynasty_id).into(),
            detail: "legacy treasury history".to_owned(),
        });
        let expected = state.clone();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(17);
        let (_directory, path) = write_test_json_fixture("v17-endowment-kind.json", &value);

        let loaded = load_state(&path).expect("version-seventeen campaign must migrate");

        assert_state_eq(
            &expected,
            &loaded,
            "adding an audit kind must not rewrite existing version-seventeen history",
        );
        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn v18_attributes_legacy_office_directives_to_the_player_dynasty() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: format!("institution:{institution_id};dynasty:{player_id}").into(),
            detail: "legacy player directive".to_owned(),
        });
        crate::systems::refresh_campaign_phases(&mut state);
        let expected = state.clone();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(18);
        let audit_log = value["audit_log"]
            .as_array_mut()
            .expect("audit log must be an array");
        let directive = audit_log
            .iter_mut()
            .find(|record| record.get("kind").and_then(Value::as_str) == Some("OfficeDirective"))
            .expect("fixture must contain an office directive");
        directive["subject"] = Value::String(format!("institution:{institution_id}"));
        let (_directory, path) =
            write_test_json_fixture("v18-office-directive-attribution.json", &value);

        let loaded = load_state(&path).expect("version-eighteen campaign must migrate");

        assert_state_eq(
            &expected,
            &loaded,
            "version-eighteen player directives must gain durable actor attribution",
        );
        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn v16_credits_prior_collateral_seizure_against_defaulted_balance() {
        let mut state = make_test_campaign();
        let loan_id = state
            .loans
            .values()
            .find(|loan| loan.collateral_property_id.is_some())
            .map(|loan| loan.id)
            .expect("campaign must contain a collateralized loan");
        let (property_id, lender_id) = {
            let loan = state.loans.get(&loan_id).expect("loan must exist");
            (
                loan.collateral_property_id
                    .expect("selected loan must have collateral"),
                loan.lender_dynasty_id,
            )
        };
        let collateral_recovery = state
            .properties
            .get(&property_id)
            .expect("collateral property must exist")
            .value
            .saturating_mul_ratio(crate::systems::PROPERTY_LIQUIDATION_BASIS_POINTS, 10_000);
        {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.balance = collateral_recovery
                .checked_add(Money::from_copper(123))
                .expect("fixture balance must fit");
            loan.status = crate::core::LoanStatus::Defaulted;
            loan.missed_payments = 3;
        }
        let occupant_owner_id = state
            .properties
            .get(&property_id)
            .and_then(|property| property.occupant_business_id)
            .map(|business_id| {
                state
                    .businesses
                    .get(business_id)
                    .expect("collateral occupant business must exist")
                    .owner_dynasty_id()
            });
        {
            let property = state
                .properties
                .get_mut(&property_id)
                .expect("collateral property must exist");
            property.owner_dynasty_id = Some(lender_id);
            property.tenant_dynasty_id = occupant_owner_id.filter(|id| *id != lender_id);
            property.collateral_loan_id = None;
        }
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["schema_version"] = Value::from(16);
        let (_directory, path) = write_test_json_fixture("v16-collateral-recovery.json", &value);

        let loaded = load_state(&path).expect("version-sixteen campaign must migrate");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        let loan = loaded
            .loans
            .get(&loan_id)
            .expect("migrated loan must exist");
        assert_eq!(loan.status, crate::core::LoanStatus::Defaulted);
        assert_eq!(
            loan.balance,
            Money::from_copper(123),
            "schema-sixteen collateral seizures must credit liquidation value against the outstanding balance"
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
    fn rejects_current_office_directive_without_actor_attribution() {
        let mut state = make_test_campaign();
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: format!("institution:{institution_id}").into(),
            detail: "untagged current directive".to_owned(),
        });
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("untagged-current-office-directive.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "OfficeDirective audit record lacks dynasty attribution",
        );
    }

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
    fn rejects_current_save_with_stale_campaign_phase() {
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_reliability_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("stale-campaign-phase.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::PrimaryRecords,
            "stale or incompatible campaign phase",
        );
    }

    #[test]
    fn rejects_current_save_with_unearned_advanced_campaign_phase() {
        for phase in [CampaignPhase::Ascendancy, CampaignPhase::Dominion] {
            let mut state = make_test_campaign();
            state
                .dynasties
                .get_mut(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .runtime
                .phase = phase;
            let value = serde_json::to_value(state).expect("state must serialize");
            let filename = format!("unearned-{phase:?}-campaign-phase.json");
            let (_directory, path) = write_test_json_fixture(&filename, &value);

            assert_invalid_state(
                load_state(&path),
                StateValidationKind::PrimaryRecords,
                "stale or incompatible campaign phase",
            );
        }
    }

    #[test]
    fn rejects_current_save_with_expired_information() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["clock"]["day"] = Value::from(100);
        let (_directory, path) = write_test_json_fixture("expired-information.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "information report",
        );
    }

    #[test]
    fn rejects_current_save_with_expired_office_directive() {
        let mut state = make_test_campaign();
        let institution = state
            .institutions
            .values_mut()
            .find(|institution| !institution.powers.is_empty())
            .expect("campaign must contain an institution with office powers");
        let power = *institution
            .powers
            .iter()
            .next()
            .expect("institution must expose an office power");
        institution.active_directive = Some(OfficeDirectiveState {
            power,
            expires_day: 0,
        });
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["clock"]["day"] = Value::from(1);
        let (_directory, path) = write_test_json_fixture("expired-office-directive.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "institution",
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
    fn rejects_market_target_stock_that_differs_from_registry() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let quote = value["market"]["quotes"]
            .as_object_mut()
            .and_then(|quotes| quotes.values_mut().next())
            .expect("serialized state must contain a market quote");
        quote["target_stock"] = Value::from(1);
        let (_directory, path) = write_test_json_fixture("mismatched-market-target.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::DefinitionReferences,
            "target stock does not match the scenario registry",
        );
    }

    #[test]
    fn rejects_institution_powers_that_differ_from_registry() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let institution = value["institutions"]
            .as_object_mut()
            .and_then(|institutions| institutions.values_mut().next())
            .expect("serialized state must contain an institution");
        institution["powers"] = Value::Array(Vec::new());
        let (_directory, path) =
            write_test_json_fixture("mismatched-institution-powers.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::DefinitionReferences,
            "powers do not match the scenario registry",
        );
    }

    #[test]
    fn rejects_empty_chronicle_content() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let entry = value["chronicle"]
            .as_array_mut()
            .and_then(|entries| entries.first_mut())
            .expect("serialized state must contain a chronicle entry");
        entry["summary"] = Value::String("   ".to_owned());
        let (_directory, path) = write_test_json_fixture("empty-chronicle-entry.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "chronicle entry lacks user-facing content",
        );
    }

    #[test]
    fn rejects_empty_audit_content() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let record = value["audit_log"]
            .as_array_mut()
            .and_then(|records| records.first_mut())
            .expect("serialized state must contain an audit record");
        record["detail"] = Value::String("\t".to_owned());
        let (_directory, path) = write_test_json_fixture("empty-audit-record.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "audit record lacks diagnostic content",
        );
    }

    #[test]
    fn rejects_blank_runtime_names() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let dynasty = value["dynasties"]
            .as_object_mut()
            .and_then(|dynasties| dynasties.values_mut().next())
            .expect("serialized state must contain a dynasty");
        dynasty["identity"]["name"] = Value::String("  ".to_owned());
        let (_directory, path) = write_test_json_fixture("blank-dynasty-name.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::PrimaryRecords,
            "has a blank name",
        );
    }

    #[test]
    fn rejects_blank_property_name() {
        let mut state = make_test_campaign();
        state
            .properties
            .values_mut()
            .next()
            .expect("campaign must contain a property")
            .name = " \t ".to_owned();
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("blank-property-name.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "has a blank name",
        );
    }

    #[test]
    fn rejects_zero_value_property() {
        let mut state = make_test_campaign();
        state
            .properties
            .values_mut()
            .next()
            .expect("campaign must contain a property")
            .value = Money::ZERO;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("zero-value-property.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid financial value",
        );
    }

    #[test]
    fn rejects_blank_information_report_content() {
        let mut state = make_test_campaign();
        state
            .information_reports
            .values_mut()
            .next()
            .expect("campaign must contain an information report")
            .summary = "   ".to_owned();
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("blank-information-summary.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "information report",
        );
    }

    #[test]
    fn rejects_blank_relationship_memory() {
        let mut state = make_test_campaign();
        state
            .relationships
            .values_mut()
            .next()
            .expect("campaign must contain a relationship")
            .memories
            .push(" \n ".to_owned());
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("blank-relationship-memory.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "relationship map",
        );
    }

    #[test]
    fn rejects_blank_ai_objective_rationale() {
        let mut state = make_test_campaign();
        state
            .ai_objectives
            .values_mut()
            .next()
            .expect("campaign must contain an AI objective")
            .rationale = "\t".to_owned();
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("blank-objective-rationale.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "has no rationale",
        );
    }

    #[test]
    fn rejects_planned_ai_objective_without_runtime_progression() {
        let mut state = make_test_campaign();
        state
            .ai_objectives
            .values_mut()
            .next()
            .expect("campaign must contain an AI objective")
            .status = crate::core::ObjectiveStatus::Planned;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("planned-ai-objective.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "unsupported planned lifecycle state",
        );
    }

    #[test]
    fn rejects_multiple_pursuing_ai_objectives_for_one_dynasty() {
        let mut state = make_test_campaign();
        let mut duplicate = state
            .ai_objectives
            .values()
            .next()
            .expect("campaign must contain an AI objective")
            .clone();
        let duplicate_id = state.next_ids.objective();
        duplicate.id = duplicate_id;
        state.ai_objectives.insert(duplicate_id, duplicate);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("duplicate-pursuing-objective.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "multiple pursuing AI objectives",
        );
    }

    #[test]
    fn rejects_nonplayer_dynasty_without_a_pursuing_ai_objective() {
        let mut state = make_test_campaign();
        state
            .ai_objectives
            .values_mut()
            .next()
            .expect("campaign must contain an AI objective")
            .status = crate::core::ObjectiveStatus::Achieved;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("missing-pursuing-objective.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "does not have exactly one pursuing AI objective",
        );
    }

    #[test]
    fn rejects_blank_external_route_name() {
        let mut state = make_test_campaign();
        state
            .external_routes
            .values_mut()
            .next()
            .expect("campaign must contain an external route")
            .name = " ".to_owned();
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("blank-route-name.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "external route",
        );
    }

    #[test]
    fn rejects_blank_crisis_cause() {
        let mut state = make_test_campaign();
        let crisis_id = state.next_ids.crisis();
        state.crises.insert(
            crisis_id,
            Crisis {
                id: crisis_id,
                kind: CrisisKind::BankingPanic,
                district_id: None,
                started_day: state.clock.day(),
                severity_basis_points: 1_000,
                status: CrisisStatus::Active,
                cause: " \t ".to_owned(),
            },
        );
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("blank-crisis-cause.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "crisis",
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
            "invalid budget, term timing",
        );
    }

    #[test]
    fn rejects_office_directive_for_an_unavailable_power() {
        let mut state = make_test_campaign();
        let institution = state
            .institutions
            .values_mut()
            .find(|institution| !institution.powers.contains(&OfficePower::Taxation))
            .expect("campaign must contain an institution without taxation power");
        institution.active_directive = Some(OfficeDirectiveState {
            power: OfficePower::Taxation,
            expires_day: 180,
        });
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("invalid-office-directive-power.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid budget, term timing, or directive",
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
    fn rejects_malformed_institution_campaign_audit_subjects() {
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InstitutionPatronage,
            subject: format!("invalid:character:{character_id}").into(),
            detail: "malformed institutional history".to_owned(),
        });
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("invalid-institution-audit-subject.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "invalid institution/character subject",
        );
    }

    #[test]
    fn rejects_noncanonical_dynasty_pair_keys() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let relationships = value["relationships"]
            .as_object_mut()
            .expect("serialized relationships must be an object");
        let canonical_key = relationships
            .keys()
            .next()
            .cloned()
            .expect("campaign must contain a relationship");
        let relationship = relationships
            .remove(&canonical_key)
            .expect("selected relationship must exist");
        let (first, second) = canonical_key
            .split_once(':')
            .expect("serialized dynasty pair must contain a separator");
        relationships.insert(format!("{second}:{first}"), relationship);
        let (_directory, path) = write_test_json_fixture("noncanonical-dynasty-pair.json", &value);

        match load_state(&path) {
            Err(PersistenceError::Parse { source, .. }) => assert!(
                source
                    .to_string()
                    .contains("dynasty pair must use ascending first:second order"),
                "unexpected parse error: {source}"
            ),
            Err(error) => panic!("expected parse error, got {error:?}"),
            Ok(_) => panic!("noncanonical relationship key unexpectedly loaded"),
        }
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
    fn rejects_fully_funded_public_work_that_is_not_completed() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let work = value["public_works"]
            .as_object_mut()
            .and_then(|works| works.values_mut().next())
            .expect("serialized state must contain a public work");
        work["spent"] = work["budget"].clone();
        work["progress_basis_points"] = Value::from(10_000);
        work["status"] = Value::from("Building");
        let (_directory, path) = write_test_json_fixture("unfinished-funded-work.json", &value);

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
    fn rejects_exhausted_simulation_day() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["clock"]["day"] = Value::from(i64::MAX);
        let (_directory, path) = write_test_json_fixture("exhausted-day.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid or exhausted elapsed day",
        );
    }

    #[track_caller]
    fn assert_stale_weekly_due_rejected(
        filename: &str,
        mutate: fn(&mut Value),
        expected_reason: &str,
    ) {
        let mut state = make_test_campaign();
        advance_days(rivergate_registry_for_test(), &mut state, 8)
            .expect("stale-due fixture must cross one weekly settlement boundary");
        let mut value = serde_json::to_value(state).expect("state must serialize");
        mutate(&mut value);
        let (_directory, path) = write_test_json_fixture(filename, &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            expected_reason,
        );
    }

    #[test]
    fn rejects_stale_active_weekly_obligation_due_dates() {
        assert_stale_weekly_due_rejected(
            "stale-loan-due-day.json",
            |value| {
                value["loans"]
                    .as_object_mut()
                    .and_then(|loans| loans.values_mut().next())
                    .expect("serialized state must contain a loan")["next_due_day"] =
                    Value::from(7);
            },
            "invalid due date",
        );
        assert_stale_weekly_due_rejected(
            "stale-contract-due-day.json",
            |value| {
                value["contracts"]
                    .as_object_mut()
                    .and_then(|contracts| contracts.values_mut().next())
                    .expect("serialized state must contain a contract")["next_due_day"] =
                    Value::from(7);
            },
            "invalid dates",
        );
    }

    #[test]
    fn accepts_pre_campaign_historical_dates() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["institutions"]
            .as_object_mut()
            .and_then(|institutions| institutions.values_mut().next())
            .expect("serialized state must contain an institution")["term_started_day"] =
            Value::from(-180);
        value["audit_log"]
            .as_array_mut()
            .and_then(|records| records.first_mut())
            .expect("serialized state must contain an audit record")["day"] = Value::from(-180);
        let (_directory, path) = write_test_json_fixture("pre-campaign-history.json", &value);

        load_state(&path).expect("pre-campaign historical dates must remain loadable");
    }

    #[test]
    fn rejects_exhausted_dynasty_generation() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let dynasty = value["dynasties"]
            .as_object_mut()
            .and_then(|dynasties| dynasties.values_mut().next())
            .expect("serialized state must contain a dynasty");
        dynasty["runtime"]["generation"] = Value::from(u16::MAX);
        let (_directory, path) = write_test_json_fixture("exhausted-generation.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid resource value",
        );
    }

    #[test]
    fn rejects_exhausted_business_finance_version() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let business = value["businesses"]["records"]
            .as_object_mut()
            .and_then(|records| records.values_mut().next())
            .expect("serialized state must contain a business");
        business["finance"]["version"] = Value::from(u64::MAX);
        let (_directory, path) = write_test_json_fixture("exhausted-business-version.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid economic value",
        );
    }

    #[test]
    fn rejects_exhausted_institution_term_number() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let institution = value["institutions"]
            .as_object_mut()
            .and_then(|institutions| institutions.values_mut().next())
            .expect("serialized state must contain an institution");
        institution["term_number"] = Value::from(u32::MAX);
        let (_directory, path) = write_test_json_fixture("exhausted-institution-term.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid budget, term timing",
        );
    }

    #[test]
    fn rejects_exhausted_family_charter_version() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let council = value["family_councils"]
            .as_object_mut()
            .and_then(|councils| councils.values_mut().next())
            .expect("serialized state must contain a family council");
        council["charter_version"] = Value::from(u64::MAX);
        let (_directory, path) = write_test_json_fixture("exhausted-charter.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "exhausted charter version",
        );
    }

    #[test]
    fn rejects_active_character_with_zero_health() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let character = value["characters"]["records"]
            .as_object_mut()
            .and_then(|records| records.values_mut().next())
            .expect("serialized state must contain a character");
        character["runtime"]["status"] = Value::String("Active".to_owned());
        character["runtime"]["health_basis_points"] = Value::from(0);
        let (_directory, path) = write_test_json_fixture("active-zero-health.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid birth date, capability, or basis-point value",
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
    fn rejects_loan_status_inconsistent_with_missed_payments() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let loan = value["loans"]
            .as_object_mut()
            .and_then(|loans| loans.values_mut().next())
            .expect("serialized state must contain a loan");
        loan["status"] = Value::String("Current".to_owned());
        loan["missed_payments"] = Value::from(1);
        let (_directory, path) = write_test_json_fixture("current-loan-with-arrears.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "status does not match its missed-payment count",
        );
    }

    #[test]
    fn rejects_loan_due_day_at_the_reserved_timeline_boundary() {
        let mut state = make_test_campaign();
        let loan = state
            .loans
            .values_mut()
            .next()
            .expect("campaign must contain a loan");
        loan.next_due_day = i64::MAX;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("exhausted-loan-due-day.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::NumericRanges,
            "invalid due date",
        );
    }

    #[test]
    fn rejects_information_report_expiry_at_the_reserved_timeline_boundary() {
        let mut state = make_test_campaign();
        let report = state
            .information_reports
            .values_mut()
            .next()
            .expect("campaign must contain an information report");
        report.expires_day = i64::MAX;
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("exhausted-information-expiry.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "information report",
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
    fn rejects_crisis_status_inconsistent_with_severity() {
        let mut state = make_test_campaign();
        let crisis_id = state.next_ids.crisis();
        state.crises.insert(
            crisis_id,
            Crisis {
                id: crisis_id,
                kind: CrisisKind::BankingPanic,
                district_id: None,
                started_day: state.clock.day(),
                severity_basis_points: 9_000,
                status: CrisisStatus::Active,
                cause: "Credit panic remains severe.".to_owned(),
            },
        );
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("active-escalated-crisis.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "crisis",
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
    fn rejects_active_contract_between_businesses_of_the_same_dynasty() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let contract = state
            .contracts
            .values()
            .find(|contract| contract.status == crate::core::ContractStatus::Active)
            .expect("campaign must contain an active contract");
        let buyer_business_id = contract.buyer_business_id;
        let seller_business_id = contract.seller_business_id;
        let buyer = state
            .businesses
            .get(buyer_business_id)
            .expect("contract buyer must exist");
        let buyer_dynasty_id = buyer.owner_dynasty_id();
        let buyer_manager_id = buyer.manager_id();
        let seller_dynasty_id = state
            .businesses
            .get(seller_business_id)
            .expect("contract seller must exist")
            .owner_dynasty_id();
        let administrative_load = registry
            .get_recipe(
                state
                    .businesses
                    .get(seller_business_id)
                    .expect("contract seller must exist")
                    .recipe_id(),
            )
            .expect("seller recipe must exist")
            .administrative_load();
        state
            .businesses
            .transfer_ownership(seller_business_id, buyer_dynasty_id, buyer_manager_id)
            .expect("seller business must transfer");
        state
            .dynasties
            .get_mut(&seller_dynasty_id)
            .expect("seller dynasty must exist")
            .resources
            .administrative_load -= administrative_load;
        state
            .dynasties
            .get_mut(&buyer_dynasty_id)
            .expect("buyer dynasty must exist")
            .resources
            .administrative_load += administrative_load;
        for property in state
            .properties
            .values_mut()
            .filter(|property| property.occupant_business_id == Some(seller_business_id))
        {
            property.tenant_dynasty_id = property
                .owner_dynasty_id
                .filter(|owner_id| *owner_id != buyer_dynasty_id)
                .map(|_| buyer_dynasty_id);
        }
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("internal-active-contract.json", &value);

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
    fn rejects_multiple_civic_debts_backed_by_one_consumed_authorization() {
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
        for _ in 0..2 {
            let debt_id = state.next_ids.civic_debt();
            state.civic_debts.insert(
                debt_id,
                CivicDebt {
                    id: debt_id,
                    creditor_dynasty_id,
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
        }
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("reused-civic-debt-authorization.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "reuses consumed public-debt authorization",
        );
    }

    #[test]
    fn rejects_multiple_repayment_active_loans_for_one_pair() {
        let mut state = make_test_campaign();
        let mut duplicate = state
            .loans
            .values()
            .find(|loan| {
                matches!(
                    loan.status,
                    crate::core::LoanStatus::Current
                        | crate::core::LoanStatus::Delinquent
                        | crate::core::LoanStatus::Restructured
                )
            })
            .expect("campaign must contain a repayment-active loan")
            .clone();
        let duplicate_id = state.next_ids.loan();
        duplicate.id = duplicate_id;
        duplicate.collateral_property_id = None;
        state.loans.insert(duplicate_id, duplicate);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("duplicate-active-loan.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "duplicates an existing repayment-active lender/borrower pair",
        );
    }

    #[test]
    fn rejects_player_character_above_institution_membership_capacity() {
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let institution_ids: Vec<_> = state
            .institutions
            .keys()
            .copied()
            .take(crate::systems::MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER + 1)
            .collect();
        assert_eq!(
            institution_ids.len(),
            crate::systems::MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER + 1,
            "campaign must contain enough institutions to exceed membership capacity"
        );
        for institution_id in institution_ids {
            state
                .institutions
                .get_mut(&institution_id)
                .expect("selected institution must exist")
                .members
                .insert(character_id);
            state.audit_log.push(AuditRecord {
                day: state.clock.day(),
                kind: AuditKind::InstitutionPatronage,
                subject: format!("institution:{institution_id}:character:{character_id}").into(),
                detail: "test patronage record".to_owned(),
            });
        }
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("excess-institution-memberships.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "exceeding the supported maximum",
        );
    }

    #[test]
    fn rejects_relationship_history_above_retention_bound() {
        let mut state = make_test_campaign();
        let relationship = state
            .relationships
            .values_mut()
            .next()
            .expect("campaign must contain a relationship");
        relationship.memories = (0..=crate::systems::MAX_RELATIONSHIP_MEMORIES)
            .map(|index| format!("memory {index}"))
            .collect();
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("excess-relationship-memory.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "relationship map contains an invalid dynasty pair",
        );
    }

    #[test]
    fn rejects_missing_relationship_pair() {
        let mut state = make_test_campaign();
        let pair = *state
            .relationships
            .keys()
            .next()
            .expect("campaign must contain a relationship");
        state.relationships.remove(&pair);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("missing-relationship-pair.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "relationship map is missing dynasty pair",
        );
    }

    #[test]
    fn rejects_character_with_multiple_active_marriages() {
        let mut state = make_test_campaign();
        let character_ids: Vec<_> = state
            .characters
            .iter()
            .map(crate::core::Character::id)
            .take(3)
            .collect();
        assert_eq!(
            character_ids.len(),
            3,
            "campaign must contain three characters for the marriage fixture"
        );
        for spouse_id in character_ids.iter().copied().skip(1) {
            let link_id = state.next_ids.family_link();
            state.family_links.insert(
                link_id,
                crate::core::FamilyLink {
                    id: link_id,
                    first_character_id: character_ids[0],
                    second_character_id: spouse_id,
                    kind: FamilyLinkKind::Marriage,
                    active: true,
                    property_claim_basis_points: 5_000,
                },
            );
        }
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("duplicate-active-marriage.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "multiple active marriages",
        );
    }

    #[test]
    fn rejects_active_marriage_with_inactive_participant() {
        let mut state = make_test_campaign();
        let active_character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let foreign_template = state
            .characters
            .iter()
            .find(|character| character.dynasty_id() != state.player_dynasty_id)
            .expect("campaign must contain a foreign character")
            .clone();
        let inactive_character_id = state.next_ids.character();
        let mut inactive_character = foreign_template;
        inactive_character.identity.id = inactive_character_id;
        inactive_character.identity.name = "Inactive Spouse".to_owned();
        inactive_character.runtime.status = crate::core::CharacterStatus::Incapacitated;
        inactive_character.runtime.health_basis_points = 0;
        inactive_character.runtime.role = crate::core::CharacterRole::Clerk;
        state.characters.insert(inactive_character);
        let link_id = state.next_ids.family_link();
        state.family_links.insert(
            link_id,
            crate::core::FamilyLink {
                id: link_id,
                first_character_id: active_character_id,
                second_character_id: inactive_character_id,
                kind: FamilyLinkKind::Marriage,
                active: true,
                property_claim_basis_points: 5_000,
            },
        );
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("inactive-active-marriage.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "invalid active marriage lifecycle",
        );
    }

    #[test]
    fn rejects_player_above_active_ward_capacity() {
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let template_id = state
            .dynasties
            .get(&dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let template = state
            .characters
            .get(template_id)
            .expect("player heir must exist")
            .clone();
        for index in 0..=crate::systems::MAX_ACTIVE_WARDS {
            let ward_id = state.next_ids.character();
            let mut ward = template.clone();
            ward.identity.id = ward_id;
            ward.identity.name = format!("Capacity Ward {index}");
            ward.runtime.role = crate::core::CharacterRole::Clerk;
            state.characters.insert(ward);
            state
                .family_councils
                .get_mut(&dynasty_id)
                .expect("player family council must exist")
                .members
                .insert(ward_id);
            let link_id = state.next_ids.family_link();
            state.family_links.insert(
                link_id,
                crate::core::FamilyLink {
                    id: link_id,
                    first_character_id: head_id,
                    second_character_id: ward_id,
                    kind: FamilyLinkKind::Ward,
                    active: true,
                    property_claim_basis_points: 1_500,
                },
            );
        }
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("excess-active-wards.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "exceeding the supported maximum",
        );
    }

    #[test]
    fn rejects_duplicate_unfinished_public_work_for_district_and_kind() {
        let mut state = make_test_campaign();
        let mut duplicate = state
            .public_works
            .values()
            .next()
            .expect("campaign must contain a public work")
            .clone();
        let duplicate_id = state.next_ids.public_work();
        duplicate.id = duplicate_id;
        state.public_works.insert(duplicate_id, duplicate);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("duplicate-unfinished-public-work.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "duplicates an unfinished project",
        );
    }

    #[test]
    fn rejects_player_above_unfinished_public_work_sponsorship_capacity() {
        let mut state = make_test_campaign();
        let districts: Vec<_> = state.districts.keys().copied().take(3).collect();
        assert_eq!(districts.len(), 3, "campaign must contain three districts");
        let kinds = [
            crate::core::PublicWorkKind::Road,
            crate::core::PublicWorkKind::Bridge,
            crate::core::PublicWorkKind::Market,
        ];
        for (district_id, kind) in districts.into_iter().zip(kinds) {
            let id = state.next_ids.public_work();
            state.public_works.insert(
                id,
                crate::core::PublicWork {
                    id,
                    district_id,
                    kind,
                    sponsor_dynasty_id: Some(state.player_dynasty_id),
                    budget: Money::from_copper(10_000),
                    spent: Money::ZERO,
                    progress_basis_points: 0,
                    status: crate::core::PublicWorkStatus::Building,
                },
            );
        }
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("excess-player-public-works.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "exceeding the supported maximum",
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
    fn rejects_outbox_ids_that_do_not_follow_notification_order() {
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let messages = value["outbox"]
            .as_array_mut()
            .expect("serialized state must contain an outbox array");
        assert!(
            messages.len() >= 2,
            "campaign fixture must contain at least two notifications"
        );
        let first_id = messages[0]["id"].clone();
        let second_id = messages[1]["id"].clone();
        messages[0]["id"] = second_id;
        messages[1]["id"] = first_id;
        let (_directory, path) = write_test_json_fixture("out-of-order-outbox-ids.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "message IDs are not strictly increasing",
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
        let mut dynasty_ids = state.dynasties.keys().copied();
        let plaintiff_dynasty_id = dynasty_ids.next().expect("campaign must contain a dynasty");
        let defendant_dynasty_id = dynasty_ids
            .next()
            .expect("campaign must contain a second dynasty");
        let first_id = state.next_ids.legal_case();
        let first = LegalCase {
            id: first_id,
            plaintiff_dynasty_id,
            defendant_dynasty_id,
            kind: LegalCaseKind::Fraud,
            claim_source: None,
            evidence_basis_points: 6_500,
            public_attention_basis_points: 2_000,
            filed_day: state.clock.day(),
            hearing_day: state.clock.day().saturating_add(60),
            damages: Money::from_copper(2_500),
            status: LegalCaseStatus::Filed,
        };
        state.legal_cases.insert(first_id, first.clone());
        let mut duplicate = first;
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
    fn rejects_relitigation_of_the_same_grounded_claim_source() {
        let mut state = make_test_campaign();
        let loan = state
            .loans
            .values()
            .next()
            .expect("campaign must contain a loan")
            .clone();
        let first_id = state.next_ids.legal_case();
        let first = LegalCase {
            id: first_id,
            plaintiff_dynasty_id: loan.lender_dynasty_id,
            defendant_dynasty_id: loan.borrower_dynasty_id,
            kind: LegalCaseKind::Debt,
            claim_source: Some(LegalClaimSource::Loan { loan_id: loan.id }),
            evidence_basis_points: 7_500,
            public_attention_basis_points: 1_500,
            filed_day: state.clock.day(),
            hearing_day: state.clock.day(),
            damages: loan.balance,
            status: LegalCaseStatus::DecidedForDefendant,
        };
        state.legal_cases.insert(first_id, first.clone());
        let duplicate_id = state.next_ids.legal_case();
        let mut duplicate = first;
        duplicate.id = duplicate_id;
        state.legal_cases.insert(duplicate_id, duplicate);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("duplicate-grounded-claim-source.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "reuses a grounded claim source",
        );
    }

    #[test]
    fn rejects_relitigation_of_the_same_grounded_contract_claim() {
        let mut state = make_test_campaign();
        let contract_id = *state
            .contracts
            .keys()
            .next()
            .expect("campaign must contain a supply contract");
        let (plaintiff_dynasty_id, defendant_dynasty_id) = {
            let contract = state
                .contracts
                .get(&contract_id)
                .expect("selected contract must exist");
            let plaintiff_dynasty_id = state
                .businesses
                .get(contract.seller_business_id)
                .expect("contract seller must exist")
                .owner_dynasty_id();
            let defendant_dynasty_id = state
                .businesses
                .get(contract.buyer_business_id)
                .expect("contract buyer must exist")
                .owner_dynasty_id();
            (plaintiff_dynasty_id, defendant_dynasty_id)
        };
        let unpaid_penalty = Money::from_copper(100);
        {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("selected contract must exist");
            contract.status = crate::core::ContractStatus::Breached;
            contract.breaching_dynasty_id = Some(defendant_dynasty_id);
            contract.breach_victim_dynasty_id = Some(plaintiff_dynasty_id);
            contract.unpaid_breach_penalty = unpaid_penalty;
        }
        let first_id = state.next_ids.legal_case();
        let first = LegalCase {
            id: first_id,
            plaintiff_dynasty_id,
            defendant_dynasty_id,
            kind: LegalCaseKind::ContractBreach,
            claim_source: Some(LegalClaimSource::Contract { contract_id }),
            evidence_basis_points: 8_500,
            public_attention_basis_points: 1_500,
            filed_day: state.clock.day(),
            hearing_day: state.clock.day(),
            damages: unpaid_penalty,
            status: LegalCaseStatus::DecidedForDefendant,
        };
        state.legal_cases.insert(first_id, first.clone());
        let duplicate_id = state.next_ids.legal_case();
        let mut duplicate = first;
        duplicate.id = duplicate_id;
        state.legal_cases.insert(duplicate_id, duplicate);
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("duplicate-grounded-contract-claim.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "reuses a grounded claim source",
        );
    }

    #[test]
    fn rejects_legal_case_with_missing_claim_source_record() {
        let mut state = make_test_campaign();
        let loan = state
            .loans
            .values()
            .next()
            .expect("campaign must contain a loan")
            .clone();
        let case_id = state.next_ids.legal_case();
        state.legal_cases.insert(
            case_id,
            LegalCase {
                id: case_id,
                plaintiff_dynasty_id: loan.lender_dynasty_id,
                defendant_dynasty_id: loan.borrower_dynasty_id,
                kind: LegalCaseKind::Debt,
                claim_source: Some(LegalClaimSource::Loan { loan_id: loan.id }),
                evidence_basis_points: 7_500,
                public_attention_basis_points: 1_500,
                filed_day: state.clock.day(),
                hearing_day: state.clock.day().saturating_add(60),
                damages: loan.balance,
                status: LegalCaseStatus::Filed,
            },
        );
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["legal_cases"][case_id.value().to_string()]["claim_source"] = serde_json::json!({
            "Loan": { "loan_id": u32::MAX }
        });
        let (_directory, path) = write_test_json_fixture("missing-legal-claim-source.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "references missing loan",
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
    fn rejects_office_directive_audit_with_invalid_institution_subject() {
        let mut state = make_test_campaign();
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: "invalid-office-directive".into(),
            detail: "fabricated directive history".to_owned(),
        });
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("invalid-office-directive-audit.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "OfficeDirective audit record has an invalid institution subject",
        );
    }

    #[test]
    fn rejects_office_directive_audit_with_missing_actor_dynasty() {
        let mut state = make_test_campaign();
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: format!("institution:{institution_id};dynasty:{}", u32::MAX).into(),
            detail: "fabricated directive actor".to_owned(),
        });
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) =
            write_test_json_fixture("invalid-office-directive-actor.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "OfficeDirective audit record references missing dynasty",
        );
    }

    #[test]
    fn rejects_office_duty_audit_with_missing_dynasty_reference() {
        let mut state = make_test_campaign();
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDutyShortfall,
            subject: format!("institution:{institution_id};dynasty:{}", u32::MAX).into(),
            detail: "fabricated office duty".to_owned(),
        });
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("invalid-office-duty-actor.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "OfficeDutyShortfall audit record references a missing institution or dynasty",
        );
    }

    #[test]
    fn rejects_endowment_audit_with_invalid_institution_subject() {
        let mut state = make_test_campaign();
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InstitutionEndowment,
            subject: "invalid-institution-endowment".into(),
            detail: "fabricated endowment history".to_owned(),
        });
        let value = serde_json::to_value(state).expect("state must serialize");
        let (_directory, path) = write_test_json_fixture("invalid-endowment-audit.json", &value);

        assert_invalid_state(
            load_state(&path),
            StateValidationKind::StrategicRecords,
            "InstitutionEndowment audit record has an invalid institution subject",
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
