//! Behavioral tests for canonical player-command validation and mutation.

use super::*;
use crate::ids::GoodId;
use crate::money::Quantity;
use crate::systems::validate_invariants;
use crate::test_support::{
    assert_state_unchanged, make_test_campaign, rivergate_registry_for_test,
};

mod validation {
    use super::*;

    #[test]
    fn rejects_invalid_public_work_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::StartPublicWork {
                district_id: DistrictId::new(u32::MAX),
                kind: PublicWorkKind::Bridge,
                budget: Money::from_copper(10_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::MissingDistrict {
                district_id: DistrictId::new(u32::MAX),
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a rejected command must not partially mutate campaign state",
        );
    }
    #[test]
    fn rejects_unchanged_business_policy_without_version_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let business = state
            .businesses
            .get(business_id)
            .expect("owned business must exist");
        let command = PlayerCommand::SetBusinessPolicy {
            business_id,
            target_input_days: business.policy.target_input_days,
            target_output_days: business.policy.target_output_days,
            minimum_cash_reserve: business.policy.minimum_cash_reserve,
            maintenance_basis_points: business.policy.maintenance_basis_points,
            quality_target_basis_points: business.policy.quality_target_basis_points,
        };
        let before = state.clone();

        let result = apply_player_command(registry, &mut state, command);

        assert_eq!(
            result,
            Err(CommandError::UnchangedBusinessPolicy { business_id })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a no-op policy command must not increment the business version",
        );
    }
}

mod business_acquisition {
    use super::*;

    fn acquisition_fixture() -> (
        &'static Registry,
        AppState,
        BusinessId,
        CharacterId,
        crate::systems::BusinessAcquisitionQuote,
    ) {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() != state.player_dynasty_id)
            .expect("campaign must contain a non-player business")
            .id();
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("selected business must exist");
        business.operations.status = crate::core::BusinessStatus::Distressed;
        business.finance.cash = Money::ZERO;
        let manager_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an eligible heir");
        let quote = crate::systems::quote_business_acquisition(
            registry,
            &state,
            state.player_dynasty_id,
            business_id,
        )
        .expect("distressed business must be acquirable");
        let required = quote
            .purchase_price
            .saturating_add(quote.minimum_recapitalization)
            .saturating_add(Money::from_copper(1_000));
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = required;
        (registry, state, business_id, manager_id, quote)
    }

    #[test]
    fn acquires_and_recapitalizes_distressed_business() {
        let (registry, mut state, business_id, manager_id, quote) = acquisition_fixture();
        let buyer_id = state.player_dynasty_id;
        let buyer_before = state
            .dynasties
            .get(&buyer_id)
            .expect("buyer must exist")
            .clone();
        let seller_before = state
            .dynasties
            .get(&quote.seller_dynasty_id)
            .expect("seller must exist")
            .clone();

        let outcome = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AcquireBusiness {
                business_id,
                manager_id,
                recapitalization: quote.minimum_recapitalization,
            },
        )
        .expect("funded acquisition must succeed");
        validate_invariants(registry, &state);

        let business = state
            .businesses
            .get(business_id)
            .expect("acquired business must remain present");
        assert_eq!(business.owner_dynasty_id(), buyer_id);
        assert_eq!(business.manager_id(), manager_id);
        assert_eq!(business.status(), crate::core::BusinessStatus::Active);
        assert_eq!(business.cash(), quote.minimum_recapitalization);
        assert!(
            state
                .businesses
                .ids_for_owner(buyer_id)
                .is_some_and(|ids| ids.contains(&business_id)),
            "owner index must move the acquired business to the buyer"
        );
        assert_eq!(
            state
                .dynasties
                .get(&buyer_id)
                .expect("buyer must exist")
                .treasury(),
            buyer_before
                .treasury()
                .saturating_sub(quote.purchase_price)
                .saturating_sub(quote.minimum_recapitalization)
        );
        assert_eq!(
            state
                .dynasties
                .get(&quote.seller_dynasty_id)
                .expect("seller must exist")
                .treasury(),
            seller_before
                .treasury()
                .saturating_add(quote.purchase_price)
        );
        assert!(outcome.summary.contains("Acquired business"));
        assert_eq!(
            state.audit_log.last().map(crate::core::AuditRecord::kind),
            Some(crate::core::AuditKind::BusinessAcquisition)
        );
    }

    #[test]
    fn rejects_underfunded_recapitalization_without_mutation() {
        let (registry, mut state, business_id, manager_id, quote) = acquisition_fixture();
        let provided =
            Money::from_copper(quote.minimum_recapitalization.copper().saturating_sub(1));
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AcquireBusiness {
                business_id,
                manager_id,
                recapitalization: provided,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Strategic(
                StrategicError::InsufficientBusinessRecapitalization {
                    business_id,
                    provided,
                    required: quote.minimum_recapitalization,
                }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "failed acquisition validation must not move funds, ownership, or indexes",
        );
    }

    #[test]
    fn invests_dynasty_treasury_into_owned_business() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let amount = Money::from_copper(1_000);
        let treasury_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();
        let cash_before = state
            .businesses
            .get(business_id)
            .expect("owned business must exist")
            .cash();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::InvestInBusiness {
                business_id,
                amount,
            },
        )
        .expect("funded investment must succeed");
        validate_invariants(registry, &state);

        assert_eq!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury(),
            treasury_before.saturating_sub(amount)
        );
        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist")
                .cash(),
            cash_before.saturating_add(amount)
        );
        assert_eq!(
            state.audit_log.last().map(crate::core::AuditRecord::kind),
            Some(crate::core::AuditKind::BusinessCapitalization)
        );
    }

    #[test]
    fn rejects_nonpositive_business_investment_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::InvestInBusiness {
                business_id,
                amount: Money::ZERO,
            },
        );

        assert_eq!(result, Err(CommandError::InvalidBusinessInvestment));
        assert_state_unchanged(
            &before,
            &state,
            "nonpositive investment must fail before moving treasury or business cash",
        );
    }
}

