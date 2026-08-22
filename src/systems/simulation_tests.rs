//! Behavioral tests for daily simulation planning, ordering, and preflight validation.

use super::*;
use crate::core::{
    BusinessStatus, ContractStatus, EnactedLaw, LawKind, NewGameConfig, StartingBackground,
};
use crate::ids::{FamilyLinkId, GoodId, InstitutionId};
use crate::systems::{build_new_game, validate_invariants};
use crate::test_support::{
    assert_state_unchanged, make_test_campaign, rivergate_registry_for_test,
};

mod preflight {
    use super::*;

    #[test]
    fn exhausted_day_range_fails_before_day_mutation() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("campaign must serialize");
        value["clock"]["day"] = serde_json::Value::from(i64::MAX);
        let mut state: AppState =
            serde_json::from_value(value).expect("modified campaign must deserialize");
        let before = state.clone();

        let result = advance_days(registry, &mut state, 1);

        assert_eq!(
            result,
            Err(SimulationError::DayRangeExhausted {
                current_day: i64::MAX,
                requested_days: 1,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "day-range exhaustion must leave the entire campaign unchanged",
        );
    }

    #[test]
    fn reserved_final_day_fails_before_day_mutation() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign();
        let mut value = serde_json::to_value(state).expect("campaign must serialize");
        value["clock"]["day"] = serde_json::Value::from(i64::MAX - 1);
        let mut state: AppState =
            serde_json::from_value(value).expect("modified campaign must deserialize");
        let before = state.clone();

        let result = advance_days(registry, &mut state, 1);

        assert_eq!(
            result,
            Err(SimulationError::DayRangeExhausted {
                current_day: i64::MAX - 1,
                requested_days: 1,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "the exhausted day sentinel must remain unreachable",
        );
    }

    #[test]
    fn missing_market_quote_fails_before_day_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let missing_good_id = registry
            .goods()
            .first()
            .expect("registry must contain a market good")
            .id();
        state.market.quotes.remove(&missing_good_id);
        let before = state.clone();

        let result = advance_days(registry, &mut state, 1);

        assert_eq!(
            result,
            Err(SimulationError::MarketQuoteMissing {
                good_id: missing_good_id,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "preflight failure must leave the entire campaign unchanged",
        );
    }

    #[test]
    fn exhausted_business_finance_version_fails_before_day_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .finance
            .version = u64::MAX;
        let before = state.clone();

        let result = advance_days(registry, &mut state, 1);

        assert_eq!(
            result,
            Err(SimulationError::BusinessFinanceVersionExhausted { business_id })
        );
        assert_state_unchanged(
            &before,
            &state,
            "finance-version exhaustion must fail before any daily system mutates state",
        );
    }

    #[test]
    fn automatic_cost_overflow_leaves_the_requested_advance_uncommitted() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .finance
            .lifetime_costs = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = advance_days(registry, &mut state, 1);

        assert!(
            matches!(
                result,
                Err(SimulationError::BusinessLifetimeCostsOverflow {
                    business_id: failed_business_id,
                    ..
                }) if failed_business_id == business_id
            ),
            "automatic accounting overflow must return a typed error, received {result:?}"
        );
        assert_state_unchanged(
            &before,
            &state,
            "a failed automatic finance mutation must not commit any part of the day",
        );
    }

    #[test]
    fn automatic_revenue_overflow_is_rejected_before_inventory_or_cash_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let good_id = registry
            .goods()
            .first()
            .expect("registry must contain a good")
            .id();
        let quantity = Quantity::from_units(1);
        let revenue = Money::from_copper(1);
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("business must exist");
        business.add_inventory(good_id, quantity);
        business.finance.lifetime_revenue = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = apply_business_sales(
            &mut state,
            BusinessSalePlan {
                lines: vec![BusinessSaleLine {
                    business_id,
                    good_id,
                    quantity,
                    revenue,
                }],
            },
        );

        assert_eq!(
            result,
            Err(SimulationError::BusinessLifetimeRevenueOverflow {
                business_id,
                current: Money::from_copper(i64::MAX),
                incoming: revenue,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "revenue overflow must fail before inventory, cash, market, or audit mutation",
        );
    }

    #[test]
    fn external_route_stock_overflow_aborts_the_requested_day() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let good_id = state
            .external_routes
            .values()
            .find(|route| route.active && route.daily_capacity > Quantity::ZERO)
            .expect("campaign must contain an active supply route")
            .good_id;
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("route good must have a quote")
            .stock = Quantity::from_milliunits(i64::MAX);
        let before = state.clone();

        let result = advance_days(registry, &mut state, 1);

        assert!(
            matches!(
                result,
                Err(SimulationError::MarketStockOverflow {
                    good_id: failed_good_id,
                    ..
                }) if failed_good_id == good_id
            ),
            "route stock overflow must return a typed error, received {result:?}"
        );
        assert_state_unchanged(
            &before,
            &state,
            "route stock overflow must not commit any part of the requested day",
        );
    }
}

mod transfer_boundaries {
    use super::*;

