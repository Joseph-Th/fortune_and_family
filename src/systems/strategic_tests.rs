//! Strategic economy, civic-system, crisis, and long-horizon behavior tests.

use super::*;
use crate::systems::{advance_days, validate_invariants};
use crate::test_support::{
    assert_state_unchanged, make_test_campaign, rivergate_registry_for_test as test_registry,
};

fn make_test_contract_terms(state: &AppState) -> SupplyContractTerms {
    let contract = state
        .contracts
        .values()
        .find(|contract| contract.status == ContractStatus::Active)
        .expect("bootstrap must create an active supply contract");
    SupplyContractTerms {
        buyer_business_id: contract.buyer_business_id,
        seller_business_id: contract.seller_business_id,
        good_id: contract.good_id,
        quantity_per_week: contract.quantity_per_week,
        unit_price: contract.unit_price,
        penalty: contract.penalty,
        duration_weeks: 4,
    }
}

fn make_test_loan_terms(state: &AppState) -> LoanTerms {
    for lender in state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.treasury() >= Money::from_copper(1))
    {
        for borrower in state
            .dynasties
            .values()
            .filter(|dynasty| dynasty.id() != lender.id())
        {
            let has_unsettled_pair = state.loans.values().any(|loan| {
                loan.lender_dynasty_id == lender.id()
                    && loan.borrower_dynasty_id == borrower.id()
                    && loan.status != LoanStatus::Repaid
            });
            if !has_unsettled_pair {
                return LoanTerms {
                    lender_dynasty_id: lender.id(),
                    borrower_dynasty_id: borrower.id(),
                    principal: Money::from_copper(1),
                    weekly_payment: Money::from_copper(1),
                    interest_basis_points: 500,
                    collateral_property_id: None,
                };
            }
        }
    }
    panic!("test campaign must contain a dynasty pair without unsettled credit");
}

fn active_contract_id(state: &AppState) -> crate::ids::ContractId {
    state
        .contracts
        .values()
        .find(|contract| contract.status == ContractStatus::Active)
        .expect("bootstrap must create an active contract")
        .id
}

fn current_loan_id(state: &AppState) -> crate::ids::LoanId {
    state
        .loans
        .values()
        .find(|loan| loan.status == LoanStatus::Current)
        .expect("bootstrap must create a current loan")
        .id
}

mod arithmetic_boundaries {
    use super::*;
    use crate::core::CivicDebt;

    #[test]
    fn weekly_interest_uses_the_full_supported_balance_range() {
        let balance = Money::from_copper(i64::MAX);
        let expected = Money::from_copper(i64::MAX / 52 + i64::from(i64::MAX % 52 != 0));

        assert_eq!(weekly_interest_due(balance, 10_000), expected);
    }

    #[test]
    fn loan_interest_overflow_aborts_the_requested_advance_without_mutation() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        for law in state
            .laws
            .values_mut()
            .filter(|law| law.kind == LawKind::InterestLimit)
        {
            law.active = false;
        }
        for loan in state.loans.values_mut() {
            loan.next_due_day = 1_000;
        }
        let loan_id = current_loan_id(&state);
        let balance = Money::from_copper(i64::MAX);
        let interest = weekly_interest_due(balance, 10_000);
        let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
        loan.balance = balance;
        loan.interest_basis_points = 10_000;
        loan.next_due_day = 7;
        let before = state.clone();

        let result = advance_days(registry, &mut state, 7);

        assert_eq!(
            result,
            Err(SimulationError::LoanBalanceOverflow {
                loan_id,
                current: balance,
                incoming: interest,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "unrepresentable loan interest must abort the complete requested advance",
        );
    }

    #[test]
    fn civic_debt_interest_overflow_is_rejected_before_settlement_mutates_state() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("Rivergate must define a civic treasury");
        let creditor_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain a non-player dynasty");
        let civic_debt_id = state.next_ids.civic_debt();
        let balance = Money::from_copper(i64::MAX);
        let interest = weekly_interest_due(balance, 10_000);
        state.civic_debts.insert(
            civic_debt_id,
            CivicDebt {
                id: civic_debt_id,
                creditor_dynasty_id,
                authorizing_law_id: crate::ids::LawId::new(0),
                sponsor_dynasty_id: None,
                principal: balance,
                balance,
                weekly_payment: Money::from_copper(1),
                interest_basis_points: 10_000,
                issued_day: state.clock.day(),
                next_due_day: state.clock.day(),
                missed_payments: 0,
                status: CivicDebtStatus::Current,
            },
        );
        let due = DueCivicDebt {
            id: civic_debt_id,
            creditor_dynasty_id,
            sponsor_dynasty_id: None,
            weekly_payment: Money::from_copper(1),
            balance,
            interest_basis_points: 10_000,
        };
        let before = state.clone();

        let result = settle_due_civic_debt(&mut state, treasury_id, due, None);

        assert_eq!(
            result,
            Err(SimulationError::CivicDebtBalanceOverflow {
                civic_debt_id,
                current: balance,
                incoming: interest,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "unrepresentable civic-debt interest must fail before settlement mutation",
        );
    }

    #[test]
    fn acquisition_discount_uses_the_full_supported_business_value_range() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() != state.player_dynasty_id)
            .expect("campaign must contain a non-player business")
            .id();
        let (recipe_id, capacity) = {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("selected business must exist");
            business.operations.status = BusinessStatus::Distressed;
            business.operations.condition_basis_points = 0;
            business.operations.quality_basis_points = 0;
            business.finance.cash = Money::from_copper(i64::MAX);
            business.inventory.clear();
            (
                business.recipe_id(),
                business.operations.capacity_batches_per_day,
            )
        };
        let operating_cost = registry
            .get_recipe(recipe_id)
            .expect("business recipe must exist")
            .daily_operating_cost()
            .copper();
        let equipment_value =
            i128::from(operating_cost) * i128::from(capacity) * 60 * 1_000 / 10_000;
        let expected = Money::from_copper(
            i64::try_from((i128::from(i64::MAX) + equipment_value) * 7_000 / 10_000)
                .expect("discounted maximum business value must fit money"),
        );

        let quote =
            quote_business_acquisition(registry, &state, state.player_dynasty_id, business_id)
                .expect("distressed business must be acquirable");

        assert_eq!(quote.purchase_price, expected);
    }

    #[test]
    fn acquisition_discount_uses_a_wide_aggregate_before_discounting() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let good_id = registry
            .get_good_id("grain")
            .expect("Rivergate must define grain");
        let business_id = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() != state.player_dynasty_id)
            .expect("campaign must contain a non-player business")
            .id();
        let (recipe_id, capacity) = {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("selected business must exist");
            business.operations.status = BusinessStatus::Closed;
            business.operations.condition_basis_points = 0;
            business.operations.quality_basis_points = 0;
            business.finance.cash = Money::from_copper(i64::MAX);
            business.inventory.clear();
            business.inventory.insert(good_id, Quantity::ONE);
            (
                business.recipe_id(),
                business.operations.capacity_batches_per_day,
            )
        };
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("grain quote must exist")
            .price = Money::from_copper(i64::MAX);
        let operating_cost = registry
            .get_recipe(recipe_id)
            .expect("business recipe must exist")
            .daily_operating_cost()
            .copper();
        let equipment_value =
            i128::from(operating_cost) * i128::from(capacity) * 60 * 1_000 / 10_000;
        let expected_copper = (i128::from(i64::MAX) * 2 + equipment_value) * 2_500 / 10_000;
        let expected = Money::from_copper(
            i64::try_from(expected_copper).expect("discounted wide valuation must fit money"),
        );

        let quote =
            quote_business_acquisition(registry, &state, state.player_dynasty_id, business_id)
                .expect("closed business valuation must remain representable after discounting");

        assert_eq!(quote.purchase_price, expected);
    }

    #[test]
    fn acquisition_quote_rejects_an_unrepresentable_discounted_valuation() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let good_id = registry
            .get_good_id("grain")
            .expect("Rivergate must define grain");
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
            business.operations.status = BusinessStatus::Distressed;
            business.operations.condition_basis_points = 0;
            business.operations.quality_basis_points = 0;
            business.finance.cash = Money::from_copper(i64::MAX);
            business.inventory.clear();
            business.inventory.insert(good_id, Quantity::ONE);
        }
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("grain quote must exist")
            .price = Money::from_copper(i64::MAX);

        let result =
            quote_business_acquisition(registry, &state, state.player_dynasty_id, business_id);

        assert_eq!(
            result,
            Err(StrategicError::BusinessValuationOverflow { business_id })
        );
    }

    #[test]
    fn acquisition_rejects_exhausted_business_finance_version_without_mutation() {
        let registry = test_registry();
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
        business.operations.status = BusinessStatus::Distressed;
        business.finance.cash = Money::ZERO;
        business.finance.version = u64::MAX;
        let manager_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an eligible manager");
        let quote =
            quote_business_acquisition(registry, &state, state.player_dynasty_id, business_id)
                .expect("distressed business must remain quotable");
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = quote
            .purchase_price
            .saturating_add(quote.minimum_recapitalization);
        let buyer_dynasty_id = state.player_dynasty_id;
        let before = state.clone();

        let result = acquire_business(
            registry,
            &mut state,
            buyer_dynasty_id,
            business_id,
            manager_id,
            quote.minimum_recapitalization,
        );

        assert_eq!(
            result,
            Err(StrategicError::BusinessFinanceVersionExhausted { business_id })
        );
        assert_state_unchanged(
            &before,
            &state,
            "exhausted business finance versions must fail before funds or ownership move",
        );
    }
}

mod integration {
    use super::*;

    #[test]
    fn player_bootstrap_contracts_are_provisional_while_city_contracts_stabilize_trade() {
        let state = make_test_campaign();
        let mut player_involved = 0;
        let mut ambient = 0;

        for contract in state
            .contracts
            .values()
            .filter(|contract| contract.status == ContractStatus::Active)
        {
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
            if buyer_owner == state.player_dynasty_id || seller_owner == state.player_dynasty_id {
                player_involved += 1;
                assert_eq!(contract.end_day, 26 * 7);
            } else {
                ambient += 1;
                assert_eq!(contract.end_day, 52 * 7);
            }
        }

        assert!(player_involved > 0);
        assert!(ambient > 0);
    }

    #[test]
    fn bootstrap_records_enter_the_weekly_simulation() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let deliveries_before: u32 = state
            .contracts
            .values()
            .map(|contract| {
                u32::from(contract.fulfilled_deliveries)
                    .saturating_add(u32::from(contract.missed_deliveries))
            })
            .sum();

        advance_days(registry, &mut state, 7).expect("first campaign week must advance");

        let deliveries_after: u32 = state
            .contracts
            .values()
            .map(|contract| {
                u32::from(contract.fulfilled_deliveries)
                    .saturating_add(u32::from(contract.missed_deliveries))
            })
            .sum();
        assert!(
            deliveries_after > deliveries_before,
            "bootstrap contracts must participate in weekly settlement"
        );
        let fulfilled_contracts: Vec<_> = state
            .contracts
            .values()
            .filter(|contract| contract.fulfilled_deliveries > 0)
            .collect();
        assert!(
            !fulfilled_contracts.is_empty(),
            "the bootstrap economy must complete at least one first-week delivery"
        );
        for contract in fulfilled_contracts {
            let buyer_owner_id = state
                .businesses
                .get(contract.buyer_business_id)
                .expect("contract buyer must exist")
                .owner_dynasty_id();
            let seller_owner_id = state
                .businesses
                .get(contract.seller_business_id)
                .expect("contract seller must exist")
                .owner_dynasty_id();
            assert_eq!(
                contract
                    .fulfilled_deliveries_by_dynasty
                    .get(&buyer_owner_id),
                Some(&contract.fulfilled_deliveries),
                "the buyer dynasty must receive durable credit for each completed obligation"
            );
            assert_eq!(
                contract
                    .fulfilled_deliveries_by_dynasty
                    .get(&seller_owner_id),
                Some(&contract.fulfilled_deliveries),
                "the seller dynasty must receive durable credit for each completed obligation"
            );
        }
        validate_invariants(registry, &state);
    }
}

mod public_works {
    use super::*;

    #[test]
    fn public_work_progress_matches_civic_treasury_spending() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define civic treasury");
        let work_id = state
            .public_works
            .values()
            .find(|work| work.status == PublicWorkStatus::Building)
            .expect("bootstrap must create a building public work")
            .id;
        let treasury_before = state
            .institutions
            .get(&treasury_id)
            .expect("treasury runtime must exist")
            .budget;
        let spent_before = state
            .public_works
            .get(&work_id)
            .expect("public work must exist")
            .spent;

        progress_public_works(registry, &mut state);

        let treasury_after = state
            .institutions
            .get(&treasury_id)
            .expect("treasury runtime must exist")
            .budget;
        let spent_after = state
            .public_works
            .get(&work_id)
            .expect("public work must exist")
            .spent;
        assert!(
            spent_after > spent_before,
            "a funded building project must make progress"
        );
        assert_eq!(
            treasury_before.saturating_sub(treasury_after),
            spent_after.saturating_sub(spent_before),
            "recorded project spending must equal the treasury funds consumed"
        );
        validate_invariants(registry, &state);
    }
}

mod gameplay_stability {
    use super::*;

    #[test]
    fn disputed_employment_can_recover_through_sustained_full_payroll() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let employment_id = *state
            .employment
            .keys()
            .next()
            .expect("campaign must contain employment");
        let business_id = {
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("employment must exist");
            agreement.status = EmploymentStatus::Disputed;
            agreement.loyalty_basis_points = 2_800;
            agreement.conditions_basis_points = 2_900;
            agreement.business_id
        };
        state
            .businesses
            .get_mut(business_id)
            .expect("employment business must exist")
            .finance
            .cash = Money::from_copper(100_000);

        settle_employment(registry, &mut state).expect("employment settlement must succeed");
        settle_employment(registry, &mut state).expect("employment settlement must succeed");