mod laws {
    use super::*;

    #[test]
    fn enact_through_the_canonical_command_path() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let treasury_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::BreadPriceCeiling,
                value: 30,
            },
        )
        .expect("law command must succeed");
        validate_invariants(registry, &state);

        let active: Vec<_> = state
            .laws
            .values()
            .filter(|law| law.active && law.kind == LawKind::BreadPriceCeiling)
            .collect();
        let [law] = active.as_slice() else {
            panic!(
                "expected exactly one active bread price ceiling, found {}",
                active.len()
            );
        };
        assert_eq!(law.value, 30, "the enacted value must be preserved");
        assert_eq!(
            law.sponsor_dynasty_id,
            Some(state.player_dynasty_id),
            "player-sponsored laws must record their sponsor"
        );
        assert_eq!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury(),
            treasury_before.saturating_sub(Money::from_copper(2_000)),
            "law sponsorship must charge the documented treasury cost"
        );
    }

    #[test]
    fn reject_unsupported_kind_without_spending_or_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::PublicDebtAuthorization,
                value: 10_000,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::UnsupportedLaw {
                kind: LawKind::PublicDebtAuthorization,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "unsupported laws must fail before charging or mutating state",
        );
    }
}

mod politics {
    use super::*;

    #[test]
    fn repeated_office_nomination_is_rejected_without_reward() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let dynasty = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        let character_id = dynasty.heir_id().expect("player dynasty must have an heir");
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id,
            },
        )
        .expect("first nomination must add the heir as a member");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::AlreadyInstitutionMember {
                institution_id,
                character_id,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "repeated nominations must not grant legitimacy or mutate membership",
        );
    }

    #[test]
    fn unchanged_house_governance_is_rejected_without_charter_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let governance = state
            .family_councils
            .get(&state.player_dynasty_id)
            .expect("player family council must exist")
            .governance;
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance { governance },
        );

        assert_eq!(
            result,
            Err(CommandError::UnchangedHouseGovernance { governance })
        );
        assert_state_unchanged(
            &before,
            &state,
            "reasserting the current governance must not amend the charter or reduce unity",
        );
    }
}

mod crises {
    use super::*;

    #[test]
    fn exploitation_requires_the_full_legitimacy_cost() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 599;
        let crisis_id = state.next_ids.crisis();
        state.crises.insert(
            crisis_id,
            crate::core::Crisis {
                id: crisis_id,
                kind: crate::core::CrisisKind::NobleDemand,
                district_id: None,
                started_day: state.clock.day(),
                severity_basis_points: 4_000,
                status: CrisisStatus::Active,
                cause: "test crisis".to_owned(),
            },
        );
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::RespondToCrisis {
                crisis_id,
                response: CrisisResponse::Exploit,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::InsufficientPlayerLegitimacy {
                available: 599,
                required: 600,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "failed exploitation must not mint treasury or intensify the crisis",
        );
    }
}

mod serialization {
    use super::*;
    use std::collections::BTreeSet;

    const COMMAND_KINDS: [&str; 15] = [
        "acquire-business",
        "acknowledge-notification",
        "buy-property",
        "create-supply-contract",
        "enact-law",
        "file-legal-case",
        "issue-loan",
        "invest-in-business",
        "nominate-for-office",
        "resolve-labor-dispute",
        "respond-to-crisis",
        "set-business-policy",
        "set-house-governance",
        "start-public-work",
        "transfer-business-cash",
    ];

