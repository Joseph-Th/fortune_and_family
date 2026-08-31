#![allow(dead_code)]
//! Workshop maintenance: condition/quality drift, tool demand, and repair.
//!
//! Purpose: own the daily maintenance decision/apply pair so the simulation
//! orchestrator tracks purchases/production/sales without owning condition
//! arithmetic.
//! Owns: `MaintenanceSnapshot`/`Line`/`Plan`, `decide_maintenance`,
//! `maintenance_cost`, `maintenance_effect`, `apply_maintenance` plus tool
//! demand sharing with production.
//! Reads: `Registry`, `AppState` businesses and managers, guild standing.
//! Mutates: business condition/quality/cash via the validated plan; market
//! tool stock/demand and clearing pool.
//! Does not own: business lifecycle or succession.
//! Invariants: every business has positive capacity; distressed firms ignore
//! `minimum_cash_reserve`; quality mean-reverts to its guild-adjusted target.
//! Focused tests: `simulation_tests` maintenance and tool priority.

use super::SimulationError;
use crate::core::{AppState, AuditKind, AuditRecord, BusinessStatus};
use crate::ids::{BusinessId, GoodId};
use crate::money::{Money, Quantity, affordable_quantity, cost_for};
use crate::registry::Registry;
use crate::systems::transactions::next_business_finance_version;