        assert_eq!(
            state
                .employment
                .get(&employment_id)
                .expect("employment must exist")
                .status,
            EmploymentStatus::Active,
            "reliable payroll must provide a systemic recovery path from labor disputes"
        );
    }

    #[test]
    fn saturated_business_pays_only_a_retainer_without_creating_a_dispute() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let employment_id = *state
            .employment
            .keys()
            .next()
            .expect("campaign must contain employment");
        state.employment.retain(|id, _| *id == employment_id);
        let (business_id, household_id, weekly_wage) = {
            let agreement = state
                .employment
                .get(&employment_id)
                .expect("employment must exist");
            (
                agreement.business_id,
                agreement.household_id,
                agreement.weekly_wage,
            )
        };
        let output_good_id = {
            let business = state
                .businesses
                .get(business_id)
                .expect("employment business must exist");
            registry
                .get_recipe(business.recipe_id())
                .expect("business recipe must exist")
                .output_good_id()
        };
        state
            .contracts
            .retain(|_, contract| contract.seller_business_id != business_id);
        {
            let quote = state
                .market
                .quotes
                .get_mut(&output_good_id)
                .expect("output quote must exist");
            quote.stock = quote.target_stock.saturating_mul_ratio(3, 2);
        }
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("employment business must exist");
            business
                .inventory
                .insert(output_good_id, Quantity::from_units(10_000));
            business.finance.cash = Money::from_copper(100_000);
        }
        let business_cash_before = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .cash();
        let household_cash_before = state
            .households
            .get(household_id)
            .expect("household must exist")
            .cash();

        settle_employment(registry, &mut state).expect("employment settlement must succeed");

        let retainer = Money::from_copper(weekly_wage.copper() / 4);
        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .cash(),
            business_cash_before.saturating_sub(retainer)
        );
        assert_eq!(
            state
                .households
                .get(household_id)
                .expect("household must exist")
                .cash(),
            household_cash_before.saturating_add(retainer)
        );
        assert_eq!(
            state
                .employment
                .get(&employment_id)
                .expect("employment must exist")
                .status,
            EmploymentStatus::Active,
            "market saturation must not manufacture a labor dispute"
        );
    }

    #[test]
    fn profitable_businesses_return_excess_cash_to_their_dynasty() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let owner_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .owner_dynasty_id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.operations.status = BusinessStatus::Active;
            business.finance.cash = Money::from_copper(100_000);
            business.finance.lifetime_revenue = Money::from_copper(200_000);
            business.finance.lifetime_costs = Money::from_copper(10_000);
        }
        let treasury_before = state
            .dynasties
            .get(&owner_id)
            .expect("owner dynasty must exist")
            .treasury();

        distribute_business_dividends(registry, &mut state)
            .expect("dividend distribution must succeed");

        assert!(
            state
                .dynasties
                .get(&owner_id)
                .expect("owner dynasty must exist")
                .treasury()
                > treasury_before,
            "profitable businesses must create usable dynasty income"
        );
    }

    #[test]
    fn unoccupied_owned_property_generates_external_rent() {
        let mut state = make_test_campaign();
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id.is_none()
                    && property.tenant_dynasty_id.is_none()
                    && property.occupant_business_id.is_none()
                    && property.weekly_rent > Money::ZERO
            })
            .expect("campaign must contain rentable unowned property")
            .id;
        state
            .properties
            .get_mut(&property_id)
            .expect("property must exist")
            .owner_dynasty_id = Some(state.player_dynasty_id);
        let treasury_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();

        settle_property_rents(&mut state).expect("property rent settlement must succeed");

        assert!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury()
                > treasury_before,
            "property acquisition must create a durable economic consequence"
        );
    }

    #[test]
    fn office_powers_create_holder_and_world_effects() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define treasury");
        let player_head = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        state
            .institutions
            .get_mut(&treasury_id)
            .expect("treasury institution must exist")
            .office_holder_id = Some(player_head);
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let cash_before = state
            .businesses
            .get(business_id)
            .expect("player business must exist")
            .cash();

        apply_office_power_effects(registry, &mut state).expect("office power effects must apply");

        assert!(
            state
                .businesses
                .get(business_id)
                .expect("player business must exist")
                .cash()
                > cash_before,
            "holding an office with city-contract power must affect the holder's economy"
        );
    }

    #[test]
    fn office_revenue_is_limited_by_institution_budget_headroom() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let institution_id = registry
            .get_institution_id("market_office")
            .expect("registry must define the market office");
        let holder_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let available_headroom = Money::from_copper(40);
        for institution in state.institutions.values_mut() {
            institution.office_holder_id = None;
        }
        {
            let institution = state
                .institutions
                .get_mut(&institution_id)
                .expect("market-tolls institution must exist");
            institution.office_holder_id = Some(holder_id);
            institution.budget = Money::from_copper(i64::MAX).saturating_sub(available_headroom);
        }
        let clearing_before = state.market.clearing_account;

        apply_office_power_effects(registry, &mut state).expect("office power effects must apply");

        assert_eq!(
            state
                .institutions
                .get(&institution_id)
                .expect("market-tolls institution must exist")
                .budget,
            Money::from_copper(i64::MAX)
        );
        assert_eq!(
            state.market.clearing_account,
            clearing_before.saturating_sub(available_headroom),
            "the external clearing account must fund only revenue the institution can retain"
        );
    }

    #[test]
    fn funded_office_duties_transfer_private_money_into_the_institution() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let institution_id = registry
            .get_institution_id("city_council")
            .expect("registry must define the city council");
        let player_id = state.player_dynasty_id;
        let holder_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        for institution in state.institutions.values_mut() {
            institution.office_holder_id = None;
        }
        state
            .institutions
            .get_mut(&institution_id)
            .expect("city council must exist")
            .office_holder_id = Some(holder_id);
        let power_count = i64::try_from(
            state
                .institutions
                .get(&institution_id)
                .expect("city council must exist")
                .powers
                .len(),
        )
        .expect("power count must fit i64");
        let required = OFFICE_DUTY_COST_PER_POWER.saturating_mul(power_count);
        let treasury_before = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .treasury();
        let budget_before = state
            .institutions
            .get(&institution_id)
            .expect("city council must exist")
            .budget;

        apply_office_duties(&mut state).expect("office duties must remain representable");

        let player = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist");
        assert_eq!(player.treasury(), treasury_before.saturating_sub(required));
        assert_eq!(player.civic_contributions(), required);
        assert_eq!(player.unmet_office_duties(), 0);
        assert_eq!(
            state
                .institutions
                .get(&institution_id)
                .expect("city council must exist")
                .budget,
            budget_before.saturating_add(required)
        );
    }

    #[test]
    fn office_duty_contribution_overflow_is_atomic() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let institution_id = registry
            .get_institution_id("city_council")
            .expect("registry must define the city council");
        let player_id = state.player_dynasty_id;
        let holder_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        for institution in state.institutions.values_mut() {
            institution.office_holder_id = None;
        }
        state
            .institutions
            .get_mut(&institution_id)
            .expect("city council must exist")
            .office_holder_id = Some(holder_id);
        let incoming = OFFICE_DUTY_COST_PER_POWER.saturating_mul(
            i64::try_from(
                state
                    .institutions
                    .get(&institution_id)
                    .expect("city council must exist")
                    .powers
                    .len(),
            )
            .expect("power count must fit i64"),
        );
        let player = state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist");
        player.resources.treasury = Money::from_copper(10_000);
        player.resources.civic_contributions = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = apply_office_duties(&mut state);

        assert_eq!(
            result,
            Err(SimulationError::DynastyCivicContributionsOverflow {
                dynasty_id: player_id,
                current: Money::from_copper(i64::MAX),
                incoming,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "civic contribution overflow must be rejected before any ledger moves",
        );
    }

    #[test]
    fn unfunded_office_duties_create_reputational_and_institutional_exposure() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let institution_id = registry
            .get_institution_id("city_council")
            .expect("registry must define the city council");
        let player_id = state.player_dynasty_id;
        let holder_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        for institution in state.institutions.values_mut() {
            institution.office_holder_id = None;
        }
        state
            .institutions
            .get_mut(&institution_id)
            .expect("city council must exist")
            .office_holder_id = Some(holder_id);
        let player = state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist");
        player.resources.treasury = Money::from_copper(100);
        let legitimacy_before = player.resources.legitimacy_basis_points;
        let reliability_before = player.resources.reputation_reliability_basis_points;
        let institution_legitimacy_before = state
            .institutions
            .get(&institution_id)
            .expect("city council must exist")
            .legitimacy_basis_points;

        apply_office_duties(&mut state).expect("office duties must remain representable");
        apply_office_duties(&mut state).expect("office duties must remain representable");

        let player = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist");
        assert_eq!(player.treasury(), Money::ZERO);
        assert_eq!(player.civic_contributions(), Money::from_copper(100));
        assert_eq!(player.unmet_office_duties(), 2);
        assert_eq!(
            player.resources.legitimacy_basis_points,
            legitimacy_before.saturating_sub(240)
        );
        assert_eq!(
            player.resources.reputation_reliability_basis_points,
            reliability_before.saturating_sub(160)
        );
        assert_eq!(
            state
                .institutions
                .get(&institution_id)
                .expect("city council must exist")
                .legitimacy_basis_points,
            institution_legitimacy_before.saturating_sub(200)
        );
        assert_eq!(
            state
                .audit_log
                .iter()
                .filter(|record| record.kind() == AuditKind::OfficeDutyShortfall)
                .count(),
            2
        );
        assert_eq!(
            state
                .outbox
                .iter()
                .filter(|message| message.kind == OutboxKind::Politics)
                .filter(|message| message.subject.contains("Office duty shortfall"))
                .count(),
            1,
            "repeated shortfalls inside the notification window should not spam the player"
        );
    }

    #[test]
    fn repeated_office_duty_failures_forfeit_office_and_block_immediate_return() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let institution_id = registry
            .get_institution_id("city_council")
            .expect("registry must define the city council");
        let player_id = state.player_dynasty_id;
        let holder_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        for institution in state.institutions.values_mut() {
            institution.office_holder_id = None;
        }
        state
            .institutions
            .get_mut(&institution_id)
            .expect("city council must exist")
            .office_holder_id = Some(holder_id);
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::ZERO;

        apply_office_duties(&mut state).expect("office duties must remain representable");
        apply_office_duties(&mut state).expect("office duties must remain representable");
        apply_office_duties(&mut state).expect("office duties must remain representable");

        let institution = state
            .institutions
            .get(&institution_id)
            .expect("city council must exist");
        assert_eq!(institution.office_holder_id, None);
        assert_eq!(
            institution.next_selection_day,
            state.clock.day().saturating_add(30)
        );
        assert_eq!(
            state
                .audit_log
                .iter()
                .filter(|record| record.kind() == AuditKind::OfficeDutyForfeiture)
                .count(),
            1
        );
        assert!(
            state
                .outbox
                .iter()
                .any(|message| message.subject.contains("Office forfeited"))
        );
        for _ in 0..30 {
            state.clock.advance_one_day();
        }

        resolve_institution_selections(registry, &mut state)
            .expect("replacement selection must remain representable");

        let winner_id = state
            .institutions
            .get(&institution_id)
            .expect("city council must exist")
            .office_holder_id
            .expect("a replacement must be selected");
        assert_ne!(
            state
                .characters
                .get(winner_id)
                .expect("winner must exist")
                .dynasty_id(),
            player_id,
            "a dynasty removed for unmet duties must not immediately reclaim the office"
        );
    }

    #[test]
    fn player_cannot_win_office_without_an_explicit_nomination() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let institution_id = state
            .institutions
            .values()
            .find(|institution| institution.office_holder_id.is_none())
            .expect("campaign must contain an open institution")
            .institution_id;
        let player_head_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        for character in state.characters.iter_mut() {
            character.capabilities.social = if character.id() == player_head_id {
                100
            } else {
                0
            };
        }
        for dynasty in state.dynasties.values_mut() {
            dynasty.resources.legitimacy_basis_points =
                if dynasty.id() == player_id { 10_000 } else { 0 };
        }
        let day = state.clock.day();
        state
            .institutions
            .get_mut(&institution_id)
            .expect("selected institution must exist")
            .next_selection_day = day;

        resolve_institution_selections(test_registry(), &mut state)
            .expect("office selection must remain representable");

        let winner_id = state
            .institutions
            .get(&institution_id)
            .expect("selected institution must exist")
            .office_holder_id
            .expect("an eligible non-player member must win the office");
        assert_ne!(
            state
                .characters
                .get(winner_id)
                .expect("winner must exist")
                .dynasty_id(),
            player_id,
            "membership and raw statistics must not grant passive player political power"
        );
    }

    #[test]
    fn exhausted_institution_term_number_rejects_selection_atomically() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        let day = state.clock.day();
        for institution in state.institutions.values_mut() {
            institution.next_selection_day = day.saturating_add(1);
        }
        let institution = state
            .institutions
            .get_mut(&institution_id)
            .expect("selected institution must exist");
        institution.next_selection_day = day;
        institution.term_number = u32::MAX;
        let before = state.clone();

        let result = resolve_institution_selections(registry, &mut state);

        assert_eq!(
            result,
            Err(SimulationError::InstitutionTermNumberExhausted { institution_id })
        );
        assert_state_unchanged(
            &before,
            &state,
            "term-number exhaustion must not partially select or announce an officeholder",
        );
    }
}

mod contracts {
    use super::*;

    #[test]
    fn rejects_registry_mismatch_without_mutation() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let terms = make_test_contract_terms(&state);
        state.scenario_key = "another-scenario".to_owned();
        let before = state.clone();

        let result = sign_supply_contract(registry, &mut state, terms);

        assert_eq!(
            result,
            Err(StrategicError::RegistryMismatch {
                state_scenario: "another-scenario".to_owned(),
                registry_scenario: "rivergate".to_owned(),
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "registry mismatch must be rejected before strategic validation or mutation",
        );
    }

    #[test]
    fn signing_a_contract_creates_relationship_memory_and_counterparty_intelligence() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let terms = make_test_contract_terms(&state);
        let buyer_owner = state
            .businesses
            .get(terms.buyer_business_id)
            .expect("buyer must exist")
            .owner_dynasty_id();
        let seller_owner = state
            .businesses
            .get(terms.seller_business_id)
            .expect("seller must exist")
            .owner_dynasty_id();
        let pair = DynastyPair::new(buyer_owner, seller_owner);
        let memories_before = state
            .relationships
            .get(&pair)
            .expect("relationship must exist")
            .memories
            .len();
        let contract_id =
            sign_supply_contract(registry, &mut state, terms).expect("contract must be signed");

        let relationship = state
            .relationships
            .get(&pair)
            .expect("relationship must exist");
        assert_eq!(relationship.memories.len(), memories_before + 1);
        assert!(
            relationship
                .memories
                .last()
                .is_some_and(|memory| memory.contains(&contract_id.to_string()))
        );
        if buyer_owner == state.player_dynasty_id || seller_owner == state.player_dynasty_id {
            assert!(state.information_reports.values().any(|report| {
                report.owner_dynasty_id == state.player_dynasty_id
                    && report.source == "Contract negotiation and delivery records"
                    && report.subject.starts_with("Counterparty report:")
            }));
        }
    }

    fn contract_relationship(
        state: &AppState,
        contract_id: crate::ids::ContractId,
    ) -> &RelationshipState {
        let contract = state
            .contracts
            .get(&contract_id)
            .expect("contract must exist");
        let buyer_owner = state
            .businesses
            .get(contract.buyer_business_id)
            .expect("buyer must exist")
            .owner_dynasty_id();
        let seller_owner = state
            .businesses
            .get(contract.seller_business_id)
            .expect("seller must exist")
            .owner_dynasty_id();
        state
            .relationships
            .get(&DynastyPair::new(buyer_owner, seller_owner))
            .expect("contract parties must have a relationship")
    }

