//! Business input procurement: decided before production and applied atomically.

use crate::core::{AppState, AuditKind, AuditRecord, BusinessStatus};
use crate::ids::{BusinessId, GoodId};
use crate::money::{Money, Quantity, affordable_quantity, cost_for};
use crate::registry::Registry;
use crate::systems::SimulationError;
use crate::systems::transactions::next_business_finance_version;

#[derive(Clone, Debug)]
pub(crate) struct BusinessPurchaseLine {
    pub(crate) business_id: BusinessId,
    pub(crate) good_id: GoodId,
    pub(crate) quantity: Quantity,
    pub(crate) cost: Money,
}

#[derive(Clone, Debug)]
pub(crate) struct BusinessPurchasePlan {
    pub(crate) lines: Vec<BusinessPurchaseLine>,
}

pub(crate) fn decide_business_purchases(
    registry: &Registry,
    state: &AppState,
    capacity_scratch: &crate::systems::DailyCapacityScratch,
) -> Result<BusinessPurchasePlan, SimulationError> {
    let mut remaining_stock = vec![Quantity::ZERO; registry.goods().len()];
    for (good_id, quote) in &state.market.quotes {
        remaining_stock[good_id.value() as usize] = quote.stock;
    }
    let cash_slots = state
        .businesses
        .records()
        .keys()
        .next_back()
        .map_or(0, |id| id.value() as usize + 1);
    let mut available_cash = vec![Money::ZERO; cash_slots];
    for business in state.businesses.iter() {
        available_cash[business.id().value() as usize] = business.cash();
    }
    let mut lines = Vec::new();

    for business in state.businesses.iter() {
        if matches!(
            business.status(),
            BusinessStatus::Closed | BusinessStatus::Insolvent
        ) {
            continue;
        }
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("business recipe reference must be valid");
        let effective_batches = super::effective_capacity_batches(
            state,
            business,
            capacity_scratch.office_administrative_load(business.owner_dynasty_id()),
        )
        .min(super::output_limited_batches(
            state,
            business,
            recipe,
            capacity_scratch.business_contract_reserve(business.id(), recipe.output_good_id()),
        ))
        .min(capacity_scratch.worker_limited_batches(business.id()));
        for input in recipe.inputs() {
            let target_batches = i64::from(effective_batches)
                .saturating_mul(i64::from(business.policy.target_input_days));
            let desired = input.quantity().saturating_mul_ratio(target_batches, 1);
            let current = business.inventory_quantity(input.good_id());
            if current >= desired {
                continue;
            }

            let quote = state.market.quotes.get(&input.good_id()).ok_or(
                SimulationError::MarketQuoteMissing {
                    good_id: input.good_id(),
                },
            )?;
            let good_slot = input.good_id().value() as usize;
            let stock = remaining_stock[good_slot];
            let cash = available_cash[business.id().value() as usize];
            let cash_reserve = if business.status() == BusinessStatus::Distressed {
                Money::ZERO
            } else {
                business.policy.minimum_cash_reserve
            };
            let spendable = cash.saturating_sub(cash_reserve);
            let shortfall = desired.saturating_sub(current);
            let quantity = shortfall
                .min(stock)
                .min(affordable_quantity(spendable, quote.price));
            if quantity.is_zero() {
                continue;
            }
            let cost = cost_for(quantity, quote.price);
            remaining_stock[good_slot] = stock
                .checked_sub(quantity)
                .expect("planned business purchase must not exceed market stock");
            available_cash[business.id().value() as usize] = cash
                .checked_sub(cost)
                .expect("affordable business purchase must not exceed available cash");
            lines.push(BusinessPurchaseLine {
                business_id: business.id(),
                good_id: input.good_id(),
                quantity,
                cost,
            });
        }
    }

    Ok(BusinessPurchasePlan { lines })
}

pub(crate) fn apply_business_purchases(
    state: &mut AppState,
    plan: BusinessPurchasePlan,
) -> Result<(), SimulationError> {
    let mut total_cost_copper = 0_i128;
    let mut total_quantity_milliunits = 0_i128;
    for line in plan.lines {
        let BusinessPurchaseLine {
            business_id,
            good_id,
            quantity,
            cost,
        } = line;
        let (resulting_market_stock, resulting_market_demand) = {
            let quote = state
                .market
                .quotes
                .get(&good_id)
                .expect("planned market purchase quote must exist");
            (
                quote
                    .stock
                    .checked_sub(quantity)
                    .expect("planned business purchase must not exceed market stock"),
                quote.demand_today.checked_add(quantity).ok_or(
                    SimulationError::MarketDemandOverflow {
                        good_id,
                        current: quote.demand_today,
                        incoming: quantity,
                    },
                )?,
            )
        };
        let clearing_before = state.market.clearing_account;
        let resulting_clearing = clearing_before.checked_add(cost).ok_or(
            SimulationError::MarketClearingAccountOverflow {
                current: clearing_before,
                change: cost,
            },
        )?;
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("planned business purchase target must exist");
            let current_inventory = business.inventory_quantity(good_id);
            current_inventory.checked_add(quantity).ok_or(
                SimulationError::BusinessInventoryOverflow {
                    business_id,
                    good_id,
                    current: current_inventory,
                    incoming: quantity,
                },
            )?;
            let resulting_cash = business
                .finance
                .cash
                .checked_sub(cost)
                .expect("planned business purchase must fit available cash");
            let resulting_lifetime_costs =
                business.finance.lifetime_costs.checked_add(cost).ok_or(
                    SimulationError::BusinessLifetimeCostsOverflow {
                        business_id,
                        current: business.finance.lifetime_costs,
                        incoming: cost,
                    },
                )?;
            let next_finance_version = next_business_finance_version(business)?;
            business.finance.cash = resulting_cash;
            business.finance.lifetime_costs = resulting_lifetime_costs;
            business.finance.version = next_finance_version;
            business.add_inventory(good_id, quantity);
        }
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("planned market purchase quote must exist");
            quote.stock = resulting_market_stock;
            quote.demand_today = resulting_market_demand;
        }
        state.market.clearing_account = resulting_clearing;
        total_cost_copper += i128::from(cost.copper());
        total_quantity_milliunits += i128::from(quantity.milliunits());
    }
    if total_quantity_milliunits != 0 {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::MarketPurchase,
            subject: "businesses".into(),
            detail: format!("quantity={total_quantity_milliunits}; cost={total_cost_copper}")
                .into(),
        });
    }
    Ok(())
}
