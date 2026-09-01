//! Market spoilage, price formation, and production break-even.
//!
//! Purpose: own the daily spoilage (market + business inventories) and the
//! price-update pipeline (pressure, floors, shocks, chronicle) so the
//! simulation orchestrator stays causal without owning price arithmetic.
//! Owns: `apply_market_spoilage`, `update_market_prices`, price-shock
//! helpers, `business_sustainable_unit_cost`, `production_price_floors`,
//! seasonal pressure, and `ceil_div_nonnegative_wide` re-export.
//! Reads: `Registry` goods/recipes, `AppState` market/businesses/employment.
//! Mutates: `AppState.market` quotes/prices and chronicle for price shocks.
//! Does not own: production or household decisions.
//! Relevant invariants: every good has a market quote; break-even median is robust to
//! outliers; price shocks are suppressed for 14 days per good.
//! Canonical operations: `apply_market_spoilage`, `production_price_floors`, pricing.
//! Focused tests: `src/systems/simulation/simulation_tests.rs` market.

use super::{effective_capacity_batches, maintenance_cost};
use crate::core::{AppState, ChronicleEntry, ChronicleKind, MarketCause};
use crate::ids::GoodId;
use crate::money::{Money, Quantity, cost_for};
use crate::registry::{GoodCategory, Registry};
use crate::systems::SimulationError;
use std::collections::{BTreeMap, BTreeSet};

/// Days a good stays silent in the chronicle after a recorded price shock.
pub(crate) const PRICE_SHOCK_REPEAT_SUPPRESSION_DAYS: i64 = 14;
pub(crate) const PRICE_SHOCKS_PER_DAY: u32 = 3;

/// One authored sentence shape for price shocks, shared by the writer and the
/// suppression reader so wording drift can never silently break suppression.
pub(crate) const PRICE_SHOCK_SUMMARY_SEPARATOR: &str = " moved by ";

/// Applies daily spoilage to market stock and every business inventory.
///
/// Spoilage is a pure proportional decay (`stock * basis_points / 10_000`)
/// so a zero-spoilage good never loses stock and a high-spoilage staple
/// cannot go negative — `saturating_sub` prevents underflow even under
/// extreme rounding. This runs before pricing so decayed supply
/// immediately tightens the next price formation.
pub(crate) fn apply_market_spoilage(registry: &Registry, state: &mut AppState) {
    for good in registry.goods() {
        let quote = state
            .market
            .quotes
            .get_mut(&good.id())
            .expect("every registry good must have a market quote");
        let spoiled = quote
            .stock
            .saturating_mul_ratio(i64::from(good.daily_spoilage_basis_points()), 10_000);
        quote.stock = quote.stock.saturating_sub(spoiled);
    }
    for business in state.businesses.iter_mut() {
        let goods_to_spoil: Vec<(GoodId, Quantity)> = business
            .inventory
            .iter()
            .map(|(good_id, quantity)| (*good_id, *quantity))
            .collect();
        for (good_id, quantity) in goods_to_spoil {
            let spoilage_bp = registry
                .get_good(good_id)
                .map_or(0, crate::registry::GoodDef::daily_spoilage_basis_points);
            if spoilage_bp == 0 {
                continue;
            }
            let spoiled = quantity.saturating_mul_ratio(i64::from(spoilage_bp), 10_000);
            if spoiled.is_zero() {
                continue;
            }
            business.remove_inventory(good_id, spoiled);
        }
    }
}