    #[test]
    fn rejects_identical_buyer_and_seller() {
        let registry = test_registry();
        let state = make_test_campaign();
        let mut terms = make_test_contract_terms(&state);
        terms.seller_business_id = terms.buyer_business_id;

        let result = validate_supply_contract(registry, &state, terms);

        assert_eq!(
            result.expect_err("identical contract parties must be rejected"),
            StrategicError::SameContractParty
        );
    }

    #[test]
    fn rejects_zero_week_duration() {
        let registry = test_registry();
        let state = make_test_campaign();
        let mut terms = make_test_contract_terms(&state);
        terms.duration_weeks = 0;

        assert_eq!(
            validate_supply_contract(registry, &state, terms)
                .expect_err("zero-duration contracts must be rejected"),
            StrategicError::EmptyContractDuration
        );
    }

    #[test]
    fn rejects_an_unrepresentable_weekly_invoice_without_mutation() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let mut terms = make_test_contract_terms(&state);
        terms.quantity_per_week = Quantity::from_milliunits(i64::MAX);
        terms.unit_price = Money::from_copper(i64::MAX);
        let expected_quantity = terms.quantity_per_week;
        let expected_unit_price = terms.unit_price;
        let before = state.clone();

        let result = sign_supply_contract(registry, &mut state, terms);

        assert_eq!(
            result,
            Err(StrategicError::ContractPaymentOverflow {
                quantity: expected_quantity,
                unit_price: expected_unit_price,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "unrepresentable contract invoices must fail before creating durable records",
        );
    }

    #[test]
    fn revalidates_parties_before_commit() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let terms = make_test_contract_terms(&state);
        let seller_business_id = terms.seller_business_id;
        let token = validate_supply_contract(registry, &state, terms)
            .expect("contract terms must initially validate");
        state
            .businesses
            .get_mut(seller_business_id)
            .expect("seller must exist")
            .operations
            .status = crate::core::BusinessStatus::Closed;
        let before_commit = state.clone();

        let result = token.commit(registry, &mut state);

        assert_eq!(
            result,
            Err(StrategicError::BusinessInactive {
                business_id: seller_business_id,
            })
        );
        assert_state_unchanged(
            &before_commit,
            &state,
            "a failed stale-token commit must not partially mutate state",
        );
    }

    #[test]
    fn signing_a_contract_builds_social_capital_between_houses() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let terms = make_test_contract_terms(&state);
        let buyer_owner = state
            .businesses
            .get(terms.buyer_business_id)
            .expect("buyer must exist")
            .owner_dynasty_id();
        let seller_owner = state
            .businesses
            .get(terms.seller_business_id)
            .expect("seller must exist")
            .owner_dynasty_id();
        let pair = DynastyPair::new(buyer_owner, seller_owner);
        let before = state
            .relationships
            .get(&pair)
            .expect("relationship must exist")
            .clone();

        let contract_id = sign_supply_contract(registry, &mut state, terms)
            .expect("compatible houses must sign the contract");
        let after = contract_relationship(&state, contract_id);

