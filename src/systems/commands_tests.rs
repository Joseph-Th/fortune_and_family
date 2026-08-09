//! Behavioral tests for canonical player-command validation and mutation.

use super::*;
use crate::ids::GoodId;
use crate::money::Quantity;
use crate::systems::OFFICE_TERM_DAYS;
use crate::systems::validate_invariants;
use crate::test_support::{
    assert_state_unchanged, make_test_campaign, rivergate_registry_for_test,
};
use std::collections::BTreeSet;

fn grant_player_office_for_test(state: &mut AppState) {
    let mature_term_started_day = state
        .clock
        .day()
        .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS);
    let mature_next_selection_day = state
        .clock
        .day()
        .saturating_add(OFFICE_TERM_DAYS)
        .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS);
    let dynasty = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let holders = [
        dynasty.head_id(),
        dynasty.heir_id().expect("player dynasty must have an heir"),
    ];
    for (power, holder_id) in [
        (OfficePower::PublicWorks, holders[0]),
        (OfficePower::MarketTolls, holders[1]),
    ] {
        let institution = state
            .institutions
            .values_mut()
            .find(|institution| institution.powers.contains(&power))
            .expect("campaign must contain an office with the requested power");
        institution.members.insert(holder_id);
        institution.office_holder_id = Some(holder_id);
        institution.term_started_day = mature_term_started_day;
        institution.next_selection_day = mature_next_selection_day;
    }
}

fn grant_player_office_with_power_for_test(state: &mut AppState, power: OfficePower) {
    let mature_term_started_day = state
        .clock
        .day()
        .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS);
    let mature_next_selection_day = state
        .clock
        .day()
        .saturating_add(OFFICE_TERM_DAYS)
        .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS);
    let holder_id = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .head_id();
    let institution = state
        .institutions
        .values_mut()
        .find(|institution| institution.powers.contains(&power))
        .expect("campaign must contain an office with the requested power");
    institution.members.insert(holder_id);
    institution.office_holder_id = Some(holder_id);
    institution.term_started_day = mature_term_started_day;
    institution.next_selection_day = mature_next_selection_day;
}

fn grant_commercial_deliveries_for_test(state: &mut AppState, required_deliveries: u32) {
    let player_business_ids: BTreeSet<_> = state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() == state.player_dynasty_id)
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
    let deliveries = u16::try_from(required_deliveries)
        .expect("office delivery requirement must fit contract counters");
    contract.fulfilled_deliveries = deliveries;
    contract
        .fulfilled_deliveries_by_dynasty
        .insert(state.player_dynasty_id, deliveries);
}

fn grant_commercial_standing_for_test(state: &mut AppState) {
    grant_commercial_deliveries_for_test(state, OFFICE_NOMINATION_DELIVERY_REQUIREMENT);
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .reputation_reliability_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
}

fn grant_office_nomination_record_for_test(state: &mut AppState) {
    grant_commercial_standing_for_test(state);
    grant_commercial_deliveries_for_test(
        state,
        OFFICE_NOMINATION_DELIVERY_REQUIREMENT
            .saturating_add(OFFICE_NOMINATION_MAX_PREPARATION_DELIVERIES),
    );
    let support_day = state
        .clock
        .day()
        .saturating_sub(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS);
    let player_character_ids: Vec<_> = state
        .characters
        .iter()
        .filter(|character| character.dynasty_id() == state.player_dynasty_id)
        .map(Character::id)
        .collect();
    let institution_ids: Vec<_> = state.institutions.keys().copied().collect();
    for institution_id in institution_ids {
        for character_id in &player_character_ids {
            state
                .institutions
                .get_mut(&institution_id)
                .expect("institution must exist")
                .members
                .insert(*character_id);
            state.audit_log.push(AuditRecord {
                day: support_day,
                kind: AuditKind::InstitutionPatronage,
                subject: institution_support_subject(institution_id, *character_id).into(),
                detail: "test support".to_owned(),
            });
        }
    }
    state.audit_log.sort_by_key(AuditRecord::day);
}

mod validation {
    use super::*;
    use crate::systems::strategic::PROPERTY_LIQUIDATION_BASIS_POINTS;

    fn non_player_credit_counterparty(state: &AppState, player_is_lender: bool) -> DynastyId {
        state
            .dynasties
            .values()
            .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
            .filter(|dynasty| {
                let (lender_id, borrower_id) = if player_is_lender {
                    (state.player_dynasty_id, dynasty.id())
                } else {
                    (dynasty.id(), state.player_dynasty_id)
                };
                !state.loans.values().any(|loan| {
                    loan.lender_dynasty_id == lender_id
                        && loan.borrower_dynasty_id == borrower_id
                        && matches!(
                            loan.status,
                            LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
                        )
                })
            })
            .max_by_key(|dynasty| dynasty.treasury())
            .map(crate::core::Dynasty::id)
            .expect("campaign must contain an unused non-player credit counterparty")
    }

    fn player_buyer_contract_terms(state: &AppState) -> SupplyContractTerms {
        let contract = state
            .contracts
            .values()
            .find(|contract| {
                let buyer_owner = state
                    .businesses
                    .get(contract.buyer_business_id)
                    .map(crate::core::Business::owner_dynasty_id);
                let seller_owner = state
                    .businesses
                    .get(contract.seller_business_id)
                    .map(crate::core::Business::owner_dynasty_id);
                contract.status == ContractStatus::Active
                    && buyer_owner == Some(state.player_dynasty_id)
                    && seller_owner.is_some_and(|owner| owner != state.player_dynasty_id)
            })
            .expect("campaign must contain a player-buyer contract");
        SupplyContractTerms {
            buyer_business_id: contract.buyer_business_id,
            seller_business_id: contract.seller_business_id,
            good_id: contract.good_id,
            quantity_per_week: contract.quantity_per_week,
            unit_price: contract.unit_price,
            penalty: contract.penalty,
            duration_weeks: 8,
        }
    }

    #[test]
    fn rejects_registry_mismatch_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state.scenario_key = "another-scenario".to_owned();
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
            Err(CommandError::Simulation(
                crate::systems::SimulationError::RegistryMismatch {
                    state_scenario: "another-scenario".to_owned(),
                    registry_scenario: "rivergate".to_owned(),
                }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "registry mismatch must be rejected before command-specific validation or mutation",
        );
    }

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
    fn rejects_duplicate_unfinished_public_work_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let existing = state
            .public_works
            .values()
            .find(|work| {
                matches!(
                    work.status,
                    PublicWorkStatus::Building | PublicWorkStatus::Suspended
                )
            })
            .expect("campaign must contain an unfinished public work");
        let command = PlayerCommand::StartPublicWork {
            district_id: existing.district_id,
            kind: existing.kind,
            budget: Money::from_copper(10_000),
        };
        let expected = Err(CommandError::DuplicateActivePublicWork {
            district_id: existing.district_id,
            kind: existing.kind,
        });
        let before = state.clone();

        let result = apply_player_command(registry, &mut state, command);