    #[test]
    fn external_income_rejects_household_cash_overflow_without_mutation() {
        let mut state = make_test_campaign();
        for household in state.households.iter_mut() {
            household.weekly_income = Money::ZERO;
        }
        let household_id = state
            .households
            .iter()
            .next()
            .expect("campaign must contain a household")
            .id();
        {
            let household = state
                .households
                .get_mut(household_id)
                .expect("household must exist");
            household.cash = Money::from_copper(i64::MAX);
            household.weekly_income = Money::from_copper(100);
        }
        let before = state.clone();

        let result = settle_weekly_external_income(&mut state);

        assert_eq!(
            result,
            Err(SimulationError::HouseholdCashOverflow {
                household_id,
                current: Money::from_copper(i64::MAX),
                incoming: Money::from_copper(100),
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "external income overflow must fail before the clearing account or household cash changes",
        );
    }

    #[test]
    fn household_purchase_rejects_market_demand_overflow_without_mutation() {
        let mut state = make_test_campaign();
        let household_id = state
            .households
            .iter()
            .next()
            .expect("campaign must contain a household")
            .id();
        let good_id = state
            .market
            .quotes
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a market good");
        let quantity = Quantity::from_milliunits(1);
        let cost = Money::from_copper(1);
        state
            .households
            .get_mut(household_id)
            .expect("household must exist")
            .cash = cost;
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("market quote must exist");
            quote.stock = quantity;
            quote.demand_today = Quantity::from_milliunits(i64::MAX);
        }
        let plan = HouseholdConsumptionPlan {
            lines: vec![HouseholdPurchaseLine {
                household_id,
                good_id,
                quantity,
                cost,
            }],
            food_satisfaction: BTreeMap::new(),
        };
        let before = state.clone();

        let result = apply_household_consumption(&mut state, plan);

        assert_eq!(
            result,
            Err(SimulationError::MarketDemandOverflow {
                good_id,
                current: Quantity::from_milliunits(i64::MAX),
                incoming: quantity,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "market demand overflow must be rejected before household cash or market stock changes",
        );
    }

    #[test]
    fn household_purchase_rejects_clearing_credit_overflow_without_mutation() {
        let mut state = make_test_campaign();
        let household_id = state
            .households
            .iter()
            .next()
            .expect("campaign must contain a household")
            .id();
        let good_id = state
            .market
            .quotes
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a market good");
        let quantity = Quantity::from_milliunits(1);
        let cost = Money::from_copper(1);
        state
            .households
            .get_mut(household_id)
            .expect("household must exist")
            .cash = cost;
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("market quote must exist");
            quote.stock = quantity;
            quote.demand_today = Quantity::ZERO;
        }
        state.market.clearing_account = Money::from_copper(i64::MAX);
        let plan = HouseholdConsumptionPlan {
            lines: vec![HouseholdPurchaseLine {
                household_id,
                good_id,
                quantity,
                cost,
            }],
            food_satisfaction: BTreeMap::new(),
        };
        let before = state.clone();

        let result = apply_household_consumption(&mut state, plan);

        assert_eq!(
            result,
            Err(SimulationError::MarketClearingAccountOverflow {
                current: Money::from_copper(i64::MAX),
                change: cost,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "clearing-account overflow must be rejected before household cash or market stock changes",
        );
    }

    #[test]
    fn external_income_aggregate_overflow_is_atomic() {
        let mut state = make_test_campaign();
        for household in state.households.iter_mut() {
            household.cash = Money::ZERO;
            household.weekly_income = Money::ZERO;
        }
        let household_ids: Vec<_> = state
            .households
            .iter()
            .take(2)
            .map(crate::core::Household::id)
            .collect();
        let [first_id, second_id] = household_ids.as_slice() else {
            panic!("campaign must contain at least two households");
        };
        let payment = Money::from_copper(i64::MAX / 2 + 1);
        state
            .households
            .get_mut(*first_id)
            .expect("first household must exist")
            .weekly_income = payment;
        state
            .households
            .get_mut(*second_id)
            .expect("second household must exist")
            .weekly_income = payment;
        state.market.clearing_account = Money::from_copper(i64::MAX);
        let before = state.clone();

        let result = settle_weekly_external_income(&mut state);

        assert_eq!(
            result,
            Err(SimulationError::WeeklyExternalIncomeOverflow {
                accumulated: payment,
                incoming: payment,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "aggregate income overflow must fail before crediting any household",
        );
    }

    #[test]
    fn clamped_external_income_conserves_the_paid_out_total() {
        let mut state = make_test_campaign();
        for household in state.households.iter_mut() {
            household.cash = Money::ZERO;
        }
        // Deliberately awkward per-household amounts so truncated pro-rated
        // shares would strand copper if the rounding remainder were dropped.
        let mut income = 1_000_i64;
        for household in state.households.iter_mut() {
            household.weekly_income = Money::from_copper(income);
            income += 7;
        }
        let promised: i64 = state
            .households
            .iter()
            .map(|household| household.weekly_income.copper())
            .sum();
        let pool_before = promised / 3;
        state.market.clearing_account = Money::from_copper(pool_before);

        settle_weekly_external_income(&mut state).expect("clamped settlement must commit");

        let total_credited: i64 = state
            .households
            .iter()
            .map(|household| household.cash.copper())
            .sum();
        assert_eq!(
            i128::from(total_credited),
            i128::from(pool_before),
            "every copper debited from the clearing account must reach a household"
        );
        assert_eq!(
            state.market.clearing_account,
            Money::ZERO,
            "the drained pool must be fully distributed"
        );
        validate_invariants(rivergate_registry_for_test(), &state);
    }

    #[test]
    fn production_audit_total_remains_exact_above_quantity_range() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let output_quantity = Quantity::from_milliunits(i64::MAX / 2 + 1_000_000);
        let lines: Vec<_> = state
            .businesses
            .iter()
            .take(2)
            .map(|business| {
                let recipe = registry
                    .get_recipe(business.recipe_id())
                    .expect("business recipe must exist");
                ProductionLine {
                    business_id: business.id(),
                    inputs: Vec::new(),
                    output_good_id: recipe.output_good_id(),
                    output_quantity,
                    operating_cost: Money::ZERO,
                    tool_quantity: Quantity::ZERO,
                    tool_cost: Money::ZERO,
                }
            })
            .collect();
        assert_eq!(
            lines.len(),
            2,
            "campaign must contain at least two businesses"
        );
        let expected_output = i128::from(output_quantity.milliunits()) * 2;
        assert!(expected_output > i128::from(i64::MAX));

        let tools_id = registry
            .get_good_id("tools")
            .expect("Rivergate registry must define tools");
        apply_production(&mut state, ProductionPlan { tools_id, lines })
            .expect("production must succeed");

        let audit = state
            .audit_log
            .last()
            .expect("production must emit an audit record");
        assert_eq!(audit.kind(), AuditKind::Production);
        assert_eq!(
            audit.detail(),
            format!("output={expected_output}; operating_cost=0; tools=0; tool_spending=0")
        );
    }
}

mod starting_economies {
    use super::*;

    #[test]
    fn production_overhead_creates_tool_demand_without_extra_business_cost() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        reset_market_flows(&mut state);
        let plan = decide_production(registry, &state);
        let tools_id = plan.tools_id;
        let planned_tool_quantity = plan.lines.iter().fold(Quantity::ZERO, |total, line| {
            total.saturating_add(line.tool_quantity)
        });
        assert!(
            planned_tool_quantity > Quantity::ZERO,
            "production planning must decide replacement-tool demand before commit"
        );
        let cash_expectations = plan
            .lines
            .iter()
            .map(|line| {
                let before = state
                    .businesses
                    .get(line.business_id)
                    .expect("planned business must exist")
                    .cash();
                (
                    line.business_id,
                    before
                        .checked_sub(line.operating_cost)
                        .expect("planned operating cost must fit business cash"),
                )
            })
            .collect::<Vec<_>>();

        apply_production(&mut state, plan).expect("production must commit");

        assert_eq!(
            state
                .market
                .get_quote(tools_id)
                .expect("tools quote must exist")
                .demand_today,
            planned_tool_quantity,
            "commit must apply exactly the tool demand decided by the production plan"
        );
        for (business_id, expected_cash) in cash_expectations {
            assert_eq!(
                state
                    .businesses
                    .get(business_id)
                    .expect("planned business must remain")
                    .cash(),
                expected_cash,
                "tool purchases must be funded from existing operating overhead, not charged again"
            );
        }
    }

    #[test]
    fn tool_shortage_stops_non_tool_production() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let tools_id = registry
            .get_good_id("tools")
            .expect("registry must define tools");
        assert!(
            state.businesses.iter().any(|business| {
                registry
                    .get_recipe(business.recipe_id())
                    .expect("business recipe must exist")
                    .output_good_id()
                    != tools_id
            }),
            "campaign must contain a non-tool business"
        );
        let baseline_plan = decide_production(registry, &state);
        assert!(
            baseline_plan
                .lines
                .iter()
                .any(|line| line.output_good_id != tools_id),
            "campaign fixture must be able to produce a non-tool good before the shortage"
        );
        state
            .market
            .quotes
            .get_mut(&tools_id)
            .expect("tools quote must exist")
            .stock = Quantity::ZERO;

        let plan = decide_production(registry, &state);

        assert!(
            plan.lines
                .iter()
                .filter(|line| line.output_good_id != tools_id)
                .all(|line| line.tool_quantity == Quantity::ZERO),
            "production must not consume inputs and produce output when replacement tools are unavailable"
        );
        assert!(
            plan.lines
                .iter()
                .all(|line| line.output_good_id == tools_id),
            "only a toolmaker may continue production while the tool market is empty"
        );
    }

    #[test]
    fn tool_shortage_turns_maintenance_into_neglect() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let tools_id = registry
            .get_good_id("tools")
            .expect("registry must define tools");
        state
            .market
            .quotes
            .get_mut(&tools_id)
            .expect("tools quote must exist")
            .stock = Quantity::ZERO;
        let business_id = state
            .businesses
            .iter()
            .find(|business| {
                !matches!(
                    business.status(),
                    BusinessStatus::Closed | BusinessStatus::Insolvent
                ) && registry
                    .get_recipe(business.recipe_id())
                    .expect("business recipe must exist")
                    .output_good_id()
                    != tools_id
            })
            .expect("campaign must contain a non-tool business")
            .id();

        let plan = decide_maintenance(registry, &mut state);
        let line = plan
            .lines
            .iter()
            .find(|line| line.business_id == business_id)
            .expect("maintenance plan must include the selected business");

        assert_eq!(line.tool_quantity, Quantity::ZERO);
        assert_eq!(line.cost, Money::ZERO);
        assert!(
            line.condition_delta < 0 && line.quality_delta < 0,
            "maintenance without tools must degrade the business instead of reporting successful upkeep"
        );
    }

    #[test]
    fn maintenance_spending_creates_industrial_tool_demand() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        reset_market_flows(&mut state);
        let tools_id = registry
            .get_good_id("tools")
            .expect("Rivergate registry must define tools");
        let stock_before = state
            .market
            .get_quote(tools_id)
            .expect("tools quote must exist")
            .stock;

        let plan = decide_maintenance(registry, &mut state);
        let planned_quantity = plan.lines.iter().fold(Quantity::ZERO, |total, line| {
            total.saturating_add(line.tool_quantity)
        });
        assert!(
            planned_quantity > Quantity::ZERO,
            "operating businesses must create demand for replacement tools"
        );
        assert!(plan.lines.iter().all(|line| {
            let business = state
                .businesses
                .get(line.business_id)
                .expect("planned business must exist");
            let makes_tools = registry
                .get_recipe(business.recipe_id())
                .expect("business recipe must exist")
                .output_good_id()
                == tools_id;
            !makes_tools || line.tool_quantity.is_zero()
        }));

        apply_maintenance(&mut state, plan).expect("maintenance tool demand must commit");

        let quote = state
            .market
            .get_quote(tools_id)
            .expect("tools quote must exist");
        assert_eq!(quote.demand_today, planned_quantity);
        assert_eq!(
            quote.stock,
            stock_before
                .checked_sub(planned_quantity)
                .expect("planned industrial demand must fit tools stock")
        );
    }

    #[test]
    fn blacksmith_start_remains_operational_with_demand_scaled_contracts_and_payroll() {
        let registry = rivergate_registry_for_test();
        let mut state = build_new_game(
            registry,
            NewGameConfig {
                seed: 1,
                dynasty_name: "Audit".to_owned(),
                founder_name: "Audit Founder".to_owned(),
                background: StartingBackground::Blacksmith,
            },
        )
        .expect("blacksmith campaign must build");
        let starting_treasury = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();

        advance_days(registry, &mut state, 1_080).expect("campaign must simulate");

        let business = state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .and_then(|business_id| state.businesses.get(*business_id))
            .expect("player blacksmith business must exist");
        assert_eq!(business.status(), BusinessStatus::Active);
        assert!(
            business.operations.condition_basis_points >= 9_000,
            "demand-scaled contracts and payroll must preserve the smithy's physical condition"
        );
        assert!(
            business.cash() > Money::ZERO,
            "the smithy must retain working cash after three years without player intervention"
        );
        assert!(
            business.finance.lifetime_revenue > business.finance.lifetime_costs,
            "industrial tool demand must make the starting smithy sustainably profitable: revenue={}, costs={}, cash={}, tool_inventory={}, iron_inventory={}, charcoal_inventory={}, tool_market_stock={}, iron_market_stock={}, charcoal_market_stock={}, tools={}, iron={}, charcoal={}",
            business.finance.lifetime_revenue,
            business.finance.lifetime_costs,
            business.cash(),
            business.inventory_quantity(registry.get_good_id("tools").expect("tools must exist")),
            business.inventory_quantity(registry.get_good_id("iron").expect("iron must exist")),
            business.inventory_quantity(
                registry
                    .get_good_id("charcoal")
                    .expect("charcoal must exist")
            ),
            state
                .market
                .get_quote(registry.get_good_id("tools").expect("tools must exist"))
                .expect("tools quote must exist")
                .stock,
            state
                .market
                .get_quote(registry.get_good_id("iron").expect("iron must exist"))
                .expect("iron quote must exist")
                .stock,
            state
                .market
                .get_quote(
                    registry
                        .get_good_id("charcoal")
                        .expect("charcoal must exist")
                )
                .expect("charcoal quote must exist")
                .stock,
            state
                .market
                .get_quote(registry.get_good_id("tools").expect("tools must exist"))
                .expect("tools quote must exist")
                .price(),
            state
                .market
                .get_quote(registry.get_good_id("iron").expect("iron must exist"))
                .expect("iron quote must exist")
                .price(),
            state
                .market
                .get_quote(
                    registry
                        .get_good_id("charcoal")
                        .expect("charcoal must exist")
                )
                .expect("charcoal quote must exist")
                .price(),
        );
        assert!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury()
                > starting_treasury,
            "a viable smithy must eventually distribute surplus to the dynasty"
        );
    }
}

mod labor {
    use super::*;

    #[test]
    fn disputed_employment_reduces_but_does_not_deadlock_production() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let candidate_id = state
            .businesses
            .iter()
            .find(|business| business.operations.capacity_batches_per_day >= 2)
            .expect("campaign must contain a multi-batch business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(candidate_id)
                .expect("selected business must exist");
            business.operations.condition_basis_points = 10_000;
            business.finance.cash = Money::from_copper(100_000);
            business.policy.minimum_cash_reserve = Money::ZERO;
        }
        let initial_plan = decide_production(registry, &state);
        let initial_line = initial_plan
            .lines
            .iter()
            .find(|line| line.business_id == candidate_id)
            .expect("bootstrap must include a business able to produce")
            .clone();
        let business_id = initial_line.business_id;
        for agreement in state
            .employment
            .values_mut()
            .filter(|agreement| agreement.business_id == business_id)
        {
            agreement.status = EmploymentStatus::Disputed;
        }

        let plan = decide_production(registry, &state);
        let disputed_line = plan
            .lines
            .iter()
            .find(|line| line.business_id == business_id)
            .expect("a disputed workforce must retain reduced productive capacity");