        assert!(after.trust_basis_points > before.trust_basis_points);
        assert!(after.respect_basis_points > before.respect_basis_points);
        assert!(after.obligation > before.obligation);
        assert_eq!(after.last_interaction_day, state.clock.day());
    }

    #[test]
    fn charges_nonpayment_penalty_to_the_buyer() {
        let mut state = make_test_campaign();
        let contract_id = active_contract_id(&state);
        let (buyer_id, seller_id, good_id, quantity) = {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("contract must exist");
            contract.unit_price = Money::from_copper(10_000);
            contract.penalty = Money::from_copper(500);
            contract.next_due_day = state.clock.day();
            (
                contract.buyer_business_id,
                contract.seller_business_id,
                contract.good_id,
                contract.quantity_per_week,
            )
        };
        state
            .businesses
            .get_mut(seller_id)
            .expect("seller must exist")
            .add_inventory(good_id, quantity);
        state
            .businesses
            .get_mut(buyer_id)
            .expect("buyer must exist")
            .finance
            .cash = Money::from_copper(500);
        let seller_before = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .cash();
        let buyer_owner_id = state
            .businesses
            .get(buyer_id)
            .expect("buyer must exist")
            .owner_dynasty_id();
        let seller_owner_id = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .owner_dynasty_id();
        assert_ne!(
            buyer_owner_id, seller_owner_id,
            "the reputation regression requires an external contract"
        );
        let buyer_reliability_before = state
            .dynasties
            .get(&buyer_owner_id)
            .expect("buyer owner must exist")
            .resources
            .reputation_reliability_basis_points;
        let relationship_before = contract_relationship(&state, contract_id).clone();

        settle_contracts(&mut state).expect("contract settlement must succeed");

        assert_eq!(
            state
                .businesses
                .get(buyer_id)
                .expect("buyer must exist")
                .cash(),
            Money::ZERO
        );
        assert_eq!(
            state
                .businesses
                .get(seller_id)
                .expect("seller must exist")
                .cash(),
            seller_before.saturating_add(Money::from_copper(500))
        );
        assert_eq!(
            state
                .contracts
                .get(&contract_id)
                .expect("contract must exist")
                .missed_deliveries,
            1,
            "buyer nonpayment must count as a missed contract delivery"
        );
        assert!(
            state
                .dynasties
                .get(&buyer_owner_id)
                .expect("buyer owner must exist")
                .resources
                .reputation_reliability_basis_points
                < buyer_reliability_before,
            "contract nonpayment must reduce the responsible dynasty's reliability reputation"
        );
        let relationship_after = contract_relationship(&state, contract_id);
        assert!(relationship_after.trust_basis_points < relationship_before.trust_basis_points);
        assert!(
            relationship_after.resentment_basis_points
                > relationship_before.resentment_basis_points
        );
    }

    #[test]
    fn dual_nonperformance_does_not_arbitrarily_penalize_one_party() {
        let mut state = make_test_campaign();
        let contract_id = active_contract_id(&state);
        let (buyer_id, seller_id, good_id) = {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("contract must exist");
            contract.penalty = Money::from_copper(500);
            contract.next_due_day = state.clock.day();
            (
                contract.buyer_business_id,
                contract.seller_business_id,
                contract.good_id,
            )
        };
        state
            .businesses
            .get_mut(seller_id)
            .expect("seller must exist")
            .inventory
            .insert(good_id, Quantity::ZERO);
        state
            .businesses
            .get_mut(buyer_id)
            .expect("buyer must exist")
            .finance
            .cash = Money::ZERO;
        let buyer_before = state
            .businesses
            .get(buyer_id)
            .expect("buyer must exist")
            .cash();
        let seller_before = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .cash();

        settle_contracts(&mut state).expect("contract settlement must succeed");

        assert_eq!(
            state
                .businesses
                .get(buyer_id)
                .expect("buyer must exist")
                .cash(),
            buyer_before
        );
        assert_eq!(
            state
                .businesses
                .get(seller_id)
                .expect("seller must exist")
                .cash(),
            seller_before,
            "when both parties fail, settlement must not choose an arbitrary penalty payer"
        );
        assert_eq!(
            state
                .contracts
                .get(&contract_id)
                .expect("contract must exist")
                .missed_deliveries,
            1
        );
    }

    #[test]
    fn settlement_moves_only_penalty_when_seller_cannot_receive_payment() {
        let mut state = make_test_campaign();
        let contract_id = active_contract_id(&state);
        let (buyer_id, seller_id, good_id, quantity, payment, penalty) = {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("contract must exist");
            contract.next_due_day = state.clock.day();
            let payment = cost_for(contract.quantity_per_week, contract.unit_price);
            (
                contract.buyer_business_id,
                contract.seller_business_id,
                contract.good_id,
                contract.quantity_per_week,
                payment,
                contract.penalty,
            )
        };
        {
            let seller = state
                .businesses
                .get_mut(seller_id)
                .expect("seller must exist");
            seller.inventory.insert(good_id, quantity);
            seller.finance.cash = Money::from_copper(i64::MAX);
        }
        state
            .businesses
            .get_mut(buyer_id)
            .expect("buyer must exist")
            .finance
            .cash = payment;
        let buyer_inventory_before = state
            .businesses
            .get(buyer_id)
            .expect("buyer must exist")
            .inventory_quantity(good_id);

        settle_contracts(&mut state).expect("contract settlement must succeed");

        let buyer = state.businesses.get(buyer_id).expect("buyer must exist");
        let seller = state.businesses.get(seller_id).expect("seller must exist");
        assert_eq!(buyer.cash(), payment.saturating_add(penalty));
        assert_eq!(buyer.inventory_quantity(good_id), buyer_inventory_before);
        assert_eq!(
            seller.cash(),
            Money::from_copper(i64::MAX).saturating_sub(penalty)
        );
        assert_eq!(seller.inventory_quantity(good_id), quantity);
        assert_eq!(
            state
                .contracts
                .get(&contract_id)
                .expect("contract must exist")
                .missed_deliveries,
            1,
            "an unrepresentable settlement must remain an unfulfilled obligation"
        );
    }

    #[test]
    fn final_delivery_attributes_seller_when_payment_cannot_be_received() {
        let mut state = make_test_campaign();
        let contract_id = active_contract_id(&state);
        let (buyer_id, seller_id, good_id, quantity, payment, penalty) = {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("contract must exist");
            contract.next_due_day = state.clock.day();
            contract.end_day = state.clock.day();
            (
                contract.buyer_business_id,
                contract.seller_business_id,
                contract.good_id,
                contract.quantity_per_week,
                cost_for(contract.quantity_per_week, contract.unit_price),
                contract.penalty,
            )
        };
        let seller_dynasty_id = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .owner_dynasty_id();
        let buyer_dynasty_id = state
            .businesses
            .get(buyer_id)
            .expect("buyer must exist")
            .owner_dynasty_id();
        assert_ne!(seller_dynasty_id, buyer_dynasty_id);
        let seller_reliability_before = state
            .dynasties
            .get(&seller_dynasty_id)
            .expect("seller dynasty must exist")
            .resources
            .reputation_reliability_basis_points;
        {
            let seller = state
                .businesses
                .get_mut(seller_id)
                .expect("seller must exist");
            seller.inventory.insert(good_id, quantity);
            seller.finance.cash = Money::from_copper(i64::MAX);
        }
        state
            .businesses
            .get_mut(buyer_id)
            .expect("buyer must exist")
            .finance
            .cash = payment;

        settle_contracts(&mut state).expect("contract settlement must succeed");

        let contract = state
            .contracts
            .get(&contract_id)
            .expect("contract must exist");
        assert_eq!(contract.status, ContractStatus::Breached);
        assert_eq!(contract.breaching_dynasty_id, Some(seller_dynasty_id));
        assert_eq!(
            state
                .dynasties
                .get(&seller_dynasty_id)
                .expect("seller dynasty must exist")
                .resources
                .reputation_reliability_basis_points,
            seller_reliability_before.saturating_sub(120)
        );
        assert_eq!(
            state
                .businesses
                .get(buyer_id)
                .expect("buyer must exist")
                .cash(),
            payment.saturating_add(penalty),
            "the party unable to receive payment must owe the contractual penalty"
        );
    }

    #[test]
    fn final_delivery_attributes_buyer_when_goods_cannot_be_received() {
        let mut state = make_test_campaign();
        let contract_id = active_contract_id(&state);
        let (buyer_id, seller_id, good_id, quantity, payment, penalty) = {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("contract must exist");
            contract.next_due_day = state.clock.day();
            contract.end_day = state.clock.day();
            (
                contract.buyer_business_id,
                contract.seller_business_id,
                contract.good_id,
                contract.quantity_per_week,
                cost_for(contract.quantity_per_week, contract.unit_price),
                contract.penalty,
            )
        };
        let buyer_dynasty_id = state
            .businesses
            .get(buyer_id)
            .expect("buyer must exist")
            .owner_dynasty_id();
        let seller_dynasty_id = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .owner_dynasty_id();
        assert_ne!(buyer_dynasty_id, seller_dynasty_id);
        let buyer_reliability_before = state
            .dynasties
            .get(&buyer_dynasty_id)
            .expect("buyer dynasty must exist")
            .resources
            .reputation_reliability_basis_points;
        let seller_cash_before = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .cash();
        state
            .businesses
            .get_mut(seller_id)
            .expect("seller must exist")
            .inventory
            .insert(good_id, quantity);
        {
            let buyer = state
                .businesses
                .get_mut(buyer_id)
                .expect("buyer must exist");
            buyer
                .inventory
                .insert(good_id, Quantity::from_milliunits(i64::MAX));
            buyer.finance.cash = payment.saturating_add(penalty);
        }

        settle_contracts(&mut state).expect("contract settlement must succeed");

        let contract = state
            .contracts
            .get(&contract_id)
            .expect("contract must exist");
        assert_eq!(contract.status, ContractStatus::Breached);
        assert_eq!(contract.breaching_dynasty_id, Some(buyer_dynasty_id));
        assert_eq!(
            state
                .dynasties
                .get(&buyer_dynasty_id)
                .expect("buyer dynasty must exist")
                .resources
                .reputation_reliability_basis_points,
            buyer_reliability_before.saturating_sub(120)
        );
        assert_eq!(
            state
                .businesses
                .get(seller_id)
                .expect("seller must exist")
                .cash(),
            seller_cash_before.saturating_add(penalty),
            "the party unable to receive goods must owe the contractual penalty"
        );
    }

    #[test]
    fn contract_cost_overflow_is_rejected_before_goods_or_money_move() {
        let mut state = make_test_campaign();
        let contract_id = active_contract_id(&state);
        let (buyer_id, seller_id, good_id, quantity, payment) = {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("contract must exist");
            contract.next_due_day = state.clock.day();
            let payment = cost_for(contract.quantity_per_week, contract.unit_price);
            (
                contract.buyer_business_id,
                contract.seller_business_id,
                contract.good_id,
                contract.quantity_per_week,
                payment,
            )
        };
        state
            .businesses
            .get_mut(seller_id)
            .expect("seller must exist")
            .add_inventory(good_id, quantity);
        let buyer = state
            .businesses
            .get_mut(buyer_id)
            .expect("buyer must exist");
        buyer.finance.cash = payment;
        buyer.finance.lifetime_costs = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = settle_contracts(&mut state);

        assert_eq!(
            result,
            Err(SimulationError::BusinessLifetimeCostsOverflow {
                business_id: buyer_id,
                current: Money::from_copper(i64::MAX),
                incoming: payment,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "contract accounting overflow must fail before payment, delivery, or contract mutation",
        );
    }

    #[test]
    fn missed_final_delivery_cannot_end_as_fulfilled() {
        let mut state = make_test_campaign();
        let contract_id = active_contract_id(&state);
        let (seller_id, good_id) = {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("contract must exist");
            contract.next_due_day = state.clock.day();
            contract.end_day = state.clock.day();
            (contract.seller_business_id, contract.good_id)
        };
        state
            .businesses
            .get_mut(seller_id)
            .expect("seller must exist")
            .inventory
            .insert(good_id, Quantity::ZERO);
        let seller_dynasty_id = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .owner_dynasty_id();

        settle_contracts(&mut state).expect("contract settlement must succeed");

        assert_eq!(
            state
                .contracts
                .get(&contract_id)
                .expect("contract must exist")
                .status,
            ContractStatus::Breached,
            "a missed final obligation must not be recorded as contract fulfillment"
        );
        assert_eq!(
            state
                .contracts
                .get(&contract_id)
                .expect("contract must exist")
                .breaching_dynasty_id,
            Some(seller_dynasty_id),
            "the dynasty that could not deliver must be attributed as the breaching party"
        );
    }

    #[test]
    fn inactive_contract_party_terminates_without_mutating_business_finances() {
        let mut state = make_test_campaign();
        let contract_id = active_contract_id(&state);
        let (buyer_id, seller_id, good_id, quantity) = {
            let contract = state
                .contracts
                .get_mut(&contract_id)
                .expect("contract must exist");
            contract.next_due_day = state.clock.day();
            (
                contract.buyer_business_id,
                contract.seller_business_id,
                contract.good_id,
                contract.quantity_per_week,
            )
        };
        state
            .businesses
            .get_mut(seller_id)
            .expect("seller must exist")
            .add_inventory(good_id, quantity);
        state
            .businesses
            .get_mut(seller_id)
            .expect("seller must exist")
            .operations
            .status = BusinessStatus::Closed;
        let seller_dynasty_id = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .owner_dynasty_id();
        let buyer_before = state
            .businesses
            .get(buyer_id)
            .expect("buyer must exist")
            .finance
            .clone();
        let seller_before = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .finance
            .clone();
        let seller_inventory_before = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .inventory_quantity(good_id);

        settle_contracts(&mut state).expect("contract settlement must succeed");

        assert_eq!(
            state
                .contracts
                .get(&contract_id)
                .expect("contract must exist")
                .status,
            ContractStatus::Breached
        );
        assert_eq!(
            state
                .contracts
                .get(&contract_id)
                .expect("contract must exist")
                .breaching_dynasty_id,
            Some(seller_dynasty_id),
            "closing the seller must attribute the breach to the seller's dynasty"
        );
        assert_eq!(
            &state
                .businesses
                .get(buyer_id)
                .expect("buyer must exist")
                .finance,
            &buyer_before,
            "inactive-party termination must not charge the other business"
        );
        assert_eq!(
            &state
                .businesses
                .get(seller_id)
                .expect("seller must exist")
                .finance,
            &seller_before,
            "a closed business must not be financially mutated by settlement"
        );
        assert_eq!(
            state
                .businesses
                .get(seller_id)
                .expect("seller must exist")
                .inventory_quantity(good_id),
            seller_inventory_before,
            "a closed business must not ship inventory"
        );
    }
}

mod reputation {
    use super::*;

    #[test]
    fn business_quality_moves_dynasty_reputation() {
        let mut state = make_test_campaign();
        let dynasty_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .owner_dynasty_id();
        for business in state
            .businesses
            .iter_mut()
            .filter(|business| business.owner_dynasty_id() == dynasty_id)
        {
            business.operations.quality_basis_points = 9_000;
            business.finance.lifetime_revenue = Money::from_copper(10_000);
            business.finance.lifetime_costs = Money::from_copper(8_000);
        }
        state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("business owner must exist")
            .resources
            .reputation_quality_basis_points = 5_000;

        update_quality_reputations(&mut state);

        assert_eq!(
            state
                .dynasties
                .get(&dynasty_id)
                .expect("business owner must exist")
                .resources
                .reputation_quality_basis_points,
            5_050,
            "quality reputation must move gradually toward current portfolio quality"
        );
    }

    #[test]
    fn unproven_or_unprofitable_quality_builds_reputation_more_slowly() {
        let mut state = make_test_campaign();
        let dynasty_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .owner_dynasty_id();
        for business in state
            .businesses
            .iter_mut()
            .filter(|business| business.owner_dynasty_id() == dynasty_id)
        {
            business.operations.quality_basis_points = 9_000;
            business.finance.lifetime_revenue = Money::from_copper(8_000);
            business.finance.lifetime_costs = Money::from_copper(10_000);
        }
        state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("business owner must exist")
            .resources
            .reputation_quality_basis_points = 5_000;

        update_quality_reputations(&mut state);

        assert_eq!(
            state
                .dynasties
                .get(&dynasty_id)
                .expect("business owner must exist")
                .resources
                .reputation_quality_basis_points,
            5_025,
            "visible quality must build standing more slowly until trade proves commercially sustainable"
        );
    }
}

mod family_councils {
    use super::*;

    #[test]
    fn forced_governance_changes_are_audited_and_reported() {
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        {
            let council = state
                .family_councils
                .get_mut(&dynasty_id)
                .expect("player family council must exist");
            council.governance = HouseGovernance::Primogeniture;
            council.unity_basis_points = 1;
        }
        let audit_before = state.audit_log.len();
        let outbox_before = state.outbox.len();

        update_family_councils(&mut state)
            .expect("forced governance change must remain representable");

        assert_eq!(
            state
                .family_councils
                .get(&dynasty_id)
                .expect("player family council must exist")
                .governance,
            HouseGovernance::FamilyPartnership
        );
        assert_eq!(state.audit_log.len(), audit_before + 1);
        let record = state.audit_log.last().expect("change must be audited");
        assert_eq!(record.kind(), AuditKind::HouseGovernanceChange);
        assert_eq!(record.subject(), format!("dynasty:{dynasty_id}"));
        assert!(record.detail().contains("automatic=true"));
        assert_eq!(state.outbox.len(), outbox_before + 1);
        assert_eq!(
            state.outbox.last().expect("change must be reported").kind,
            OutboxKind::Family
        );
    }

    #[test]
    fn member_loyalty_affects_annual_council_unity() {
        let state = make_test_campaign();
        let dynasty_id = *state
            .family_councils
            .keys()
            .next()
            .expect("campaign must contain a family council");
        let member_ids: Vec<_> = state
            .family_councils
            .get(&dynasty_id)
            .expect("family council must exist")
            .members
            .iter()
            .copied()
            .collect();
        let mut disloyal = state.clone();
        let mut loyal = state;
        for current in [&mut disloyal, &mut loyal] {
            let council = current
                .family_councils
                .get_mut(&dynasty_id)
                .expect("family council must exist");
            council.unity_basis_points = 6_000;
            council.governance = HouseGovernance::FamilyPartnership;
        }
        for character_id in &member_ids {
            disloyal
                .characters
                .get_mut(*character_id)
                .expect("council member must exist")
                .runtime
                .loyalty_basis_points = 1_000;
            loyal
                .characters
                .get_mut(*character_id)
                .expect("council member must exist")
                .runtime
                .loyalty_basis_points = 10_000;
        }

        update_family_councils(&mut disloyal)
            .expect("disloyal council update must remain representable");
        update_family_councils(&mut loyal).expect("loyal council update must remain representable");

        assert!(
            loyal
                .family_councils
                .get(&dynasty_id)
                .expect("loyal family council must exist")
                .unity_basis_points
                > disloyal
                    .family_councils
                    .get(&dynasty_id)
                    .expect("disloyal family council must exist")
                    .unity_basis_points,
            "family-member loyalty must influence council cohesion"
        );
    }
}

mod employment {
    use super::*;

    #[test]
    fn sustained_high_utilization_with_low_maintenance_creates_labor_pressure() {
        let state = make_test_campaign();
        let mut agreement = state
            .employment
            .values()
            .find(|agreement| agreement.status == EmploymentStatus::Active)
            .expect("campaign must contain active employment")
            .clone();
        let environment = LaborEnvironment {
            utilization: 10_000,
            business_condition: 8_000,
            maintenance: 800,
        };
        let mut became_disputed = false;

        for _ in 0..120 {
            let (_, disputed) =
                update_fully_paid_employment(&mut agreement, EmploymentStatus::Active, environment);
            if disputed {
                became_disputed = true;
                break;
            }
        }

        assert!(became_disputed);
        assert_eq!(agreement.status, EmploymentStatus::Disputed);
        assert!(agreement.conditions_basis_points < 3_000);
    }

    #[test]
    fn maintained_workplace_does_not_generate_artificial_disputes() {
        let state = make_test_campaign();
        let mut agreement = state
            .employment
            .values()
            .find(|agreement| agreement.status == EmploymentStatus::Active)
            .expect("campaign must contain active employment")
            .clone();
        let environment = LaborEnvironment {
            utilization: 10_000,
            business_condition: 8_000,
            maintenance: 1_200,
        };

        for _ in 0..120 {
            let (_, disputed) =
                update_fully_paid_employment(&mut agreement, EmploymentStatus::Active, environment);
            assert!(!disputed);
        }

        assert_eq!(agreement.status, EmploymentStatus::Active);
        assert!(agreement.conditions_basis_points >= 6_800);
    }

    #[test]
    fn zero_payment_does_not_invalidate_business_finance_version() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let employment_id = state
            .employment
            .values()
            .find(|agreement| agreement.status == EmploymentStatus::Active)
            .expect("campaign must contain active employment")
            .id;
        let (business_id, household_id, loyalty_before) = {
            let agreement = state
                .employment
                .get(&employment_id)
                .expect("employment must exist");
            (
                agreement.business_id,
                agreement.household_id,
                agreement.loyalty_basis_points,
            )
        };
        let household_employers: Vec<_> = state
            .employment
            .values()
            .filter(|agreement| {
                agreement.status == EmploymentStatus::Active
                    && agreement.household_id == household_id
            })
            .map(|agreement| agreement.business_id)
            .collect();
        for employer_id in household_employers {
            state
                .businesses
                .get_mut(employer_id)
                .expect("employment business must exist")
                .finance
                .cash = Money::ZERO;
        }
        let version_before = state
            .businesses
            .get(business_id)
            .expect("employment business must exist")
            .finance
            .version;
        let household_cash_before = state
            .households
            .get(household_id)
            .expect("employment household must exist")
            .cash();

        settle_employment(registry, &mut state).expect("employment settlement must succeed");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("employment business must exist")
                .finance
                .version,
            version_before,
            "a zero-value settlement must not invalidate finance tokens"
        );
        assert_eq!(
            state
                .households
                .get(household_id)
                .expect("employment household must exist")
                .cash(),
            household_cash_before
        );
        assert!(
            state
                .employment
                .get(&employment_id)
                .expect("employment must exist")
                .loyalty_basis_points
                < loyalty_before,
            "the missed wage must still affect the employment relationship"
        );
    }

    #[test]
    fn payroll_affordability_never_returns_a_negative_payment() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let agreement = state
            .employment
            .values()
            .find(|agreement| agreement.status == EmploymentStatus::Active)
            .expect("campaign must contain active employment");
        let business_id = agreement.business_id;
        let household_id = agreement.household_id;
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("employment business must exist");
            business.finance.cash = Money::from_copper(100);
            business.policy.minimum_cash_reserve = Money::from_copper(500);
        }
        let version_before = state
            .businesses
            .get(business_id)
            .expect("employment business must exist")
            .finance
            .version;
        let household_cash_before = state
            .households
            .get(household_id)
            .expect("employment household must exist")
            .cash();

        let paid = pay_employment_wage(
            registry,
            &mut state,
            business_id,
            household_id,
            Money::from_copper(200),
        );

        assert_eq!(paid, Ok(Money::ZERO));
        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("employment business must exist")
                .finance
                .version,
            version_before
        );
        assert_eq!(
            state
                .households
                .get(household_id)
                .expect("employment household must exist")
                .cash(),
            household_cash_before
        );
    }

    #[test]
    fn payroll_does_not_debit_business_when_household_cash_has_no_headroom() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let agreement = state
            .employment
            .values()
            .find(|agreement| agreement.status == EmploymentStatus::Active)
            .expect("campaign must contain active employment");
        let business_id = agreement.business_id;
        let household_id = agreement.household_id;
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("employment business must exist");
            business.finance.cash = Money::from_copper(1_000);
            business.policy.minimum_cash_reserve = Money::ZERO;
        }
        state
            .households
            .get_mut(household_id)
            .expect("employment household must exist")
            .cash = Money::from_copper(i64::MAX);
        let finance_before = state
            .businesses
            .get(business_id)
            .expect("employment business must exist")
            .finance
            .clone();

        let paid = pay_employment_wage(
            registry,
            &mut state,
            business_id,
            household_id,
            Money::from_copper(100),
        );

        assert_eq!(paid, Ok(Money::ZERO));
        assert_eq!(
            &state
                .businesses
                .get(business_id)
                .expect("employment business must exist")
                .finance,
            &finance_before
        );
        assert_eq!(
            state
                .households
                .get(household_id)
                .expect("employment household must exist")
                .cash,
            Money::from_copper(i64::MAX)
        );
    }
}

mod property_liquidation {
    use super::*;
    use crate::test_support::rivergate_registry_for_test;

    fn add_test_property_lien(
        state: &mut AppState,
        lender_dynasty_id: DynastyId,
        borrower_dynasty_id: DynastyId,
        property_id: PropertyId,
        balance: Money,
    ) -> crate::ids::LoanId {
        let loan_id = state.next_ids.loan();
        state.loans.insert(
            loan_id,
            Loan {
                id: loan_id,
                lender_dynasty_id,
                borrower_dynasty_id,
                principal: balance,
                balance,
                weekly_payment: Money::from_copper(20),
                interest_basis_points: 500,
                next_due_day: state.clock.day().saturating_add(7),
                missed_payments: 0,
                collateral_property_id: Some(property_id),
                status: LoanStatus::Current,
            },
        );
        state
            .properties
            .get_mut(&property_id)
            .expect("property must exist")
            .collateral_loan_id = Some(loan_id);
        loan_id
    }