        assert_eq!(result, expected);
        assert_state_unchanged(
            &before,
            &state,
            "duplicate public works must be rejected before charging the sponsor",
        );
    }

    #[test]
    fn public_work_sponsorship_requires_an_office_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let district_id = registry
            .districts()
            .first()
            .expect("registry must contain a district")
            .id();
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::StartPublicWork {
                district_id,
                kind: PublicWorkKind::Bridge,
                budget: Money::from_copper(10_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::PublicWorkSponsorshipRequiresOffice)
        );
        assert_state_unchanged(
            &before,
            &state,
            "a dynasty without office must not fund or create a public work",
        );
    }

    #[test]
    fn public_work_sponsorship_requires_public_works_power_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::WatchPriorities);
        let district_id = registry
            .districts()
            .first()
            .expect("registry must contain a district")
            .id();
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::StartPublicWork {
                district_id,
                kind: PublicWorkKind::Bridge,
                budget: Money::from_copper(10_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::PublicWorkSponsorshipRequiresPower)
        );
        assert_state_unchanged(
            &before,
            &state,
            "an unrelated office must not authorize public-work sponsorship",
        );
    }

    #[test]
    fn public_work_sponsorship_waits_for_office_power_to_be_established() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::PublicWorks);
        let current_day = state.clock.day();
        let available_day = current_day.saturating_add(OFFICE_POWER_ESTABLISHMENT_DAYS);
        state
            .institutions
            .values_mut()
            .find(|institution| institution.powers.contains(&OfficePower::PublicWorks))
            .expect("campaign must contain a public-works office")
            .term_started_day = current_day;
        let district_id = registry
            .districts()
            .first()
            .expect("registry must contain a district")
            .id();
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::StartPublicWork {
                district_id,
                kind: PublicWorkKind::Bridge,
                budget: Money::from_copper(10_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::PublicWorkPowerNotEstablished { available_day })
        );
        assert_state_unchanged(
            &before,
            &state,
            "new office power must not fund a public work before its establishment period",
        );
    }

    #[test]
    fn enforces_public_work_sponsorship_interval_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let mut districts = registry
            .districts()
            .iter()
            .map(crate::registry::DistrictDef::id);
        let first_district = districts.next().expect("registry must contain a district");
        let second_district = districts
            .nth(1)
            .expect("registry must contain several districts");
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::StartPublicWork {
                district_id: first_district,
                kind: PublicWorkKind::Bridge,
                budget: Money::from_copper(10_000),
            },
        )
        .expect("first public work must succeed");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::StartPublicWork {
                district_id: second_district,
                kind: PublicWorkKind::Market,
                budget: Money::from_copper(10_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::PublicWorkCooldown {
                next_start_day: PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "public-work cooldown failures must not charge funds or create records",
        );
    }

    #[test]
    fn rejects_public_work_above_sponsored_capacity_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let district_ids: Vec<_> = registry
            .districts()
            .iter()
            .map(crate::registry::DistrictDef::id)
            .collect();
        for (index, district_id) in district_ids
            .iter()
            .copied()
            .take(MAX_ACTIVE_SPONSORED_PUBLIC_WORKS)
            .enumerate()
        {
            let id = state.next_ids.public_work();
            state.public_works.insert(
                id,
                PublicWork {
                    id,
                    district_id,
                    kind: if index == 0 {
                        PublicWorkKind::Bridge
                    } else {
                        PublicWorkKind::Market
                    },
                    sponsor_dynasty_id: Some(state.player_dynasty_id),
                    budget: Money::from_copper(10_000),
                    spent: Money::from_copper(1_000),
                    progress_basis_points: 1_000,
                    status: PublicWorkStatus::Building,
                },
            );
        }
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::StartPublicWork {
                district_id: *district_ids
                    .last()
                    .expect("registry must contain districts"),
                kind: PublicWorkKind::School,
                budget: Money::from_copper(10_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::PublicWorkCapacity {
                active: MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
                maximum: MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "public-work capacity failures must not charge funds or create records",
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

    #[test]
    fn business_policy_changes_create_durable_player_feedback() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let current = state
            .businesses
            .get(business_id)
            .expect("owned business must exist")
            .policy
            .clone();
        let outbox_before = state.outbox.len();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetBusinessPolicy {
                business_id,
                target_input_days: current.target_input_days.saturating_add(1).min(30),
                target_output_days: current.target_output_days,
                minimum_cash_reserve: current.minimum_cash_reserve,
                maintenance_basis_points: current.maintenance_basis_points,
                quality_target_basis_points: current.quality_target_basis_points,
            },
        )
        .expect("material policy change must succeed");

        assert_eq!(state.outbox.len(), outbox_before + 1);
        let message = state.outbox.last().expect("policy change must be reported");
        assert_eq!(message.kind(), OutboxKind::Finance);
        assert!(message.subject().contains("operating policy updated"));
    }

    #[test]
    fn portfolio_cash_transfers_create_durable_player_feedback() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let source_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a source business");
        let target_id = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() != state.player_dynasty_id)
            .expect("campaign must contain a non-player business")
            .id();
        let target = state
            .businesses
            .get_mut(target_id)
            .expect("target business must exist");
        target.operations.status = BusinessStatus::Distressed;
        target.finance.cash = Money::ZERO;
        let manager_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let quote = crate::systems::quote_business_acquisition(
            registry,
            &state,
            state.player_dynasty_id,
            target_id,
        )
        .expect("distressed target must be acquirable");
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AcquireBusiness {
                business_id: target_id,
                manager_id,
                recapitalization: quote.minimum_recapitalization,
            },
        )
        .expect("portfolio fixture acquisition must succeed");
        let amount = Money::from_copper(100);
        state
            .businesses
            .get_mut(source_id)
            .expect("source business must exist")
            .finance
            .cash = amount;
        let outbox_before = state.outbox.len();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::TransferBusinessCash {
                from_business_id: source_id,
                to_business_id: target_id,
                amount,
            },
        )
        .expect("funded portfolio transfer must succeed");

        assert_eq!(state.outbox.len(), outbox_before + 1);
        let message = state.outbox.last().expect("cash transfer must be reported");
        assert_eq!(message.kind(), OutboxKind::Finance);
        assert!(message.subject().contains("Portfolio cash moved"));
    }

    #[test]
    fn non_player_lender_rejects_coerced_zero_interest_credit_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let lender_dynasty_id = non_player_credit_counterparty(&state, false);
        let terms = LoanTerms {
            lender_dynasty_id,
            borrower_dynasty_id: state.player_dynasty_id,
            principal: Money::from_copper(1_000),
            weekly_payment: Money::from_copper(50),
            interest_basis_points: 0,
            collateral_property_id: None,
        };
        let before = state.clone();

        let result = apply_player_command(registry, &mut state, PlayerCommand::IssueLoan { terms });

        assert_eq!(
            result,
            Err(CommandError::LoanCounterpartyInterestTooLow {
                interest_basis_points: 0,
                minimum_basis_points: PRIVATE_LOAN_COUNTERPARTY_MIN_INTEREST_BASIS_POINTS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "the player must not force a non-player lender into concessionary terms",
        );
    }

    #[test]
    fn non_player_borrower_rejects_predatory_interest_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let borrower_dynasty_id = non_player_credit_counterparty(&state, true);
        let terms = LoanTerms {
            lender_dynasty_id: state.player_dynasty_id,
            borrower_dynasty_id,
            principal: Money::from_copper(1_000),
            weekly_payment: Money::from_copper(50),
            interest_basis_points: 10_000,
            collateral_property_id: None,
        };
        let before = state.clone();

        let result = apply_player_command(registry, &mut state, PlayerCommand::IssueLoan { terms });

        assert_eq!(
            result,
            Err(CommandError::LoanCounterpartyInterestTooHigh {
                interest_basis_points: 10_000,
                maximum_basis_points: PRIVATE_LOAN_COUNTERPARTY_MAX_INTEREST_BASIS_POINTS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "the player must not force a non-player borrower into predatory interest",
        );
    }

    #[test]
    fn non_player_borrower_rejects_unsolicited_credit_without_financing_pressure() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let borrower_dynasty_id = non_player_credit_counterparty(&state, true);
        state
            .dynasties
            .get_mut(&borrower_dynasty_id)
            .expect("selected borrower must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        let terms = LoanTerms {
            lender_dynasty_id: state.player_dynasty_id,
            borrower_dynasty_id,
            principal: Money::from_copper(1_000),
            weekly_payment: Money::from_copper(50),
            interest_basis_points: 900,
            collateral_property_id: None,
        };
        let before = state.clone();

        let result = apply_player_command(registry, &mut state, PlayerCommand::IssueLoan { terms });

        assert_eq!(
            result,
            Err(CommandError::LoanCounterpartyNoFinancingNeed {
                borrower_dynasty_id,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a solvent non-player house must not accept debt solely because the player offers profitable terms",
        );
    }

    #[test]
    fn non_player_borrower_deploys_new_credit_into_distressed_business() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let borrower_dynasty_id = non_player_credit_counterparty(&state, true);
        let business_id = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() == borrower_dynasty_id)
            .expect("selected borrower must own a business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("selected business must exist");
            business.operations.status = BusinessStatus::Distressed;
            business.finance.cash = Money::ZERO;
        }
        state
            .dynasties
            .get_mut(&borrower_dynasty_id)
            .expect("selected borrower must exist")
            .resources
            .treasury = Money::from_copper(20_000);
        let target_cash = business_recapitalization_target(
            registry,
            &state,
            state
                .businesses
                .get(business_id)
                .expect("selected business must exist"),
        );
        let principal = Money::from_copper(5_000).min(target_cash);
        assert!(principal > Money::ZERO);
        let borrower_treasury_before = state
            .dynasties
            .get(&borrower_dynasty_id)
            .expect("selected borrower must exist")
            .treasury();
        let player_id = state.player_dynasty_id;
        let expected_deployment = target_cash.min(
            borrower_treasury_before
                .checked_add(principal)
                .expect("fixture borrower treasury must fit after borrowing")
                .saturating_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE),
        );

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::IssueLoan {
                terms: LoanTerms {
                    lender_dynasty_id: player_id,
                    borrower_dynasty_id,
                    principal,
                    weekly_payment: ceil_positive_money_div(principal, 26),
                    interest_basis_points: 900,
                    collateral_property_id: None,
                },
            },
        )
        .expect("credit for an identified business shortfall must be accepted");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("selected business must exist")
                .cash(),
            expected_deployment,
            "new credit should unlock a borrower co-investment while preserving its protected household reserve"
        );
        assert_eq!(
            state
                .dynasties
                .get(&borrower_dynasty_id)
                .expect("selected borrower must exist")
                .treasury(),
            borrower_treasury_before
                .checked_add(principal)
                .and_then(|treasury| treasury.checked_sub(expected_deployment))
                .expect("fixture borrower treasury must fit after co-investment"),
            "the borrower should commit its own discretionary capital alongside the loan instead of keeping a fully cash-backed obligation"
        );
    }

    #[test]
    fn non_player_lender_keeps_a_household_credit_reserve() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let lender_dynasty_id = non_player_credit_counterparty(&state, false);
        let available = state
            .dynasties
            .get(&lender_dynasty_id)
            .expect("selected lender must exist")
            .treasury();
        let principal = available
            .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
            .and_then(|amount| amount.checked_add(Money::from_copper(1)))
            .expect("fixture lender must have more than the reserved amount");
        let terms = LoanTerms {
            lender_dynasty_id,
            borrower_dynasty_id: state.player_dynasty_id,
            principal,
            weekly_payment: ceil_positive_money_div(
                principal,
                PRIVATE_LOAN_COUNTERPARTY_MAX_AMORTIZATION_WEEKS,
            ),
            interest_basis_points: 700,
            collateral_property_id: None,
        };
        let before = state.clone();

        let result = apply_player_command(registry, &mut state, PlayerCommand::IssueLoan { terms });

        assert_eq!(
            result,
            Err(CommandError::LoanCounterpartyLenderReserve {
                lender_dynasty_id,
                available,
                principal,
                required_reserve: PRIVATE_LOAN_COUNTERPARTY_RESERVE,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "the player must not drain a non-player lender below its negotiated reserve",
        );
    }

    #[test]
    fn non_player_contract_seller_rejects_coerced_below_market_price() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let mut terms = player_buyer_contract_terms(&state);
        let market_price = state
            .market
            .get_quote(terms.good_id)
            .expect("contract good must have a market quote")
            .price();
        let minimum_price = contract_counterparty_price_bounds(
            &state,
            terms.buyer_business_id,
            terms.seller_business_id,
            market_price,
        )
        .minimum_seller_price;
        terms.unit_price = Money::from_copper(1).min(
            minimum_price
                .checked_sub(Money::from_copper(1))
                .expect("market floor must exceed zero"),
        );
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CreateSupplyContract {
                terms: terms.clone(),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::ContractCounterpartyPriceTooLow {
                unit_price: terms.unit_price,
                minimum_price,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "the player must not force an NPC seller into a confiscatory supply price",
        );
    }

    #[test]
    fn hostile_counterparty_requires_worse_contract_terms_until_relations_improve() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let mut terms = player_buyer_contract_terms(&state);
        let seller_dynasty_id = state
            .businesses
            .get(terms.seller_business_id)
            .expect("seller must exist")
            .owner_dynasty_id();
        let pair = DynastyPair::new(state.player_dynasty_id, seller_dynasty_id);
        {
            let relationship = state
                .relationships
                .get_mut(&pair)
                .expect("counterparty relationship must exist");
            relationship.trust_basis_points = 1_000;
            relationship.resentment_basis_points = 7_000;
        }
        let market_price = state
            .market
            .get_quote(terms.good_id)
            .expect("contract good must have a market quote")
            .price();
        terms.unit_price = market_price;
        let hostile_bounds = contract_counterparty_price_bounds(
            &state,
            terms.buyer_business_id,
            terms.seller_business_id,
            market_price,
        );

        assert_eq!(hostile_bounds.relationship_pressure_basis_points, 2_500);
        assert!(hostile_bounds.minimum_seller_price > market_price);
        assert_eq!(
            ensure_non_player_contract_counterparty_accepts(registry, &state, &terms),
            Err(CommandError::ContractCounterpartyPriceTooLow {
                unit_price: market_price,
                minimum_price: hostile_bounds.minimum_seller_price,
            })
        );

        {
            let relationship = state
                .relationships
                .get_mut(&pair)
                .expect("counterparty relationship must exist");
            relationship.trust_basis_points = 7_000;
            relationship.resentment_basis_points = 1_000;
        }
        let friendly_bounds = contract_counterparty_price_bounds(
            &state,
            terms.buyer_business_id,
            terms.seller_business_id,
            market_price,
        );
        assert_eq!(friendly_bounds.relationship_pressure_basis_points, 0);
        ensure_non_player_contract_counterparty_accepts(registry, &state, &terms)
            .expect("ordinary market terms must become acceptable after relations recover");
    }

    #[test]
    fn contract_price_bounds_use_the_non_player_buyer_when_player_is_seller() {
        let mut state = make_test_campaign();
        let terms = player_buyer_contract_terms(&state);
        let player_business_id = terms.buyer_business_id;
        let non_player_business_id = terms.seller_business_id;
        let non_player_dynasty_id = state
            .businesses
            .get(non_player_business_id)
            .expect("non-player business must exist")
            .owner_dynasty_id();
        let pair = DynastyPair::new(state.player_dynasty_id, non_player_dynasty_id);
        {
            let relationship = state
                .relationships
                .get_mut(&pair)
                .expect("counterparty relationship must exist");
            relationship.trust_basis_points = 1_000;
            relationship.resentment_basis_points = 7_000;
        }
        let market_price = state
            .market
            .get_quote(terms.good_id)
            .expect("contract good must have a market quote")
            .price();

        let bounds = contract_counterparty_price_bounds(
            &state,
            non_player_business_id,
            player_business_id,
            market_price,
        );

        assert_eq!(bounds.relationship_pressure_basis_points, 2_500);
        assert!(
            bounds.maximum_buyer_price < market_price,
            "a hostile non-player buyer must demand a discount from the player seller"
        );
    }

    #[test]
    fn non_player_contract_counterparty_requires_meaningful_breach_penalty() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let mut terms = player_buyer_contract_terms(&state);
        terms.unit_price = state
            .market
            .get_quote(terms.good_id)
            .expect("contract good must have a market quote")
            .price();
        terms.penalty = Money::ZERO;
        let weekly_payment =
            crate::money::checked_cost_for(terms.quantity_per_week, terms.unit_price)
                .expect("test contract payment must fit");
        let minimum_penalty = ceil_positive_money_div(weekly_payment, 4);
        let maximum_penalty = weekly_payment.saturating_mul(4);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CreateSupplyContract { terms },
        );

        assert_eq!(
            result,
            Err(CommandError::ContractCounterpartyPenaltyOutOfRange {
                penalty: Money::ZERO,
                minimum_penalty,
                maximum_penalty,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "NPC supply counterparties must not accept a contract with no meaningful breach protection",
        );
    }

    #[test]
    fn voluntary_property_sale_cannot_drain_the_named_npc_buyer() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(state.player_dynasty_id)
                    && property.collateral_loan_id.is_none()
            })
            .expect("player dynasty must own an unpledged property")
            .id;
        let buyer_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain a non-player property buyer");
        let price = state
            .properties
            .get(&property_id)
            .expect("selected property must exist")
            .value
            .saturating_mul_ratio(PROPERTY_LIQUIDATION_BASIS_POINTS, 10_000)
            .max(Money::from_copper(1));
        state
            .dynasties
            .get_mut(&buyer_dynasty_id)
            .expect("selected buyer must exist")
            .resources
            .treasury = price;
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SellProperty {
                property_id,
                buyer_dynasty_id,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::PropertyCounterpartyBuyerReserve {
                buyer_dynasty_id,
                available: price,
                buyer_contribution: price,
                required_reserve: PROPERTY_COUNTERPARTY_BUYER_RESERVE,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a voluntary property liquidation must not consume the named buyer's entire treasury",
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

    fn assert_acquired_business_state(
        state: &AppState,
        business_id: BusinessId,
        manager_id: CharacterId,
        premises_id: PropertyId,
        quote: crate::systems::BusinessAcquisitionQuote,
        buyer_id: DynastyId,
        condition_before: u16,
    ) {
        let business = state
            .businesses
            .get(business_id)
            .expect("acquired business must remain present");
        assert_eq!(business.owner_dynasty_id(), buyer_id);
        assert_eq!(business.manager_id(), manager_id);
        assert_eq!(business.status(), crate::core::BusinessStatus::Active);
        assert_eq!(business.cash(), quote.minimum_recapitalization);
        let rehabilitation =
            u16::try_from((quote.minimum_recapitalization.copper() / 2).clamp(0, 3_000))
                .expect("bounded rehabilitation must fit u16");
        assert_eq!(
            business.operations.condition_basis_points,
            condition_before.saturating_add(rehabilitation).min(10_000),
            "acquisition recapitalization must rehabilitate physical condition as well as cash"
        );
        let premises = state
            .properties
            .get(&premises_id)
            .expect("business premises must remain present");
        assert_eq!(premises.owner_dynasty_id, Some(quote.seller_dynasty_id));
        assert_eq!(premises.tenant_dynasty_id, Some(buyer_id));
        assert!(
            state
                .employment
                .values()
                .filter(|agreement| agreement.business_id == business_id)
                .all(|agreement| agreement.status == crate::core::EmploymentStatus::Disputed)
        );
        assert!(
            state
                .businesses
                .ids_for_owner(buyer_id)
                .is_some_and(|ids| ids.contains(&business_id))
        );
    }

    fn assert_acquisition_finances(
        state: &AppState,
        buyer_id: DynastyId,
        buyer_treasury_before: Money,
        seller_treasury_before: Money,
        quote: crate::systems::BusinessAcquisitionQuote,
    ) {
        assert_eq!(
            state
                .dynasties
                .get(&buyer_id)
                .expect("buyer must exist")
                .treasury(),
            buyer_treasury_before
                .saturating_sub(quote.purchase_price)
                .saturating_sub(quote.minimum_recapitalization)
        );
        assert_eq!(
            state
                .dynasties
                .get(&quote.seller_dynasty_id)
                .expect("seller must exist")
                .treasury(),
            seller_treasury_before.saturating_add(quote.purchase_price)
        );
    }

    #[test]
    fn acquires_and_recapitalizes_distressed_business() {
        let (registry, mut state, business_id, manager_id, quote) = acquisition_fixture();
        let premises_id = state
            .properties
            .values()
            .find(|property| property.occupant_business_id == Some(business_id))
            .expect("business must occupy premises")
            .id;
        for agreement in state
            .employment
            .values_mut()
            .filter(|agreement| agreement.business_id == business_id)
        {
            agreement.status = crate::core::EmploymentStatus::Suspended;
        }
        let buyer_id = state.player_dynasty_id;
        let buyer_treasury_before = state
            .dynasties
            .get(&buyer_id)
            .expect("buyer must exist")
            .treasury();
        let seller_treasury_before = state
            .dynasties
            .get(&quote.seller_dynasty_id)
            .expect("seller must exist")
            .treasury();
        let condition_before = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .operations
            .condition_basis_points;

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

        assert_acquired_business_state(
            &state,
            business_id,
            manager_id,
            premises_id,
            quote,
            buyer_id,
            condition_before,
        );
        assert_acquisition_finances(
            &state,
            buyer_id,
            buyer_treasury_before,
            seller_treasury_before,
            quote,
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
    fn rejects_acquisition_cost_overflow_without_mutation() {
        let (registry, mut state, business_id, manager_id, quote) = acquisition_fixture();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(i64::MAX);
        let recapitalization = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AcquireBusiness {
                business_id,
                manager_id,
                recapitalization,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Strategic(
                StrategicError::AcquisitionCostOverflow {
                    purchase_price: quote.purchase_price,
                    recapitalization,
                }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing total acquisition cost must not move funds or ownership",
        );
    }

    #[test]
    fn rejects_acquisition_when_seller_treasury_would_overflow() {
        let (registry, mut state, business_id, manager_id, quote) = acquisition_fixture();
        state
            .dynasties
            .get_mut(&quote.seller_dynasty_id)
            .expect("seller dynasty must exist")
            .resources
            .treasury = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AcquireBusiness {
                business_id,
                manager_id,
                recapitalization: quote.minimum_recapitalization,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Strategic(
                StrategicError::DynastyTreasuryOverflow {
                    dynasty_id: quote.seller_dynasty_id,
                    current: Money::from_copper(i64::MAX),
                    incoming: quote.purchase_price,
                }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing acquisition proceeds must not move funds or ownership",
        );
    }

    #[test]
    fn rejects_acquisition_when_recapitalization_would_overflow_business_cash() {
        let (registry, mut state, business_id, manager_id, _) = acquisition_fixture();
        state
            .businesses
            .get_mut(business_id)
            .expect("selected business must exist")
            .finance
            .cash = Money::from_copper(i64::MAX);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(i64::MAX);
        let quote = crate::systems::quote_business_acquisition(
            registry,
            &state,
            state.player_dynasty_id,
            business_id,
        )
        .expect("distressed business must remain acquirable");
        let recapitalization = Money::from_copper(1);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AcquireBusiness {
                business_id,
                manager_id,
                recapitalization,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Strategic(
                StrategicError::BusinessCashOverflow {
                    business_id,
                    current: Money::from_copper(i64::MAX),
                    incoming: recapitalization,
                }
            ))
        );
        assert!(quote.purchase_price < Money::from_copper(i64::MAX));
        assert_state_unchanged(
            &before,
            &state,
            "overflowing recapitalization must not move funds or ownership",
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
        let condition_before = state
            .businesses
            .get(business_id)
            .expect("owned business must exist")
            .operations
            .condition_basis_points;
        let quality_before = state
            .businesses
            .get(business_id)
            .expect("owned business must exist")
            .operations
            .quality_basis_points;

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
        let business = state
            .businesses
            .get(business_id)
            .expect("owned business must exist");
        assert_eq!(
            business.operations.condition_basis_points,
            condition_before.saturating_add(500).min(10_000),
            "capitalization must include bounded physical rehabilitation"
        );
        assert_eq!(
            business.operations.quality_basis_points,
            quality_before.saturating_add(250).min(10_000),
            "capitalization must restore some operating quality"
        );
        assert_eq!(
            state.audit_log.last().map(crate::core::AuditRecord::kind),
            Some(crate::core::AuditKind::BusinessCapitalization)
        );
    }

    #[test]
    fn rejects_business_investment_cash_overflow_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let amount = Money::from_copper(1);
        state
            .businesses
            .get_mut(business_id)
            .expect("owned business must exist")
            .finance
            .cash = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::InvestInBusiness {
                business_id,
                amount,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Simulation(
                crate::systems::SimulationError::BusinessCashOverflow {
                    business_id,
                    current: Money::from_copper(i64::MAX),
                    incoming: amount,
                }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing capitalization must not debit the dynasty or mutate the business",
        );
    }

    #[test]
    fn rejects_business_investment_when_finance_version_is_exhausted() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        state
            .businesses
            .get_mut(business_id)
            .expect("owned business must exist")
            .finance
            .version = u64::MAX;
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::InvestInBusiness {
                business_id,
                amount: Money::from_copper(1),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Simulation(
                crate::systems::SimulationError::BusinessFinanceVersionExhausted { business_id }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "exhausted finance versions must fail before debiting dynasty treasury",
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
        grant_player_office_for_test(&mut state);
        let treasury_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();
        let legitimacy_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points;

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
        assert_eq!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .resources
                .legitimacy_basis_points,
            legitimacy_before.saturating_sub(LAW_LEGITIMACY_COST),
            "law sponsorship must consume political legitimacy as well as money"
        );
    }

    #[test]
    fn public_debt_authorization_funds_the_civic_treasury_and_creates_an_obligation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::Taxation);
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define the civic treasury");
        let civic_budget_before = state
            .institutions
            .get(&treasury_id)
            .expect("treasury runtime must exist")
            .budget;
        let player_treasury_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();
        let creditor_treasuries_before = state
            .dynasties
            .iter()
            .filter(|(dynasty_id, _)| **dynasty_id != state.player_dynasty_id)
            .map(|(dynasty_id, dynasty)| (*dynasty_id, dynasty.treasury()))
            .collect::<std::collections::BTreeMap<_, _>>();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::PublicDebtAuthorization,
                value: 10_000,
            },
        )
        .expect("public debt authorization must issue a civic obligation");
        validate_invariants(registry, &state);

        let debt = state
            .civic_debts
            .values()
            .next()
            .expect("debt authorization must create a civic debt record");
        let law = state
            .laws
            .get(&debt.authorizing_law_id)
            .expect("civic debt must reference its authorizing law");
        assert_eq!(law.kind, LawKind::PublicDebtAuthorization);
        assert!(
            !law.active,
            "a one-time borrowing authorization must be consumed when the debt is issued"
        );
        assert_eq!(debt.principal, Money::from_copper(10_000));
        assert_eq!(debt.balance, debt.principal);
        assert_eq!(debt.sponsor_dynasty_id, Some(state.player_dynasty_id));
        assert_eq!(
            state
                .institutions
                .get(&treasury_id)
                .expect("treasury runtime must exist")
                .budget,
            civic_budget_before.saturating_add(debt.principal),
            "debt proceeds must enter the civic treasury"
        );
        assert_eq!(
            state
                .dynasties
                .get(&debt.creditor_dynasty_id)
                .expect("creditor dynasty must exist")
                .treasury(),
            creditor_treasuries_before[&debt.creditor_dynasty_id].saturating_sub(debt.principal),
            "the creditor must fund the principal"
        );
        assert_eq!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury(),
            player_treasury_before.saturating_sub(Money::from_copper(2_000)),
            "the sponsor must still pay the law-enactment cost"
        );
        assert!(state.information_reports.values().any(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == "Municipal debt underwriting and treasury records"
        }));
    }

    #[test]
    fn consumed_public_debt_authorization_can_be_reissued_at_the_same_amount() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::Taxation);
        let command = PlayerCommand::EnactLaw {
            kind: LawKind::PublicDebtAuthorization,
            value: 10_000,
        };

        apply_player_command(registry, &mut state, command.clone())
            .expect("first public debt authorization must succeed");
        for _ in 0..LAW_SPONSORSHIP_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        apply_player_command(registry, &mut state, command)
            .expect("a consumed authorization must not block a later issuance at the same amount");

        assert_eq!(state.civic_debts.len(), 2);
        assert_eq!(
            state
                .laws
                .values()
                .filter(|law| law.kind == LawKind::PublicDebtAuthorization)
                .count(),
            2
        );
        assert!(
            state
                .laws
                .values()
                .filter(|law| law.kind == LawKind::PublicDebtAuthorization)
                .all(|law| !law.active)
        );
        validate_invariants(registry, &state);
    }

    #[test]
    fn public_debt_authorization_rejects_missing_credit_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::Taxation);
        for dynasty in state
            .dynasties
            .values_mut()
            .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        {
            dynasty.resources.treasury = Money::from_copper(10_000);
        }
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
            Err(CommandError::NoCivicDebtCreditor {
                required: Money::from_copper(10_000),
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "failed civic debt underwriting must not charge or mutate any record",
        );
    }

    #[test]
    fn rejects_reenacting_identical_active_law_without_spending() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (kind, value) = state
            .laws
            .values()
            .find(|law| law.active)
            .map(|law| (law.kind, law.value))
            .expect("campaign must contain an active law");
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

    #[test]
    fn law_sponsorship_requires_an_office_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::BreadPriceCeiling,
                value: 30,
            },
        );

        assert_eq!(result, Err(CommandError::LawSponsorshipRequiresOffice));
        assert_state_unchanged(
            &before,
            &state,
            "a dynasty without office must not spend legitimacy or enact a law",
        );
    }

    #[test]
    fn law_sponsorship_requires_matching_office_power_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::WatchPriorities);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::BreadPriceCeiling,
                value: 30,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::LawSponsorshipRequiresPower {
                kind: LawKind::BreadPriceCeiling,
                required: OfficePower::MarketTolls,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "an unrelated office must not authorize law sponsorship",
        );
    }

    #[test]
    fn law_sponsorship_waits_for_office_power_to_be_established() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::MarketTolls);
        let current_day = state.clock.day();
        let available_day = current_day.saturating_add(OFFICE_POWER_ESTABLISHMENT_DAYS);
        state
            .institutions
            .values_mut()
            .find(|institution| institution.powers.contains(&OfficePower::MarketTolls))
            .expect("campaign must contain a market-tolls office")
            .term_started_day = current_day;
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::BreadPriceCeiling,
                value: 30,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::LawSponsorshipPowerNotEstablished {
                kind: LawKind::BreadPriceCeiling,
                required: OfficePower::MarketTolls,
                available_day,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "new office power must not enact a law before its establishment period",
        );
    }

    #[test]
    fn law_sponsorship_has_a_yearly_strategic_interval() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::BreadPriceCeiling,
                value: 30,
            },
        )
        .expect("first law must succeed");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::FireCode,
                value: 7_000,
            },
        );

        assert!(matches!(result, Err(CommandError::LawCooldown { .. })));
        assert_state_unchanged(
            &before,
            &state,
            "the law interval must prevent rapid checklist enactment without partial spending",
        );
    }

    #[test]
    fn law_sponsorship_requires_political_legitimacy() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = LAW_LEGITIMACY_REQUIREMENT.saturating_sub(1);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EnactLaw {
                kind: LawKind::BreadPriceCeiling,
                value: 30,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::InsufficientPlayerLegitimacy {
                available: LAW_LEGITIMACY_REQUIREMENT.saturating_sub(1),
                required: LAW_LEGITIMACY_REQUIREMENT,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "an illegitimate law proposal must fail before consuming treasury or history",
        );
    }
}