    fn command_kind(command: &PlayerCommand) -> &'static str {
        match command {
            PlayerCommand::TransferBusinessCash { .. } => "transfer-business-cash",
            PlayerCommand::AcquireBusiness { .. } => "acquire-business",
            PlayerCommand::InvestInBusiness { .. } => "invest-in-business",
            PlayerCommand::SetBusinessPolicy { .. } => "set-business-policy",
            PlayerCommand::CreateSupplyContract { .. } => "create-supply-contract",
            PlayerCommand::IssueLoan { .. } => "issue-loan",
            PlayerCommand::BuyProperty { .. } => "buy-property",
            PlayerCommand::EnactLaw { .. } => "enact-law",
            PlayerCommand::StartPublicWork { .. } => "start-public-work",
            PlayerCommand::FileLegalCase { .. } => "file-legal-case",
            PlayerCommand::SetHouseGovernance { .. } => "set-house-governance",
            PlayerCommand::NominateForOffice { .. } => "nominate-for-office",
            PlayerCommand::RespondToCrisis { .. } => "respond-to-crisis",
            PlayerCommand::ResolveLaborDispute { .. } => "resolve-labor-dispute",
            PlayerCommand::AcknowledgeNotification { .. } => "acknowledge-notification",
        }
    }

    #[test]
    fn every_variant_round_trips_through_json() {
        let commands = vec![
            PlayerCommand::TransferBusinessCash {
                from_business_id: BusinessId::new(1),
                to_business_id: BusinessId::new(2),
                amount: Money::from_copper(300),
            },
            PlayerCommand::AcquireBusiness {
                business_id: BusinessId::new(2),
                manager_id: CharacterId::new(3),
                recapitalization: Money::from_copper(900),
            },
            PlayerCommand::InvestInBusiness {
                business_id: BusinessId::new(1),
                amount: Money::from_copper(500),
            },
            PlayerCommand::SetBusinessPolicy {
                business_id: BusinessId::new(1),
                target_input_days: 4,
                target_output_days: 3,
                minimum_cash_reserve: Money::from_copper(500),
                maintenance_basis_points: 700,
                quality_target_basis_points: 8_000,
            },
            PlayerCommand::CreateSupplyContract {
                terms: SupplyContractTerms {
                    buyer_business_id: BusinessId::new(1),
                    seller_business_id: BusinessId::new(2),
                    good_id: GoodId::new(3),
                    quantity_per_week: Quantity::from_units(4),
                    unit_price: Money::from_copper(25),
                    penalty: Money::from_copper(100),
                    duration_weeks: 8,
                },
            },
            PlayerCommand::IssueLoan {
                terms: LoanTerms {
                    lender_dynasty_id: DynastyId::new(1),
                    borrower_dynasty_id: DynastyId::new(2),
                    principal: Money::from_copper(1_000),
                    weekly_payment: Money::from_copper(50),
                    interest_basis_points: 500,
                    collateral_property_id: Some(PropertyId::new(3)),
                },
            },
            PlayerCommand::BuyProperty {
                property_id: PropertyId::new(1),
            },
            PlayerCommand::EnactLaw {
                kind: LawKind::BreadPriceCeiling,
                value: 30,
            },
            PlayerCommand::StartPublicWork {
                district_id: DistrictId::new(1),
                kind: PublicWorkKind::Bridge,
                budget: Money::from_copper(20_000),
            },
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id: DynastyId::new(2),
                kind: LegalCaseKind::ContractBreach,
                evidence_basis_points: 7_500,
                damages: Money::from_copper(2_000),
            },
            PlayerCommand::SetHouseGovernance {
                governance: HouseGovernance::BranchFederation,
            },
            PlayerCommand::NominateForOffice {
                institution_id: InstitutionId::new(1),
                character_id: CharacterId::new(2),
            },
            PlayerCommand::RespondToCrisis {
                crisis_id: CrisisId::new(1),
                response: CrisisResponse::Reform,
            },
            PlayerCommand::ResolveLaborDispute {
                employment_id: EmploymentId::new(1),
                response: LaborResponse::Negotiate,
            },
            PlayerCommand::AcknowledgeNotification {
                message_id: OutboxMessageId::new(1),
            },
        ];

        assert_eq!(
            commands.iter().map(command_kind).collect::<BTreeSet<_>>(),
            COMMAND_KINDS.into_iter().collect(),
            "the serialization fixture must cover every command variant exactly once"
        );

        for command in commands {
            let json = serde_json::to_string(&command).expect("command must serialize");
            let decoded: PlayerCommand =
                serde_json::from_str(&json).expect("command must deserialize");
            assert_eq!(decoded, command, "JSON round-trip failed for {json}");
        }
    }
}