        assert!(
            disputed_line.output_quantity < initial_line.output_quantity,
            "a labor dispute must reduce output"
        );
        assert!(
            disputed_line.output_quantity > Quantity::ZERO,
            "a labor dispute must not permanently remove the firm's ability to recover"
        );
    }
}

mod inventory_policy {
    use super::*;

    #[test]
    fn input_reserve_uses_a_wide_capacity_day_product() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let recipe = registry
            .get_recipe(
                state
                    .businesses
                    .get(business_id)
                    .expect("business must exist")
                    .recipe_id(),
            )
            .expect("business recipe must exist");
        let input = recipe
            .inputs()
            .first()
            .expect("business recipe must consume an input");
        let capacity = u16::MAX;
        let target_days = 30_u16;
        let narrow_limit_inventory = input
            .quantity()
            .saturating_mul_ratio(i64::from(u16::MAX), 1);
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.operations.capacity_batches_per_day = capacity;
            business.policy.target_input_days = target_days;
            business.policy.minimum_cash_reserve = Money::ZERO;
            business.finance.cash = Money::from_copper(i64::MAX);
            business
                .inventory
                .insert(input.good_id(), narrow_limit_inventory);
        }
        let quote = state
            .market
            .quotes
            .get_mut(&input.good_id())
            .expect("input quote must exist");
        quote.stock = Quantity::from_milliunits(i64::MAX);
        quote.price = Money::from_copper(1);

        let plan = decide_business_purchases(registry, &state)
            .expect("business purchase plan must resolve");
        let purchase = plan
            .lines
            .iter()
            .find(|line| line.business_id == business_id && line.good_id == input.good_id())
            .expect("wide inventory target must expose the remaining shortfall");
        // Reorder targets scale to the capacity the business can actually
        // use rather than raw nameplate capacity.
        let effective_batches = i64::from(super::effective_capacity_batches(
            &state,
            state
                .businesses
                .get(business_id)
                .expect("business must exist"),
        ));
        let target_batches = effective_batches.saturating_mul(i64::from(target_days));
        let expected = input
            .quantity()
            .saturating_mul_ratio(target_batches, 1)
            .saturating_sub(narrow_limit_inventory);

        assert_eq!(purchase.quantity, expected);
    }

    #[test]
    fn business_sales_share_one_market_absorption_ceiling_per_good() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        // Find a good produced by at least two operating businesses so the
        // shared ceiling has multiple claimants.
        let mut output_counts: std::collections::BTreeMap<GoodId, Vec<BusinessId>> =
            std::collections::BTreeMap::new();
        for business in state.businesses.iter() {
            if matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            ) {
                continue;
            }
            let good_id = registry
                .get_recipe(business.recipe_id())
                .expect("business recipe must exist")
                .output_good_id();
            output_counts
                .entry(good_id)
                .or_default()
                .push(business.id());
        }
        let (good_id, seller_ids) = output_counts
            .into_iter()
            .find(|(_, sellers)| sellers.len() >= 2)
            .expect("campaign must contain a good produced by at least two businesses");
        let [first_seller_id, second_seller_id] = seller_ids[..] else {
            panic!("shared-ceiling fixture requires at least two sellers");
        };
        // Drain the market so the whole target headroom is contested, cancel
        // contract reserves, and let the first seller hold only a quarter of
        // the headroom so it cannot absorb the entire ceiling alone.
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("quote must exist")
            .stock = Quantity::ZERO;
        let headroom = crate::systems::market_absorption_capacity(&state, good_id);
        assert!(
            headroom > Quantity::ZERO,
            "fixture must leave sale headroom"
        );
        for business_id in [first_seller_id, second_seller_id] {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("seller must exist");
            business.policy.target_output_days = 0;
        }
        {
            let first = state
                .businesses
                .get_mut(first_seller_id)
                .expect("first seller must exist");
            first.inventory.insert(
                good_id,
                Quantity::from_units((headroom.milliunits() / 4_000).max(1)),
            );
        }
        {
            let second = state
                .businesses
                .get_mut(second_seller_id)
                .expect("second seller must exist");
            second
                .inventory
                .insert(good_id, Quantity::from_units(10_000));
        }
        for contract in state
            .contracts
            .values_mut()
            .filter(|contract| contract.good_id == good_id)
        {
            contract.status = ContractStatus::Cancelled;
        }

        let plan = decide_business_sales(registry, &state).expect("sale plan must build");

        let placements: std::collections::BTreeMap<BusinessId, Quantity> = plan
            .lines
            .iter()
            .filter(|line| line.good_id == good_id)
            .map(|line| (line.business_id, line.quantity))
            .collect();
        assert!(
            placements.contains_key(&first_seller_id) && placements.contains_key(&second_seller_id),
            "both sellers must place surplus against the shared ceiling, got {placements:?}"
        );
        let first_take = placements[&first_seller_id];
        let second_take = placements[&second_seller_id];
        assert!(
            first_take <= Quantity::from_units(headroom.milliunits() / 4_000),
            "a seller cannot place more than it stocks"
        );
        assert!(
            second_take > Quantity::ZERO,
            "the second seller must still reach the market after the first seller consumed part of the shared headroom"
        );
        // Renown may let one house claim up to ~17% beyond the raw remainder.
        assert!(
            first_take.saturating_add(second_take) <= headroom.saturating_mul_ratio(12_000, 10_000),
            "aggregate placements must stay near the market's absorption ceiling instead of every seller claiming it independently"
        );
    }

    #[test]
    fn market_sale_rejects_business_cash_overflow_before_inventory_moves() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let recipe = registry
            .get_recipe(
                state
                    .businesses
                    .get(business_id)
                    .expect("business must exist")
                    .recipe_id(),
            )
            .expect("business recipe must exist");
        let good_id = recipe.output_good_id();
        let inventory = Quantity::from_units(10);
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.policy.target_output_days = 0;
            business.inventory.insert(good_id, inventory);
            business.finance.cash = Money::from_copper(i64::MAX);
        }
        let manager_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .manager_id();
        state
            .characters
            .get_mut(manager_id)
            .expect("manager must exist")
            .capabilities
            .commerce = 100;
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("output quote must exist")
            .stock = Quantity::ZERO;
        for contract in state
            .contracts
            .values_mut()
            .filter(|contract| contract.seller_business_id == business_id)
        {
            contract.status = ContractStatus::Cancelled;
        }
        let before = state.clone();

        let result = decide_business_sales(registry, &state);

        assert!(
            matches!(
                result,
                Err(SimulationError::BusinessCashOverflow {
                    business_id: failed_business_id,
                    current,
                    incoming,
                }) if failed_business_id == business_id
                    && current == Money::from_copper(i64::MAX)
                    && incoming > Money::ZERO
            ),
            "an unrepresentable sale receipt must return a typed overflow error, received {result:?}"
        );
        assert_state_unchanged(
            &before,
            &state,
            "sale planning must reject recipient overflow without mutating inventory, cash, or market state",
        );
    }

    #[test]
    fn market_sale_rejects_unrepresentable_trade_value_before_cash_checks() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let recipe = registry
            .get_recipe(
                state
                    .businesses
                    .get(business_id)
                    .expect("business must exist")
                    .recipe_id(),
            )
            .expect("business recipe must exist");
        let good_id = recipe.output_good_id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.policy.target_output_days = 0;
            business.inventory.insert(good_id, Quantity::from_units(10));
            business.finance.cash = Money::ZERO;
        }
        let manager_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .manager_id();
        state
            .characters
            .get_mut(manager_id)
            .expect("manager must exist")
            .capabilities
            .commerce = 100;
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("output quote must exist");
            quote.price = Money::from_copper(i64::MAX);
            quote.stock = Quantity::ZERO;
            quote.target_stock = Quantity::from_units(100);
        }
        for contract in state
            .contracts
            .values_mut()
            .filter(|contract| contract.seller_business_id == business_id)
        {
            contract.status = ContractStatus::Cancelled;
        }
        let before = state.clone();

        let result = decide_business_sales(registry, &state);

        assert!(
            matches!(
                result,
                Err(SimulationError::MarketTradeValueOverflow {
                    good_id: failed_good_id,
                    quantity,
                    unit_price,
                }) if failed_good_id == good_id
                    && quantity > Quantity::from_units(1)
                    && unit_price == Money::from_copper(i64::MAX)
            ),
            "trade-value overflow must be reported before revenue is narrowed, received {result:?}"
        );
        assert_state_unchanged(
            &before,
            &state,
            "trade-value overflow during sale planning must not mutate the campaign",
        );
    }

    #[test]
    #[should_panic(expected = "inventory additions must not be negative")]
    fn negative_inventory_additions_are_rejected() {
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let good_id = state
            .market
            .quotes
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a market good");

        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .add_inventory(good_id, Quantity::from_units(-1));
    }

    #[test]
    #[should_panic(expected = "inventory removals must not be negative")]
    fn negative_inventory_removals_are_rejected() {
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let good_id = state
            .market
            .quotes
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a market good");

        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .remove_inventory(good_id, Quantity::from_units(-1));
    }

    #[test]
    fn output_reserve_scales_with_daily_capacity() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let recipe_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .recipe_id();
        let recipe = registry
            .get_recipe(recipe_id)
            .expect("business recipe must exist");
        let output_good_id = recipe.output_good_id();
        let output_per_batch = recipe.output_quantity();
        let capacity = 4_u16;
        let target_days = 3_u16;
        let inventory = output_per_batch.saturating_mul_ratio(
            i64::from(capacity)
                .saturating_mul(i64::from(target_days))
                .saturating_add(1),
            1,
        );
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.operations.capacity_batches_per_day = capacity;
            business.policy.target_output_days = target_days;
            business.inventory.insert(output_good_id, inventory);
        }
        let manager_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .manager_id();
        state
            .characters
            .get_mut(manager_id)
            .expect("business manager must exist")
            .capabilities
            .commerce = 100;
        state
            .market
            .quotes
            .get_mut(&output_good_id)
            .expect("output quote must exist")
            .stock = Quantity::ZERO;

        let plan = decide_business_sales(registry, &state).expect("sales plan must resolve");
        let sale = plan
            .lines
            .iter()
            .find(|line| line.business_id == business_id)
            .expect("business must sell inventory above its reserve");

        assert_eq!(
            sale.quantity, output_per_batch,
            "target output days must reserve capacity-adjusted production"
        );
    }

    #[test]
    fn inventory_below_reserve_does_not_create_negative_sale() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let recipe_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .recipe_id();
        let recipe = registry
            .get_recipe(recipe_id)
            .expect("business recipe must exist");
        let output_good_id = recipe.output_good_id();
        let below_reserve = recipe.output_quantity();
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .inventory
            .insert(output_good_id, below_reserve);

        let plan = decide_business_sales(registry, &state).expect("sales plan must resolve");

        assert!(
            plan.lines
                .iter()
                .all(|line| line.business_id != business_id),
            "inventory below the policy reserve must not produce a negative sale"
        );
    }

    #[test]
    fn active_contract_inventory_is_reserved_from_market_sales() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let contract = state
            .contracts
            .values()
            .find(|contract| contract.status == ContractStatus::Active)
            .expect("campaign must contain an active contract")
            .clone();
        let seller_id = contract.seller_business_id;
        let good_id = contract.good_id;
        {
            let seller = state
                .businesses
                .get_mut(seller_id)
                .expect("contract seller must exist");
            seller.policy.target_output_days = 0;
            seller.inventory.insert(
                good_id,
                contract
                    .quantity_per_week
                    .saturating_add(Quantity::from_units(10)),
            );
        }
        let manager_id = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .manager_id();
        state
            .characters
            .get_mut(manager_id)
            .expect("seller manager must exist")
            .capabilities
            .commerce = 100;
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("contract good quote must exist")
            .stock = Quantity::ZERO;

        let plan = decide_business_sales(registry, &state).expect("sale plan must resolve");
        apply_business_sales(&mut state, plan).expect("business sales must apply");

        assert!(
            state
                .businesses
                .get(seller_id)
                .expect("seller must exist")
                .inventory_quantity(good_id)
                >= contract.quantity_per_week,
            "market sales must preserve the next active contract delivery"
        );
    }

    #[test]
    fn distressed_business_liquidates_policy_reserve_but_preserves_contract_stock() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let contract = state
            .contracts
            .values()
            .find(|contract| contract.status == ContractStatus::Active)
            .expect("campaign must contain an active contract")
            .clone();
        let seller_id = contract.seller_business_id;
        let good_id = contract.good_id;
        let recipe_id = state
            .businesses
            .get(seller_id)
            .expect("contract seller must exist")
            .recipe_id();
        let recipe = registry
            .get_recipe(recipe_id)
            .expect("seller recipe must exist");
        let policy_reserve = recipe.output_quantity().saturating_mul_ratio(
            i64::from(
                state
                    .businesses
                    .get(seller_id)
                    .expect("seller must exist")
                    .operations
                    .capacity_batches_per_day,
            )
            .saturating_mul(2),
            1,
        );
        let initial_inventory = policy_reserve.saturating_add(contract.quantity_per_week);
        {
            let seller = state
                .businesses
                .get_mut(seller_id)
                .expect("contract seller must exist");
            seller.policy.target_output_days = 2;
            seller.operations.status = BusinessStatus::Distressed;
            seller.finance.cash = Money::ZERO;
            seller.inventory.insert(good_id, initial_inventory);
        }
        let manager_id = state
            .businesses
            .get(seller_id)
            .expect("seller must exist")
            .manager_id();
        state
            .characters
            .get_mut(manager_id)
            .expect("seller manager must exist")
            .capabilities
            .commerce = 100;
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("contract good quote must exist")
            .stock = Quantity::ZERO;

        let plan = decide_business_sales(registry, &state).expect("sale plan must resolve");
        apply_business_sales(&mut state, plan).expect("business sales must apply");

        let seller = state.businesses.get(seller_id).expect("seller must exist");
        assert!(
            seller.inventory_quantity(good_id) < initial_inventory,
            "a distressed firm must liquidate policy inventory to restore cash"
        );
        assert!(
            seller.inventory_quantity(good_id) >= contract.quantity_per_week,
            "distress liquidation must still preserve active contract obligations"
        );
        assert!(
            seller.cash() > Money::ZERO,
            "liquidation must provide working cash for recovery"
        );
    }
}

