//! Property purchase and liquidation commands.
//!
//! Purpose: own the validated player path for `BuyProperty` and
//! `SellProperty`, including unowned-property pool credit and distressed
//! lien settlement.
//! Owns: `apply_property_purchase` / `apply_property_sale` / `apply_property_sale` quotes,
//! vacancy-income routing, and lien payoff via collateral.
//! Reads: `AppState` properties / businesses / dynasties / districts.
//! Mutates: property `owner_dynasty_id` / `tenant_dynasty_id`, dynasty
//! treasury, market clearing pool for unowned proceeds and rents.
//! Does not own: rent scaling or eviction weekly systems (strategic/property).
//! Invariants: every unowned sale funds the clearing pool; distressed
//! civic guarantees overlay; weekly rent is district-indexed and
//! fire-discounted when material.
//! Focused tests: `src/systems/commands/commands_tests.rs` property paths.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn apply_property_purchase(
    state: &mut AppState,
    property_id: PropertyId,
) -> Result<CommandOutcome, CommandError> {
    buy_unowned_property(state, state.player_dynasty_id, property_id)?;
    Ok(CommandOutcome {
        summary: format!("Acquired property {property_id}."),
    })
}

pub(crate) fn apply_property_sale(
    registry: &Registry,
    state: &mut AppState,
    property_id: PropertyId,
    buyer_dynasty_id: DynastyId,
) -> Result<CommandOutcome, CommandError> {
    let buyer_treasury = state
        .dynasties
        .get(&buyer_dynasty_id)
        .ok_or(CommandError::MissingDynasty {
            dynasty_id: buyer_dynasty_id,
        })?
        .treasury();
    // Resolve the complete sale result before committing anything: the quote is
    // pure, so the counterparty reserve is enforced pre-commit and a rejected
    // sale never mutates state.
    let quote = quote_property_liquidation(
        registry,
        state,
        state.player_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )?;
    let buyer_after = buyer_treasury
        .checked_sub(quote.buyer_contribution)
        .expect("quoted property buyer contribution must fit treasury");
    // The counterparty reserve protects against selling to a buyer who
    // cannot genuinely afford the asset. In a civic-guaranteed auction the
    // buyer has committed their entire treasury by construction and the
    // civic treasury funds the shortfall, so the discretionary reserve does
    // not apply there.
    if quote.civic_guarantee == Money::ZERO && buyer_after < PROPERTY_COUNTERPARTY_BUYER_RESERVE {
        return Err(CommandError::PropertyCounterpartyBuyerReserve {
            buyer_dynasty_id,
            available: buyer_treasury,
            buyer_contribution: quote.buyer_contribution,
            required_reserve: PROPERTY_COUNTERPARTY_BUYER_RESERVE,
        });
    }
    sell_owned_property_scratch(
        registry,
        state,
        state.player_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )?;
    Ok(CommandOutcome {
        summary: format!("Sold property {property_id} for {}.", quote.price),
    })
}