mod politics {
    use super::*;
    use crate::systems::advance_days;

    fn make_nominee_deliberately_weak(
        state: &mut AppState,
        institution_id: InstitutionId,
        character_id: CharacterId,
    ) {
        let member_ids: Vec<_> = state
            .institutions
            .get(&institution_id)
            .expect("institution must exist")
            .members
            .iter()
            .copied()
            .collect();
        for member_id in member_ids {
            let capabilities = &mut state
                .characters
                .get_mut(member_id)
                .expect("institution member must exist")
                .capabilities;
            capabilities.administration = 100;
            capabilities.commerce = 100;
            capabilities.social = 100;
            capabilities.craft = 100;
            let dynasty_id = state
                .characters
                .get(member_id)
                .expect("institution member must exist")
                .dynasty_id();
            state
                .dynasties
                .get_mut(&dynasty_id)
                .expect("member dynasty must exist")
                .resources
                .legitimacy_basis_points = 10_000;
        }
        let capabilities = &mut state
            .characters
            .get_mut(character_id)
            .expect("nominee must exist")
            .capabilities;
        capabilities.administration = 0;
        capabilities.commerce = 0;
        capabilities.social = 0;
        capabilities.craft = 0;
    }

    fn resolved_failed_nomination_fixture(
        registry: &Registry,
    ) -> (AppState, CharacterId, InstitutionId, usize) {
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_quality_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        grant_office_nomination_record_for_test(&mut state);
        make_nominee_deliberately_weak(&mut state, institution_id, character_id);
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id,
            },
        )
        .expect("first nomination must add the heir as a member");
        let member_count = state
            .institutions
            .get(&institution_id)
            .expect("institution must exist")
            .members
            .len();
        advance_days(
            registry,
            &mut state,
            u32::try_from(OFFICE_NOMINATION_INTERVAL_DAYS)
                .expect("office nomination interval must fit the simulation day command"),
        )
        .expect("campaign must advance through the nomination cooldown");
        assert_ne!(
            state
                .institutions
                .get(&institution_id)
                .expect("institution must exist")
                .office_holder_id,
            Some(character_id),
            "the deliberately weak nominee must lose the first contest"
        );
        let player = state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        player.resources.reputation_quality_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        player.resources.treasury = Money::from_copper(10_000);
        (state, character_id, institution_id, member_count)
    }

    fn make_patronage_fixture() -> (
        AppState,
        DynastyId,
        CharacterId,
        InstitutionId,
        Money,
        Money,
    ) {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let character_id = state
            .dynasties
            .get(&player_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_reliability_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        let player_businesses: BTreeSet<_> = state
            .businesses
            .iter()
            .filter(|business| business.owner_dynasty_id() == player_id)
            .map(crate::core::Business::id)
            .collect();
        let contract = state
            .contracts
            .values_mut()
            .find(|contract| {
                player_businesses.contains(&contract.buyer_business_id)
                    || player_businesses.contains(&contract.seller_business_id)
            })
            .expect("campaign must contain a player contract");
        let deliveries = u16::try_from(INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT)
            .expect("delivery requirement must fit contract counters");
        contract.fulfilled_deliveries = deliveries;
        contract
            .fulfilled_deliveries_by_dynasty
            .insert(player_id, deliveries);
        let treasury_before = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .treasury();
        let budget_before = state
            .institutions
            .get(&institution_id)
            .expect("institution must exist")
            .budget;
        (
            state,
            player_id,
            character_id,
            institution_id,
            treasury_before,
            budget_before,
        )
    }

    fn grant_nomination_delivery_record(
        state: &mut AppState,
        player_id: DynastyId,
        required_deliveries: u32,
    ) {
        let player_businesses: BTreeSet<_> = state
            .businesses
            .iter()
            .filter(|business| business.owner_dynasty_id() == player_id)
            .map(crate::core::Business::id)
            .collect();
        let contract = state
            .contracts
            .values_mut()
            .find(|contract| {
                player_businesses.contains(&contract.buyer_business_id)
                    || player_businesses.contains(&contract.seller_business_id)
            })
            .expect("campaign must contain a player contract");
        let deliveries = u16::try_from(required_deliveries)
            .expect("nomination delivery requirement must fit contract counters");
        contract.fulfilled_deliveries = deliveries;
        contract
            .fulfilled_deliveries_by_dynasty
            .insert(player_id, deliveries);
    }

    #[test]
    fn new_dynasty_must_earn_institution_membership() {
        let state = make_test_campaign();
        let player_id = state.player_dynasty_id;

        assert!(state.institutions.values().all(|institution| {
            institution.members.iter().all(|character_id| {
                state
                    .characters
                    .get(*character_id)
                    .is_some_and(|character| character.dynasty_id() != player_id)
            })
        }));
    }

    #[test]
    fn candidate_capability_reduces_extra_commercial_preparation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        {
            let capabilities = &mut state
                .characters
                .get_mut(character_id)
                .expect("player character must exist")
                .capabilities;
            capabilities.administration = 0;
            capabilities.commerce = 0;
            capabilities.social = 0;
            capabilities.craft = 0;
        }
        let unprepared =
            office_nomination_delivery_requirement(registry, &state, institution_id, character_id);
        {
            let capabilities = &mut state
                .characters
                .get_mut(character_id)
                .expect("player character must exist")
                .capabilities;
            capabilities.administration = 100;
            capabilities.commerce = 100;
            capabilities.social = 100;
            capabilities.craft = 100;
        }
        let prepared =
            office_nomination_delivery_requirement(registry, &state, institution_id, character_id);

        assert_eq!(prepared, OFFICE_NOMINATION_DELIVERY_REQUIREMENT);
        assert_eq!(
            unprepared,
            OFFICE_NOMINATION_DELIVERY_REQUIREMENT + OFFICE_NOMINATION_MAX_PREPARATION_DELIVERIES
        );
        assert!(prepared < unprepared);
    }

    #[test]
    fn patronage_creates_support_that_must_mature_before_nomination() {
        let registry = rivergate_registry_for_test();
        let (mut state, player_id, character_id, institution_id, treasury_before, budget_before) =
            make_patronage_fixture();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id,
                character_id,
            },
        )
        .expect("qualified patronage must succeed");

        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .treasury(),
            treasury_before.saturating_sub(INSTITUTION_SUPPORT_COST)
        );
        let institution = state
            .institutions
            .get(&institution_id)
            .expect("institution must exist");
        assert_eq!(
            institution.budget,
            budget_before.saturating_add(INSTITUTION_SUPPORT_COST)
        );
        assert!(institution.members.contains(&character_id));
        assert!(state.audit_log.iter().any(|record| {
            record.kind() == AuditKind::InstitutionPatronage
                && record.subject() == institution_support_subject(institution_id, character_id)
        }));

        let nomination_delivery_requirement =
            office_nomination_delivery_requirement(registry, &state, institution_id, character_id);
        let before_incomplete_record_nomination = state.clone();
        let incomplete_record_nomination = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id,
            },
        );
        assert_eq!(
            incomplete_record_nomination,
            Err(CommandError::InsufficientOfficeCommercialRecord {
                delivered: INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT,
                required: nomination_delivery_requirement,
            })
        );
        assert_state_unchanged(
            &before_incomplete_record_nomination,
            &state,
            "patronage must open before the commercial record is strong enough for candidacy",
        );

        grant_nomination_delivery_record(&mut state, player_id, nomination_delivery_requirement);

        let before_early_nomination = state.clone();
        let early_nomination = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id,
            },
        );
        assert_eq!(
            early_nomination,
            Err(CommandError::InstitutionSupportNotEstablished {
                institution_id,
                character_id,
                available_day: INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS,
            })
        );
        assert_state_unchanged(
            &before_early_nomination,
            &state,
            "premature nomination must not mutate state",
        );

        advance_days(
            registry,
            &mut state,
            u32::try_from(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS)
                .expect("support establishment period must fit u32"),
        )
        .expect("campaign must reach established support");
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id,
            },
        )
        .expect("established support must permit nomination");
        validate_invariants(registry, &state);
    }

    #[test]
    fn malformed_institution_history_does_not_create_character_cooldowns() {
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
            detail: "invalid persisted history fixture".to_owned(),
        });
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeNomination,
            subject: format!("invalid:character:{character_id}").into(),
            detail: "invalid persisted history fixture".to_owned(),
        });

        assert_eq!(institution_support_next_day(&state, character_id), None);
        assert_eq!(office_nomination_next_day(&state, character_id), None);
    }

    #[test]
    fn officeholder_can_withdraw_from_an_institution_and_resign() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::PublicWorks);
        let (institution_id, character_id, next_selection_before) = state
            .institutions
            .values()
            .find_map(|institution| {
                let character_id = institution.office_holder_id?;
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
                    .then_some((
                        institution.institution_id,
                        character_id,
                        institution.next_selection_day,
                    ))
            })
            .expect("test setup must grant a player office");
        let day = state.clock.day();

        let outcome = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::WithdrawFromInstitution {
                institution_id,
                character_id,
            },
        )
        .expect("an officeholder must be able to resign and withdraw");

        let institution = state
            .institutions
            .get(&institution_id)
            .expect("institution must remain present");
        assert!(!institution.members.contains(&character_id));
        assert_eq!(institution.office_holder_id, None);
        assert!(institution.next_selection_day <= day.saturating_add(30));
        assert!(institution.next_selection_day <= next_selection_before);
        assert!(outcome.summary.contains("resigned the office"));
        assert!(state.outbox.last().is_some_and(|message| {
            message.kind == OutboxKind::Politics
                && message.subject.contains("withdrew from institution")
        }));
        validate_invariants(registry, &state);
    }

    #[test]
    fn withdrawn_institution_support_can_be_rebuilt_after_the_cooldown() {
        let registry = rivergate_registry_for_test();
        let (mut state, player_id, character_id, institution_id, _, _) = make_patronage_fixture();
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id,
                character_id,
            },
        )
        .expect("initial patronage must succeed");
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::WithdrawFromInstitution {
                institution_id,
                character_id,
            },
        )
        .expect("supported member must be able to withdraw");
        for _ in 0..INSTITUTION_WITHDRAWAL_RECOVERY_DAYS {
            state.clock.advance_one_day();
        }
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(10_000);
        let subject = institution_support_subject(institution_id, character_id);
        let patronage_records_before = state
            .audit_log
            .iter()
            .filter(|record| {
                record.kind() == AuditKind::InstitutionPatronage && record.subject() == subject
            })
            .count();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id,
                character_id,
            },
        )
        .expect("withdrawn support must be rebuildable after the cooldown");

        assert!(
            state
                .institutions
                .get(&institution_id)
                .expect("institution must exist")
                .members
                .contains(&character_id)
        );
        assert_eq!(
            state
                .audit_log
                .iter()
                .filter(|record| {
                    record.kind() == AuditKind::InstitutionPatronage && record.subject() == subject
                })
                .count(),
            patronage_records_before + 1
        );
        validate_invariants(registry, &state);
    }

    #[test]
    fn nonmember_cannot_withdraw_from_an_institution() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let institution_id = state
            .institutions
            .keys()
            .copied()
            .next()
            .expect("campaign must contain an institution");
        let institution = state
            .institutions
            .get_mut(&institution_id)
            .expect("institution must exist");
        institution.members.remove(&character_id);
        assert_ne!(institution.office_holder_id, Some(character_id));
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::WithdrawFromInstitution {
                institution_id,
                character_id,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::InvalidInstitutionWithdrawal {
                institution_id,
                character_id,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "rejected institution withdrawal must not mutate state",
        );
    }

    #[test]
    fn selling_a_business_preserves_the_sellers_delivery_record() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_dynasty_id = state.player_dynasty_id;
        let player_contract_id = state
            .contracts
            .iter()
            .find_map(|(contract_id, contract)| {
                [contract.buyer_business_id, contract.seller_business_id]
                    .into_iter()
                    .any(|business_id| {
                        state.businesses.get(business_id).is_some_and(|business| {
                            business.owner_dynasty_id() == player_dynasty_id
                        })
                    })
                    .then_some(*contract_id)
            })
            .expect("campaign must contain a player contract");
        let player_business_id = {
            let contract = state
                .contracts
                .get(&player_contract_id)
                .expect("player contract must exist");
            [contract.buyer_business_id, contract.seller_business_id]
                .into_iter()
                .find(|business_id| {
                    state
                        .businesses
                        .get(*business_id)
                        .is_some_and(|business| business.owner_dynasty_id() == player_dynasty_id)
                })
                .expect("player contract must include a player business")
        };
        {
            let contract = state
                .contracts
                .get_mut(&player_contract_id)
                .expect("player contract must exist");
            contract.fulfilled_deliveries = 7;
            contract
                .fulfilled_deliveries_by_dynasty
                .insert(player_dynasty_id, 7);
        }
        assert_eq!(player_contract_deliveries(&state), 7);
        let new_owner_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_dynasty_id)
            .expect("campaign must contain another dynasty");
        let new_manager_id = state
            .characters
            .ids_for_dynasty(new_owner_id)
            .into_iter()
            .flatten()
            .next()
            .copied()
            .expect("new owner must have a manager");
        let business = state
            .businesses
            .get_mut(player_business_id)
            .expect("player business must exist");
        business.operations.status = BusinessStatus::Distressed;
        business.finance.cash = Money::ZERO;
        let quote = crate::systems::quote_business_acquisition(
            registry,
            &state,
            new_owner_id,
            player_business_id,
        )
        .expect("distressed player business must be acquirable");
        crate::systems::acquire_business(
            registry,
            &mut state,
            new_owner_id,
            player_business_id,
            new_manager_id,
            quote.minimum_recapitalization,
        )
        .expect("player business must transfer through the acquisition system");
        assert_eq!(
            player_contract_deliveries(&state),
            7,
            "selling a business must not erase delivery history earned before the sale"
        );
    }

    #[test]
    fn acquiring_a_business_does_not_inherit_the_former_owners_delivery_record() {
        let registry = rivergate_registry_for_test();
        let mut inherited_state = make_test_campaign();
        let player_dynasty_id = inherited_state.player_dynasty_id;
        let ambient_contract_id = inherited_state
            .contracts
            .iter()
            .find_map(|(contract_id, contract)| {
                let owners: Vec<_> = [contract.buyer_business_id, contract.seller_business_id]
                    .into_iter()
                    .map(|business_id| {
                        inherited_state
                            .businesses
                            .get(business_id)
                            .expect("contract business must exist")
                            .owner_dynasty_id()
                    })
                    .collect();
                owners
                    .iter()
                    .all(|dynasty_id| *dynasty_id != inherited_state.player_dynasty_id)
                    .then_some((*contract_id, owners))
            })
            .expect("campaign must contain an ambient contract");
        let ambient_business_id = inherited_state
            .contracts
            .get(&ambient_contract_id.0)
            .expect("ambient contract must exist")
            .seller_business_id;
        {
            let contract = inherited_state
                .contracts
                .get_mut(&ambient_contract_id.0)
                .expect("ambient contract must exist");
            contract.fulfilled_deliveries = 9;
            for dynasty_id in ambient_contract_id.1 {
                contract
                    .fulfilled_deliveries_by_dynasty
                    .insert(dynasty_id, 9);
            }
        }
        assert_eq!(player_contract_deliveries(&inherited_state), 0);
        let player_manager_id = inherited_state
            .dynasties
            .get(&player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let business = inherited_state
            .businesses
            .get_mut(ambient_business_id)
            .expect("ambient business must exist");
        business.operations.status = BusinessStatus::Distressed;
        business.finance.cash = Money::ZERO;
        let quote = crate::systems::quote_business_acquisition(
            registry,
            &inherited_state,
            player_dynasty_id,
            ambient_business_id,
        )
        .expect("distressed ambient business must be acquirable");
        crate::systems::acquire_business(
            registry,
            &mut inherited_state,
            player_dynasty_id,
            ambient_business_id,
            player_manager_id,
            quote.minimum_recapitalization,
        )
        .expect("ambient business must transfer through the acquisition system");
        assert_eq!(
            player_contract_deliveries(&inherited_state),
            0,
            "acquiring a business must not confer delivery history earned by its former owner"
        );
    }

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
            let capabilities = &mut state
                .characters
                .get_mut(member_id)
                .expect("institution member must exist")
                .capabilities;
            capabilities.administration = 0;
            capabilities.commerce = 0;
            capabilities.social = 0;
            capabilities.craft = 0;
        }
        let nominee_capabilities = &mut state
            .characters
            .get_mut(nominee_id)
            .expect("nominee must exist")
            .capabilities;
        nominee_capabilities.administration = 100;
        nominee_capabilities.commerce = 100;
        nominee_capabilities.social = 100;
        nominee_capabilities.craft = 100;
        for dynasty in state.dynasties.values_mut() {
            dynasty.resources.legitimacy_basis_points = 0;
        }
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_reliability_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        grant_office_nomination_record_for_test(&mut state);
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
                <= OFFICE_NOMINATION_RESOLUTION_DAYS,
            "nomination must schedule a timely contest"
        );

        advance_days(
            registry,
            &mut state,
            u32::try_from(OFFICE_NOMINATION_RESOLUTION_DAYS)
                .expect("office nomination resolution must fit u32"),
        )
        .expect("campaign must reach the selection");

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
    fn nomination_does_not_retroactively_establish_incumbent_office_power() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        advance_days(registry, &mut state, 120).expect("campaign must advance");
        let current_day = state.clock.day();
        let player = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        let holder_id = player.head_id();
        let nominee_id = player.heir_id().expect("player dynasty must have an heir");
        let institution_id = state
            .institutions
            .values()
            .find(|institution| {
                institution.powers.contains(&OfficePower::MarketTolls)
                    && !institution.members.contains(&nominee_id)
            })
            .expect("campaign must contain a suitable market office")
            .institution_id;
        {
            let institution = state
                .institutions
                .get_mut(&institution_id)
                .expect("selected institution must exist");
            institution.members.insert(holder_id);
            institution.office_holder_id = Some(holder_id);
            institution.term_started_day = current_day;
            institution.next_selection_day = current_day.saturating_add(OFFICE_TERM_DAYS);
        }
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_reliability_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        grant_office_nomination_record_for_test(&mut state);

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id: nominee_id,
            },
        )
        .expect("nomination must succeed");

        let expected_available_day = current_day.saturating_add(OFFICE_POWER_ESTABLISHMENT_DAYS);
        assert_eq!(
            state
                .institutions
                .get(&institution_id)
                .expect("institution must exist")
                .next_selection_day,
            current_day.saturating_add(OFFICE_NOMINATION_RESOLUTION_DAYS),
            "nomination should still schedule a timely contest"
        );
        assert_eq!(
            player_office_power_available_day(&state, OfficePower::MarketTolls),
            Some(expected_available_day),
            "election scheduling must not rewrite the incumbent term start"
        );
        assert!(
            !has_established_player_office_power(&state, OfficePower::MarketTolls),
            "a newly acquired office power must remain unavailable during establishment"
        );
    }

    #[test]
    fn office_nomination_requires_earned_reputation_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let institution_id = state
            .institutions
            .values()
            .find(|institution| !institution.members.contains(&character_id))
            .expect("campaign must contain an institution open to the heir")
            .institution_id;
        let player = state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        player.resources.reputation_quality_basis_points =
            OFFICE_NOMINATION_REPUTATION_REQUIREMENT.saturating_sub(1);
        player.resources.reputation_reliability_basis_points =
            OFFICE_NOMINATION_REPUTATION_REQUIREMENT.saturating_sub(1);
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
            Err(CommandError::InsufficientOfficeReputation {
                quality: OFFICE_NOMINATION_REPUTATION_REQUIREMENT.saturating_sub(1),
                reliability: OFFICE_NOMINATION_REPUTATION_REQUIREMENT.saturating_sub(1),
                required: OFFICE_NOMINATION_REPUTATION_REQUIREMENT,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "an unproven dynasty must not spend money or join an institution",
        );
    }

    #[test]
    fn office_nomination_requires_completed_deliveries_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let institution_id = state
            .institutions
            .values()
            .find(|institution| !institution.members.contains(&character_id))
            .expect("campaign must contain an institution open to the heir")
            .institution_id;
        let player = state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        player.resources.reputation_reliability_basis_points =
            OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        let delivered = player_contract_deliveries(&state);
        assert!(delivered < OFFICE_NOMINATION_DELIVERY_REQUIREMENT);
        let required =
            office_nomination_delivery_requirement(registry, &state, institution_id, character_id);
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
            Err(CommandError::InsufficientOfficeCommercialRecord {
                delivered,
                required,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "an unproven commercial record must not spend money or alter membership",
        );
    }

    #[test]
    fn office_nomination_cooldown_follows_the_character_across_institutions() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_reliability_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        grant_office_nomination_record_for_test(&mut state);
        let institution_ids: Vec<_> = state
            .institutions
            .values()
            .filter(|institution| institution.members.contains(&character_id))
            .map(|institution| institution.institution_id)
            .take(2)
            .collect();
        let [first_institution_id, second_institution_id] = institution_ids.as_slice() else {
            panic!("fixture must contain at least two eligible institutions: {institution_ids:?}");
        };

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id: *first_institution_id,
                character_id,
            },
        )
        .expect("first earned nomination must succeed");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id: *second_institution_id,
                character_id,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::OfficeNominationCooldown {
                next_nomination_day: OFFICE_NOMINATION_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "the same character must not launch overlapping office campaigns",
        );
    }

    #[test]
    fn different_family_members_can_campaign_in_parallel() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_office_nomination_record_for_test(&mut state);
        let dynasty = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        let first_character_id = dynasty.head_id();
        let second_character_id = dynasty.heir_id().expect("player dynasty must have an heir");
        let institution_ids: Vec<_> = state.institutions.keys().copied().take(2).collect();
        let [first_institution_id, second_institution_id] = institution_ids.as_slice() else {
            panic!("fixture must contain two institutions: {institution_ids:?}");
        };

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id: *first_institution_id,
                character_id: first_character_id,
            },
        )
        .expect("the first family campaign must succeed");
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id: *second_institution_id,
                character_id: second_character_id,
            },
        )
        .expect("another family member must be able to campaign in parallel");

        assert_eq!(
            state
                .audit_log
                .iter()
                .filter(|record| record.kind() == AuditKind::OfficeNomination)
                .count(),
            2,
            "parallel campaigns must leave separate durable records",
        );
    }

    #[test]
    fn institution_support_cooldown_is_per_character() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_commercial_standing_for_test(&mut state);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(20_000);
        let dynasty = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        let first_character_id = dynasty.head_id();
        let second_character_id = dynasty.heir_id().expect("player dynasty must have an heir");
        let institution_ids: Vec<_> = state.institutions.keys().copied().take(3).collect();
        let [
            first_institution_id,
            second_institution_id,
            third_institution_id,
        ] = institution_ids.as_slice()
        else {
            panic!("fixture must contain three institutions: {institution_ids:?}");
        };

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id: *first_institution_id,
                character_id: first_character_id,
            },
        )
        .expect("the first character must cultivate support");
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id: *second_institution_id,
                character_id: second_character_id,
            },
        )
        .expect("another character must cultivate support without waiting a year");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id: *third_institution_id,
                character_id: first_character_id,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::InstitutionSupportCooldown {
                next_support_day: INSTITUTION_SUPPORT_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "one character's support campaign must retain its own cooldown",
        );
    }

    #[test]
    fn character_institutional_portfolio_is_bounded() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_commercial_standing_for_test(&mut state);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(20_000);
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let institution_ids: Vec<_> = state.institutions.keys().copied().take(3).collect();
        let [
            first_institution_id,
            second_institution_id,
            third_institution_id,
        ] = institution_ids.as_slice()
        else {
            panic!("fixture must contain three institutions: {institution_ids:?}");
        };

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id: *first_institution_id,
                character_id,
            },
        )
        .expect("first institutional affiliation must succeed");
        advance_days(
            registry,
            &mut state,
            u32::try_from(INSTITUTION_SUPPORT_INTERVAL_DAYS)
                .expect("support interval must fit day command"),
        )
        .expect("campaign must advance through the support cooldown");
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id: *second_institution_id,
                character_id,
            },
        )
        .expect("second institutional affiliation must succeed");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id: *third_institution_id,
                character_id,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::InstitutionMembershipCapacity {
                character_id,
                current: MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER,
                maximum: MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a character at institutional capacity must not spend treasury or alter membership",
        );
    }

    #[test]
    fn failed_candidate_can_campaign_again_after_the_recovery_period() {
        let registry = rivergate_registry_for_test();
        let (mut state, character_id, institution_id, member_count) =
            resolved_failed_nomination_fixture(registry);
        let before_recovery = state.clone();

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
            Err(CommandError::OfficeNominationCooldown {
                next_nomination_day: OFFICE_NOMINATION_RECOVERY_DAYS,
            })
        );
        assert_state_unchanged(
            &before_recovery,
            &state,
            "a resolved failed campaign must require a longer recovery before renomination",
        );

        advance_days(
            registry,
            &mut state,
            u32::try_from(
                OFFICE_NOMINATION_RECOVERY_DAYS.saturating_sub(OFFICE_NOMINATION_INTERVAL_DAYS),
            )
            .expect("remaining nomination recovery must fit the simulation day command"),
        )
        .expect("campaign must advance through the remaining nomination recovery");
        let player = state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        player.resources.reputation_quality_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        player.resources.treasury = Money::from_copper(10_000);
        let treasury_before = player.treasury();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id,
            },
        )
        .expect("an existing member must be allowed to fund a later campaign after recovery");

        assert_eq!(
            state
                .institutions
                .get(&institution_id)
                .expect("institution must exist")
                .members
                .len(),
            member_count,
            "renomination must not duplicate institution membership"
        );
        assert_eq!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury(),
            treasury_before.saturating_sub(Money::from_copper(300))
        );
        assert_eq!(
            state
                .audit_log
                .iter()
                .filter(|record| record.kind() == AuditKind::OfficeNomination)
                .count(),
            2,
            "each funded campaign must leave a distinct durable nomination record"
        );
    }

    #[test]
    fn current_officeholder_cannot_be_nominated_to_a_second_institution() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let character_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let held_institution_id = state
            .institutions
            .keys()
            .next()
            .copied()
            .expect("campaign must contain an institution");
        state
            .institutions
            .get_mut(&held_institution_id)
            .expect("held institution must exist")
            .office_holder_id = Some(character_id);
        let target_institution_id = state
            .institutions
            .values()
            .find(|institution| {
                institution.institution_id != held_institution_id
                    && !institution.members.contains(&character_id)
            })
            .map(|institution| institution.institution_id)
            .expect("another institution must accept a nomination attempt");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id: target_institution_id,
                character_id,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::NomineeAlreadyHoldsOffice {
                character_id,
                institution_id: held_institution_id,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "an officeholder cannot campaign for a second simultaneous office",
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
    fn reserved_family_charter_version_rejects_governance_change_atomically() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let current = state
            .family_councils
            .get(&dynasty_id)
            .expect("player family council must exist")
            .governance;
        let governance = [
            HouseGovernance::Primogeniture,
            HouseGovernance::FamilyPartnership,
            HouseGovernance::BranchFederation,
        ]
        .into_iter()
        .find(|governance| *governance != current)
        .expect("fixture must expose an alternative governance");
        state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("player family council must exist")
            .charter_version = u64::MAX - 1;
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance { governance },
        );

        assert_eq!(
            result,
            Err(CommandError::Simulation(
                crate::systems::SimulationError::FamilyCharterVersionExhausted { dynasty_id }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "charter exhaustion must not partially amend governance or family unity",
        );
    }

    #[test]
    fn governance_cannot_be_rewritten_twice_within_three_years() {
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
        let [first_alternative, second_alternative] = alternatives.as_slice() else {
            panic!("fixture must expose two governance alternatives: {alternatives:?}");
        };

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance {
                governance: *first_alternative,
            },
        )
        .expect("first charter amendment must succeed");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance {
                governance: *second_alternative,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::HouseGovernanceCooldown {
                next_change_day: HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a premature charter amendment must not mutate family governance",
        );

        for _ in 0..HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance {
                governance: *second_alternative,
            },
        )
        .expect("the family must be able to amend the charter again after three years");
        assert_eq!(
            state
                .family_councils
                .get(&state.player_dynasty_id)
                .expect("player family council must exist")
                .governance,
            *second_alternative
        );
    }

    #[test]
    fn family_council_meeting_restores_cohesion_and_has_an_annual_cooldown() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let member_ids: Vec<_> = state
            .family_councils
            .get(&dynasty_id)
            .expect("player family council must exist")
            .members
            .iter()
            .copied()
            .collect();
        state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("player family council must exist")
            .unity_basis_points = 4_000;
        for character_id in &member_ids {
            state
                .characters
                .get_mut(*character_id)
                .expect("family council member must exist")
                .runtime
                .loyalty_basis_points = 5_000;
        }
        let treasury_before = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .treasury();

        apply_player_command(registry, &mut state, PlayerCommand::ConveneFamilyCouncil)
            .expect("funded family council meeting must succeed");

        assert_eq!(
            state
                .family_councils
                .get(&dynasty_id)
                .expect("player family council must exist")
                .unity_basis_points,
            5_500
        );
        assert_eq!(
            state
                .dynasties
                .get(&dynasty_id)
                .expect("player dynasty must exist")
                .treasury(),
            treasury_before.saturating_sub(FAMILY_COUNCIL_MEETING_COST)
        );
        assert!(member_ids.iter().all(|character_id| {
            state
                .characters
                .get(*character_id)
                .is_some_and(|character| character.runtime.loyalty_basis_points == 5_600)
        }));
        assert!(state.audit_log.iter().any(|record| {
            record.kind() == AuditKind::HouseGovernanceChange
                && record.subject() == format!("dynasty:{dynasty_id};council-meeting")
        }));
        let before = state.clone();

        let result =
            apply_player_command(registry, &mut state, PlayerCommand::ConveneFamilyCouncil);

        assert_eq!(
            result,
            Err(CommandError::FamilyCouncilMeetingCooldown {
                next_meeting_day: FAMILY_COUNCIL_MEETING_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "premature family reconciliation must not mutate the dynasty",
        );

        for _ in 0..FAMILY_COUNCIL_MEETING_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        apply_player_command(registry, &mut state, PlayerCommand::ConveneFamilyCouncil)
            .expect("family council must be available again after one year");
        assert_eq!(
            state
                .family_councils
                .get(&dynasty_id)
                .expect("player family council must exist")
                .unity_basis_points,
            7_000
        );
    }

    #[test]
    fn heir_designation_turns_a_family_member_into_the_planned_successor() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let prior_heir_id = state
            .dynasties
            .get(&player_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let candidate_id = state.next_ids.character();
        let mut candidate = state
            .characters
            .get(prior_heir_id)
            .expect("prior heir must exist")
            .clone();
        candidate.identity.id = candidate_id;
        candidate.identity.name = "Deliberate Successor".to_owned();
        candidate.identity.birth_day = state.clock.day().saturating_sub(30 * 360);
        candidate.runtime.role = CharacterRole::Clerk;
        state.characters.insert(candidate);
        state
            .family_councils
            .get_mut(&player_id)
            .expect("player family council must exist")
            .members
            .insert(candidate_id);
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 1_000;
        let unity_before = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist")
            .unity_basis_points;
        let charter_before = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist")
            .charter_version;

        let outcome = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::DesignateHeir {
                character_id: candidate_id,
            },
        )
        .expect("eligible council member must be designatable as heir");

        assert!(outcome.summary.contains(&candidate_id.to_string()));
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .and_then(crate::core::Dynasty::heir_id),
            Some(candidate_id)
        );
        assert_eq!(
            state
                .characters
                .get(prior_heir_id)
                .expect("prior heir must exist")
                .role(),
            CharacterRole::Clerk
        );
        assert_eq!(
            state
                .characters
                .get(candidate_id)
                .expect("new heir must exist")
                .role(),
            CharacterRole::Heir
        );
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .resources
                .legitimacy_basis_points,
            1_000 - HEIR_DESIGNATION_LEGITIMACY_COST
        );
        let council = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist");
        assert_eq!(council.unity_basis_points, unity_before.saturating_sub(250));
        assert_eq!(council.charter_version, charter_before.saturating_add(1));
        assert!(state.audit_log.iter().any(|record| {
            record.kind() == AuditKind::HeirDesignation
                && record.detail().contains(&candidate_id.to_string())
        }));
        assert!(state.chronicle.iter().any(|entry| {
            entry.kind() == ChronicleKind::SuccessionPrepared
                && entry.summary().contains(&candidate_id.to_string())
        }));
        validate_invariants(registry, &state);
    }

    #[test]
    fn default_heir_can_be_formally_confirmed_once() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let heir_id = state
            .dynasties
            .get(&player_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 1_000;
        let unity_before = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist")
            .unity_basis_points;
        let charter_before = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist")
            .charter_version;

        let outcome = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::DesignateHeir {
                character_id: heir_id,
            },
        )
        .expect("the default heir must be formally confirmable before any charter designation");

        assert!(outcome.summary.contains("confirmed"));
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .and_then(crate::core::Dynasty::heir_id),
            Some(heir_id)
        );
        assert_eq!(
            state
                .characters
                .get(heir_id)
                .expect("heir must exist")
                .role(),
            CharacterRole::Heir
        );
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .resources
                .legitimacy_basis_points,
            1_000 - HEIR_DESIGNATION_LEGITIMACY_COST
        );
        let council = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist");
        assert_eq!(
            council.unity_basis_points,
            unity_before.saturating_sub(HEIR_DESIGNATION_UNITY_COST)
        );
        assert_eq!(council.charter_version, charter_before.saturating_add(1));
        assert!(state.audit_log.iter().any(|record| {
            record.kind() == AuditKind::HeirDesignation
                && record.detail().contains("confirmation=true")
        }));
        assert!(state.chronicle.iter().any(|entry| {
            entry.kind() == ChronicleKind::SuccessionPrepared
                && entry.summary().contains("formally confirmed")
        }));
        let before_repeat = state.clone();

        let repeat = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::DesignateHeir {
                character_id: heir_id,
            },
        );

        assert_eq!(
            repeat,
            Err(CommandError::UnchangedHeir {
                character_id: heir_id
            })
        );
        assert_state_unchanged(
            &before_repeat,
            &state,
            "a formally confirmed heir must not be confirmed repeatedly",
        );
        validate_invariants(registry, &state);
    }

    #[test]
    fn heir_designation_cooldown_is_atomic() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let prior_heir_id = state
            .dynasties
            .get(&player_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        let candidate_id = state.next_ids.character();
        let mut candidate = state
            .characters
            .get(prior_heir_id)
            .expect("prior heir must exist")
            .clone();
        candidate.identity.id = candidate_id;
        candidate.identity.name = "First Successor".to_owned();
        candidate.identity.birth_day = state.clock.day().saturating_sub(30 * 360);
        candidate.runtime.role = CharacterRole::Clerk;
        state.characters.insert(candidate);
        state
            .family_councils
            .get_mut(&player_id)
            .expect("player family council must exist")
            .members
            .insert(candidate_id);
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 1_000;
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::DesignateHeir {
                character_id: candidate_id,
            },
        )
        .expect("first heir designation must succeed");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::DesignateHeir {
                character_id: prior_heir_id,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::HeirDesignationCooldown {
                next_designation_day: HEIR_DESIGNATION_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "premature heir replacement must not alter roles, legitimacy, or family records",
        );
    }

    #[test]
    fn office_power_directive_converts_officeholding_into_district_change() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::WatchPriorities);
        let institution_id = state
            .institutions
            .values()
            .find(|institution| {
                institution.powers.contains(&OfficePower::WatchPriorities)
                    && institution.office_holder_id.is_some_and(|character_id| {
                        state.characters.get(character_id).is_some_and(|character| {
                            character.dynasty_id() == state.player_dynasty_id
                        })
                    })
            })
            .expect("player must hold the watch office")
            .institution_id;
        let district_id = registry
            .get_institution(institution_id)
            .expect("institution definition must exist")
            .district_id();
        let district = state
            .districts
            .get_mut(&district_id)
            .expect("institution district must exist");
        district.safety_basis_points = 5_000;
        district.unrest_basis_points = 5_000;
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 1_000;

        let outcome = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ExerciseOfficePower {
                institution_id,
                power: OfficePower::WatchPriorities,
            },
        )
        .expect("the incumbent must be able to direct an established office power");

        assert!(outcome.summary.contains("WatchPriorities"));
        let district = state
            .districts
            .get(&district_id)
            .expect("institution district must exist");
        assert_eq!(district.safety_basis_points, 5_350);
        assert_eq!(district.unrest_basis_points, 4_850);
        assert_eq!(
            state
                .institutions
                .get(&institution_id)
                .expect("watch institution must exist")
                .active_directive,
            Some(crate::core::OfficeDirectiveState {
                power: OfficePower::WatchPriorities,
                expires_day: OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS,
            })
        );
        assert_eq!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .resources
                .legitimacy_basis_points,
            1_000 - OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST
        );
        assert!(state.audit_log.iter().any(|record| {
            record.kind() == AuditKind::OfficeDirective
                && record.subject() == format!("institution:{institution_id}")
        }));
        assert!(state.chronicle.iter().any(|entry| {
            entry.kind() == ChronicleKind::OfficeDirective
                && entry.summary().contains(&institution_id.to_string())
        }));
        validate_invariants(registry, &state);
    }

    #[test]
    fn office_power_directive_waits_for_the_power_to_be_established() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::WatchPriorities);
        let institution_id = state
            .institutions
            .values()
            .find(|institution| {
                institution.powers.contains(&OfficePower::WatchPriorities)
                    && institution.office_holder_id.is_some_and(|character_id| {
                        state.characters.get(character_id).is_some_and(|character| {
                            character.dynasty_id() == state.player_dynasty_id
                        })
                    })
            })
            .expect("player must hold the watch office")
            .institution_id;
        state
            .institutions
            .get_mut(&institution_id)
            .expect("watch institution must exist")
            .term_started_day = state.clock.day();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 1_000;
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ExerciseOfficePower {
                institution_id,
                power: OfficePower::WatchPriorities,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::OfficePowerDirectiveNotEstablished {
                institution_id,
                power: OfficePower::WatchPriorities,
                available_day: OFFICE_POWER_ESTABLISHMENT_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "unestablished office power must not spend legitimacy or alter district conditions",
        );
    }

    #[test]
    fn office_power_directive_cooldown_is_institution_wide_and_atomic() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_with_power_for_test(&mut state, OfficePower::WatchPriorities);
        let institution = state
            .institutions
            .values()
            .find(|institution| {
                institution.powers.contains(&OfficePower::WatchPriorities)
                    && institution.office_holder_id.is_some_and(|character_id| {
                        state.characters.get(character_id).is_some_and(|character| {
                            character.dynasty_id() == state.player_dynasty_id
                        })
                    })
            })
            .expect("player must hold the watch office")
            .clone();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 1_000;
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ExerciseOfficePower {
                institution_id: institution.institution_id,
                power: OfficePower::WatchPriorities,
            },
        )
        .expect("first directive must succeed");
        let second_power = institution
            .powers
            .iter()
            .copied()
            .find(|power| *power != OfficePower::WatchPriorities)
            .unwrap_or(OfficePower::WatchPriorities);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ExerciseOfficePower {
                institution_id: institution.institution_id,
                power: second_power,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::OfficePowerDirectiveCooldown {
                institution_id: institution.institution_id,
                power: second_power,
                next_directive_day: OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "an institution-wide directive cooldown must fail before legitimacy or district mutation",
        );
    }

    #[test]
    fn ward_adoption_expands_the_family_and_creates_a_trainable_officeholder() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        {
            let player = state
                .dynasties
                .get_mut(&player_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(20_000);
            player.resources.legitimacy_basis_points = WARD_ADOPTION_LEGITIMACY_REQUIREMENT;
            player.resources.reputation_reliability_basis_points =
                WARD_ADOPTION_REPUTATION_REQUIREMENT;
        }
        grant_office_nomination_record_for_test(&mut state);
        let characters_before = state.characters.iter().count();
        let chronicle_before = state.chronicle.len();
        let outbox_before = state.outbox.len();
        let council_members_before = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist")
            .members
            .len();
        let capacity_before = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .administrative_capacity();

        let outcome = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AdoptWard {
                focus: EducationFocus::Social,
            },
        )
        .expect("an established dynasty must be able to adopt a ward");

        assert!(outcome.summary.contains("Adopted ward"));
        assert_eq!(state.characters.iter().count(), characters_before + 1);
        assert_eq!(state.chronicle.len(), chronicle_before + 1);
        assert_eq!(state.outbox.len(), outbox_before + 1);
        let ward_link = state
            .family_links
            .values()
            .find(|link| link.kind == FamilyLinkKind::Ward)
            .expect("ward adoption must create a family link");
        let ward = state
            .characters
            .get(ward_link.second_character_id)
            .expect("ward character must exist");
        assert_eq!(ward.dynasty_id(), player_id);
        assert_eq!(ward.status(), CharacterStatus::Active);
        assert_eq!(ward.runtime.role, CharacterRole::Clerk);
        assert_eq!(ward.capabilities.social, 62);
        assert_eq!(
            state
                .family_councils
                .get(&player_id)
                .expect("player family council must exist")
                .members
                .len(),
            council_members_before + 1
        );
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .administrative_capacity(),
            capacity_before.saturating_add(8)
        );
        assert!(state.audit_log.iter().any(|record| {
            record.kind() == AuditKind::WardAdoption
                && record.subject().contains(&ward.id().to_string())
        }));
        validate_invariants(registry, &state);
    }

    #[test]
    fn ward_adoption_requires_a_proven_commercial_record_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player = state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        player.resources.treasury = Money::from_copper(20_000);
        player.resources.legitimacy_basis_points = WARD_ADOPTION_LEGITIMACY_REQUIREMENT;
        player.resources.reputation_quality_basis_points = WARD_ADOPTION_REPUTATION_REQUIREMENT;
        let delivered = player_contract_deliveries(&state);
        assert!(delivered < WARD_ADOPTION_DELIVERY_REQUIREMENT);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AdoptWard {
                focus: EducationFocus::Administration,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::InsufficientWardCommercialRecord {
                delivered,
                required: WARD_ADOPTION_DELIVERY_REQUIREMENT,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "failed ward adoption must not spend funds or create family records",
        );
    }

    #[test]
    fn family_education_improves_capability_and_obeys_the_annual_interval() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let student_id = state
            .dynasties
            .get(&player_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(20_000);
        let commerce_before = state
            .characters
            .get(student_id)
            .expect("student must exist")
            .capabilities
            .commerce;

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EducateFamilyMember {
                character_id: student_id,
                focus: EducationFocus::Commerce,
            },
        )
        .expect("family education must succeed");

        assert_eq!(
            state
                .characters
                .get(student_id)
                .expect("student must exist")
                .capabilities
                .commerce,
            commerce_before.saturating_add(8).min(100)
        );
        let before = state.clone();
        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EducateFamilyMember {
                character_id: student_id,
                focus: EducationFocus::Social,
            },
        );
        assert_eq!(
            result,
            Err(CommandError::FamilyEducationCooldown {
                next_education_day: FAMILY_EDUCATION_INTERVAL_DAYS,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a premature second education must not spend funds or change capabilities",
        );
        validate_invariants(registry, &state);
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
    fn exploitation_rejects_treasury_overflow_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(i64::MAX);
        let crisis_id = state.next_ids.crisis();
        let severity = 4_000;
        state.crises.insert(
            crisis_id,
            crate::core::Crisis {
                id: crisis_id,
                kind: crate::core::CrisisKind::NobleDemand,
                district_id: None,
                started_day: state.clock.day(),
                severity_basis_points: severity,
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
            Err(CommandError::Strategic(
                StrategicError::DynastyTreasuryOverflow {
                    dynasty_id: state.player_dynasty_id,
                    current: Money::from_copper(i64::MAX),
                    incoming: Money::from_copper(i64::from(severity)),
                }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing exploitation must not consume legitimacy or intensify the crisis",
        );
    }

    #[test]
    fn crisis_accepts_only_one_strategic_player_response() {
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
            Err(CommandError::CrisisAlreadyAddressed { crisis_id })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a second response must not spend funds or change crisis severity",
        );
    }

    #[test]
    fn crisis_can_be_contained_after_one_exploitation() {
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
                response: CrisisResponse::Exploit,
            },
        )
        .expect("one exploitation must succeed");
        assert_eq!(
            state
                .crises
                .get(&crisis_id)
                .expect("crisis must exist")
                .severity_basis_points,
            8_500
        );

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::RespondToCrisis {
                crisis_id,
                response: CrisisResponse::Reform,
            },
        )
        .expect("an exploited crisis must still permit later containment");
        assert_eq!(
            state
                .crises
                .get(&crisis_id)
                .expect("crisis must exist")
                .severity_basis_points,
            6_700
        );

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
            Err(CommandError::CrisisAlreadyAddressed { crisis_id })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a containment response must still close further crisis actions",
        );
    }

    #[test]
    fn crisis_rejects_repeated_exploitation() {
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
                severity_basis_points: 4_000,
                status: CrisisStatus::Active,
                cause: "test crisis".to_owned(),
            },
        );
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::RespondToCrisis {
                crisis_id,
                response: CrisisResponse::Exploit,
            },
        )
        .expect("first exploitation must succeed");
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
            Err(CommandError::CrisisAlreadyAddressed { crisis_id })
        );
        assert_state_unchanged(&before, &state, "a crisis must not be exploited repeatedly");
    }
}