mod household_demand {
    use super::*;

    #[test]
    fn households_use_upstream_staples_when_bread_is_unavailable() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let bread_id = registry
            .get_good_id("bread")
            .expect("registry must define bread");
        let flour_id = registry
            .get_good_id("flour")
            .expect("registry must define flour");
        let grain_id = registry
            .get_good_id("grain")
            .expect("registry must define grain");
        for household in state.households.iter_mut() {
            household.cash = Money::from_copper(100_000);
        }
        for quote in state.market.quotes.values_mut() {
            quote.stock = Quantity::ZERO;
        }
        state
            .market
            .quotes
            .get_mut(&flour_id)
            .expect("flour quote must exist")
            .stock = Quantity::from_units(10_000);
        state
            .market
            .quotes
            .get_mut(&grain_id)
            .expect("grain quote must exist")
            .stock = Quantity::from_units(10_000);

        let plan =
            decide_household_consumption(registry, &state).expect("household demand must resolve");

        assert!(plan.lines.iter().all(|line| line.good_id != bread_id));
        assert!(
            plan.lines
                .iter()
                .any(|line| line.good_id == flour_id || line.good_id == grain_id),
            "households must consume available upstream staples instead of starving"
        );
        assert!(plan.food_satisfaction.iter().all(|(household_id, value)| {
            *value
                > state
                    .households
                    .get(*household_id)
                    .expect("planned household must exist")
                    .food_satisfaction_basis_points()
        }));
    }

    #[test]
    fn households_prefer_bread_before_upstream_staples() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let flour_id = registry
            .get_good_id("flour")
            .expect("registry must define flour");
        let grain_id = registry
            .get_good_id("grain")
            .expect("registry must define grain");
        for household in state.households.iter_mut() {
            household.cash = Money::from_copper(100_000);
        }
        for quote in state.market.quotes.values_mut() {
            quote.stock = Quantity::from_units(10_000);
        }

        let plan =
            decide_household_consumption(registry, &state).expect("household demand must resolve");

        assert!(
            plan.lines
                .iter()
                .all(|line| line.good_id != flour_id && line.good_id != grain_id),
            "upstream staples must remain a fallback rather than displacing available bread"
        );
    }

    #[test]
    fn households_create_demand_for_nonfood_consumer_goods() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let charcoal_id = registry
            .get_good_id("charcoal")
            .expect("registry must define charcoal");
        let cloth_id = registry
            .get_good_id("cloth")
            .expect("registry must define cloth");
        let tools_id = registry
            .get_good_id("tools")
            .expect("registry must define tools");
        for household in state.households.iter_mut() {
            household.cash = Money::from_copper(100_000);
        }
        for quote in state.market.quotes.values_mut() {
            quote.stock = Quantity::from_units(10_000);
        }

        let plan =
            decide_household_consumption(registry, &state).expect("household demand must resolve");

        for good_id in [charcoal_id, cloth_id, tools_id] {
            assert!(
                plan.lines
                    .iter()
                    .any(|line| line.good_id == good_id && line.quantity > Quantity::ZERO),
                "households must create durable demand for good {good_id}"
            );
        }
        let tool_demand = plan
            .lines
            .iter()
            .filter(|line| line.good_id == tools_id)
            .fold(Quantity::ZERO, |total, line| {
                total.saturating_add(line.quantity)
            });
        assert!(
            tool_demand >= Quantity::from_units(3),
            "aggregate Rivergate households must create material recurring demand for durable tools: demand={tool_demand}"
        );
    }

    #[test]
    fn household_textile_demand_can_absorb_one_standard_weaver() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let cloth_id = registry
            .get_good_id("cloth")
            .expect("registry must define cloth");
        let weaving = registry
            .get_recipe(
                registry
                    .get_recipe_id("weaving")
                    .expect("registry must define weaving"),
            )
            .expect("weaving recipe must exist");
        let standard_daily_output = weaving.output_quantity().saturating_mul_ratio(2, 1);
        for household in state.households.iter_mut() {
            household.cash = Money::from_copper(100_000);
        }
        for quote in state.market.quotes.values_mut() {
            quote.stock = Quantity::from_units(10_000);
        }

        let plan =
            decide_household_consumption(registry, &state).expect("household demand must resolve");
        let cloth_demand = plan
            .lines
            .iter()
            .filter(|line| line.good_id == cloth_id)
            .fold(Quantity::ZERO, |total, line| {
                total.saturating_add(line.quantity)
            });

        assert!(
            cloth_demand >= standard_daily_output,
            "Rivergate household demand must be able to absorb one normal two-batch weaving shop before additional producers create competition: demand={cloth_demand}, output={standard_daily_output}"
        );
    }
}

mod office_exposure {
    use super::*;

    #[test]
    fn office_administrative_load_reduces_overextended_business_capacity() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let business_id = *state
            .businesses
            .ids_for_owner(player_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let business_load = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .administrative_load();
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .administrative_capacity = business_load;
        state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist")
            .operations
            .capacity_batches_per_day = 6;
        for institution in state.institutions.values_mut() {
            institution.office_holder_id = None;
        }
        let baseline = effective_capacity_batches(
            &state,
            state
                .businesses
                .get(business_id)
                .expect("player business must exist"),
        );
        let holder_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        let council_id = registry
            .get_institution_id("city_council")
            .expect("registry must define the city council");
        state
            .institutions
            .get_mut(&council_id)
            .expect("city council must exist")
            .office_holder_id = Some(holder_id);

        let burdened = effective_capacity_batches(
            &state,
            state
                .businesses
                .get(business_id)
                .expect("player business must exist"),
        );

        assert!(
            burdened < baseline,
            "institutional authority must compete with private administrative capacity"
        );
    }

    #[test]
    fn office_overextension_increases_succession_risk() {
        let registry = rivergate_registry_for_test();
        let mut baseline = make_test_campaign();
        let player_id = baseline.player_dynasty_id;
        for institution in baseline.institutions.values_mut() {
            institution.office_holder_id = None;
        }
        let load = baseline
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .administrative_load();
        baseline
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .administrative_capacity = load;
        let mut burdened = baseline.clone();
        let holder_id = burdened
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        let council_id = registry
            .get_institution_id("city_council")
            .expect("registry must define the city council");
        burdened
            .institutions
            .get_mut(&council_id)
            .expect("city council must exist")
            .office_holder_id = Some(holder_id);

        update_succession_risks(&mut baseline);
        update_succession_risks(&mut burdened);

        assert!(
            burdened
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .runtime
                .succession_risk_basis_points
                > baseline
                    .dynasties
                    .get(&player_id)
                    .expect("player dynasty must exist")
                    .runtime
                    .succession_risk_basis_points,
            "institutional overextension must expose the dynasty during succession"
        );
    }
}