    fn assert_completed_lien_repayment_feedback(
        state: &AppState,
        lender_id: DynastyId,
        borrower_id: DynastyId,
        loan_id: crate::ids::LoanId,
        reliability_before: u16,
    ) {
        assert_eq!(
            state
                .dynasties
                .get(&borrower_id)
                .expect("borrower must exist")
                .resources
                .reputation_reliability_basis_points,
            reliability_before.saturating_add(10)
        );
        let relationship = state
            .relationships
            .get(&DynastyPair::new(lender_id, borrower_id))
            .expect("loan parties must retain a relationship");
        assert!(
            relationship
                .memories
                .last()
                .is_some_and(|memory| memory.contains(&loan_id.to_string()))
        );
        assert!(state.information_reports.values().any(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == "Completed loan repayment records"
        }));
    }

    #[test]
    fn owned_property_can_be_liquidated_without_displacing_its_occupant() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let seller_id = state.player_dynasty_id;
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(seller_id)
                    && property.collateral_loan_id.is_none()
            })
            .expect("campaign must contain an unpledged player property")
            .id;
        let buyer_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != seller_id)
            .expect("campaign must contain another dynasty");
        state
            .dynasties
            .get_mut(&buyer_id)
            .expect("buyer must exist")
            .resources
            .treasury = Money::from_copper(1_000_000);
        let occupant_owner_id = state
            .properties
            .get(&property_id)
            .and_then(|property| property.occupant_business_id)
            .and_then(|business_id| state.businesses.get(business_id))
            .map(crate::core::Business::owner_dynasty_id);
        let seller_before = state
            .dynasties
            .get(&seller_id)
            .expect("seller must exist")
            .treasury();
        let buyer_before = state
            .dynasties
            .get(&buyer_id)
            .expect("buyer must exist")
            .treasury();
        let expected_quote =
            quote_property_liquidation(registry, &state, seller_id, buyer_id, property_id)
                .expect("property must be liquidatable");

        let quote = sell_owned_property(registry, &mut state, seller_id, buyer_id, property_id)
            .expect("property sale must succeed");

        assert_eq!(quote, expected_quote);
        assert_eq!(
            state
                .dynasties
                .get(&seller_id)
                .expect("seller must exist")
                .treasury(),
            seller_before.saturating_add(quote.seller_proceeds)
        );
        assert_eq!(
            state
                .dynasties
                .get(&buyer_id)
                .expect("buyer must exist")
                .treasury(),
            buyer_before.saturating_sub(quote.buyer_contribution)
        );
        let property = state
            .properties
            .get(&property_id)
            .expect("property must remain present");
        assert_eq!(property.owner_dynasty_id, Some(buyer_id));
        assert_eq!(
            property.tenant_dynasty_id,
            occupant_owner_id.filter(|owner_id| *owner_id != buyer_id)
        );
        assert!(property.occupant_business_id.is_some());
    }

    #[test]
    fn missing_collateral_loan_rejects_liquidation_atomically() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let seller_id = state.player_dynasty_id;
        let buyer_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != seller_id)
            .expect("campaign must contain another dynasty");
        let property_id = state
            .properties
            .values()
            .find(|property| property.owner_dynasty_id == Some(seller_id))
            .expect("campaign must contain a player property")
            .id;
        let loan_id = crate::ids::LoanId::new(u32::MAX);
        state
            .properties
            .get_mut(&property_id)
            .expect("property must exist")
            .collateral_loan_id = Some(loan_id);
        let before = state.clone();

        let result = sell_owned_property(registry, &mut state, seller_id, buyer_id, property_id);

        assert_eq!(
            result,
            Err(StrategicError::MissingCollateralLoan { loan_id })
        );
        assert_state_unchanged(
            &before,
            &state,
            "rejected liquidation must not mutate campaign state",
        );
    }

    #[test]
    fn property_sale_settles_its_lien_before_paying_the_seller() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let seller_id = state.player_dynasty_id;
        let buyer_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != seller_id)
            .expect("campaign must contain a buyer");
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != seller_id && *dynasty_id != buyer_id)
            .expect("campaign must contain a lender");
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(seller_id)
                    && property.collateral_loan_id.is_none()
            })
            .expect("campaign must contain an unpledged player property")
            .id;
        state
            .dynasties
            .get_mut(&buyer_id)
            .expect("buyer must exist")
            .resources
            .treasury = Money::from_copper(1_000_000);
        let unencumbered_quote =
            quote_property_liquidation(registry, &state, seller_id, buyer_id, property_id)
                .expect("selected property must be liquidatable before adding the lien");
        let balance = unencumbered_quote
            .price
            .saturating_mul_ratio(1, 2)
            .max(Money::from_copper(1));
        let loan_id =
            add_test_property_lien(&mut state, lender_id, seller_id, property_id, balance);
        let seller_before = state
            .dynasties
            .get(&seller_id)
            .expect("seller must exist")
            .treasury();
        let lender_before = state
            .dynasties
            .get(&lender_id)
            .expect("lender must exist")
            .treasury();
        let reliability_before = state
            .dynasties
            .get(&seller_id)
            .expect("seller must exist")
            .resources
            .reputation_reliability_basis_points;

        let quote = sell_owned_property(registry, &mut state, seller_id, buyer_id, property_id)
            .expect("sale proceeds must settle the lien");

        assert_eq!(quote.lien_payoff, balance);
        assert_eq!(quote.seller_proceeds, quote.price.saturating_sub(balance));
        assert_eq!(
            state
                .dynasties
                .get(&seller_id)
                .expect("seller must exist")
                .treasury(),
            seller_before.saturating_add(quote.seller_proceeds)
        );
        assert_eq!(
            state
                .dynasties
                .get(&lender_id)
                .expect("lender must exist")
                .treasury(),
            lender_before.saturating_add(balance)
        );
        let loan = state
            .loans
            .get(&loan_id)
            .expect("loan must remain auditable");
        assert_eq!(loan.status, LoanStatus::Repaid);
        assert_eq!(loan.balance, Money::ZERO);
        assert_eq!(loan.collateral_property_id, None);
        let property = state
            .properties
            .get(&property_id)
            .expect("property must remain present");
        assert_eq!(property.collateral_loan_id, None);
        assert_eq!(property.owner_dynasty_id, Some(buyer_id));
        assert_completed_lien_repayment_feedback(
            &state,
            lender_id,
            seller_id,
            loan_id,
            reliability_before,
        );
    }

    #[test]
    fn collateral_lender_can_buy_the_property_at_maximum_treasury() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let seller_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != seller_id)
            .expect("campaign must contain a lender");
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(seller_id)
                    && property.collateral_loan_id.is_none()
            })
            .expect("campaign must contain an unpledged player property")
            .id;
        state
            .dynasties
            .get_mut(&lender_id)
            .expect("lender must exist")
            .resources
            .treasury = Money::from_copper(i64::MAX);
        let unencumbered_quote =
            quote_property_liquidation(registry, &state, seller_id, lender_id, property_id)
                .expect("selected property must be liquidatable before adding the lien");
        let balance = unencumbered_quote
            .price
            .saturating_mul_ratio(1, 2)
            .max(Money::from_copper(1));
        let loan_id =
            add_test_property_lien(&mut state, lender_id, seller_id, property_id, balance);
        let seller_before = state
            .dynasties
            .get(&seller_id)
            .expect("seller must exist")
            .treasury();
        let lender_before = state
            .dynasties
            .get(&lender_id)
            .expect("lender must exist")
            .treasury();

        let quote = sell_owned_property(registry, &mut state, seller_id, lender_id, property_id)
            .expect("purchase debit must create room for the lender's lien payoff");

        assert_eq!(quote.lien_payoff, balance);
        assert_eq!(quote.buyer_contribution, quote.price);
        assert_eq!(
            state
                .dynasties
                .get(&lender_id)
                .expect("lender must exist")
                .treasury(),
            lender_before
                .saturating_sub(quote.buyer_contribution)
                .saturating_add(quote.lien_payoff)
        );
        assert_eq!(
            state
                .dynasties
                .get(&seller_id)
                .expect("seller must exist")
                .treasury(),
            seller_before.saturating_add(quote.seller_proceeds)
        );
        assert_eq!(
            state
                .properties
                .get(&property_id)
                .expect("property must remain present")
                .owner_dynasty_id,
            Some(lender_id)
        );
        assert_eq!(
            state.loans.get(&loan_id).expect("loan must remain").status,
            LoanStatus::Repaid
        );
        validate_invariants(registry, &state);
    }

    #[test]
    fn distinct_lender_overflow_rejects_property_sale_atomically() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let seller_id = state.player_dynasty_id;
        let mut counterparties = state
            .dynasties
            .keys()
            .copied()
            .filter(|dynasty_id| *dynasty_id != seller_id);
        let buyer_id = counterparties
            .next()
            .expect("campaign must contain a buyer");
        let lender_id = counterparties
            .next()
            .expect("campaign must contain a lender");
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(seller_id)
                    && property.collateral_loan_id.is_none()
            })
            .expect("campaign must contain an unpledged player property")
            .id;
        state
            .dynasties
            .get_mut(&buyer_id)
            .expect("buyer must exist")
            .resources
            .treasury = Money::from_copper(1_000_000);
        let unencumbered_quote =
            quote_property_liquidation(registry, &state, seller_id, buyer_id, property_id)
                .expect("selected property must be liquidatable before adding the lien");
        let balance = unencumbered_quote
            .price
            .saturating_mul_ratio(1, 2)
            .max(Money::from_copper(1));
        add_test_property_lien(&mut state, lender_id, seller_id, property_id, balance);
        state
            .dynasties
            .get_mut(&lender_id)
            .expect("lender must exist")
            .resources
            .treasury = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = sell_owned_property(registry, &mut state, seller_id, buyer_id, property_id);

        assert_eq!(
            result,
            Err(StrategicError::DynastyTreasuryOverflow {
                dynasty_id: lender_id,
                current: Money::from_copper(i64::MAX),
                incoming: balance,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "a lien payoff that cannot be credited must reject the entire property sale",
        );
    }

    #[test]
    fn civic_treasury_can_guarantee_a_distressed_property_auction() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let seller_id = state.player_dynasty_id;
        let buyer_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != seller_id)
            .expect("campaign must contain a buyer");
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(seller_id)
                    && property.collateral_loan_id.is_none()
            })
            .expect("campaign must contain an unpledged player property")
            .id;
        state
            .dynasties
            .get_mut(&seller_id)
            .expect("seller must exist")
            .resources
            .treasury = Money::from_copper(58);
        state
            .dynasties
            .get_mut(&buyer_id)
            .expect("buyer must exist")
            .resources
            .treasury = Money::ZERO;
        let business_id = state
            .businesses
            .ids_for_owner(seller_id)
            .and_then(|businesses| businesses.iter().next())
            .copied()
            .expect("seller must own a business");
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("business must exist");
        business.finance.cash = Money::ZERO;
        business.operations.status = BusinessStatus::Distressed;
        business.operations.condition_basis_points = 1_000;
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define the treasury");
        let civic_before = state
            .institutions
            .get(&treasury_id)
            .expect("treasury runtime must exist")
            .budget;
        let seller_before = state
            .dynasties
            .get(&seller_id)
            .expect("seller must exist")
            .treasury();

        let quote = sell_owned_property(registry, &mut state, seller_id, buyer_id, property_id)
            .expect("civic guarantee must make the distressed auction liquid");

        assert_eq!(quote.buyer_contribution, Money::ZERO);
        assert_eq!(quote.civic_guarantee, quote.price);
        assert_eq!(
            state
                .institutions
                .get(&treasury_id)
                .expect("treasury runtime must exist")
                .budget,
            civic_before.saturating_sub(quote.civic_guarantee)
        );
        assert_eq!(
            state
                .dynasties
                .get(&seller_id)
                .expect("seller must exist")
                .treasury(),
            seller_before.saturating_add(quote.seller_proceeds)
        );
    }
}

mod loans {
    use super::*;

