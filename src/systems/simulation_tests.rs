//! Behavioral tests for daily simulation planning, ordering, and preflight validation.

use super::*;
use crate::core::{EnactedLaw, LawKind};
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

mod labor {
    use super::*;

    #[test]
    fn disputed_employment_prevents_production() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let initial_plan = decide_production(registry, &state);
        let business_id = initial_plan
            .lines
            .first()
            .expect("bootstrap must include a business able to produce")
            .business_id;
        for agreement in state
            .employment
            .values_mut()
            .filter(|agreement| agreement.business_id == business_id)
        {
            agreement.status = EmploymentStatus::Disputed;
        }

        let plan = decide_production(registry, &state);

        assert!(
            plan.lines
                .iter()
                .all(|line| line.business_id != business_id),
            "a business without active workers must not produce"
        );
    }
}

mod inventory_policy {
    use super::*;

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
}

mod health_and_succession {
    use super::*;

    #[test]
    fn annual_health_reflects_age_and_epidemic_pressure() {
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