mod manager_capabilities {
    use super::*;

    #[test]
    fn manager_craft_affects_production_output() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign();
        let business_id = decide_production(registry, &state)
            .lines
            .first()
            .expect("campaign must contain a producing business")
            .business_id;
        let manager_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .manager_id();
        let mut low_skill = state.clone();
        let mut high_skill = state;
        low_skill
            .characters
            .get_mut(manager_id)
            .expect("manager must exist")
            .capabilities
            .craft = 0;
        high_skill
            .characters
            .get_mut(manager_id)
            .expect("manager must exist")
            .capabilities
            .craft = 100;

        let low_output = decide_production(registry, &low_skill)
            .lines
            .into_iter()
            .find(|line| line.business_id == business_id)
            .expect("low-skill business must produce")
            .output_quantity;
        let high_output = decide_production(registry, &high_skill)
            .lines
            .into_iter()
            .find(|line| line.business_id == business_id)
            .expect("high-skill business must produce")
            .output_quantity;

        assert!(
            high_output > low_output,
            "manager craft must affect the business's production outcome"
        );
    }

    #[test]
    fn manager_commerce_affects_market_throughput() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let (manager_id, recipe_id) = {
            let business = state
                .businesses
                .get(business_id)
                .expect("business must exist");
            (business.manager_id(), business.recipe_id())
        };
        let recipe = registry
            .get_recipe(recipe_id)
            .expect("business recipe must exist");
        let good_id = recipe.output_good_id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.policy.target_output_days = 0;
            business.inventory.insert(
                good_id,
                recipe.output_quantity().saturating_mul_ratio(100, 1),
            );
        }
        state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("market quote must exist")
            .stock = Quantity::ZERO;
        let mut low_skill = state.clone();
        let mut high_skill = state;
        low_skill
            .characters
            .get_mut(manager_id)
            .expect("manager must exist")
            .capabilities
            .commerce = 0;
        high_skill
            .characters
            .get_mut(manager_id)
            .expect("manager must exist")
            .capabilities
            .commerce = 100;

        let low_quantity = decide_business_sales(registry, &low_skill)
            .expect("low-skill sales plan must resolve")
            .lines
            .into_iter()
            .find(|line| line.business_id == business_id)
            .expect("low-skill business must sell")
            .quantity;
        let high_quantity = decide_business_sales(registry, &high_skill)
            .expect("high-skill sales plan must resolve")
            .lines
            .into_iter()
            .find(|line| line.business_id == business_id)
            .expect("high-skill business must sell")
            .quantity;

        assert!(
            high_quantity > low_quantity,
            "manager commerce must affect the business's market throughput"
        );
    }
}

mod cash_reserve_policy {
    use super::*;

    #[test]
    fn production_does_not_spend_the_minimum_cash_reserve() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = decide_production(registry, &state)
            .lines
            .iter()
            .find_map(|line| {
                let business = state.businesses.get(line.business_id)?;
                let recipe = registry.get_recipe(business.recipe_id())?;
                (recipe.daily_operating_cost() > Money::ZERO).then_some(line.business_id)
            })
            .expect("campaign must contain a producing business with operating costs");
        let cash = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .cash();
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .policy
            .minimum_cash_reserve = cash;

        let plan = decide_production(registry, &state);

        assert!(
            plan.lines
                .iter()
                .all(|line| line.business_id != business_id),
            "production operating costs must not consume protected cash"
        );
    }
}

mod business_lifecycle {
    use super::*;

    #[test]
    fn insolvency_suspends_attached_employment() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.operations.status = BusinessStatus::Active;
            business.finance.cash = Money::ZERO;
            business.inventory.clear();
        }

        update_business_lifecycle(registry, &mut state)
            .expect("business lifecycle update must succeed");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Insolvent
        );
        assert!(
            state
                .employment
                .values()
                .filter(|agreement| agreement.business_id == business_id)
                .all(|agreement| agreement.status == EmploymentStatus::Suspended),
            "inactive employers must not retain active or disputed labor agreements"
        );
    }

    #[test]
    fn cash_locked_in_policy_reserve_counts_as_distress() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let reserve = Money::from_copper(500);
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.operations.status = BusinessStatus::Active;
            business.policy.minimum_cash_reserve = reserve;
            business.finance.cash = reserve;
        }

        update_business_lifecycle(registry, &mut state)
            .expect("business lifecycle update must succeed");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Distressed,
            "gross cash is not operating liquidity when all of it is policy-reserved"
        );
    }

    #[test]
    fn closed_businesses_do_not_reopen_automatically() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .operations
            .status = BusinessStatus::Closed;

        update_business_lifecycle(registry, &mut state)
            .expect("business lifecycle update must succeed");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Closed,
            "closure is a terminal lifecycle state unless an explicit command reopens it"
        );
    }

    #[test]
    fn unresolved_insolvency_progresses_to_terminal_closure() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.operations.status = BusinessStatus::Insolvent;
            business.finance.cash = Money::ZERO;
            business.inventory.clear();
        }
        let chronicle_before = state.chronicle.len();

        update_business_lifecycle(registry, &mut state)
            .expect("business lifecycle update must succeed");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Closed
        );
        assert_eq!(state.chronicle.len(), chronicle_before + 1);
        assert!(
            state
                .employment
                .values()
                .filter(|agreement| agreement.business_id == business_id)
                .all(|agreement| agreement.status == EmploymentStatus::Suspended),
            "closure must suspend labor agreements so an explicit acquisition can renegotiate them"
        );
    }

    #[test]
    fn insolvent_recovery_is_recorded() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.operations.status = BusinessStatus::Insolvent;
            business.finance.cash = Money::from_copper(100_000);
        }
        for agreement in state
            .employment
            .values_mut()
            .filter(|agreement| agreement.business_id == business_id)
        {
            agreement.status = EmploymentStatus::Suspended;
        }
        let chronicle_before = state.chronicle.len();

        update_business_lifecycle(registry, &mut state)
            .expect("business lifecycle update must succeed");

        // Fresh capital ends insolvency, but recovery must pass through a
        // distressed rehabilitation period before full operation resumes.
        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Distressed
        );
        assert!(
            state
                .employment
                .values()
                .filter(|agreement| agreement.business_id == business_id)
                .all(|agreement| agreement.status == EmploymentStatus::Disputed),
            "reopening after insolvency must preserve labor consequences instead of resetting loyalty"
        );

        update_business_lifecycle(registry, &mut state)
            .expect("second business lifecycle update must succeed");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Active
        );
        assert_eq!(state.chronicle.len(), chronicle_before + 2);
        assert_eq!(
            state
                .chronicle
                .iter()
                .nth_back(1)
                .expect("rehabilitation must add a distress entry")
                .kind,
            ChronicleKind::BusinessDistress
        );
        assert_eq!(
            state
                .chronicle
                .last()
                .expect("recovery must add a chronicle entry")
                .kind,
            ChronicleKind::BusinessRecovered
        );
    }

    #[test]
    fn distressed_recovery_requires_more_cash_than_distress_onset() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let recipe = registry
            .get_recipe(
                state
                    .businesses
                    .get(business_id)
                    .expect("business must exist")
                    .recipe_id(),
            )
            .expect("recipe reference must be valid");
        let daily_cost = recipe.daily_operating_cost();
        // Enough cover to avoid distress onset from Active, but not enough to
        // climb back out of Distressed.
        let between_thresholds =
            Money::from_copper(500).saturating_add(daily_cost.saturating_mul(3));
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.operations.status = BusinessStatus::Distressed;
            business.policy.minimum_cash_reserve = Money::from_copper(500);
            business.finance.cash = between_thresholds;
        }

        update_business_lifecycle(registry, &mut state)
            .expect("first lifecycle update must succeed");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Distressed,
            "cash that would merely avoid distress onset must not end a distressed period"
        );

        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.finance.cash =
                Money::from_copper(500).saturating_add(daily_cost.saturating_mul(6));
        }

        update_business_lifecycle(registry, &mut state)
            .expect("second lifecycle update must succeed");

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Active,
            "six days of operating cover should complete rehabilitation"
        );
    }
}

mod maintenance_policy {
    use super::*;

    #[test]
    fn funded_maintenance_moves_quality_toward_policy_target() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let quality_before = 5_000;
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.finance.cash = Money::from_copper(100_000);
            business.policy.minimum_cash_reserve = Money::ZERO;
            business.policy.maintenance_basis_points = 10_000;
            business.policy.quality_target_basis_points = 7_000;
            business.operations.quality_basis_points = quality_before;
        }

        let plan = decide_maintenance(registry, &mut state);
        apply_maintenance(&mut state, plan).expect("maintenance plan must apply");

        assert!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .operations
                .quality_basis_points
                > quality_before,
            "funded maintenance must make the configured quality target operational"
        );
    }

    #[test]
    fn zero_maintenance_budget_cannot_create_free_improvements_or_finance_churn() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let condition_before = 5_000;
        let quality_before = 5_000;
        let version_before = {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.finance.cash = Money::from_copper(100_000);
            business.policy.minimum_cash_reserve = Money::ZERO;
            business.policy.maintenance_basis_points = 0;
            business.policy.quality_target_basis_points = 7_000;
            business.operations.condition_basis_points = condition_before;
            business.operations.quality_basis_points = quality_before;
            business.finance.version
        };

        let plan = decide_maintenance(registry, &mut state);
        apply_maintenance(&mut state, plan).expect("maintenance plan must apply");

        let business = state
            .businesses
            .get(business_id)
            .expect("business must exist");
        assert!(
            business.operations.condition_basis_points < condition_before,
            "an unfunded maintenance policy must not improve condition"
        );
        assert!(
            business.operations.quality_basis_points < quality_before,
            "an unfunded maintenance policy must not improve quality"
        );
        assert_eq!(
            business.finance.version, version_before,
            "a zero-value maintenance settlement must not invalidate finance tokens"
        );
    }

    #[test]
    fn positive_maintenance_budget_never_rounds_down_to_free() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.finance.cash = Money::from_copper(100_000);
            business.policy.minimum_cash_reserve = Money::ZERO;
            business.policy.maintenance_basis_points = 1;
        }

        let plan = decide_maintenance(registry, &mut state);
        let line = plan
            .lines
            .iter()
            .find(|line| line.business_id == business_id)
            .expect("active business must receive a maintenance decision");

        assert!(
            line.cost > Money::ZERO,
            "any positive maintenance allocation must have a positive economic cost"
        );
    }

    #[test]
    fn default_maintenance_prevents_long_horizon_condition_collapse_when_funded() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let condition_before = 8_000;
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.finance.cash = Money::from_copper(1_000_000);
            business.policy.minimum_cash_reserve = Money::ZERO;
            business.policy.maintenance_basis_points = 1_200;
            business.operations.condition_basis_points = condition_before;
        }

        for _ in 0..360 {
            let plan = decide_maintenance(registry, &mut state);
            apply_maintenance(&mut state, plan).expect("maintenance plan must apply");
        }

        assert!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .operations
                .condition_basis_points
                >= condition_before,
            "the default funded policy must sustain rather than inevitably destroy a business"
        );
    }

    #[test]
    fn funded_maintenance_can_recover_a_severely_degraded_business() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("business must exist");
            business.finance.cash = Money::from_copper(1_000_000);
            business.policy.minimum_cash_reserve = Money::ZERO;
            business.policy.maintenance_basis_points = 1_000;
            business.policy.quality_target_basis_points = 7_800;
            business.operations.condition_basis_points = 500;
            business.operations.quality_basis_points = 500;
        }
        let tools_id = registry
            .get_good_id("tools")
            .expect("registry must define tools");
        state
            .market
            .quotes
            .get_mut(&tools_id)
            .expect("tools quote must exist")
            .stock = Quantity::from_units(100_000);

        for _ in 0..360 {
            let plan = decide_maintenance(registry, &mut state);
            apply_maintenance(&mut state, plan).expect("maintenance plan must apply");
        }

        let business = state
            .businesses
            .get(business_id)
            .expect("business must exist");
        assert!(
            business.operations.condition_basis_points >= 6_000,
            "funded catch-up maintenance must provide a credible route out of the low-condition trap; condition={}",
            business.operations.condition_basis_points
        );
        assert!(
            business.operations.quality_basis_points >= 7_000,
            "funded repair work must restore quality as well as physical condition; quality={}",
            business.operations.quality_basis_points
        );
    }
}