    fn insert_test_civic_debt(state: &mut AppState) -> (crate::ids::CivicDebtId, DynastyId) {
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
                value: 1_000,
                active: false,
            },
        );
        let debt_id = state.next_ids.civic_debt();
        state.civic_debts.insert(
            debt_id,
            crate::core::CivicDebt {
                id: debt_id,
                creditor_dynasty_id,
                authorizing_law_id: law_id,
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                principal: Money::from_copper(1_000),
                balance: Money::from_copper(1_000),
                weekly_payment: Money::from_copper(100),
                interest_basis_points: 0,
                issued_day: state.clock.day(),
                next_due_day: state.clock.day(),
                missed_payments: 0,
                status: CivicDebtStatus::Current,
            },
        );
        (debt_id, creditor_dynasty_id)
    }

    #[test]
    fn issuing_a_loan_creates_relationship_memory_and_counterparty_intelligence() {
        let mut state = make_test_campaign();
        let terms = make_test_loan_terms(&state);
        let pair = DynastyPair::new(terms.lender_dynasty_id, terms.borrower_dynasty_id);
        let player_is_party = terms.lender_dynasty_id == state.player_dynasty_id
            || terms.borrower_dynasty_id == state.player_dynasty_id;
        let memories_before = state
            .relationships
            .get(&pair)
            .expect("relationship must exist")
            .memories
            .len();
        let loan_id = issue_loan(&mut state, terms).expect("loan must be issued");

        let relationship = state
            .relationships
            .get(&pair)
            .expect("relationship must exist");
        assert_eq!(relationship.memories.len(), memories_before + 1);
        assert!(
            relationship
                .memories
                .last()
                .is_some_and(|memory| memory.contains(&loan_id.to_string()))
        );
        if player_is_party {
            assert!(state.information_reports.values().any(|report| {
                report.owner_dynasty_id == state.player_dynasty_id
                    && report.source == "Credit underwriting and repayment records"
                    && report.subject.starts_with("Counterparty report:")
            }));
        }
    }

    #[test]
    fn relationship_memory_is_bounded_and_keeps_recent_history() {
        let mut state = make_test_campaign();
        let dynasty_ids: Vec<_> = state.dynasties.keys().copied().take(2).collect();
        let [first_dynasty_id, second_dynasty_id] = dynasty_ids.as_slice() else {
            panic!("fixture must contain at least two dynasties: {dynasty_ids:?}");
        };
        let pair = DynastyPair::new(*first_dynasty_id, *second_dynasty_id);

        for index in 0..20 {
            remember_dynasty_interaction(
                &mut state,
                *first_dynasty_id,
                *second_dynasty_id,
                &format!("interaction {index}"),
            );
        }

        let memories = &state
            .relationships
            .get(&pair)
            .expect("relationship must exist")
            .memories;
        assert_eq!(memories.len(), MAX_RELATIONSHIP_MEMORIES);
        assert!(
            memories
                .first()
                .is_some_and(|memory| memory.contains("interaction 8"))
        );
        assert!(
            memories
                .last()
                .is_some_and(|memory| memory.contains("interaction 19"))
        );
    }

    #[test]
    fn rejects_interest_above_one_hundred_percent() {
        let state = make_test_campaign();
        let mut invalid_interest = make_test_loan_terms(&state);
        invalid_interest.interest_basis_points = 10_001;

        assert_eq!(
            validate_loan(&state, invalid_interest).expect_err("interest must be rejected"),
            StrategicError::InterestOutOfRange {
                interest_basis_points: 10_001,
            }
        );
    }

    #[test]
    fn rejects_collateral_already_pledged_to_an_active_loan() {
        let state = make_test_campaign();
        let existing_loan = state
            .loans
            .values()
            .find(|loan| loan.collateral_property_id.is_some())
            .expect("bootstrap must create a collateralized loan");
        let property_id = existing_loan
            .collateral_property_id
            .expect("selected loan must have collateral");
        let alternate_lender_id = state
            .dynasties
            .values()
            .filter(|dynasty| {
                dynasty.id() != existing_loan.borrower_dynasty_id
                    && dynasty.id() != existing_loan.lender_dynasty_id
                    && dynasty.treasury() >= Money::from_copper(1)
            })
            .map(crate::core::Dynasty::id)
            .find(|lender_id| {
                !state.loans.values().any(|loan| {
                    loan.lender_dynasty_id == *lender_id
                        && loan.borrower_dynasty_id == existing_loan.borrower_dynasty_id
                        && loan.status != LoanStatus::Repaid
                })
            })
            .expect("fixture must contain an alternate lender without borrower exposure");
        let pledged_terms = LoanTerms {
            lender_dynasty_id: alternate_lender_id,
            borrower_dynasty_id: existing_loan.borrower_dynasty_id,
            principal: Money::from_copper(1),
            weekly_payment: Money::from_copper(1),
            interest_basis_points: 500,
            collateral_property_id: Some(property_id),
        };

        assert_eq!(
            validate_loan(&state, pledged_terms).expect_err("pledged collateral must be rejected"),
            StrategicError::PropertyAlreadyPledged {
                property_id,
                loan_id: existing_loan.id,
            }
        );
    }

    #[test]
    fn rejects_new_credit_while_the_same_pair_has_unsettled_debt() {
        let state = make_test_campaign();
        let existing = state
            .loans
            .values()
            .find(|loan| {
                matches!(
                    loan.status,
                    LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
                )
            })
            .expect("bootstrap must create unsettled credit");
        let duplicate_terms = LoanTerms {
            lender_dynasty_id: existing.lender_dynasty_id,
            borrower_dynasty_id: existing.borrower_dynasty_id,
            principal: Money::from_copper(1),
            weekly_payment: Money::from_copper(1),
            interest_basis_points: 500,
            collateral_property_id: None,
        };

        assert_eq!(
            validate_loan(&state, duplicate_terms)
                .expect_err("unsettled debt must block a second loan for the same pair"),
            StrategicError::ExistingUnsettledLoan {
                lender_dynasty_id: existing.lender_dynasty_id,
                borrower_dynasty_id: existing.borrower_dynasty_id,
                loan_id: existing.id,
            }
        );
    }

    #[test]
    fn defaulted_credit_cannot_be_restructured_before_the_cooling_period() {
        let mut state = make_test_campaign();
        let terms = make_test_loan_terms(&state);
        let loan_id = issue_loan(&mut state, terms.clone()).expect("loan must be issued");
        let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
        loan.status = LoanStatus::Defaulted;
        loan.missed_payments = 3;
        loan.next_due_day = state.clock.day();
        let refinancing = LoanTerms {
            principal: Money::from_copper(1_000),
            weekly_payment: Money::from_copper(10),
            interest_basis_points: 900,
            ..terms
        };

        assert_eq!(
            validate_loan(&state, refinancing)
                .expect_err("recently defaulted credit must remain unavailable"),
            StrategicError::DefaultedLoanRestructuringCooldown {
                loan_id,
                available_day: state
                    .clock
                    .day()
                    .saturating_add(DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS),
            }
        );
    }

    #[test]
    fn aged_defaulted_credit_is_restructured_in_place() {
        let mut state = make_test_campaign();
        let terms = make_test_loan_terms(&state);
        let lender_id = terms.lender_dynasty_id;
        let borrower_id = terms.borrower_dynasty_id;
        let loan_id = issue_loan(&mut state, terms.clone()).expect("loan must be issued");
        let (principal_before, balance_before) = {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            let balances = (loan.principal, loan.balance);
            loan.status = LoanStatus::Defaulted;
            loan.missed_payments = 3;
            loan.next_due_day = state
                .clock
                .day()
                .saturating_sub(DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS);
            balances
        };
        state
            .dynasties
            .get_mut(&lender_id)
            .expect("lender must exist")
            .resources
            .treasury = Money::from_copper(10_000);
        let lender_before = state
            .dynasties
            .get(&lender_id)
            .expect("lender must exist")
            .treasury();
        let borrower_before = state
            .dynasties
            .get(&borrower_id)
            .expect("borrower must exist")
            .treasury();
        let loan_count = state.loans.len();
        let advance = Money::from_copper(1_000);
        let refinancing = LoanTerms {
            principal: advance,
            weekly_payment: Money::from_copper(10),
            interest_basis_points: 900,
            ..terms
        };

        let returned_id =
            issue_loan(&mut state, refinancing).expect("aged default must restructure");

        assert_eq!(returned_id, loan_id);
        assert_eq!(state.loans.len(), loan_count);
        let loan = state.loans.get(&loan_id).expect("loan must remain present");
        assert_eq!(loan.status, LoanStatus::Restructured);
        assert_eq!(
            loan.principal,
            principal_before
                .checked_add(advance)
                .expect("test principal must remain in range")
        );
        assert_eq!(
            loan.balance,
            balance_before
                .checked_add(advance)
                .expect("test balance must remain in range")
        );
        assert_eq!(loan.weekly_payment, Money::from_copper(10));
        assert_eq!(loan.interest_basis_points, 900);
        assert_eq!(loan.missed_payments, 0);
        assert_eq!(loan.next_due_day, state.clock.day().saturating_add(7));
        assert_eq!(
            state
                .dynasties
                .get(&lender_id)
                .expect("lender must exist")
                .treasury(),
            lender_before.saturating_sub(advance)
        );
        assert_eq!(
            state
                .dynasties
                .get(&borrower_id)
                .expect("borrower must exist")
                .treasury(),
            borrower_before.saturating_add(advance)
        );
    }

    #[test]
    fn revalidates_lender_funds_before_commit() {
        let mut state = make_test_campaign();
        let terms = make_test_loan_terms(&state);
        let lender_dynasty_id = terms.lender_dynasty_id;
        let token = validate_loan(&state, terms).expect("loan terms must initially validate");
        state
            .dynasties
            .get_mut(&lender_dynasty_id)
            .expect("lender must exist")
            .resources
            .treasury = Money::ZERO;
        let before_commit = state.clone();

        let result = token.commit(&mut state);

        assert_eq!(
            result,
            Err(StrategicError::InsufficientDynastyFunds {
                dynasty_id: lender_dynasty_id,
                available: Money::ZERO,
                required: Money::from_copper(1),
            })
        );
        assert_state_unchanged(
            &before_commit,
            &state,
            "a failed stale-token commit must not partially mutate state",
        );
    }

    #[test]
    fn rejects_borrower_treasury_overflow_without_mutation() {
        let mut state = make_test_campaign();
        let terms = make_test_loan_terms(&state);
        state
            .dynasties
            .get_mut(&terms.borrower_dynasty_id)
            .expect("borrower must exist")
            .resources
            .treasury = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = issue_loan(&mut state, terms.clone());

        assert_eq!(
            result,
            Err(StrategicError::DynastyTreasuryOverflow {
                dynasty_id: terms.borrower_dynasty_id,
                current: Money::from_copper(i64::MAX),
                incoming: terms.principal,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "overflowing loan issuance must not debit the lender or create records",
        );
    }

    #[test]
    fn negative_manual_payment_is_a_no_op() {
        let mut state = make_test_campaign();
        let loan_id = current_loan_id(&state);
        let before = state.clone();

        let paid = apply_loan_payment(&mut state, loan_id, Money::from_copper(-1));

        assert_eq!(paid, Money::ZERO);
        assert_state_unchanged(
            &before,
            &state,
            "invalid negative payment must not mutate loan parties or balance",
        );
    }

    #[test]
    fn due_loan_does_not_debit_borrower_when_lender_treasury_has_no_headroom() {
        let mut state = make_test_campaign();
        let loan_id = current_loan_id(&state);
        let (lender_id, borrower_id, payment, missed_before, balance_before) = {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.next_due_day = state.clock.day();
            (
                loan.lender_dynasty_id,
                loan.borrower_dynasty_id,
                loan.weekly_payment,
                loan.missed_payments,
                loan.balance,
            )
        };
        state
            .dynasties
            .get_mut(&lender_id)
            .expect("lender must exist")
            .resources
            .treasury = Money::from_copper(i64::MAX);
        state
            .dynasties
            .get_mut(&borrower_id)
            .expect("borrower must exist")
            .resources
            .treasury = payment;

        settle_loans(&mut state).expect("loan settlement must succeed");

        assert_eq!(
            state
                .dynasties
                .get(&lender_id)
                .expect("lender must exist")
                .treasury(),
            Money::from_copper(i64::MAX)
        );
        assert_eq!(
            state
                .dynasties
                .get(&borrower_id)
                .expect("borrower must exist")
                .treasury(),
            payment,
            "a payment that cannot be credited must not be removed from the borrower"
        );
        assert_eq!(
            state
                .loans
                .get(&loan_id)
                .expect("loan must exist")
                .missed_payments,
            missed_before,
            "a lender-side capacity limit must not count as borrower nonpayment"
        );
        assert_eq!(
            state.loans.get(&loan_id).expect("loan must exist").balance,
            balance_before,
            "a deferred settlement must not accrue interest while the lender cannot receive it"
        );
    }

    #[test]
    fn accrue_interest_when_payment_is_missed() {
        let mut state = make_test_campaign();
        let loan_id = current_loan_id(&state);
        let borrower_id = state
            .loans
            .get(&loan_id)
            .expect("loan must exist")
            .borrower_dynasty_id;
        state
            .dynasties
            .get_mut(&borrower_id)
            .expect("borrower must exist")
            .resources
            .treasury = Money::ZERO;
        let reliability_before = state
            .dynasties
            .get(&borrower_id)
            .expect("borrower must exist")
            .resources
            .reputation_reliability_basis_points;
        {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.balance = Money::from_copper(52_000);
            loan.interest_basis_points = 1_000;
            loan.next_due_day = state.clock.day();
        }

        settle_loans(&mut state).expect("loan settlement must succeed");

        let loan = state.loans.get(&loan_id).expect("loan must exist");
        assert_eq!(
            loan.balance,
            Money::from_copper(52_100),
            "weekly interest must accrue before recording the missed payment"
        );
        assert_eq!(loan.missed_payments, 1, "one due payment was missed");
        assert_eq!(
            loan.status,
            LoanStatus::Delinquent,
            "a first missed payment must make the loan delinquent"
        );
        assert!(
            state
                .dynasties
                .get(&borrower_id)
                .expect("borrower must exist")
                .resources
                .reputation_reliability_basis_points
                < reliability_before,
            "a missed loan payment must reduce borrower reliability"
        );
    }

    #[test]
    fn positive_annual_interest_does_not_disappear_from_weekly_accrual() {
        let mut state = make_test_campaign();
        let loan_id = current_loan_id(&state);
        let borrower_id = {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.balance = Money::from_copper(1_000);
            loan.interest_basis_points = 500;
            loan.next_due_day = state.clock.day();
            loan.borrower_dynasty_id
        };
        state
            .dynasties
            .get_mut(&borrower_id)
            .expect("borrower must exist")
            .resources
            .treasury = Money::ZERO;

        settle_loans(&mut state).expect("loan settlement must succeed");

        assert_eq!(
            state.loans.get(&loan_id).expect("loan must exist").balance,
            Money::from_copper(1_001),
            "positive annual interest must accrue when its weekly share is fractional"
        );
    }

    #[test]
    fn successful_payment_improves_borrower_reliability() {
        let mut state = make_test_campaign();
        let loan_id = current_loan_id(&state);
        let borrower_id = {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.balance = Money::from_copper(1_000);
            loan.weekly_payment = Money::from_copper(100);
            loan.next_due_day = state.clock.day();
            loan.borrower_dynasty_id
        };
        state
            .dynasties
            .get_mut(&borrower_id)
            .expect("borrower must exist")
            .resources
            .treasury = Money::from_copper(10_000);
        let reliability_before = state
            .dynasties
            .get(&borrower_id)
            .expect("borrower must exist")
            .resources
            .reputation_reliability_basis_points;

        settle_loans(&mut state).expect("loan settlement must succeed");

        assert_eq!(
            state
                .dynasties
                .get(&borrower_id)
                .expect("borrower must exist")
                .resources
                .reputation_reliability_basis_points,
            reliability_before.saturating_add(10).min(10_000)
        );
    }

    #[test]
    fn civic_debt_payment_moves_treasury_budget_to_the_creditor() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let (debt_id, creditor_dynasty_id) = insert_test_civic_debt(&mut state);
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define a treasury");
        state
            .institutions
            .get_mut(&treasury_id)
            .expect("treasury runtime must exist")
            .budget = Money::from_copper(1_000);
        let creditor_before = state
            .dynasties
            .get(&creditor_dynasty_id)
            .expect("creditor must exist")
            .treasury();
        let pair = DynastyPair::new(state.player_dynasty_id, creditor_dynasty_id);
        state
            .relationships
            .get_mut(&pair)
            .expect("creditor relationship must exist")
            .obligation = 1;

        settle_civic_debts(registry, &mut state).expect("civic debt settlement must succeed");

        let debt = state.civic_debts.get(&debt_id).expect("debt must exist");
        assert_eq!(debt.balance, Money::from_copper(900));
        assert_eq!(debt.status, CivicDebtStatus::Current);
        assert_eq!(
            state
                .institutions
                .get(&treasury_id)
                .expect("treasury runtime must exist")
                .budget,
            Money::from_copper(900)
        );
        assert_eq!(
            state
                .dynasties
                .get(&creditor_dynasty_id)
                .expect("creditor must exist")
                .treasury(),
            creditor_before.saturating_add(Money::from_copper(100))
        );
        assert_eq!(
            state
                .relationships
                .get(&pair)
                .expect("creditor relationship must exist")
                .obligation,
            1,
            "partial repayment must not clear the municipal-credit obligation"
        );
    }

    #[test]
    fn final_civic_debt_payment_marks_the_obligation_repaid() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let (debt_id, creditor_dynasty_id) = insert_test_civic_debt(&mut state);
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define a treasury");
        state
            .institutions
            .get_mut(&treasury_id)
            .expect("treasury runtime must exist")
            .budget = Money::from_copper(100);
        {
            let debt = state
                .civic_debts
                .get_mut(&debt_id)
                .expect("debt must exist");
            debt.balance = Money::from_copper(100);
            debt.weekly_payment = Money::from_copper(100);
        }
        let pair = DynastyPair::new(state.player_dynasty_id, creditor_dynasty_id);
        state
            .relationships
            .get_mut(&pair)
            .expect("creditor relationship must exist")
            .obligation = 1;

        settle_civic_debts(registry, &mut state).expect("civic debt settlement must succeed");

        let debt = state.civic_debts.get(&debt_id).expect("debt must exist");
        assert_eq!(debt.balance, Money::ZERO);
        assert_eq!(debt.status, CivicDebtStatus::Repaid);
        assert_eq!(debt.missed_payments, 0);
        assert_eq!(
            state
                .relationships
                .get(&pair)
                .expect("creditor relationship must exist")
                .obligation,
            0,
            "full repayment must clear the municipal-credit obligation"
        );
        assert!(state.outbox.iter().any(|message| {
            message.kind == OutboxKind::Finance && message.subject.contains("repaid")
        }));
        assert!(state.information_reports.values().any(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == "Completed municipal debt repayment records"
        }));
    }

    #[test]
    fn interest_limit_caps_civic_debt_accrual_without_rewriting_terms() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let (debt_id, _) = insert_test_civic_debt(&mut state);
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define a treasury");
        state
            .institutions
            .get_mut(&treasury_id)
            .expect("treasury runtime must exist")
            .budget = Money::from_copper(100);
        state
            .civic_debts
            .get_mut(&debt_id)
            .expect("debt must exist")
            .interest_basis_points = 1_000;
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::InterestLimit,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: None,
                value: 0,
                active: true,
            },
        );

        settle_civic_debts(registry, &mut state).expect("civic debt settlement must succeed");

        let debt = state.civic_debts.get(&debt_id).expect("debt must exist");
        assert_eq!(debt.balance, Money::from_copper(900));
        assert_eq!(debt.interest_basis_points, 1_000);
    }

    #[test]
    fn three_missed_civic_debt_payments_default_and_create_civic_pressure() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let (debt_id, _) = insert_test_civic_debt(&mut state);
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define a treasury");
        state
            .institutions
            .get_mut(&treasury_id)
            .expect("treasury runtime must exist")
            .budget = Money::ZERO;
        let player_legitimacy_before = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points;
        let unrest_before: u32 = state
            .districts
            .values()
            .map(|district| u32::from(district.unrest_basis_points))
            .sum();

        for _ in 0..3 {
            state
                .civic_debts
                .get_mut(&debt_id)
                .expect("debt must exist")
                .next_due_day = state.clock.day();
            settle_civic_debts(registry, &mut state).expect("civic debt settlement must succeed");
        }

        let debt = state.civic_debts.get(&debt_id).expect("debt must exist");
        assert_eq!(debt.status, CivicDebtStatus::Defaulted);
        assert_eq!(debt.missed_payments, 3);
        assert!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .resources
                .legitimacy_basis_points
                < player_legitimacy_before
        );
        assert!(
            state
                .districts
                .values()
                .map(|district| u32::from(district.unrest_basis_points))
                .sum::<u32>()
                > unrest_before
        );
        assert!(state.outbox.iter().any(|message| {
            message.kind == OutboxKind::Finance && message.subject.contains("defaulted")
        }));
        assert!(state.information_reports.values().any(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == "Municipal debt default and civic treasury records"
        }));
    }
}

