//! Household living costs and monthly family pressure.
//!
//! Purpose: own the monthly living-cost levy that makes cash-rich houses
//! still face district-scaled pressure and erodes satisfaction on shortfall.
//! Owns: `apply_household_living_costs` (rent-index-scaled charge, payment
//! to clearing pool, satisfaction erosion on underpayment).
//! Reads: `AppState` households + districts (rent index) and `MarketState`
//! clearing account.
//! Mutates: `AppState` household cash/satisfaction and market clearing.
//! Does not own: daily market consumption — `simulation/mod.rs`.
//! Invariants: charge scales with members × class × rent index; market
//! staples paid separately so no double bread inflation; surpluses credit
//! the clearing pool per AUDIT.md.
//! Focused tests: `src/systems/strategic/strategic_tests.rs` living-cost
//! and satisfaction behavior.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Monthly cost of living for every household: food beyond market staples,
/// clothing upkeep, rent-equivalent services, and unmodeled consumables.
/// The charge scales with household size, social class, and district
/// desirability so households in a prosperous district or at higher class
/// pay more to live there, and the copper flows into the market clearing
/// pool like every other payment for unmodeled services. Households that
/// cannot cover the cost spend down to zero and see food satisfaction erode,
/// so a household budget that looks cash-rich on its external-income alone
/// still faces realistic pressure. Market staples are paid separately through
/// daily household consumption; living costs therefore track district rents,
/// not bread-price doubling, so a staple spike does not charge households
/// twice for the same bread.
pub(crate) fn apply_household_living_costs(
    _registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let mut total_living_cost = Money::ZERO;
    let household_ids: Vec<_> = state.households.records().keys().copied().collect();
    for household_id in household_ids {
        let (members, district_id, social_class, cash_before) = {
            let household = state
                .households
                .get(household_id)
                .expect("household must exist");
            (
                household.members(),
                household.district_id(),
                household.social_class(),
                household.cash(),
            )
        };
        let rent_index = state
            .districts
            .get(&district_id)
            .expect("household district must exist")
            .rent_index_basis_points;
        let per_member_copper = match social_class {
            crate::core::SocialClass::Laboring => 28,
            crate::core::SocialClass::Artisan => 52,
            crate::core::SocialClass::Merchant => 78,
        };
        let base_copper = i64::from(members).saturating_mul(per_member_copper);
        let scaled_copper = base_copper.saturating_mul(i64::from(rent_index)) / 10_000;
        if scaled_copper <= 0 {
            continue;
        }
        let upkeep = Money::from_copper(scaled_copper);
        let paid = upkeep.min(cash_before);
        if paid <= Money::ZERO {
            continue;
        }
        let household = state
            .households
            .get_mut(household_id)
            .expect("household must exist");
        household.cash = household
            .cash
            .checked_sub(paid)
            .expect("bounded living cost must not exceed household cash");
        // Under-pressure households lose food satisfaction proportionally:
        // a household that could cover only half its living cost loses
        // 250 bp that month, so cash-poor districts show unrest pressure.
        if paid < upkeep {
            let shortfall_ratio =
                u32::try_from((upkeep.copper() - paid.copper()).max(0)).unwrap_or(u32::MAX);
            let upkeep_copper = u32::try_from(upkeep.copper().max(1)).unwrap_or(u32::MAX);
            let satisfaction_loss = (shortfall_ratio * 500 / upkeep_copper).min(500) as u16;
            household.food_satisfaction_basis_points = household
                .food_satisfaction_basis_points
                .saturating_sub(satisfaction_loss);
        }
        total_living_cost = total_living_cost.checked_add(paid).ok_or(
            SimulationError::HouseholdLivingCostOverflow {
                accumulated: total_living_cost,
                incoming: paid,
            },
        )?;
    }
    if total_living_cost > Money::ZERO {
        credit_market_clearing_account(state, total_living_cost)?;
    }
    Ok(())
}