mod money_conservation {
    use super::*;

    #[test]
    fn production_credits_the_clearing_account_with_every_operating_cost() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let plan = decide_production(registry, &state);
        assert!(
            !plan.lines.is_empty(),
            "fixture campaign must plan production"
        );
        let expected_clearing =
            plan.lines
                .iter()
                .fold(state.market.clearing_account, |total, line| {
                    total
                        .checked_add(line.operating_cost)
                        .expect("clearing credit must fit")
                });

        apply_production(&mut state, plan).expect("production must commit");

        assert_eq!(
            state.market.clearing_account, expected_clearing,
            "every copper of charged operating cost must reach the market clearing pool"
        );
        validate_invariants(registry, &state);
    }

    #[test]
    fn maintenance_credits_the_clearing_account_with_every_charged_cost() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        for business in state.businesses.iter_mut() {
            business.finance.cash = Money::from_copper(1_000_000);
            business.policy.minimum_cash_reserve = Money::ZERO;
            business.policy.maintenance_basis_points = 10_000;
        }
        let plan = decide_maintenance(registry, &mut state);
        assert!(
            !plan.lines.is_empty(),
            "fixture campaign must plan maintenance"
        );
        let expected_clearing =
            plan.lines
                .iter()
                .fold(state.market.clearing_account, |total, line| {
                    total
                        .checked_add(line.cost.max(line.tool_cost))
                        .expect("clearing credit must fit")
                });

        apply_maintenance(&mut state, plan).expect("maintenance must commit");

        assert_eq!(
            state.market.clearing_account, expected_clearing,
            "the full maintenance charge, tool-backed or not, must reach the clearing pool"
        );
        validate_invariants(registry, &state);
    }

    #[test]
    fn unowned_property_purchases_conserve_the_purchase_price() {
        let mut state = make_test_campaign();
        let property_id = state
            .properties
            .values()
            .find(|property| property.owner_dynasty_id.is_none())
            .map(|property| property.id)
            .expect("fixture campaign must contain unowned property");
        let price = state
            .properties
            .get(&property_id)
            .expect("property must exist")
            .value;
        let buyer_id = state.player_dynasty_id;
        {
            let buyer = state
                .dynasties
                .get_mut(&buyer_id)
                .expect("player dynasty must exist");
            buyer.resources.treasury = buyer
                .treasury()
                .checked_add(price)
                .expect("test funding must fit treasury");
        }
        let clearing_before = state.market.clearing_account;

        crate::systems::buy_unowned_property(&mut state, buyer_id, property_id)
            .expect("funded purchase must commit");

        assert_eq!(
            state.market.clearing_account,
            clearing_before
                .checked_add(price)
                .expect("purchase proceeds must fit")
        );
    }
}

mod health_and_succession {
    use super::*;