mod information {
    use super::*;
    use crate::core::{RelationshipState, SupplyContract};

    fn set_clock_day_for_test(state: AppState, day: i64) -> AppState {
        let mut value = serde_json::to_value(state).expect("test state must serialize");
        value["clock"]["day"] = serde_json::Value::from(day);
        serde_json::from_value(value).expect("test state with adjusted clock must deserialize")
    }

    struct MarketLeverageFixture {
        contract: SupplyContract,
        buyer_owner: DynastyId,
        pair: DynastyPair,
        relationship_before: RelationshipState,
    }

    fn market_leverage_fixture(state: &AppState) -> MarketLeverageFixture {
        let player_id = state.player_dynasty_id;
        let contract = state
            .contracts
            .values()
            .find(|contract| {
                contract.status == ContractStatus::Active
                    && state
                        .businesses
                        .get(contract.buyer_business_id)
                        .is_some_and(|business| business.owner_dynasty_id() == player_id)
                        != state
                            .businesses
                            .get(contract.seller_business_id)
                            .is_some_and(|business| business.owner_dynasty_id() == player_id)
            })
            .expect("campaign must contain an active player contract")
            .clone();
        let buyer_owner = state
            .businesses
            .get(contract.buyer_business_id)
            .expect("contract buyer must exist")
            .owner_dynasty_id();
        let seller_owner = state
            .businesses
            .get(contract.seller_business_id)
            .expect("contract seller must exist")
            .owner_dynasty_id();
        let counterparty_id = if buyer_owner == player_id {
            seller_owner
        } else {
            buyer_owner
        };
        let pair = DynastyPair::new(player_id, counterparty_id);
        let relationship_before = state
            .relationships
            .get(&pair)
            .expect("contract parties must have a relationship")
            .clone();
        MarketLeverageFixture {
            contract,
            buyer_owner,
            pair,
            relationship_before,
        }
    }

