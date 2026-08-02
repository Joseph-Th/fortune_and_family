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

    #[test]
    fn rejects_repeated_business_policy_changes_during_the_strategy_interval() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let current_input_days = state
            .businesses
            .get(business_id)
            .expect("owned business must exist")
            .policy
            .target_input_days;
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetBusinessPolicy {
                business_id,
                target_input_days: current_input_days.saturating_add(1).min(30),
                target_output_days: 4,
                minimum_cash_reserve: Money::from_copper(700),
                maintenance_basis_points: 6_000,
                quality_target_basis_points: 7_500,
            },
        )
        .expect("the first material policy change must be accepted");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetBusinessPolicy {
                business_id,
                target_input_days: current_input_days.saturating_add(2).min(30),
                target_output_days: 6,
                minimum_cash_reserve: Money::from_copper(900),
                maintenance_basis_points: 7_000,
                quality_target_basis_points: 8_000,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::BusinessPolicyCooldown {
                business_id,
                next_change_day: BUSINESS_POLICY_CHANGE_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "operating strategy must remain stable long enough to generate consequences",
        );
    }

    #[test]
    fn rejects_policy_changes_for_inactive_businesses() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("owned business must exist");
        business.operations.status = crate::core::BusinessStatus::Insolvent;
        let command = PlayerCommand::SetBusinessPolicy {
            business_id,
            target_input_days: business.policy.target_input_days.saturating_add(1),
            target_output_days: business.policy.target_output_days,
            minimum_cash_reserve: business.policy.minimum_cash_reserve,
            maintenance_basis_points: business.policy.maintenance_basis_points,
            quality_target_basis_points: business.policy.quality_target_basis_points,
        };
        let before = state.clone();

        let result = apply_player_command(registry, &mut state, command);

        assert_eq!(
            result,
            Err(CommandError::Strategic(StrategicError::BusinessInactive {
                business_id,
            }))
        );
        assert_state_unchanged(
            &before,
            &state,
            "inactive businesses must not accept operating-policy mutations",
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

    #[test]
    fn rejects_reenacting_identical_active_law_without_spending() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (kind, value) = state
            .laws
            .values()
            .find(|law| law.active && law.kind.is_implemented())
            .map(|law| (law.kind, law.value))
            .expect("campaign must contain an active implemented law");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw { kind, value },
        );

        assert_eq!(result, Err(CommandError::UnchangedLaw { kind, value }));
        assert_state_unchanged(
            &before,
            &state,
            "reenacting an identical active law must not consume treasury or create history",
        );
    }
}

mod politics {
    use super::*;
    use crate::systems::advance_days;

    #[test]
    fn nomination_creates_a_funded_campaign_and_can_win_the_scheduled_selection() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let nominee_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let institution_id = state
            .institutions
            .iter()
            .find(|(_, institution)| !institution.members.contains(&nominee_id))
            .map(|(institution_id, _)| *institution_id)
            .expect("campaign must contain an institution open to the nominee");
        let member_ids: Vec<_> = state
            .institutions
            .get(&institution_id)
            .expect("institution must exist")
            .members
            .iter()
            .copied()
            .collect();
        for member_id in member_ids {
            state
                .characters
                .get_mut(member_id)
                .expect("institution member must exist")
                .capabilities
                .social = 0;
        }
        state
            .characters
            .get_mut(nominee_id)
            .expect("nominee must exist")
            .capabilities
            .social = 100;
        for dynasty in state.dynasties.values_mut() {
            dynasty.resources.legitimacy_basis_points = 0;
        }
        let treasury_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id: nominee_id,
            },
        )
        .expect("nomination must succeed");

        assert_eq!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury(),
            treasury_before.saturating_sub(Money::from_copper(300))
        );
        assert!(
            state
                .institutions
                .get(&institution_id)
                .expect("institution must exist")
                .next_selection_day
                <= 60,
            "nomination must schedule a timely contest"
        );

        advance_days(registry, &mut state, 60).expect("campaign must reach the selection");

        assert_eq!(
            state
                .institutions
                .get(&institution_id)
                .expect("institution must exist")
                .office_holder_id,
            Some(nominee_id),
            "a strong funded nomination must be capable of winning office"
        );
    }

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

    #[test]
    fn governance_cannot_be_rewritten_twice_in_one_year() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let current = state
            .family_councils
            .get(&state.player_dynasty_id)
            .expect("player family council must exist")
            .governance;
        let alternatives: Vec<_> = [
            HouseGovernance::Primogeniture,
            HouseGovernance::FamilyPartnership,
            HouseGovernance::BranchFederation,
        ]
        .into_iter()
        .filter(|governance| *governance != current)
        .collect();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance {
                governance: alternatives[0],
            },
        )
        .expect("first charter amendment must succeed");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance {
                governance: alternatives[1],
            },
        );

        assert_eq!(
            result,
            Err(CommandError::HouseGovernanceCooldown {
                next_change_day: 360,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a premature charter amendment must not mutate family governance",
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

    #[test]
    fn repeated_crisis_response_is_throttled_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let crisis_id = state.next_ids.crisis();
        state.crises.insert(
            crisis_id,
            crate::core::Crisis {
                id: crisis_id,
                kind: crate::core::CrisisKind::NobleDemand,
                district_id: None,
                started_day: state.clock.day(),
                severity_basis_points: 8_000,
                status: CrisisStatus::Active,
                cause: "test crisis".to_owned(),
            },
        );

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::RespondToCrisis {
                crisis_id,
                response: CrisisResponse::Reform,
            },
        )
        .expect("first response must succeed");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::RespondToCrisis {
                crisis_id,
                response: CrisisResponse::Suppress,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::CrisisResponseCooldown {
                crisis_id,
                next_response_day: 30,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "cooldown rejection must not spend funds or change crisis severity",
        );
    }
}

mod notifications {
    use super::*;

    #[test]
    fn acknowledgement_clears_the_notification_backlog_through_selected_message() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let message_id = state
            .outbox
            .last()
            .expect("campaign must contain notifications")
            .id;

        let outcome = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AcknowledgeNotification { message_id },
        )
        .expect("acknowledgement must succeed");

        assert!(
            state
                .outbox
                .iter()
                .filter(|message| message.id <= message_id)
                .all(|message| message.acknowledged),
            "acknowledging the latest visible message must clear the older backlog"
        );
        assert!(outcome.summary.contains("notifications"));
    }
}