    #[test]
    fn succession_does_not_inherit_personal_institution_memberships() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let dynasty = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist");
        let outgoing_head_id = dynasty.head_id();
        let incoming_head_id = dynasty.heir_id().expect("player dynasty must have an heir");
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        {
            let institution = state
                .institutions
                .get_mut(&institution_id)
                .expect("institution must exist");
            institution.members.insert(outgoing_head_id);
            institution.office_holder_id = Some(outgoing_head_id);
        }
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InstitutionPatronage,
            subject: format!("institution:{institution_id}:character:{outgoing_head_id}").into(),
            detail: "test cultivated support".to_owned(),
        });
        state
            .characters
            .get_mut(outgoing_head_id)
            .expect("outgoing head must exist")
            .runtime
            .health_basis_points = 0;

        let successions =
            decide_successions(&mut state).expect("forced succession must remain representable");
        apply_successions(&mut state, successions).expect("succession application must succeed");

        let institution = state
            .institutions
            .get(&institution_id)
            .expect("institution must exist");
        assert!(!institution.members.contains(&outgoing_head_id));
        assert!(
            !institution.members.contains(&incoming_head_id),
            "personal patronage must not transfer automatically to a successor"
        );
        assert_eq!(institution.office_holder_id, None);
        validate_invariants(registry, &state);
    }

    #[test]
    fn ai_succession_transfers_dynasty_standing_institution_memberships() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain an AI dynasty");
        let dynasty = state
            .dynasties
            .get(&dynasty_id)
            .expect("AI dynasty must exist");
        let outgoing_head_id = dynasty.head_id();
        let incoming_head_id = dynasty.heir_id().expect("AI dynasty must have an heir");
        assert!(
            state
                .institutions
                .values()
                .all(|institution| { institution.members.contains(&outgoing_head_id) }),
            "fixture AI heads must sit on every institutional membership"
        );
        state
            .characters
            .get_mut(outgoing_head_id)
            .expect("outgoing head must exist")
            .runtime
            .health_basis_points = 0;

        let successions =
            decide_successions(&mut state).expect("forced succession must remain representable");
        apply_successions(&mut state, successions).expect("succession application must succeed");

        for institution in state.institutions.values() {
            assert!(!institution.members.contains(&outgoing_head_id));
            assert!(
                institution.members.contains(&incoming_head_id),
                "every institution must seat the incoming AI dynasty head so offices stay fillable"
            );
        }
        validate_invariants(registry, &state);
    }

    #[test]
    fn succession_deactivates_ward_links_to_the_outgoing_head() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let outgoing_head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let ward_id = state.next_ids.character();
        state.characters.insert(Character {
            identity: CharacterIdentity {
                id: ward_id,
                dynasty_id,
                name: "Succession Ward".to_owned(),
                birth_day: state.clock.day().saturating_sub(18 * 360),
            },
            capabilities: CharacterCapabilities {
                administration: 45,
                commerce: 45,
                social: 45,
                craft: 45,
            },
            runtime: CharacterRuntime {
                status: CharacterStatus::Active,
                health_basis_points: 9_000,
                loyalty_basis_points: 8_000,
                role: CharacterRole::Clerk,
            },
        });
        state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("player family council must exist")
            .members
            .insert(ward_id);
        let ward_link_id = state.next_ids.family_link();
        state.family_links.insert(
            ward_link_id,
            FamilyLink {
                id: ward_link_id,
                first_character_id: outgoing_head_id,
                second_character_id: ward_id,
                kind: FamilyLinkKind::Ward,
                active: true,
            },
        );
        state
            .characters
            .get_mut(outgoing_head_id)
            .expect("outgoing head must exist")
            .runtime
            .health_basis_points = 0;

        let successions =
            decide_successions(&mut state).expect("forced succession must remain representable");
        apply_successions(&mut state, successions).expect("succession application must succeed");

        assert!(
            !state
                .family_links
                .get(&ward_link_id)
                .expect("ward link must remain recorded")
                .active,
            "a deceased guardian cannot retain an active ward relationship"
        );
        assert_eq!(
            state
                .characters
                .get(ward_id)
                .expect("ward must remain recorded")
                .status(),
            CharacterStatus::Active
        );
        assert!(
            state
                .family_councils
                .get(&dynasty_id)
                .expect("player family council must exist")
                .members
                .contains(&ward_id),
            "succession should end guardianship without expelling the adopted family member"
        );
        validate_invariants(registry, &state);
    }

    #[test]
    fn zero_health_forces_succession_before_normal_retirement_age() {
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        state
            .characters
            .get_mut(head_id)
            .expect("dynasty head must exist")
            .identity
            .birth_day = state.clock.day().saturating_sub(40 * 360);
        let age_years = state.clock.day().saturating_sub(
            state
                .characters
                .get(head_id)
                .expect("dynasty head must exist")
                .birth_day(),
        ) / 360;
        assert!(
            age_years < SUCCESSION_ELIGIBILITY_AGE_YEARS,
            "fixture head must be below the succession eligibility age"
        );
        state
            .characters
            .get_mut(head_id)
            .expect("dynasty head must exist")
            .runtime
            .health_basis_points = 0;

        let successions =
            decide_successions(&mut state).expect("forced succession must remain representable");

        let line = successions
            .iter()
            .find(|line| line.dynasty_id == dynasty_id && line.outgoing_head_id == head_id)
            .expect("zero health must force the player succession");
        assert_eq!(
            line.new_heir_link_kind,
            FamilyLinkKind::Sibling,
            "a young incoming head must receive a collateral heir rather than an impossible adult child"
        );
        let incoming_birth_day = state
            .characters
            .get(line.incoming_head_id)
            .expect("incoming head must exist")
            .birth_day();
        assert!(
            line.new_heir_birth_day.saturating_sub(incoming_birth_day)
                < crate::core::MIN_PARENT_CHILD_AGE_GAP_DAYS,
            "the fixture must exercise the chronology that cannot be represented as parent-child"
        );
    }

    #[test]
    fn formal_heir_preparation_reduces_the_succession_shock() {
        let mut unprepared = make_test_campaign();
        let mut prepared = unprepared.clone();
        let dynasty_id = unprepared.player_dynasty_id;
        let (head_id, heir_id) = {
            let dynasty = unprepared
                .dynasties
                .get(&dynasty_id)
                .expect("player dynasty must exist");
            (
                dynasty.head_id(),
                dynasty.heir_id().expect("player dynasty must have an heir"),
            )
        };
        for state in [&mut unprepared, &mut prepared] {
            state
                .characters
                .get_mut(head_id)
                .expect("dynasty head must exist")
                .runtime
                .health_basis_points = 0;
            state
                .dynasties
                .get_mut(&dynasty_id)
                .expect("player dynasty must exist")
                .runtime
                .succession_risk_basis_points = 4_800;
            state
                .family_councils
                .get_mut(&dynasty_id)
                .expect("player council must exist")
                .unity_basis_points = 8_000;
        }
        prepared.audit_log.push(AuditRecord {
            day: prepared.clock.day(),
            kind: AuditKind::HeirDesignation,
            subject: format!("dynasty:{dynasty_id}").into(),
            detail: format!(
                "prior_heir={heir_id};heir={heir_id};confirmation=true;legitimacy_cost=0;unity_cost=0"
            ),
        });

        let unprepared_lines =
            decide_successions(&mut unprepared).expect("forced succession must be planned");
        let prepared_lines =
            decide_successions(&mut prepared).expect("forced succession must be planned");
        let unprepared_line = unprepared_lines
            .iter()
            .find(|line| line.dynasty_id == dynasty_id)
            .expect("unprepared player succession must exist");
        let prepared_line = prepared_lines
            .iter()
            .find(|line| line.dynasty_id == dynasty_id)
            .expect("prepared player succession must exist");

        assert!(!unprepared_line.formally_prepared);
        assert!(prepared_line.formally_prepared);
        assert!(prepared_line.family_unity_loss < unprepared_line.family_unity_loss);
        assert!(prepared_line.family_loyalty_loss < unprepared_line.family_loyalty_loss);
        assert!(prepared_line.legitimacy_loss < unprepared_line.legitimacy_loss);

        apply_successions(&mut unprepared, unprepared_lines)
            .expect("unprepared succession must succeed");
        apply_successions(&mut prepared, prepared_lines).expect("prepared succession must succeed");

        assert!(
            prepared
                .family_councils
                .get(&dynasty_id)
                .expect("prepared council must exist")
                .unity_basis_points
                > unprepared
                    .family_councils
                    .get(&dynasty_id)
                    .expect("unprepared council must exist")
                    .unity_basis_points,
            "formal succession planning must preserve more family cohesion"
        );
        assert!(
            prepared
                .dynasties
                .get(&dynasty_id)
                .expect("prepared dynasty must exist")
                .resources
                .legitimacy_basis_points
                > unprepared
                    .dynasties
                    .get(&dynasty_id)
                    .expect("unprepared dynasty must exist")
                    .resources
                    .legitimacy_basis_points,
            "formal succession planning must preserve more legitimacy"
        );
        assert!(prepared.outbox.iter().any(|message| {
            message.kind == OutboxKind::Family
                && message.subject.contains("new generation inherited")
        }));
    }

    #[test]
    fn reserved_generation_rejects_forced_succession() {
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        state
            .characters
            .get_mut(head_id)
            .expect("dynasty head must exist")
            .runtime
            .health_basis_points = 0;
        state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("player dynasty must exist")
            .runtime
            .generation = u16::MAX - 1;
        let before = state.clone();

        let result = decide_successions(&mut state);

        assert!(matches!(
            result,
            Err(SimulationError::DynastyGenerationExhausted {
                dynasty_id: exhausted_dynasty_id,
            }) if exhausted_dynasty_id == dynasty_id
        ));
        assert_state_unchanged(
            &before,
            &state,
            "generation exhaustion must not partially plan or apply succession",
        );
    }

    #[test]
    fn exhausted_succession_record_allocation_leaves_state_unchanged() {
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        state
            .characters
            .get_mut(head_id)
            .expect("dynasty head must exist")
            .runtime
            .health_basis_points = 0;
        let lines = decide_successions(&mut state).expect("forced succession must be planned");
        assert!(
            lines.iter().any(|line| line.dynasty_id == dynasty_id),
            "fixture must plan a player succession"
        );
        let mut value = serde_json::to_value(state).expect("state must serialize");
        value["next_ids"]["family_link"] = serde_json::Value::from(u32::MAX - 1);
        let mut state: AppState =
            serde_json::from_value(value).expect("allocator exhaustion fixture must deserialize");
        let before = state.clone();

        let result = apply_successions(&mut state, lines);

        assert!(matches!(
            result,
            Err(SimulationError::IdentifierAllocation(_))
        ));
        assert_state_unchanged(
            &before,
            &state,
            "succession allocation failure must not expose a retired head or partially inserted heir",
        );
    }

    #[test]
    fn annual_health_reflects_age_and_epidemic_pressure() {
        assert_eq!(
            resolve_annual_health(0, 30, 0),
            0,
            "zero health must be terminal rather than recovering automatically"
        );
        assert_eq!(
            resolve_annual_health(9_000, 30, 0),
            9_100,
            "young healthy characters should recover modestly"
        );
        assert_eq!(
            resolve_annual_health(9_000, 70, 0),
            8_300,
            "old age should reduce health"
        );
        assert_eq!(
            resolve_annual_health(9_000, 70, 5_000),
            7_800,
            "an active epidemic should compound age-related health loss"
        );
    }

    struct IncapacitationFixture {
        state: AppState,
        dynasty_id: DynastyId,
        head_id: CharacterId,
        character_id: CharacterId,
        ward_link_id: FamilyLinkId,
        kinship_link_id: FamilyLinkId,
        institution_id: InstitutionId,
        business_id: BusinessId,
    }

    fn make_incapacitation_fixture() -> IncapacitationFixture {
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let character_id = state.next_ids.character();
        state.characters.insert(Character {
            identity: CharacterIdentity {
                id: character_id,
                dynasty_id,
                name: "Ailing Steward".to_owned(),
                birth_day: state.clock.day().saturating_sub(30 * 360),
            },
            capabilities: CharacterCapabilities {
                administration: 70,
                commerce: 60,
                social: 50,
                craft: 40,
            },
            runtime: CharacterRuntime {
                status: CharacterStatus::Active,
                health_basis_points: 1,
                loyalty_basis_points: 8_000,
                role: CharacterRole::Clerk,
            },
        });
        state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("player dynasty must have a family council")
            .members
            .insert(character_id);
        let (ward_link_id, kinship_link_id) =
            insert_incapacitation_family_links(&mut state, head_id, character_id);
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        let institution = state
            .institutions
            .get_mut(&institution_id)
            .expect("institution must exist");
        institution.members.insert(character_id);
        institution.office_holder_id = Some(character_id);
        let business_id = state
            .businesses
            .ids_for_owner(dynasty_id)
            .and_then(|businesses| businesses.iter().next())
            .copied()
            .expect("player dynasty must own a business");
        state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist")
            .operations
            .manager_id = character_id;
        insert_test_epidemic(&mut state);
        IncapacitationFixture {
            state,
            dynasty_id,
            head_id,
            character_id,
            ward_link_id,
            kinship_link_id,
            institution_id,
            business_id,
        }
    }

    fn insert_incapacitation_family_links(
        state: &mut AppState,
        head_id: CharacterId,
        character_id: CharacterId,
    ) -> (FamilyLinkId, FamilyLinkId) {
        let ward_link_id = state.next_ids.family_link();
        state.family_links.insert(
            ward_link_id,
            FamilyLink {
                id: ward_link_id,
                first_character_id: head_id,
                second_character_id: character_id,
                kind: FamilyLinkKind::Ward,
                active: true,
            },
        );
        let kinship_link_id = state.next_ids.family_link();
        state.family_links.insert(
            kinship_link_id,
            FamilyLink {
                id: kinship_link_id,
                first_character_id: head_id,
                second_character_id: character_id,
                kind: FamilyLinkKind::Sibling,
                active: true,
            },
        );
        (ward_link_id, kinship_link_id)
    }

    fn insert_test_epidemic(state: &mut AppState) {
        let crisis_id = state.next_ids.crisis();
        state.crises.insert(
            crisis_id,
            crate::core::Crisis {
                id: crisis_id,
                kind: CrisisKind::Epidemic,
                district_id: None,
                started_day: state.clock.day(),
                severity_basis_points: 10_000,
                status: crate::core::CrisisStatus::Escalated,
                cause: "test epidemic".to_owned(),
            },
        );
    }

    #[test]
    fn zero_health_incapacitation_synchronizes_dependent_records() {
        let registry = rivergate_registry_for_test();
        let IncapacitationFixture {
            mut state,
            dynasty_id,
            head_id,
            character_id,
            ward_link_id,
            kinship_link_id,
            institution_id,
            business_id,
        } = make_incapacitation_fixture();

        update_character_health(&mut state).expect("character health update must succeed");

        let character = state
            .characters
            .get(character_id)
            .expect("test character must remain recorded");
        assert_eq!(character.status(), CharacterStatus::Incapacitated);
        assert_eq!(character.runtime.health_basis_points, 0);
        assert!(
            !state
                .family_councils
                .get(&dynasty_id)
                .expect("family council must exist")
                .members
                .contains(&character_id)
        );
        assert!(
            !state
                .family_links
                .get(&ward_link_id)
                .expect("ward link must remain recorded")
                .active
        );
        assert!(
            state
                .family_links
                .get(&kinship_link_id)
                .expect("kinship link must remain recorded")
                .active,
            "incapacitation must not erase historical kinship"
        );
        let institution = state
            .institutions
            .get(&institution_id)
            .expect("institution must exist");
        assert!(!institution.members.contains(&character_id));
        assert_eq!(institution.office_holder_id, None);
        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("player business must exist")
                .manager_id(),
            head_id
        );
        assert!(state.outbox.iter().any(|message| {
            message.kind() == OutboxKind::Family
                && message.body.contains(&character_id.to_string())
                && message.body.contains("health reached zero")
        }));
        validate_invariants(registry, &state);
    }

    #[test]
    fn designated_heir_retains_minimum_health_until_succession_can_replace_them() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let heir_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("player dynasty must have an heir");
        state
            .characters
            .get_mut(heir_id)
            .expect("player heir must exist")
            .runtime
            .health_basis_points = 1;
        let crisis_id = state.next_ids.crisis();
        state.crises.insert(
            crisis_id,
            crate::core::Crisis {
                id: crisis_id,
                kind: CrisisKind::Epidemic,
                district_id: None,
                started_day: state.clock.day(),
                severity_basis_points: 10_000,
                status: crate::core::CrisisStatus::Escalated,
                cause: "test epidemic".to_owned(),
            },
        );

        update_character_health(&mut state).expect("character health update must succeed");

        let heir = state
            .characters
            .get(heir_id)
            .expect("player heir must remain recorded");
        assert_eq!(heir.status(), CharacterStatus::Active);
        assert_eq!(heir.runtime.health_basis_points, 1);
        validate_invariants(registry, &state);
    }

    #[test]
    fn collapsing_head_without_heir_designates_an_emergency_successor() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        // Strip the fixture heir so the house has no designated successor,
        // then collapse the head's health past any recovery.
        state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("player dynasty must exist")
            .relationships
            .heir_id = None;
        state
            .characters
            .get_mut(head_id)
            .expect("dynasty head must exist")
            .runtime
            .health_basis_points = 0;

        super::process_year_boundary(registry, &mut state)
            .expect("the annual succession pass must run");

        let successor_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        assert_ne!(successor_id, head_id);
        let successor = state
            .characters
            .get(successor_id)
            .expect("emergency successor must exist");
        assert_eq!(successor.dynasty_id(), dynasty_id);
        assert_eq!(successor.status(), CharacterStatus::Active);
        assert!(
            state.clock.day().saturating_sub(successor.birth_day())
                >= crate::systems::commands::HEIR_MINIMUM_AGE_DAYS,
            "the emergency successor must be an adult"
        );
        assert_eq!(
            state
                .characters
                .get(head_id)
                .expect("outgoing head must remain recorded")
                .status(),
            CharacterStatus::Deceased,
            "a collapsed head must not keep operating the house"
        );
        validate_invariants(registry, &state);
    }

    #[test]
    fn succession_chance_uses_health_and_recorded_risk() {
        let baseline = succession_chance_basis_points(60, 1_000, 9_000);
        let overextended = succession_chance_basis_points(60, 5_000, 9_000);
        let unhealthy = succession_chance_basis_points(60, 1_000, 3_000);

        assert!(
            overextended > baseline,
            "the stored succession-risk measure must affect succession"
        );
        assert!(
            unhealthy > baseline,
            "poor health must increase annual succession probability"
        );
        assert_eq!(
            succession_chance_basis_points(49, 10_000, 0),
            0,
            "the minimum succession age remains explicit"
        );
        assert!(
            succession_chance_basis_points(50, 1_000, 9_000) > 0,
            "an eligible head must begin to accumulate annual succession pressure"
        );
        assert!(
            succession_chance_basis_points(60, 1_000, 9_000)
                > succession_chance_basis_points(50, 1_000, 9_000),
            "age must increase annual succession pressure once eligible"
        );
    }
}