    fn commissioned_report_id(state: &AppState) -> InformationReportId {
        state
            .information_reports
            .values()
            .find(|report| report.source == COMMISSIONED_INFORMATION_SOURCE)
            .expect("commission must create a report")
            .id()
    }

    #[test]
    fn commissions_confirmed_market_intelligence_with_durable_feedback() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let good_id = registry
            .goods()
            .first()
            .expect("registry must contain a good")
            .id();
        let treasury_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();
        let reports_before = state.information_reports.len();
        let outbox_before = state.outbox.len();

        let outcome = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::Market { good_id },
            },
        )
        .expect("funded intelligence commission must succeed");

        assert_eq!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury(),
            treasury_before.saturating_sub(INFORMATION_COMMISSION_COST)
        );
        assert_eq!(state.information_reports.len(), reports_before + 1);
        let report = state
            .information_reports
            .values()
            .find(|report| report.source == COMMISSIONED_INFORMATION_SOURCE)
            .expect("commission must create a report");
        assert_eq!(report.confidence, InformationConfidence::Confirmed);
        assert!(report.subject.contains("market brief"));
        assert!(report.summary.contains("Price"));
        assert_eq!(state.outbox.len(), outbox_before + 1);
        assert_eq!(
            state.outbox.last().map(crate::core::OutboxMessage::kind),
            Some(OutboxKind::Information)
        );
        assert!(outcome.summary.contains("Commissioned intelligence report"));
        validate_invariants(registry, &state);
    }

    #[test]
    fn commission_rejects_unrepresentable_expiry_without_mutation() {
        let registry = rivergate_registry_for_test();
        let day = i64::MAX - INFORMATION_REPORT_LIFETIME_DAYS;
        let mut state = set_clock_day_for_test(make_test_campaign(), day);
        let good_id = registry
            .goods()
            .first()
            .expect("registry must contain a good")
            .id();
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::Market { good_id },
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Timeline(TimelineError::FutureDayOutOfRange {
                base_day: day,
                offset_days: INFORMATION_REPORT_LIFETIME_DAYS,
            }))
        );
        assert_state_unchanged(
            &before,
            &state,
            "unrepresentable report expiry must be rejected before any command mutation",
        );
    }

    #[test]
    fn intelligence_commission_cooldown_rejects_without_mutation() {
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
                focus: InformationFocus::Market { good_id },
            },
        )
        .expect("first commission must succeed");
        let district_id = registry
            .districts()
            .first()
            .expect("registry must contain a district")
            .id();
        let next_commission_day = state
            .clock
            .day()
            .saturating_add(INFORMATION_COMMISSION_INTERVAL_DAYS);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::District { district_id },
            },
        );

        assert_eq!(
            result,
            Err(CommandError::InformationCommissionCooldown {
                next_commission_day,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "cooldown rejection must not charge the dynasty or replace its report",
        );
    }

    #[test]
    fn counterparty_intelligence_rejects_the_player_dynasty_atomically() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_dynasty_id = state.player_dynasty_id;
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::Counterparty {
                    dynasty_id: player_dynasty_id,
                },
            },
        );

        assert_eq!(result, Err(CommandError::InformationCannotTargetPlayer));
        assert_state_unchanged(
            &before,
            &state,
            "invalid intelligence targets must not charge or mutate campaign state",
        );
    }

    #[test]
    fn market_intelligence_renegotiates_a_player_contract_with_relationship_costs() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let fixture = market_leverage_fixture(&state);
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::Market {
                    good_id: fixture.contract.good_id,
                },
            },
        )
        .expect("market commission must succeed");
        let report_id = commissioned_report_id(&state);
        let report = state
            .information_reports
            .get_mut(&report_id)
            .expect("commissioned report must exist");
        assert_eq!(
            report.target,
            Some(crate::core::InformationTarget::Market {
                good_id: fixture.contract.good_id,
            })
        );
        report.subject = "Presentation text changed after commissioning".to_owned();
        let treasury_before = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .treasury();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::LeverageInformation { report_id },
        )
        .expect("active market intelligence must be leverageable");

        let expected_price = if fixture.buyer_owner == player_id {
            let discounted = fixture
                .contract
                .unit_price
                .checked_mul_ratio(95, 100)
                .expect("test price must fit");
            let one_copper_less = fixture
                .contract
                .unit_price
                .checked_sub(Money::from_copper(1))
                .expect("test contract price must exceed one copper");
            discounted.min(one_copper_less).max(Money::from_copper(1))
        } else {
            let increased = fixture
                .contract
                .unit_price
                .checked_mul_ratio(105, 100)
                .expect("test price must fit");
            let one_copper_more = fixture
                .contract
                .unit_price
                .checked_add(Money::from_copper(1))
                .expect("test price must have headroom");
            increased.max(one_copper_more)
        };
        assert_eq!(
            state
                .contracts
                .get(&fixture.contract.id)
                .expect("contract must remain active")
                .unit_price,
            expected_price
        );
        assert!(!state.information_reports.contains_key(&report_id));
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .treasury(),
            treasury_before.saturating_sub(INFORMATION_LEVERAGE_COST)
        );
        let relationship = state
            .relationships
            .get(&fixture.pair)
            .expect("contract relationship must remain present");
        assert!(relationship.trust_basis_points < fixture.relationship_before.trust_basis_points);
        assert!(
            relationship.respect_basis_points > fixture.relationship_before.respect_basis_points
        );
        assert!(
            relationship.resentment_basis_points
                > fixture.relationship_before.resentment_basis_points
        );
        assert!(state.audit_log.iter().any(|record| {
            record.kind() == AuditKind::InformationLeverage
                && record.subject() == format!("information-report:{report_id}")
        }));
        validate_invariants(registry, &state);
    }

    #[test]
    fn market_intelligence_rejects_a_renegotiation_that_would_overflow_the_weekly_invoice() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let contract = state
            .contracts
            .values()
            .find(|contract| {
                let buyer_owner = state
                    .businesses
                    .get(contract.buyer_business_id)
                    .map(crate::core::Business::owner_dynasty_id);
                let seller_owner = state
                    .businesses
                    .get(contract.seller_business_id)
                    .map(crate::core::Business::owner_dynasty_id);
                contract.status == ContractStatus::Active
                    && buyer_owner == Some(player_id)
                    && seller_owner.is_some_and(|owner| owner != player_id)
            })
            .expect("campaign must contain a player-buyer contract")
            .clone();
        let counterparty_id = state
            .businesses
            .get(contract.seller_business_id)
            .expect("contract seller must exist")
            .owner_dynasty_id();
        let counterparty_manager_id = state
            .dynasties
            .get(&counterparty_id)
            .expect("counterparty dynasty must exist")
            .head_id();
        let player_manager_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        state.businesses.transfer_ownership(
            contract.buyer_business_id,
            counterparty_id,
            counterparty_manager_id,
        );
        state.businesses.transfer_ownership(
            contract.seller_business_id,
            player_id,
            player_manager_id,
        );
        {
            let contract = state
                .contracts
                .get_mut(&contract.id)
                .expect("selected contract must exist");
            contract.quantity_per_week = Quantity::from_units(2);
            contract.unit_price = Money::from_copper(i64::MAX / 2);
            assert!(
                crate::money::checked_cost_for(contract.quantity_per_week, contract.unit_price)
                    .is_some(),
                "arranged contract must begin with a representable weekly invoice"
            );
        }
        let contract = state
            .contracts
            .get(&contract.id)
            .expect("selected contract must remain present");

        assert_eq!(
            market_contract_leverage_terms(&state, player_id, contract),
            None
        );
    }

    #[test]
    fn market_intelligence_skips_an_unactionable_contract_for_a_later_viable_one() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let fixture = market_leverage_fixture(&state);
        let first_id = fixture.contract.id;
        let good_id = fixture.contract.good_id;
        let unrelated_matching_ids: Vec<_> = state
            .contracts
            .values()
            .filter(|contract| {
                contract.id != first_id
                    && contract.status == ContractStatus::Active
                    && contract.good_id == good_id
                    && state
                        .businesses
                        .get(contract.buyer_business_id)
                        .is_some_and(|business| business.owner_dynasty_id() == player_id)
                        != state
                            .businesses
                            .get(contract.seller_business_id)
                            .is_some_and(|business| business.owner_dynasty_id() == player_id)
            })
            .map(|contract| contract.id)
            .collect();
        for contract_id in unrelated_matching_ids {
            state.contracts.remove(&contract_id);
        }
        let second_id = state.next_ids.contract();
        let mut second_contract = fixture.contract.clone();
        second_contract.id = second_id;
        second_contract.unit_price = Money::from_copper(100);
        state.contracts.insert(second_id, second_contract);
        let first_contract = state
            .contracts
            .get_mut(&first_id)
            .expect("fixture contract must remain present");
        if fixture.buyer_owner == player_id {
            first_contract.unit_price = Money::from_copper(1);
        } else {
            first_contract.quantity_per_week = Quantity::from_units(2);
            first_contract.unit_price = Money::from_copper(i64::MAX / 2);
        }
        let report_id = InformationReportId::new(u32::MAX);

        let plan = resolve_market_information_leverage(registry, &state, report_id, good_id)
            .expect("a later actionable contract must remain leverageable");

        let InformationLeverageEffect::Contract { contract_id, .. } = plan.effect else {
            panic!("market intelligence must resolve to a contract effect");
        };
        assert_eq!(contract_id, second_id);
    }

    #[test]
    fn counterparty_intelligence_improves_targeted_relationships() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let counterparty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != player_id
                    && state
                        .relationships
                        .contains_key(&DynastyPair::new(player_id, *dynasty_id))
            })
            .expect("campaign must contain a known counterparty");
        let pair = DynastyPair::new(player_id, counterparty_id);
        let before = state
            .relationships
            .get(&pair)
            .expect("relationship must exist")
            .clone();
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::Counterparty {
                    dynasty_id: counterparty_id,
                },
            },
        )
        .expect("counterparty commission must succeed");
        let report_id = state
            .information_reports
            .values()
            .find(|report| report.source == COMMISSIONED_INFORMATION_SOURCE)
            .expect("commission must create a report")
            .id();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::LeverageInformation { report_id },
        )
        .expect("counterparty intelligence must support targeted outreach");

        let relationship = state
            .relationships
            .get(&pair)
            .expect("relationship must remain present");
        assert!(relationship.trust_basis_points > before.trust_basis_points);
        assert!(relationship.respect_basis_points > before.respect_basis_points);
        assert!(relationship.resentment_basis_points < before.resentment_basis_points);
        validate_invariants(registry, &state);
    }

    #[test]
    fn district_intelligence_funds_a_targeted_material_initiative() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let district_id = registry
            .districts()
            .first()
            .expect("registry must contain a district")
            .id();
        let district = state
            .districts
            .get_mut(&district_id)
            .expect("district must exist");
        district.employment_basis_points = 3_000;
        district.sanitation_basis_points = 6_000;
        district.safety_basis_points = 7_000;
        district.unrest_basis_points = 5_000;
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::District { district_id },
            },
        )
        .expect("district commission must succeed");
        let report_id = state
            .information_reports
            .values()
            .find(|report| report.source == COMMISSIONED_INFORMATION_SOURCE)
            .expect("commission must create a report")
            .id();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::LeverageInformation { report_id },
        )
        .expect("district intelligence must support a targeted initiative");

        let district = state
            .districts
            .get(&district_id)
            .expect("district must exist");
        assert_eq!(district.employment_basis_points, 3_250);
        assert_eq!(district.sanitation_basis_points, 6_000);
        assert_eq!(district.safety_basis_points, 7_000);
        assert_eq!(district.unrest_basis_points, 4_900);
        validate_invariants(registry, &state);
    }

    #[test]
    fn expired_intelligence_cannot_be_leveraged_and_is_atomic() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let district_id = registry
            .districts()
            .first()
            .expect("registry must contain a district")
            .id();
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::District { district_id },
            },
        )
        .expect("district commission must succeed");
        let report_id = state
            .information_reports
            .values()
            .find(|report| report.source == COMMISSIONED_INFORMATION_SOURCE)
            .expect("commission must create a report")
            .id();
        state
            .information_reports
            .get_mut(&report_id)
            .expect("report must exist")
            .expires_day = state.clock.day().saturating_sub(1);
        let expired_day = state.clock.day().saturating_sub(1);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::LeverageInformation { report_id },
        );

        assert_eq!(
            result,
            Err(CommandError::InformationReportExpired {
                report_id,
                expired_day,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "expired intelligence must not spend funds or mutate its target",
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

    fn delinquent_player_loan(state: &mut AppState) -> (DynastyId, crate::ids::LoanId) {
        let player_id = state.player_dynasty_id;
        let borrower_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != player_id
                    && !state.loans.values().any(|loan| {
                        loan.lender_dynasty_id == player_id
                            && loan.borrower_dynasty_id == *dynasty_id
                            && loan.status != LoanStatus::Repaid
                    })
            })
            .expect("campaign must contain a rival available for player lending");
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        let loan_id = crate::systems::issue_loan(
            state,
            LoanTerms {
                lender_dynasty_id: player_id,
                borrower_dynasty_id: borrower_id,
                principal: Money::from_copper(5_000),
                weekly_payment: Money::from_copper(300),
                interest_basis_points: 1_000,
                collateral_property_id: None,
            },
        )
        .expect("fixture player loan must be issuable");
        let loan = state
            .loans
            .get_mut(&loan_id)
            .expect("fixture loan must exist");
        loan.status = LoanStatus::Delinquent;
        loan.missed_payments = 1;
        (borrower_id, loan_id)
    }

    #[test]
    fn rejects_rapid_repeat_filing_without_charging_cost() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let prior_defendant = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain a nonplayer dynasty");
        let prior_id = state.next_ids.legal_case();
        state.legal_cases.insert(
            prior_id,
            LegalCase {
                id: prior_id,
                plaintiff_dynasty_id: state.player_dynasty_id,
                defendant_dynasty_id: prior_defendant,
                kind: LegalCaseKind::Fraud,
                claim_source: None,
                evidence_basis_points: 6_000,
                public_attention_basis_points: 1_500,
                filed_day: state.clock.day(),
                hearing_day: state.clock.day().saturating_add(60),
                damages: Money::from_copper(2_000),
                status: LegalCaseStatus::Filed,
            },
        );
        let defendant_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != state.player_dynasty_id && *dynasty_id != prior_defendant
            })
            .expect("campaign must contain another nonplayer dynasty");
        let next_filing_day = state
            .clock
            .day()
            .saturating_add(LEGAL_CASE_FILING_INTERVAL_DAYS);
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id,
                kind: LegalCaseKind::Fraud,
                evidence_basis_points: 7_000,
                damages: Money::from_copper(4_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::LegalCaseCooldown { next_filing_day })
        );
        assert_state_unchanged(
            &before,
            &state,
            "legal filing cooldowns must fail before charging the filing cost",
        );
    }

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
                claim_source: None,
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

    #[test]
    fn rejects_unsubstantiated_legal_claim_without_charging_cost() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let defendant_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain a nonplayer dynasty");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id,
                kind: LegalCaseKind::Fraud,
                evidence_basis_points: 7_000,
                damages: Money::from_copper(2_000),
            },
        );

        assert_eq!(
            result,
            Err(CommandError::LegalClaimNotGrounded {
                defendant_dynasty_id,
                kind: LegalCaseKind::Fraud,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "player commands must not manufacture unsupported legal claims",
        );
    }

    #[test]
    fn debt_case_is_grounded_in_exact_delinquent_loan() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (borrower_id, loan_id) = delinquent_player_loan(&mut state);
        let quote = quote_player_legal_claim(&state, borrower_id, LegalCaseKind::Debt)
            .expect("delinquent player credit must support a debt claim");

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id: borrower_id,
                kind: LegalCaseKind::Debt,
                evidence_basis_points: quote.evidence_basis_points,
                damages: quote.maximum_damages,
            },
        )
        .expect("grounded debt claim must be fileable");

        let legal_case = state
            .legal_cases
            .values()
            .find(|legal_case| legal_case.plaintiff_dynasty_id == state.player_dynasty_id)
            .expect("filed player case must exist");
        assert_eq!(
            legal_case.claim_source,
            Some(LegalClaimSource::Loan { loan_id })
        );
        assert_eq!(legal_case.damages, quote.maximum_damages);
    }

    #[test]
    fn rejects_legal_evidence_above_grounded_claim_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (borrower_id, _) = delinquent_player_loan(&mut state);
        let quote = quote_player_legal_claim(&state, borrower_id, LegalCaseKind::Debt)
            .expect("delinquent player credit must support a debt claim");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id: borrower_id,
                kind: LegalCaseKind::Debt,
                evidence_basis_points: quote.evidence_basis_points.saturating_add(1),
                damages: quote.maximum_damages,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::LegalEvidenceExceedsClaim {
                evidence_basis_points: quote.evidence_basis_points + 1,
                maximum_basis_points: quote.evidence_basis_points,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "unsupported legal evidence must fail before filing cost or relationship mutation",
        );
    }

    #[test]
    fn rejects_legal_damages_above_grounded_claim_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (borrower_id, _) = delinquent_player_loan(&mut state);
        let quote = quote_player_legal_claim(&state, borrower_id, LegalCaseKind::Debt)
            .expect("delinquent player credit must support a debt claim");
        let requested_damages = quote
            .maximum_damages
            .checked_add(Money::from_copper(1))
            .expect("fixture damages must fit money");
        let before = state.clone();

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id: borrower_id,
                kind: LegalCaseKind::Debt,
                evidence_basis_points: quote.evidence_basis_points,
                damages: requested_damages,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::LegalDamagesExceedClaim {
                damages: requested_damages,
                maximum_damages: quote.maximum_damages,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "unsupported damages must fail before filing cost or relationship mutation",
        );
    }
}