mod legal_cases {
    use super::*;

    #[test]
    fn rejects_duplicate_unresolved_case_without_charging_filing_cost() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let defendant_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain a nonplayer dynasty");
        let kind = LegalCaseKind::ContractBreach;
        let id = state.next_ids.legal_case();
        state.legal_cases.insert(
            id,
            LegalCase {
                id,
                plaintiff_dynasty_id: state.player_dynasty_id,
                defendant_dynasty_id,
                kind,
                evidence_basis_points: 6_000,
                public_attention_basis_points: 1_500,
                filed_day: state.clock.day(),
                hearing_day: state.clock.day().saturating_add(60),
                damages: Money::from_copper(2_000),
                status: LegalCaseStatus::Filed,
            },
        );
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id,
                kind,
                evidence_basis_points: 7_000,
                damages: Money::from_copper(4_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::DuplicateActiveLegalCase {
                defendant_dynasty_id,
                kind,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "duplicate unresolved cases must fail before charging the filing cost",
        );
    }
}

mod labor {
    use super::*;

    #[test]
    fn rejects_labor_response_for_inactive_business() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let employment_id = state
            .employment
            .values()
            .find(|agreement| {
                state
                    .businesses
                    .get(agreement.business_id)
                    .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
            })
            .expect("player business must have employment")
            .id;
        let business_id = {
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("employment must exist");
            agreement.status = EmploymentStatus::Disputed;
            agreement.business_id
        };
        state
            .businesses
            .get_mut(business_id)
            .expect("employment business must exist")
            .operations
            .status = BusinessStatus::Insolvent;
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ResolveLaborDispute {
                employment_id,
                response: LaborResponse::Negotiate,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Strategic(StrategicError::BusinessInactive {
                business_id,
            }))
        );
        assert_state_unchanged(
            &before,
            &state,
            "inactive businesses must not spend cash or reactivate employment through labor commands",
        );
    }

    #[test]
    fn replacement_requires_a_household_with_available_workers() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let employment_id = state
            .employment
            .values()
            .find(|agreement| {
                state
                    .businesses
                    .get(agreement.business_id)
                    .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
            })
            .expect("player business must have an employment agreement")
            .id;
        let (business_id, current_household_id, workers) = {
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("selected employment must exist");
            agreement.status = EmploymentStatus::Disputed;
            (
                agreement.business_id,
                agreement.household_id,
                agreement.workers,
            )
        };
        assert!(workers > 1, "fixture must require more than one worker");
        for (id, agreement) in &mut state.employment {
            if *id != employment_id {
                agreement.status = EmploymentStatus::Ended;
            }
        }
        let district_id = state
            .businesses
            .get(business_id)
            .expect("employment business must exist")
            .district_id();
        let household_ids: Vec<_> = state
            .households
            .ids_for_district(district_id)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        for household_id in household_ids {
            if household_id != current_household_id {
                state
                    .households
                    .get_mut(household_id)
                    .expect("indexed household must exist")
                    .members = 1;
            }
        }
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ResolveLaborDispute {
                employment_id,
                response: LaborResponse::ReplaceWorkers,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::NoReplacementLaborAvailable {
                employment_id,
                district_id,
                workers,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "failed worker replacement must not reassign labor or increase unrest",
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