mod market_prices {
    use super::*;

    #[test]
    fn extreme_valid_market_flows_clamp_pressure_without_overflow() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let grain_id = registry
            .get_good_id("grain")
            .expect("registry must define grain");
        let quote = state
            .market
            .quotes
            .get_mut(&grain_id)
            .expect("grain quote must exist");
        quote.stock = Quantity::from_milliunits(i64::MAX);
        quote.demand_today = Quantity::ZERO;
        quote.supply_today = Quantity::from_milliunits(i64::MAX);
        let previous_price = quote.price;

        update_market_prices(registry, &mut state)
            .expect("extreme but valid market state must update deterministically");

        let updated = state
            .market
            .get_quote(grain_id)
            .expect("grain quote must remain present");
        assert_eq!(updated.previous_price(), previous_price);
        assert!(updated.price() > Money::ZERO);
    }

    #[test]
    fn production_floor_covers_operating_labor_and_maintenance_costs() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let flour_id = registry
            .get_good_id("flour")
            .expect("registry must define flour");
        let floor = production_price_floors(registry, &state)
            .get(&flour_id)
            .copied()
            .expect("an operating mill must define a flour price floor");
        let authored_floor = Money::from_copper(
            registry
                .get_good(flour_id)
                .expect("flour definition must exist")
                .base_price()
                .copper()
                / 2,
        );
        assert!(
            floor > authored_floor,
            "the sustainable floor must include staffing and maintenance overhead"
        );
        {
            let quote = state
                .market
                .quotes
                .get_mut(&flour_id)
                .expect("flour quote must exist");
            quote.price = Money::from_copper(1);
            quote.stock = quote.target_stock.saturating_mul_ratio(4, 1);
            quote.demand_today = Quantity::ZERO;
            quote.supply_today = Quantity::from_units(10_000);
        }

        update_market_prices(registry, &mut state).expect("market price update must succeed");

        assert!(
            state
                .market
                .get_quote(flour_id)
                .expect("flour quote must exist")
                .price()
                >= floor,
            "oversupply must not push a produced good below sustainable cost"
        );
    }

    #[test]
    fn repeated_shocks_for_the_same_good_do_not_flood_the_chronicle() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let flour_id = registry
            .get_good_id("flour")
            .expect("registry must define flour");
        let oversupply = |state: &mut AppState| {
            let quote = state
                .market
                .quotes
                .get_mut(&flour_id)
                .expect("flour quote must exist");
            quote.price = Money::from_copper(40);
            quote.stock = quote.target_stock.saturating_mul_ratio(4, 1);
            quote.demand_today = Quantity::ZERO;
            quote.supply_today = Quantity::from_units(10_000);
        };
        let shock_count = |state: &AppState| {
            state
                .chronicle
                .iter()
                .filter(|entry| {
                    entry.kind == ChronicleKind::PriceShock && entry.summary.starts_with("Flour")
                })
                .count()
        };

        oversupply(&mut state);
        update_market_prices(registry, &mut state).expect("first price update must succeed");
        assert_eq!(
            shock_count(&state),
            1,
            "the first shock for a good must be recorded"
        );

        // The next day's update still moves flour past the shock threshold, but
        // a sustained slide must not add a chronicle entry every day.
        state.clock.advance_one_day();
        oversupply(&mut state);
        update_market_prices(registry, &mut state).expect("second price update must succeed");
        assert_eq!(
            shock_count(&state),
            1,
            "a repeat shock within the suppression window must stay silent"
        );

        // After the window passes, the trend's continuation is news again.
        for _ in 0..PRICE_SHOCK_REPEAT_SUPPRESSION_DAYS {
            state.clock.advance_one_day();
        }
        oversupply(&mut state);
        update_market_prices(registry, &mut state).expect("third price update must succeed");
        assert_eq!(
            shock_count(&state),
            2,
            "a shock after the suppression window must be recorded again"
        );
    }

    #[test]
    fn production_floor_keeps_exact_payroll_before_daily_conversion() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let target_business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let target_recipe_id = state
            .businesses
            .get(target_business_id)
            .expect("target business must exist")
            .recipe_id();
        let target_good_id = registry
            .get_recipe(target_recipe_id)
            .expect("target recipe must exist")
            .output_good_id();
        for business in state.businesses.iter_mut() {
            if business.id() != target_business_id
                && registry
                    .get_recipe(business.recipe_id())
                    .expect("business recipe must exist")
                    .output_good_id()
                    == target_good_id
            {
                business.operations.status = BusinessStatus::Closed;
            }
        }
        let employment_ids: Vec<_> = state.employment.keys().copied().take(2).collect();
        let [first_id, second_id] = employment_ids.as_slice() else {
            panic!("campaign must contain at least two employment agreements");
        };
        let wage = Money::from_copper((i64::MAX / 4) * 3);
        for employment_id in [*first_id, *second_id] {
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("employment agreement must exist");
            agreement.business_id = target_business_id;
            agreement.weekly_wage = wage;
            agreement.status = EmploymentStatus::Active;
        }

        let recipe = registry
            .get_recipe(target_recipe_id)
            .expect("target recipe must exist");
        let exact_daily_labor = ceil_div_nonnegative_wide(i128::from(wage.copper()) * 2, 7);
        assert!(
            exact_daily_labor > ceil_div_nonnegative_wide(i128::from(i64::MAX), 7),
            "test setup must exceed a saturate-then-divide payroll calculation"
        );
        let expected_batches = i64::from(effective_capacity_batches(
            &state,
            state
                .businesses
                .get(target_business_id)
                .expect("target business must exist"),
        ));
        let minimum_labor_only_floor = Money::from_copper(
            i64::try_from(exact_daily_labor)
                .expect("test daily labor remains within the Money range"),
        )
        .saturating_mul_ratio_ceil_nonnegative(1_000, expected_batches * 1_000)
        .saturating_mul_ratio_ceil_nonnegative(1_000, recipe.output_quantity().milliunits())
        .saturating_mul_ratio_ceil_nonnegative(11, 10);

        let floor = production_price_floors(registry, &state)
            .get(&target_good_id)
            .copied()
            .expect("target business must define a production floor");

        assert!(
            floor >= minimum_labor_only_floor,
            "the sustainable price floor must reflect exact aggregate payroll before weekly-to-daily conversion"
        );
    }
}

mod laws {
    use super::*;

    #[test]
    fn bread_price_ceiling_is_final_price_constraint() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::BreadPriceCeiling,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                value: 1,
                active: true,
            },
        );

        advance_days(registry, &mut state, 1).expect("simulation must advance");

        let bread_id = registry
            .get_good_id("bread")
            .expect("registry must define bread");
        assert_eq!(
            state
                .market
                .get_quote(bread_id)
                .expect("bread quote must exist")
                .price(),
            Money::from_copper(1),
            "the statutory ceiling must be the final daily price constraint"
        );
    }
}

mod time_limited_state {
    use super::*;

    #[test]
    fn advancing_purges_records_after_their_expiry_day() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let report_id = state
            .information_reports
            .keys()
            .copied()
            .next()
            .expect("campaign must contain an information report");
        let current_day = state.clock.day();
        state
            .information_reports
            .get_mut(&report_id)
            .expect("information report must exist")
            .expires_day = current_day;

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
            .active_directive = Some(crate::core::OfficeDirectiveState {
            power,
            expires_day: current_day,
        });

        advance_days(registry, &mut state, 1).expect("simulation must advance");

        assert!(
            !state.information_reports.contains_key(&report_id),
            "expired information must not survive the daily boundary"
        );
        assert!(
            state
                .institutions
                .get(&institution_id)
                .expect("institution must exist")
                .active_directive
                .is_none(),
            "expired directives must not remain as stale active state"
        );
    }
}