mod laws {
    use super::*;

    #[test]
    fn interest_limit_caps_settlement_without_rewriting_terms() {
        let mut state = make_test_campaign();
        let loan_id = current_loan_id(&state);
        let borrower_id = {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.balance = Money::from_copper(52_000);
            loan.weekly_payment = Money::from_copper(1_000);
            loan.interest_basis_points = 1_000;
            loan.next_due_day = state.clock.day();
            loan.borrower_dynasty_id
        };
        state
            .dynasties
            .get_mut(&borrower_id)
            .expect("borrower must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::InterestLimit,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: None,
                value: 0,
                active: true,
            },
        );

        settle_loans(&mut state).expect("loan settlement must succeed");

        let loan = state.loans.get(&loan_id).expect("loan must exist");
        assert_eq!(
            loan.interest_basis_points, 1_000,
            "statutory limits must not rewrite the agreed contract rate"
        );
        assert_eq!(
            loan.balance,
            Money::from_copper(51_000),
            "a zero-rate statutory cap must prevent interest accrual for settlement"
        );
    }

    #[test]
    fn foreign_toll_reduces_route_supply_and_updates_runtime_rate() {
        let mut state = make_test_campaign();
        let law = state
            .laws
            .values_mut()
            .find(|law| law.kind == LawKind::ForeignMerchantToll && law.active)
            .expect("bootstrap must enact a foreign toll");
        law.value = 2_500;
        let route_id = *state
            .external_routes
            .keys()
            .next()
            .expect("bootstrap must create an external route");
        let (good_id, capacity) = {
            let route = state
                .external_routes
                .get_mut(&route_id)
                .expect("route must exist");
            route.disruption_basis_points = 0;
            (route.good_id, route.daily_capacity)
        };
        let stock_before = state
            .market
            .get_quote(good_id)
            .expect("route good quote must exist")
            .stock;

        apply_route_laws(&mut state);
        apply_external_route_supply(&mut state);

        let route = state
            .external_routes
            .get(&route_id)
            .expect("route must exist");
        assert_eq!(route.toll_basis_points, 2_500);
        assert_eq!(
            state
                .market
                .get_quote(good_id)
                .expect("route good quote must exist")
                .stock,
            stock_before.saturating_add(capacity.saturating_mul_ratio(7_500, 10_000))
        );
    }

    #[test]
    fn rent_restriction_caps_payment_without_rewriting_the_lease() {
        let mut state = make_test_campaign();
        let property_id = *state
            .properties
            .iter()
            .find_map(|(id, property)| property.owner_dynasty_id.map(|_| id))
            .expect("bootstrap must create an owned property");
        let owner_id = state
            .properties
            .get(&property_id)
            .and_then(|property| property.owner_dynasty_id)
            .expect("property must have owner");
        let tenant_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != owner_id)
            .expect("campaign must include a second dynasty");
        let (owner_id, tenant_id) = {
            let property = state
                .properties
                .get_mut(&property_id)
                .expect("property must exist");
            property.value = Money::from_copper(52_000);
            property.weekly_rent = Money::from_copper(1_000);
            property.tenant_dynasty_id = Some(tenant_id);
            (
                property.owner_dynasty_id.expect("property must have owner"),
                property
                    .tenant_dynasty_id
                    .expect("property must have tenant"),
            )
        };
        state
            .dynasties
            .get_mut(&tenant_id)
            .expect("tenant must exist")
            .resources
            .treasury = Money::from_copper(10_000);
        let owner_before = state
            .dynasties
            .get(&owner_id)
            .expect("owner must exist")
            .treasury();
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::RentRestriction,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: None,
                value: 1_000,
                active: true,
            },
        );

        settle_property_rents(&mut state).expect("property rent settlement must succeed");

        assert_eq!(
            state
                .dynasties
                .get(&owner_id)
                .expect("owner must exist")
                .treasury(),
            owner_before.saturating_add(Money::from_copper(100))
        );
        assert_eq!(
            state
                .properties
                .get(&property_id)
                .expect("property must exist")
                .weekly_rent,
            Money::from_copper(1_000)
        );
    }

    #[test]
    fn emergency_imports_use_the_enacted_quantity() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let grain_id = registry
            .get_good_id("grain")
            .expect("registry must define grain");
        let stock_before = state
            .market
            .get_quote(grain_id)
            .expect("grain quote must exist")
            .stock;
        let supply_before = state
            .market
            .get_quote(grain_id)
            .expect("grain quote must exist")
            .supply_today;
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::EmergencyImports,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: None,
                value: 7,
                active: true,
            },
        );

        apply_law_economic_effects(registry, &mut state);

        let quote = state
            .market
            .get_quote(grain_id)
            .expect("grain quote must exist");
        assert_eq!(
            quote.stock,
            stock_before.saturating_add(Quantity::from_units(7))
        );
        assert_eq!(
            quote.supply_today,
            supply_before.saturating_add(Quantity::from_units(7))
        );
    }

    #[test]
    fn fire_code_reduces_fire_probability_and_severity() {
        let safety = 2_000;

        assert!(
            urban_fire_probability_basis_points(safety, 8_000)
                < urban_fire_probability_basis_points(safety, 0),
            "stronger fire codes must reduce outbreak probability"
        );
        assert!(
            urban_fire_severity_basis_points(safety, 8_000)
                < urban_fire_severity_basis_points(safety, 0),
            "stronger fire codes must reduce fire severity"
        );
    }
}

mod legal_cases {
    use super::*;

    #[test]
    fn filed_case_enters_hearing_before_judgment() {
        let mut state = make_test_campaign();
        let legal_case_id = *state
            .legal_cases
            .keys()
            .next()
            .expect("campaign must contain a legal case");
        {
            let legal_case = state
                .legal_cases
                .get_mut(&legal_case_id)
                .expect("legal case must exist");
            legal_case.status = LegalCaseStatus::Filed;
            legal_case.hearing_day = state.clock.day().saturating_add(30);
        }
        let outbox_before = state.outbox.len();

        advance_legal_case_hearings(&mut state);

        assert_eq!(
            state
                .legal_cases
                .get(&legal_case_id)
                .expect("legal case must exist")
                .status,
            LegalCaseStatus::Hearing
        );
        assert_eq!(state.outbox.len(), outbox_before + 1);
        assert!(
            state
                .outbox
                .last()
                .expect("hearing notification must exist")
                .subject
                .contains("entered hearing")
        );
    }
}

mod crises {
    use super::*;

    #[test]
    fn grain_shortage_adds_demand_without_changing_target_stock() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let bread_id = registry
            .get_good_id("bread")
            .expect("registry must define bread");
        let target_before = state
            .market
            .get_quote(bread_id)
            .expect("bread quote must exist")
            .target_stock;
        insert_crisis(
            &mut state,
            CrisisKind::GrainShortage,
            None,
            5_000,
            "test shortage",
        );

        apply_crisis_daily_effects(registry, &mut state).expect("daily crisis effects must apply");

        let quote = state
            .market
            .get_quote(bread_id)
            .expect("bread quote must exist");
        assert_eq!(
            quote.target_stock, target_before,
            "temporary crisis pressure must not rewrite the market baseline"
        );
        assert!(
            quote.demand_today > Quantity::ZERO,
            "an active grain shortage must add bread demand"
        );
    }

    #[test]
    fn severe_crisis_enters_escalated_status() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let crisis_id = insert_crisis(
            &mut state,
            CrisisKind::GrainShortage,
            None,
            8_500,
            "test escalation",
        );

        detect_and_advance_crises(registry, &mut state);

        assert_eq!(
            state
                .crises
                .get(&crisis_id)
                .expect("crisis must exist")
                .status,
            CrisisStatus::Escalated
        );
    }

    #[test]
    fn unaddressed_crisis_intensifies_and_notifies_on_escalation() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let crisis_id = insert_crisis(
            &mut state,
            CrisisKind::NobleDemand,
            None,
            7_900,
            "test escalation",
        );
        let outbox_before = state.outbox.len();

        detect_and_advance_crises(registry, &mut state);

        let crisis = state.crises.get(&crisis_id).expect("crisis must exist");
        assert_eq!(crisis.status, CrisisStatus::Escalated);
        assert_eq!(
            crisis.severity_basis_points,
            7_900 + UNADDRESSED_CRISIS_MONTHLY_ESCALATION_BASIS_POINTS
        );
        assert_eq!(state.outbox.len(), outbox_before + 1);
        assert!(
            state
                .outbox
                .last()
                .expect("escalation notification must exist")
                .subject
                .contains("escalated")
        );
    }

    #[test]
    fn exploited_crisis_remains_uncontained_and_intensifies() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let crisis_id = insert_crisis(
            &mut state,
            CrisisKind::NobleDemand,
            None,
            4_500,
            "test exploitation",
        );
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CrisisResponse,
            subject: format!("crisis:{crisis_id}"),
            detail: "response=Exploit".to_owned(),
        });

        detect_and_advance_crises(registry, &mut state);

        let crisis = state.crises.get(&crisis_id).expect("crisis must exist");
        assert_eq!(crisis.status, CrisisStatus::Active);
        assert_eq!(
            crisis.severity_basis_points,
            4_500 + UNADDRESSED_CRISIS_MONTHLY_ESCALATION_BASIS_POINTS
        );
    }

    #[test]
    fn addressed_crisis_resolution_adds_durable_notification() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let crisis_id = insert_crisis(
            &mut state,
            CrisisKind::NobleDemand,
            None,
            550,
            "test resolution",
        );
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CrisisResponse,
            subject: format!("crisis:{crisis_id}"),
            detail: "response=Reform".to_owned(),
        });
        let outbox_before = state.outbox.len();

        detect_and_advance_crises(registry, &mut state);

        assert_eq!(
            state
                .crises
                .get(&crisis_id)
                .expect("crisis must exist")
                .status,
            CrisisStatus::Resolved
        );
        assert_eq!(state.outbox.len(), outbox_before + 1);
        assert!(
            state
                .outbox
                .last()
                .expect("resolution notification must exist")
                .subject
                .contains("resolved"),
            "natural resolution must be visible to adapters"
        );
    }

    #[test]
    fn resolved_banking_panic_is_not_recreated_from_the_same_defaults() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let loan_ids: Vec<_> = state.loans.keys().copied().take(2).collect();
        let [first_loan_id, second_loan_id] = loan_ids.as_slice() else {
            panic!("fixture must contain at least two loans: {loan_ids:?}");
        };
        for loan_id in [*first_loan_id, *second_loan_id] {
            state
                .loans
                .get_mut(&loan_id)
                .expect("loan must exist")
                .status = LoanStatus::Defaulted;
        }
        let crisis_id = insert_crisis(
            &mut state,
            CrisisKind::BankingPanic,
            None,
            400,
            "historical panic",
        );
        {
            let crisis = state.crises.get_mut(&crisis_id).expect("crisis must exist");
            crisis.severity_basis_points = 0;
            crisis.status = CrisisStatus::Resolved;
        }

        detect_and_advance_crises(registry, &mut state);

        assert_eq!(
            state
                .crises
                .values()
                .filter(|crisis| crisis.kind == CrisisKind::BankingPanic)
                .count(),
            1,
            "historical defaults must not generate an endless sequence of duplicate panics"
        );
    }

    #[test]
    fn severe_route_disruption_creates_trade_crisis() {
        let mut state = make_test_campaign();
        for route in state.external_routes.values_mut() {
            route.disruption_basis_points = 7_500;
        }

        detect_trade_disruption(&mut state);

        assert!(
            has_active_crisis(&state, CrisisKind::TradeDisruption),
            "severe network-wide disruption must create a trade crisis"
        );
    }

    #[test]
    fn banking_panic_records_business_liquidity_losses() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .find(|business| business.cash() > Money::from_copper(100))
            .expect("bootstrap must create a funded business")
            .id();
        let before = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .finance
            .clone();
        insert_crisis(
            &mut state,
            CrisisKind::BankingPanic,
            None,
            10_000,
            "test panic",
        );

        apply_crisis_daily_effects(registry, &mut state).expect("daily crisis effects must apply");

        let after = &state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .finance;
        assert!(
            after.cash < before.cash,
            "a banking panic must reduce liquid business cash"
        );
        assert!(
            after.lifetime_costs > before.lifetime_costs,
            "liquidity losses must be recorded as business costs"
        );
        assert!(
            after.version > before.version,
            "financial mutation must invalidate stale business tokens"
        );
    }

    #[test]
    fn noble_demand_drains_civic_treasury_and_raises_unrest() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define treasury");
        let district_id = *state
            .districts
            .keys()
            .next()
            .expect("bootstrap must create a district");
        let treasury_before = state
            .institutions
            .get(&treasury_id)
            .expect("treasury runtime must exist")
            .budget;
        let unrest_before = state
            .districts
            .get(&district_id)
            .expect("district must exist")
            .unrest_basis_points;
        insert_crisis(
            &mut state,
            CrisisKind::NobleDemand,
            Some(district_id),
            10_000,
            "test demand",
        );

        apply_crisis_daily_effects(registry, &mut state).expect("daily crisis effects must apply");

        let treasury_after = state
            .institutions
            .get(&treasury_id)
            .expect("treasury runtime must exist")
            .budget;
        assert!(
            treasury_after < treasury_before,
            "an active noble demand must levy civic funds"
        );
        assert!(
            treasury_after >= Money::ZERO,
            "a levy must never make the civic treasury negative"
        );
        assert!(
            state
                .districts
                .get(&district_id)
                .expect("district must exist")
                .unrest_basis_points
                > unrest_before,
            "noble extraction must increase local unrest"
        );
    }

    #[test]
    fn guild_revolt_requires_material_pressure() {
        assert_eq!(
            guild_revolt_probability_basis_points(0, 0),
            0,
            "a city without disputes or restrictive guild law must not roll for revolt"
        );
        assert!(
            guild_revolt_probability_basis_points(1, 0) > 0,
            "an actual labor dispute must create revolt pressure"
        );
        assert!(
            guild_revolt_probability_basis_points(0, 5_000) > 0,
            "restrictive guild law must create revolt pressure"
        );
    }
}

