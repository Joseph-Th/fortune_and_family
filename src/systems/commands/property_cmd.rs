//! Property purchase and liquidation commands.

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