#[derive(Clone, Copy)]
pub(crate) struct MaintenanceSnapshot {
    pub(crate) business_id: BusinessId,
    pub(crate) recipe_id: crate::ids::RecipeId,
    pub(crate) cash: Money,
    pub(crate) minimum_cash_reserve: Money,
    pub(crate) maintenance_basis_points: u16,
    pub(crate) quality_target_basis_points: u16,
    pub(crate) condition_basis_points: u16,
    pub(crate) quality_basis_points: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct MaintenanceLine {
    pub(crate) business_id: BusinessId,
    pub(crate) cost: Money,
    pub(crate) tool_quantity: Quantity,
    pub(crate) tool_cost: Money,
    pub(crate) condition_delta: i16,
    pub(crate) quality_delta: i16,
}

#[derive(Clone, Debug)]
pub(crate) struct MaintenancePlan {
    pub(crate) tools_id: GoodId,
    pub(crate) lines: Vec<MaintenanceLine>,
}

pub(crate) fn decide_maintenance(registry: &Registry, state: &mut AppState) -> MaintenancePlan {
    let tools_id = registry
        .get_good_id("tools")
        .expect("Rivergate registry must define tools");
    let tools_quote = state
        .market
        .quotes
        .get(&tools_id)
        .expect("Rivergate market must define tools");
    let tools_price = tools_quote.price;
    let mut remaining_tools_stock = tools_quote.stock;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let day_hash = state.clock.day() as u32;
    let mut snapshots: Vec<_> = state
        .businesses
        .iter()
        .filter(|business| {
            !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
        })
        .map(|business| {
            let guild_quality_bonus = if crate::systems::manager_holds_chartered_guild_membership(
                registry,
                state,
                business.recipe_id(),
                business.manager_id(),
            ) {
                crate::systems::GUILD_CRAFT_QUALITY_TARGET_BONUS
            } else {
                0
            };
            MaintenanceSnapshot {
                business_id: business.id(),
                recipe_id: business.recipe_id(),
                cash: business.cash(),
                minimum_cash_reserve: if business.status() == BusinessStatus::Distressed {
                    Money::ZERO
                } else {
                    business.policy.minimum_cash_reserve
                },
                maintenance_basis_points: business.policy.maintenance_basis_points,
                quality_target_basis_points: business
                    .policy
                    .quality_target_basis_points
                    .saturating_add(guild_quality_bonus),
                condition_basis_points: business.operations.condition_basis_points,
                quality_basis_points: business.operations.quality_basis_points,
            }
        })
        .collect();
    snapshots.sort_by_key(|snapshot| snapshot.business_id.value().wrapping_add(day_hash));
    let lines = snapshots
        .into_iter()
        .map(|snapshot| {
            maintenance_line(
                registry,
                state,
                snapshot,
                tools_id,
                tools_price,
                &mut remaining_tools_stock,
            )
        })
        .collect();
    MaintenancePlan { tools_id, lines }
}

fn maintenance_line(
    registry: &Registry,
    state: &mut AppState,
    snapshot: MaintenanceSnapshot,
    tools_id: GoodId,
    tools_price: Money,
    remaining_tools_stock: &mut Quantity,
) -> MaintenanceLine {
    let MaintenanceSnapshot {
        business_id,
        recipe_id,
        cash,
        minimum_cash_reserve,
        maintenance_basis_points,
        quality_target_basis_points,
        condition_basis_points,
        quality_basis_points,
    } = snapshot;
    let recipe = registry
        .get_recipe(recipe_id)
        .expect("business recipe reference must be valid");
    let desired_cost = maintenance_cost(recipe.daily_operating_cost(), maintenance_basis_points);
    let effect_points = maintenance_effect(maintenance_basis_points, condition_basis_points);
    let can_maintain =
        desired_cost > Money::ZERO && cash.saturating_sub(minimum_cash_reserve) >= desired_cost;
    let tool_budget = if can_maintain && recipe.output_good_id() != tools_id {
        desired_cost
    } else {
        Money::ZERO
    };
    let required_tool_quantity = affordable_quantity(tool_budget, tools_price);
    let tool_quantity = (*remaining_tools_stock).min(required_tool_quantity);
    let tool_cost = cost_for(tool_quantity, tools_price);
    *remaining_tools_stock = (*remaining_tools_stock).saturating_sub(tool_quantity);
    let tools_available = tool_quantity >= required_tool_quantity;
    let maintenance_succeeds =
        can_maintain && (recipe.output_good_id() == tools_id || tools_available);
    let random_wear = i16::try_from(state.rng.range_u32(4)).expect("wear fits i16");
    let neglect_penalty = if maintenance_succeeds { 0 } else { 5 };
    let accident_penalty = if condition_basis_points < 4_000 && state.rng.is_chance_success(120) {
        30
    } else {
        0
    };
    let improvement = if maintenance_succeeds && condition_basis_points < 9_500 {
        effect_points
    } else {
        0
    };
    let quality_improvement =
        if maintenance_succeeds && quality_basis_points < quality_target_basis_points {
            i16::try_from(
                (quality_target_basis_points - quality_basis_points)
                    .min(u16::try_from(effect_points).expect("maintenance effect is nonnegative")),
            )
            .expect("bounded quality improvement must fit i16")
        } else {
            0
        };
    let quality_target_excess_decline =
        if maintenance_succeeds && quality_basis_points > quality_target_basis_points {
            i16::try_from(
                (quality_basis_points - quality_target_basis_points)
                    .min(u16::try_from(effect_points).expect("maintenance effect is nonnegative"))
                    .div_ceil(4),
            )
            .expect("bounded quality decline must fit i16")
        } else {
            0
        };
    let quality_decline = if maintenance_succeeds {
        quality_target_excess_decline
    } else {
        3
    };
    MaintenanceLine {
        business_id,
        cost: if maintenance_succeeds {
            desired_cost
        } else {
            Money::ZERO
        },
        tool_quantity,
        tool_cost,
        condition_delta: improvement - 2 - random_wear - neglect_penalty - accident_penalty,
        quality_delta: quality_improvement - quality_decline,
    }
}

pub(crate) fn maintenance_cost(
    daily_operating_cost: Money,
    maintenance_basis_points: u16,
) -> Money {
    if maintenance_basis_points == 0 || daily_operating_cost <= Money::ZERO {
        return Money::ZERO;
    }
    daily_operating_cost
        .saturating_mul_ratio_ceil_nonnegative(i64::from(maintenance_basis_points), 20_000)
}

fn maintenance_effect(maintenance_basis_points: u16, condition_basis_points: u16) -> i16 {
    let scaled = u32::from(maintenance_basis_points)
        .saturating_mul(32)
        .div_ceil(10_000);
    let catch_up = u32::from(9_500_u16.saturating_sub(condition_basis_points)).div_ceil(400);
    i16::try_from(scaled.saturating_add(catch_up)).expect("bounded maintenance effect must fit i16")
}

pub(crate) fn apply_maintenance(
    state: &mut AppState,
    plan: MaintenancePlan,
) -> Result<(), SimulationError> {
    let MaintenancePlan { tools_id, lines } = plan;
    let mut total_cost_copper = 0_i128;
    let mut total_tool_cost_copper = 0_i128;
    let mut total_tool_quantity_milliunits = 0_i128;
    for line in lines {
        let MaintenanceLine {
            business_id,
            cost,
            tool_quantity,
            tool_cost,
            condition_delta,
            quality_delta,
        } = line;
        let market_update =
            super::planned_tool_market_update(state, tools_id, tool_quantity, tool_cost)?;
        let charge = cost.max(tool_cost);
        let tool_backed_clearing = match market_update {
            Some((_, _, clearing)) => clearing,
            None => state.market.clearing_account,
        };
        let service_residual = charge.saturating_sub(tool_cost);
        let resulting_clearing = tool_backed_clearing.checked_add(service_residual).ok_or(
            SimulationError::MarketClearingAccountOverflow {
                current: tool_backed_clearing,
                change: service_residual,
            },
        )?;
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("planned maintenance business must exist");
        if charge > Money::ZERO {
            let resulting_cash = business
                .finance
                .cash
                .checked_sub(charge)
                .expect("planned maintenance must fit available cash");
            let resulting_lifetime_costs =
                business.finance.lifetime_costs.checked_add(charge).ok_or(
                    SimulationError::BusinessLifetimeCostsOverflow {
                        business_id,
                        current: business.finance.lifetime_costs,
                        incoming: charge,
                    },
                )?;
            let next_finance_version = next_business_finance_version(business)?;
            business.finance.cash = resulting_cash;
            business.finance.lifetime_costs = resulting_lifetime_costs;
            business.finance.version = next_finance_version;
        }
        let condition = i32::from(business.operations.condition_basis_points)
            .saturating_add(i32::from(condition_delta))
            .clamp(0, 10_000);
        business.operations.condition_basis_points =
            u16::try_from(condition).expect("clamped condition must fit u16");
        let quality = i32::from(business.operations.quality_basis_points)
            .saturating_add(i32::from(quality_delta))
            .clamp(0, 10_000);
        business.operations.quality_basis_points =
            u16::try_from(quality).expect("clamped quality must fit u16");
        if let Some((resulting_stock, resulting_demand, _)) = market_update {
            let quote = state
                .market
                .quotes
                .get_mut(&tools_id)
                .expect("planned maintenance tools quote must exist");
            quote.stock = resulting_stock;
            quote.demand_today = resulting_demand;
        }
        state.market.clearing_account = resulting_clearing;
        total_cost_copper += i128::from(cost.copper());
        total_tool_cost_copper += i128::from(tool_cost.copper());
        total_tool_quantity_milliunits += i128::from(tool_quantity.milliunits());
    }
    if total_cost_copper != 0 {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::Maintenance,
            subject: "businesses".into(),
            detail: format!(
                "cost={total_cost_copper}; tools={total_tool_quantity_milliunits}; tool_spending={total_tool_cost_copper}"
            )
            .into(),
        });
    }
    Ok(())
}