mod labor {
    use super::*;

    fn disputed_player_employment(state: &mut AppState) -> (EmploymentId, BusinessId) {
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
        let agreement = state
            .employment
            .get_mut(&employment_id)
            .expect("selected employment must exist");
        agreement.status = EmploymentStatus::Disputed;
        agreement.loyalty_basis_points = 0;
        agreement.conditions_basis_points = 0;
        (employment_id, agreement.business_id)
    }

    #[test]
    fn condition_investment_restores_both_recovery_requirements() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (employment_id, business_id) = disputed_player_employment(&mut state);
        state
            .businesses
            .get_mut(business_id)
            .expect("employment business must exist")
            .finance
            .cash = Money::from_copper(2_000);

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ResolveLaborDispute {
                employment_id,
                response: LaborResponse::ImproveConditions,
            },
        )
        .expect("condition investment must resolve the dispute");

        let agreement = state
            .employment
            .get(&employment_id)
            .expect("employment must remain present");
        assert_eq!(agreement.status, EmploymentStatus::Active);
        assert!(agreement.loyalty_basis_points >= crate::systems::EMPLOYMENT_RECOVERY_BASIS_POINTS);
        assert!(
            agreement.conditions_basis_points >= crate::systems::EMPLOYMENT_RECOVERY_BASIS_POINTS
        );
    }

    #[test]
    fn negotiation_restores_conditions_before_reactivating_employment() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (employment_id, business_id) = disputed_player_employment(&mut state);
        state
            .businesses
            .get_mut(business_id)
            .expect("employment business must exist")
            .finance
            .cash = Money::from_copper(2_000);
        let wage_before = state
            .employment
            .get(&employment_id)
            .expect("employment must exist")
            .weekly_wage;

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ResolveLaborDispute {
                employment_id,
                response: LaborResponse::Negotiate,
            },
        )
        .expect("negotiation must resolve the dispute");

        let agreement = state
            .employment
            .get(&employment_id)
            .expect("employment must remain present");
        assert_eq!(agreement.status, EmploymentStatus::Active);
        assert_eq!(
            agreement.weekly_wage,
            wage_before.saturating_mul_ratio(11, 10)
        );
        assert!(agreement.loyalty_basis_points >= 4_500);
        assert!(
            agreement.conditions_basis_points >= crate::systems::EMPLOYMENT_RECOVERY_BASIS_POINTS
        );
    }

    #[test]
    fn negotiation_rejects_wage_overflow_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (employment_id, business_id) = disputed_player_employment(&mut state);
        state
            .businesses
            .get_mut(business_id)
            .expect("employment business must exist")
            .finance
            .cash = Money::from_copper(2_000);
        state
            .employment
            .get_mut(&employment_id)
            .expect("employment must exist")
            .weekly_wage = Money::from_copper(i64::MAX);
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
            Err(CommandError::LaborWageOverflow {
                employment_id,
                current: Money::from_copper(i64::MAX),
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing wage negotiations must fail before charging the business or changing employment",
        );
    }

    #[test]
    fn condition_investment_rejects_lifetime_cost_overflow_without_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let (employment_id, business_id) = disputed_player_employment(&mut state);
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("employment business must exist");
        business.finance.cash = Money::from_copper(2_000);
        business.finance.lifetime_costs = Money::from_copper(i64::MAX);
        let before = state.clone();
        let incoming = Money::from_copper(1_000);

        let result = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ResolveLaborDispute {
                employment_id,
                response: LaborResponse::ImproveConditions,
            },
        );

        assert_eq!(
            result,
            Err(CommandError::Simulation(
                crate::systems::SimulationError::BusinessLifetimeCostsOverflow {
                    business_id,
                    current: Money::from_copper(i64::MAX),
                    incoming,
                }
            ))
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing lifetime costs must fail before cash or employment mutation",
        );
    }

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

    #[test]
    fn replacement_pays_recruitment_cost_and_resets_the_agreement() {
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
        let (business_id, original_household_id) = {
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("employment must exist");
            agreement.status = EmploymentStatus::Disputed;
            (agreement.business_id, agreement.household_id)
        };
        let cash_before = Money::from_copper(2_000);
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .finance
            .cash = cash_before;

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::ResolveLaborDispute {
                employment_id,
                response: LaborResponse::ReplaceWorkers,
            },
        )
        .expect("replacement must succeed");

        let agreement = state
            .employment
            .get(&employment_id)
            .expect("employment must exist");
        assert_ne!(agreement.household_id, original_household_id);
        assert_eq!(agreement.status, EmploymentStatus::Active);
        assert_eq!(agreement.loyalty_basis_points, 6_000);
        assert_eq!(agreement.conditions_basis_points, 6_000);
        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .cash(),
            cash_before.saturating_sub(LABOR_REPLACEMENT_COST)
        );
    }
}

