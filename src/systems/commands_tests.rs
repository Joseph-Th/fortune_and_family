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

mod serialization {
    use super::*;
    use std::collections::BTreeSet;

    const COMMAND_KINDS: [&str; 13] = [
        "acknowledge-notification",
        "buy-property",
        "create-supply-contract",
        "enact-law",
        "file-legal-case",
        "issue-loan",
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
