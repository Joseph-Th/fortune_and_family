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

fn grant_office_nomination_record_for_test(state: &mut AppState) {
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
    let deliveries = u16::try_from(OFFICE_NOMINATION_DELIVERY_REQUIREMENT)
        .expect("office delivery requirement must fit contract counters");
    contract.fulfilled_deliveries = deliveries;
    contract
        .fulfilled_deliveries_by_dynasty
        .insert(state.player_dynasty_id, deliveries);
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
                subject: institution_support_subject(institution_id, *character_id),
                detail: "test support".to_owned(),
            });
        }
    }
    state.audit_log.sort_by_key(AuditRecord::day);
}

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
        let deliveries = u16::try_from(OFFICE_NOMINATION_DELIVERY_REQUIREMENT)
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
            current_day.saturating_add(60),
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
                required: OFFICE_NOMINATION_DELIVERY_REQUIREMENT,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "an unproven commercial record must not spend money or alter membership",
        );
    }

    #[test]
    fn office_nomination_has_a_dynasty_wide_cooldown() {
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
            "a rapid second campaign must not spend money or alter membership",
        );
    }

    #[test]
    fn existing_institution_member_can_campaign_again_after_the_cooldown() {
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
        let treasury_before = player.treasury();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::NominateForOffice {
                institution_id,
                character_id,
            },
        )
        .expect("an existing member must be allowed to fund a later campaign");

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
    fn governance_cannot_be_rewritten_twice_within_five_years() {
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
}

mod information {
    use super::*;

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
    fn rejects_rapid_repeat_filing_without_charging_cost() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let prior = state
            .legal_cases
            .values()
            .find(|legal_case| legal_case.plaintiff_dynasty_id == state.player_dynasty_id)
            .expect("campaign must contain a player-filed opening case");
        let defendant_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != state.player_dynasty_id && *dynasty_id != prior.defendant_dynasty_id
            })
            .expect("campaign must contain another nonplayer dynasty");
        let next_filing_day = prior
            .filed_day
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

    const COMMAND_KINDS: [&str; 21] = [
        "acquire-business",
        "acknowledge-notification",
        "adopt-ward",
        "buy-property",
        "commission-information",
        "cultivate-institution-support",
        "sell-property",
        "create-supply-contract",
        "educate-family-member",
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
            PlayerCommand::AdoptWard { .. } => "adopt-ward",
            PlayerCommand::EducateFamilyMember { .. } => "educate-family-member",
            PlayerCommand::CultivateInstitutionSupport { .. } => "cultivate-institution-support",
            PlayerCommand::NominateForOffice { .. } => "nominate-for-office",
            PlayerCommand::WithdrawFromInstitution { .. } => "withdraw-from-institution",
            PlayerCommand::RespondToCrisis { .. } => "respond-to-crisis",
            PlayerCommand::ResolveLaborDispute { .. } => "resolve-labor-dispute",
            PlayerCommand::CommissionInformation { .. } => "commission-information",
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

    fn representative_commands() -> Vec<PlayerCommand> {
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
            PlayerCommand::AcknowledgeNotification {
                message_id: OutboxMessageId::new(1),
            },
        ]
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