mod districts {
    use super::*;

    #[test]
    fn rent_pressure_uses_the_full_safety_and_sanitation_signal() {
        let mut state = make_test_campaign();
        let district_id = *state
            .districts
            .keys()
            .next()
            .expect("campaign must contain a district");
        let district = state
            .districts
            .get_mut(&district_id)
            .expect("district must exist");
        district.safety_basis_points = 10_000;
        district.sanitation_basis_points = 10_000;

        update_district_conditions(&mut state);

        assert_eq!(
            state
                .districts
                .get(&district_id)
                .expect("district must exist")
                .rent_index_basis_points,
            13_666,
            "maximum desirability must produce meaningful rent pressure below the hard cap"
        );
    }

    #[test]
    fn rent_pressure_respects_the_domain_floor() {
        let mut state = make_test_campaign();
        let district_id = *state
            .districts
            .keys()
            .next()
            .expect("campaign must contain a district");
        let district = state
            .districts
            .get_mut(&district_id)
            .expect("district must exist");
        district.safety_basis_points = 0;
        district.sanitation_basis_points = 0;

        update_district_conditions(&mut state);

        assert_eq!(
            state
                .districts
                .get(&district_id)
                .expect("district must exist")
                .rent_index_basis_points,
            crate::systems::MIN_DISTRICT_RENT_INDEX_BASIS_POINTS
        );
    }
}

mod routes {
    use super::*;

    #[test]
    fn recover_by_the_monthly_recovery_amount() {
        let mut state = make_test_campaign();
        for route in state.external_routes.values_mut() {
            route.disruption_basis_points = 7_500;
        }

        recover_external_routes(&mut state);

        for route in state.external_routes.values() {
            assert_eq!(
                route.disruption_basis_points, 6_750,
                "monthly recovery must remove exactly 750 basis points"
            );
        }
    }

    #[test]
    fn active_trade_crisis_reduces_supply_on_the_same_day() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        state.crises.clear();
        for law in state.laws.values_mut() {
            if law.kind == LawKind::ForeignMerchantToll {
                law.active = false;
            }
        }
        let route_id = *state
            .external_routes
            .keys()
            .next()
            .expect("campaign must contain an external route");
        for route in state.external_routes.values_mut() {
            route.active = false;
        }
        let (good_id, daily_capacity) = {
            let route = state
                .external_routes
                .get_mut(&route_id)
                .expect("selected route must exist");
            route.active = true;
            route.disruption_basis_points = 0;
            route.toll_basis_points = 0;
            (route.good_id, route.daily_capacity)
        };
        let stock_before = state
            .market
            .get_quote(good_id)
            .expect("route good must have a quote")
            .stock();
        insert_crisis(
            &mut state,
            CrisisKind::TradeDisruption,
            None,
            8_000,
            "test route disruption",
        );

        run_daily_strategic_systems(registry, &mut state)
            .expect("daily strategic systems must run");

        let stock_after = state
            .market
            .get_quote(good_id)
            .expect("route good must have a quote")
            .stock();
        assert_eq!(
            stock_after.saturating_sub(stock_before),
            daily_capacity.saturating_mul_ratio(2_000, 10_000),
            "trade disruption must constrain route supply before that day's imports arrive"
        );
    }

    #[test]
    fn monthly_detection_precedes_route_recovery() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        state.crises.clear();
        for route in state.external_routes.values_mut() {
            route.risk_basis_points = 0;
            route.disruption_basis_points = 0;
        }
        state
            .external_routes
            .values_mut()
            .next()
            .expect("campaign must contain an external route")
            .disruption_basis_points = 7_300;

        run_monthly_strategic_systems(registry, &mut state)
            .expect("monthly strategic systems must run");

        assert!(
            has_active_crisis(&state, CrisisKind::TradeDisruption),
            "severe disruption must be detected before routine monthly recovery masks it"
        );
    }
}

mod ai {
    use super::*;

    #[test]
    fn stalled_objectives_are_abandoned_and_replaced() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let objective_id = state
            .ai_objectives
            .values()
            .find(|objective| objective.status == ObjectiveStatus::Pursuing)
            .expect("campaign must contain a pursuing AI objective")
            .id;
        let dynasty_id = state
            .ai_objectives
            .get(&objective_id)
            .expect("objective must exist")
            .dynasty_id;
        {
            let objective = state
                .ai_objectives
                .get_mut(&objective_id)
                .expect("objective must exist");
            objective.kind = ObjectiveKind::AccumulateCash;
            objective.created_day = 0;
        }
        for objective in state
            .ai_objectives
            .values_mut()
            .filter(|objective| objective.id != objective_id)
        {
            objective.status = ObjectiveStatus::Planned;
        }
        state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("objective dynasty must exist")
            .resources
            .treasury = Money::ZERO;
        for _ in 0..AI_OBJECTIVE_REVIEW_DAYS {
            state.clock.advance_one_day();
        }
        let objective_count_before = state.ai_objectives.len();

        advance_ai_objectives(registry, &mut state);

        let abandoned = state
            .ai_objectives
            .get(&objective_id)
            .expect("original objective must remain traceable");
        assert_eq!(abandoned.status, ObjectiveStatus::Abandoned);
        assert!(abandoned.rationale.contains("abandoned this route"));
        assert_eq!(state.ai_objectives.len(), objective_count_before + 1);
        assert!(state.ai_objectives.values().any(|objective| {
            objective.id != objective_id
                && objective.dynasty_id == dynasty_id
                && objective.status == ObjectiveStatus::Pursuing
                && objective.kind == ObjectiveKind::AcquireProperty
                && objective.created_day == state.clock.day()
        }));
    }

    #[test]
    fn defaulted_occupied_collateral_becomes_a_tenancy() {
        let mut state = make_test_campaign();
        let property = state
            .properties
            .values()
            .find(|property| {
                property.occupant_business_id.is_some()
                    && property.owner_dynasty_id.is_some()
                    && property.tenant_dynasty_id.is_none()
                    && property.collateral_loan_id.is_none()
            })
            .expect("campaign must contain owner-occupied unpledged business premises");
        let property_id = property.id;
        let borrower_id = property
            .owner_dynasty_id
            .expect("occupied property must have an owner");
        let lender_id = state
            .dynasties
            .values()
            .filter(|dynasty| dynasty.id() != borrower_id)
            .filter(|dynasty| dynasty.treasury() >= Money::from_copper(1))
            .find(|dynasty| {
                !state.loans.values().any(|loan| {
                    loan.lender_dynasty_id == dynasty.id()
                        && loan.borrower_dynasty_id == borrower_id
                        && loan.status != LoanStatus::Repaid
                })
            })
            .map(crate::core::Dynasty::id)
            .expect("campaign must contain an eligible lender");
        let loan_id = issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: lender_id,
                borrower_dynasty_id: borrower_id,
                principal: Money::from_copper(1),
                weekly_payment: Money::from_copper(1),
                interest_basis_points: 0,
                collateral_property_id: Some(property_id),
            },
        )
        .expect("occupied property must be accepted as collateral");
        state
            .dynasties
            .get_mut(&borrower_id)
            .expect("borrower must exist")
            .resources
            .treasury = Money::ZERO;
        let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
        loan.missed_payments = 2;
        loan.next_due_day = state.clock.day();

        settle_loans(&mut state).expect("loan settlement must succeed");

        assert_eq!(
            state.loans.get(&loan_id).expect("loan must exist").status,
            LoanStatus::Defaulted
        );
        let property = state
            .properties
            .get(&property_id)
            .expect("collateral property must remain");
        assert_eq!(property.owner_dynasty_id, Some(lender_id));
        assert_eq!(property.tenant_dynasty_id, Some(borrower_id));
        assert_eq!(property.collateral_loan_id, None);
        validate_invariants(test_registry(), &state);
    }

    #[test]
    fn debt_repayment_transfers_money_and_releases_collateral() {
        let mut state = make_test_campaign();
        let loan_id = state
            .loans
            .values()
            .find(|loan| loan.collateral_property_id.is_some())
            .expect("bootstrap must create a collateralized loan")
            .id;
        let (lender_id, borrower_id, property_id) = {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.balance = Money::from_copper(500);
            (
                loan.lender_dynasty_id,
                loan.borrower_dynasty_id,
                loan.collateral_property_id
                    .expect("selected loan must have collateral"),
            )
        };
        let lender_before = state
            .dynasties
            .get(&lender_id)
            .expect("lender must exist")
            .treasury();
        let borrower_before = state
            .dynasties
            .get(&borrower_id)
            .expect("borrower must exist")
            .treasury();

        assert_eq!(
            advance_ai_debt_objective(&mut state, borrower_id),
            ObjectiveProgress::Achieved,
            "the AI objective must report completion after full repayment"
        );

        assert_eq!(
            state.loans.get(&loan_id).expect("loan must exist").status,
            LoanStatus::Repaid
        );
        assert_eq!(
            state
                .properties
                .get(&property_id)
                .expect("property must exist")
                .collateral_loan_id,
            None
        );
        assert_eq!(
            state
                .dynasties
                .get(&lender_id)
                .expect("lender must exist")
                .treasury(),
            lender_before.saturating_add(Money::from_copper(500))
        );
        assert_eq!(
            state
                .dynasties
                .get(&borrower_id)
                .expect("borrower must exist")
                .treasury(),
            borrower_before.saturating_sub(Money::from_copper(500))
        );
    }

    #[test]
    fn supply_objective_skips_inactive_businesses_and_tries_viable_alternatives() {
        let registry = test_registry();
        let mut state = make_test_campaign();
        let dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != state.player_dynasty_id
                    && state
                        .businesses
                        .ids_for_owner(*dynasty_id)
                        .is_some_and(|ids| ids.len() >= 2)
            })
            .expect("campaign must contain a nonplayer dynasty with multiple businesses");
        let business_ids: Vec<_> = state
            .businesses
            .ids_for_owner(dynasty_id)
            .expect("selected dynasty must own businesses")
            .iter()
            .copied()
            .take(2)
            .collect();
        let [inactive_id, viable_id] = business_ids.as_slice() else {
            panic!("selected dynasty must own at least two businesses: {business_ids:?}");
        };
        let inactive_id = *inactive_id;
        let viable_id = *viable_id;
        state
            .businesses
            .get_mut(inactive_id)
            .expect("inactive business must exist")
            .operations
            .status = BusinessStatus::Closed;
        state
            .businesses
            .get_mut(inactive_id)
            .expect("inactive business must exist")
            .identity
            .recipe_id = registry
            .get_recipe_id("milling")
            .expect("registry must define milling");
        state
            .businesses
            .get_mut(viable_id)
            .expect("viable business must exist")
            .operations
            .status = BusinessStatus::Active;
        state
            .businesses
            .get_mut(viable_id)
            .expect("viable business must exist")
            .identity
            .recipe_id = registry
            .get_recipe_id("weaving")
            .expect("registry must define weaving");
        state.contracts.retain(|_, contract| {
            contract.buyer_business_id != inactive_id && contract.buyer_business_id != viable_id
        });

        assert_eq!(
            advance_ai_supply_objective(registry, &mut state, dynasty_id),
            ObjectiveProgress::Achieved
        );
        assert!(state.contracts.values().any(|contract| {
            contract.buyer_business_id == viable_id && contract.status == ContractStatus::Active
        }));
    }

    #[test]
    fn legitimacy_objectives_do_not_progress_without_spending() {
        let mut state = make_test_campaign();
        let dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain a nonplayer dynasty");
        let legitimacy_before = {
            let dynasty = state
                .dynasties
                .get_mut(&dynasty_id)
                .expect("AI dynasty must exist");
            dynasty.resources.treasury = Money::ZERO;
            dynasty.resources.legitimacy_basis_points
        };

        assert_eq!(
            advance_ai_legitimacy_objective(&mut state, dynasty_id),
            ObjectiveProgress::Pending
        );
        assert_eq!(
            state
                .dynasties
                .get(&dynasty_id)
                .expect("AI dynasty must exist")
                .resources
                .legitimacy_basis_points,
            legitimacy_before,
            "bankrupt AI dynasties must not gain legitimacy for free"
        );

        advance_ai_office_objective(&mut state, dynasty_id);
        assert_eq!(
            state
                .dynasties
                .get(&dynasty_id)
                .expect("AI dynasty must exist")
                .resources
                .legitimacy_basis_points,
            legitimacy_before,
            "office objectives must also scale progress to actual spending"
        );
    }
}
