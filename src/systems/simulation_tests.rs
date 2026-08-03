//! Behavioral tests for daily simulation planning, ordering, and preflight validation.

use super::*;
use crate::core::{
    BusinessStatus, ContractStatus, EnactedLaw, LawKind, NewGameConfig, StartingBackground,
};
use crate::systems::build_new_game;
use crate::test_support::{
    assert_state_unchanged, make_test_campaign, rivergate_registry_for_test,
};

mod preflight {
    use super::*;

    #[test]
    fn missing_market_quote_fails_before_day_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let missing_good_id = registry.goods()[0].id();
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
}

mod starting_economies {
    use super::*;

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
        apply_business_sales(&mut state, plan);

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
        apply_business_sales(&mut state, plan);

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

        update_business_lifecycle(registry, &mut state);

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

        update_business_lifecycle(registry, &mut state);

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

        update_business_lifecycle(registry, &mut state);

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

        update_business_lifecycle(registry, &mut state);

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

        update_business_lifecycle(registry, &mut state);

        assert_eq!(
            state
                .businesses
                .get(business_id)
                .expect("business must exist")
                .status(),
            BusinessStatus::Active
        );
        assert_eq!(state.chronicle.len(), chronicle_before + 1);
        assert!(
            state
                .employment
                .values()
                .filter(|agreement| agreement.business_id == business_id)
                .all(|agreement| agreement.status == EmploymentStatus::Disputed),
            "reopening after insolvency must preserve labor consequences instead of resetting loyalty"
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
        apply_maintenance(&mut state, plan);

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
        apply_maintenance(&mut state, plan);

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
            apply_maintenance(&mut state, plan);
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

        for _ in 0..360 {
            let plan = decide_maintenance(registry, &mut state);
            apply_maintenance(&mut state, plan);
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

mod health_and_succession {
    use super::*;

    #[test]
    fn zero_health_forces_succession_before_normal_retirement_age() {
        let mut state = make_test_campaign();
        let dynasty_id = state.player_dynasty_id;
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let age_years = state.clock.day().saturating_sub(
            state
                .characters
                .get(head_id)
                .expect("dynasty head must exist")
                .birth_day(),
        ) / 360;
        assert!(age_years < 55, "fixture head must be younger than 55");
        state
            .characters
            .get_mut(head_id)
            .expect("dynasty head must exist")
            .runtime
            .health_basis_points = 0;

        let successions = decide_successions(&mut state);

        assert!(
            successions
                .iter()
                .any(|line| { line.dynasty_id == dynasty_id && line.outgoing_head_id == head_id })
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
            succession_chance_basis_points(54, 10_000, 0),
            0,
            "the minimum succession age remains explicit"
        );
    }
}

mod market_prices {
    use super::*;

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

        update_market_prices(registry, &mut state);

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