mod serialization {
    use super::*;
    use std::collections::BTreeSet;

    const COMMAND_KINDS: [&str; 25] = [
        "acquire-business",
        "acknowledge-notification",
        "adopt-ward",
        "buy-property",
        "commission-information",
        "convene-family-council",
        "cultivate-institution-support",
        "designate-heir",
        "sell-property",
        "create-supply-contract",
        "educate-family-member",
        "enact-law",
        "exercise-office-power",
        "file-legal-case",
        "issue-loan",
        "invest-in-business",
        "leverage-information",
        "nominate-for-office",
        "resolve-labor-dispute",
        "respond-to-crisis",
        "set-business-policy",
        "set-house-governance",
        "start-public-work",
        "transfer-business-cash",
        "withdraw-from-institution",
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
            PlayerCommand::SellProperty { .. } => "sell-property",
            PlayerCommand::EnactLaw { .. } => "enact-law",
            PlayerCommand::StartPublicWork { .. } => "start-public-work",
            PlayerCommand::FileLegalCase { .. } => "file-legal-case",
            PlayerCommand::SetHouseGovernance { .. } => "set-house-governance",
            PlayerCommand::ConveneFamilyCouncil => "convene-family-council",
            PlayerCommand::DesignateHeir { .. } => "designate-heir",
            PlayerCommand::AdoptWard { .. } => "adopt-ward",
            PlayerCommand::EducateFamilyMember { .. } => "educate-family-member",
            PlayerCommand::CultivateInstitutionSupport { .. } => "cultivate-institution-support",
            PlayerCommand::NominateForOffice { .. } => "nominate-for-office",
            PlayerCommand::ExerciseOfficePower { .. } => "exercise-office-power",
            PlayerCommand::WithdrawFromInstitution { .. } => "withdraw-from-institution",
            PlayerCommand::RespondToCrisis { .. } => "respond-to-crisis",
            PlayerCommand::ResolveLaborDispute { .. } => "resolve-labor-dispute",
            PlayerCommand::CommissionInformation { .. } => "commission-information",
            PlayerCommand::LeverageInformation { .. } => "leverage-information",
            PlayerCommand::AcknowledgeNotification { .. } => "acknowledge-notification",
        }
    }

    fn representative_information_command() -> PlayerCommand {
        PlayerCommand::CommissionInformation {
            focus: InformationFocus::Market {
                good_id: GoodId::new(3),
            },
        }
    }

    fn representative_institution_support_command() -> PlayerCommand {
        PlayerCommand::CultivateInstitutionSupport {
            institution_id: InstitutionId::new(1),
            character_id: CharacterId::new(2),
        }
    }

    fn representative_heir_designation_command() -> PlayerCommand {
        PlayerCommand::DesignateHeir {
            character_id: CharacterId::new(2),
        }
    }

    fn representative_office_power_command() -> PlayerCommand {
        PlayerCommand::ExerciseOfficePower {
            institution_id: InstitutionId::new(1),
            power: OfficePower::WatchPriorities,
        }
    }

    fn representative_economic_commands() -> Vec<PlayerCommand> {
        vec![
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
            PlayerCommand::SellProperty {
                property_id: PropertyId::new(1),
                buyer_dynasty_id: DynastyId::new(4),
            },
        ]
    }

    fn representative_civic_family_commands() -> Vec<PlayerCommand> {
        vec![
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
            PlayerCommand::ConveneFamilyCouncil,
            representative_heir_designation_command(),
            PlayerCommand::AdoptWard {
                focus: EducationFocus::Administration,
            },
            PlayerCommand::EducateFamilyMember {
                character_id: CharacterId::new(2),
                focus: EducationFocus::Commerce,
            },
            representative_institution_support_command(),
            PlayerCommand::NominateForOffice {
                institution_id: InstitutionId::new(1),
                character_id: CharacterId::new(2),
            },
            representative_office_power_command(),
            PlayerCommand::WithdrawFromInstitution {
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
            representative_information_command(),
            PlayerCommand::LeverageInformation {
                report_id: InformationReportId::new(1),
            },
            PlayerCommand::AcknowledgeNotification {
                message_id: OutboxMessageId::new(1),
            },
        ]
    }

    fn representative_commands() -> Vec<PlayerCommand> {
        let mut commands = representative_economic_commands();
        commands.extend(representative_civic_family_commands());
        commands
    }

    #[test]
    fn every_variant_round_trips_through_json() {
        let commands = representative_commands();

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