pub(crate) fn update_market_prices(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day_of_year = state.clock.day_of_year();
    let production_floors = production_price_floors(registry, state);
    let mut price_shocks = Vec::new();

    for good in registry.goods() {
        let quote = state
            .market
            .quotes
            .get_mut(&good.id())
            .expect("every registry good must have a market quote");
        let target = quote.target_stock.milliunits().max(1);
        let stock_gap = target - quote.stock.milliunits();
        let stock_pressure = (stock_gap.saturating_mul(1_000) / target).clamp(-1_000, 1_000);
        let total_flow = quote
            .demand_today
            .milliunits()
            .saturating_add(quote.supply_today.milliunits())
            .max(1);
        let flow_gap = quote
            .demand_today
            .milliunits()
            .saturating_sub(quote.supply_today.milliunits());
        let mut flow_pressure = (flow_gap.saturating_mul(500) / total_flow).clamp(-500, 500);
        if flow_pressure > 0 && quote.stock.milliunits() >= target {
            flow_pressure = 0;
        }
        let seasonal_pressure = seasonal_pressure_basis_points(good.category(), day_of_year);
        let total_pressure = i128::from(stock_pressure)
            .saturating_add(i128::from(flow_pressure))
            .saturating_add(i128::from(seasonal_pressure))
            .clamp(-800, 800);
        let previous_price = quote.price;
        let raw_price = previous_price
            .saturating_mul_ratio(
                10_000
                    + i64::try_from(total_pressure)
                        .expect("clamped market pressure must fit the fixed-point ratio"),
                10_000,
            )
            .copper();
        let half_base = Money::from_copper((good.base_price().copper() / 2).max(1));
        let maximum_price = good.base_price().copper().saturating_mul(4).max(1);
        let minimum_price = production_floors
            .get(&good.id())
            .copied()
            .unwrap_or(half_base)
            .max(half_base)
            .copper();
        quote.previous_price = previous_price;
        quote.price =
            Money::from_copper(raw_price.clamp(minimum_price, maximum_price.max(minimum_price)));
        quote.causes = decide_market_causes(quote, seasonal_pressure);

        let change_basis_points = if previous_price.copper() == 0 {
            0
        } else {
            Money::from_copper(quote.price.copper().saturating_sub(previous_price.copper()))
                .saturating_mul_ratio(10_000, previous_price.copper())
                .copper()
        };
        if change_basis_points.unsigned_abs() >= 700 {
            price_shocks.push((good.name().to_owned(), quote.price, change_basis_points));
        }
    }

    price_shocks.sort_by(|left, right| {
        right
            .2
            .unsigned_abs()
            .cmp(&left.2.unsigned_abs())
            .then_with(|| left.0.cmp(&right.0))
    });
    let recently_shocked = recently_shocked_goods(state);
    let mut emitted = 0_u32;
    for (good_name, price, change_basis_points) in price_shocks {
        if emitted >= PRICE_SHOCKS_PER_DAY {
            break;
        }
        if recently_shocked.contains(&good_name) {
            continue;
        }
        let id = state.next_ids.try_chronicle()?;
        state.chronicle.push(ChronicleEntry {
            id,
            day: state.clock.day(),
            kind: ChronicleKind::PriceShock,
            summary: price_shock_summary(&good_name, change_basis_points, price),
        });
        emitted += 1;
    }
    Ok(())
}

pub(crate) fn price_shock_summary(
    good_name: &str,
    change_basis_points: i64,
    price: Money,
) -> String {
    format!(
        "{good_name}{PRICE_SHOCK_SUMMARY_SEPARATOR}{change_basis_points} basis points to {price}."
    )
}

pub(crate) fn price_shock_good_name(summary: &str) -> &str {
    summary
        .split(PRICE_SHOCK_SUMMARY_SEPARATOR)
        .next()
        .unwrap_or(summary)
}

pub(crate) fn recently_shocked_goods(state: &AppState) -> BTreeSet<String> {
    let cutoff_day = state
        .clock
        .day()
        .saturating_sub(PRICE_SHOCK_REPEAT_SUPPRESSION_DAYS);
    let mut goods = BTreeSet::new();
    for entry in state.chronicle.iter().rev() {
        if entry.day < cutoff_day {
            break;
        }
        if entry.kind == ChronicleKind::PriceShock {
            goods.insert(price_shock_good_name(&entry.summary).to_owned());
        }
    }
    goods
}

/// The sustainable unit cost of one business's output at current market input
/// prices: input costs plus labor and maintenance overhead spread across the
/// batches the firm can actually run, charged against the efficiency-adjusted
/// output the firm really expects to yield, with a tenth again for margin.
pub(crate) fn business_sustainable_unit_cost(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
    office_administrative_load: u16,
) -> Money {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must exist");
    let weekly_labor_copper = state
        .employment
        .values()
        .filter(|agreement| {
            agreement.business_id == business.id()
                && matches!(
                    agreement.status,
                    crate::core::EmploymentStatus::Active | crate::core::EmploymentStatus::Disputed
                )
        })
        .fold(0_i128, |total, agreement| {
            total + i128::from(agreement.weekly_wage.copper())
        });
    let daily_labor_copper =
        ceil_div_nonnegative_wide(weekly_labor_copper, 7).min(i128::from(i64::MAX));
    let daily_labor = Money::from_copper(
        i64::try_from(daily_labor_copper).expect("clamped daily labor cost must fit i64"),
    );
    let daily_maintenance = maintenance_cost(
        recipe.daily_operating_cost(),
        business.policy.maintenance_basis_points,
    );
    let expected_batches = i64::from(effective_capacity_batches(
        state,
        business,
        office_administrative_load,
    ))
    .max(1);
    let overhead_per_batch = daily_labor
        .saturating_add(daily_maintenance)
        .saturating_mul_ratio_ceil_nonnegative(1_000, expected_batches * 1_000);
    let batch_cost = recipe.inputs().iter().fold(
        recipe
            .daily_operating_cost()
            .saturating_add(overhead_per_batch),
        |total, input| {
            let price = state
                .market
                .quotes
                .get(&input.good_id())
                .expect("recipe input good must have a market quote")
                .price;
            total.saturating_add(cost_for(input.quantity(), price))
        },
    );
    let expected_output = recipe
        .output_quantity()
        .saturating_mul_ratio(
            i64::from(expected_output_efficiency(state, business)),
            10_000,
        )
        .max(Quantity::from_milliunits(1));
    let break_even =
        batch_cost.saturating_mul_ratio_ceil_nonnegative(1_000, expected_output.milliunits());
    break_even.saturating_mul_ratio_ceil_nonnegative(11, 10)
}

