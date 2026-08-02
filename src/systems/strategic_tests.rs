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
    let loan = state
        .loans
        .values()
        .find(|loan| loan.status == LoanStatus::Current)
        .expect("bootstrap must create a current loan");
    LoanTerms {
        lender_dynasty_id: loan.lender_dynasty_id,
        borrower_dynasty_id: loan.borrower_dynasty_id,
        principal: Money::from_copper(1),
        weekly_payment: Money::from_copper(1),
        interest_basis_points: 500,
        collateral_property_id: None,
    }
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

mod integration {
    use super::*;

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

mod contracts {
    use super::*;

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

        settle_contracts(&mut state);

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
    }
}

mod loans {
    use super::*;

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
        let pledged_terms = LoanTerms {
            lender_dynasty_id: existing_loan.lender_dynasty_id,
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
        {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.balance = Money::from_copper(52_000);
            loan.interest_basis_points = 1_000;
            loan.next_due_day = state.clock.day();
        }

        settle_loans(&mut state);

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

        settle_loans(&mut state);

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

        settle_property_rents(&mut state);

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

        apply_crisis_daily_effects(registry, &mut state);

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

        apply_crisis_daily_effects(registry, &mut state);

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

        apply_crisis_daily_effects(registry, &mut state);

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
}

mod ai {
    use super::*;

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
}