fn expected_output_efficiency(state: &AppState, business: &crate::core::Business) -> u32 {
    let quality_efficiency = 9_000_u32
        .saturating_add(u32::from(business.operations.quality_basis_points) / 10)
        .min(10_000);
    let craft_efficiency = state
        .characters
        .get(business.manager_id())
        .map_or(10_000, |manager| {
            9_000_u32
                .saturating_add(u32::from(manager.capabilities.craft).saturating_mul(10))
                .min(10_000)
        });
    quality_efficiency * craft_efficiency / 10_000
}

pub(crate) fn production_price_floors(
    registry: &Registry,
    state: &AppState,
) -> BTreeMap<GoodId, Money> {
    let mut floors: BTreeMap<GoodId, Vec<Money>> = BTreeMap::new();
    let office_loads = super::super::strategic::dynasty_office_administrative_loads(state);
    for business in state.businesses.iter().filter(|business| {
        !matches!(
            business.status(),
            crate::core::BusinessStatus::Closed | crate::core::BusinessStatus::Insolvent
        )
    }) {
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("business recipe must exist");
        if recipe.output_quantity().milliunits() <= 0 {
            continue;
        }
        floors
            .entry(recipe.output_good_id())
            .or_default()
            .push(business_sustainable_unit_cost(
                registry,
                state,
                business,
                office_loads
                    .get(&business.owner_dynasty_id())
                    .copied()
                    .unwrap_or(0),
            ));
    }
    floors
        .into_iter()
        .map(|(good_id, mut producers)| {
            producers.sort_by_key(|price| price.copper());
            let median_i128: i128 = if producers.is_empty() {
                0
            } else if producers.len() % 2 == 1 {
                i128::from(producers[producers.len() / 2].copper())
            } else {
                let upper = i128::from(producers[producers.len() / 2].copper());
                let lower = i128::from(producers[producers.len() / 2 - 1].copper());
                ceil_div_nonnegative_wide(lower + upper, 2)
            };
            let median = i64::try_from(median_i128).unwrap_or(i64::MAX);
            (good_id, Money::from_copper(median))
        })
        .collect()
}

pub(crate) fn ceil_div_nonnegative_wide(numerator: i128, denominator: i128) -> i128 {
    crate::money::ceil_div_nonnegative_wide(numerator, denominator)
}

fn seasonal_pressure_basis_points(category: GoodCategory, day_of_year: u16) -> i64 {
    match category {
        GoodCategory::Staple => {
            if day_of_year >= 300 || day_of_year <= 60 {
                140
            } else if (210..=280).contains(&day_of_year) {
                -100
            } else {
                0
            }
        }
        GoodCategory::Fuel => {
            if day_of_year >= 300 || day_of_year <= 75 {
                110
            } else {
                0
            }
        }
        GoodCategory::Drink
        | GoodCategory::Textile
        | GoodCategory::Material
        | GoodCategory::Tool => 0,
    }
}

fn decide_market_causes(
    quote: &crate::core::MarketQuote,
    seasonal_pressure: i64,
) -> Vec<MarketCause> {
    let mut causes = Vec::new();
    if quote.stock < quote.target_stock {
        causes.push(MarketCause::StockBelowTarget);
    } else if quote.stock > quote.target_stock {
        causes.push(MarketCause::StockAboveTarget);
    }
    if quote.demand_today > quote.supply_today && quote.stock < quote.target_stock {
        causes.push(MarketCause::DemandExceededSupply);
    } else if quote.supply_today > quote.demand_today {
        causes.push(MarketCause::SupplyExceededDemand);
    }
    if seasonal_pressure != 0 {
        causes.push(MarketCause::SeasonalPressure);
    }
    if causes.is_empty() {
        causes.push(MarketCause::StableConditions);
    }
    causes
}
