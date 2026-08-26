//! Grounded legal-case filing and negotiated settlement commands.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn quote_player_legal_claim(
    state: &AppState,
    defendant_dynasty_id: DynastyId,
    kind: LegalCaseKind,
) -> Result<crate::systems::LegalClaimQuote, CommandError> {
    if defendant_dynasty_id == state.player_dynasty_id {
        return Err(CommandError::SameLegalParty);
    }
    if !state.dynasties.contains_key(&defendant_dynasty_id) {
        return Err(CommandError::MissingDynasty {
            dynasty_id: defendant_dynasty_id,
        });
    }
    crate::systems::quote_grounded_legal_claim(
        state,
        state.player_dynasty_id,
        defendant_dynasty_id,
        kind,
    )
    .ok_or(CommandError::LegalClaimNotGrounded {
        defendant_dynasty_id,
        kind,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LegalSettlementQuote {
    pub(crate) case_id: LegalCaseId,
    pub(crate) plaintiff_dynasty_id: DynastyId,
    pub(crate) kind: LegalCaseKind,
    pub(crate) amount: Money,
}

pub(crate) fn quote_player_legal_settlement(
    state: &AppState,
    case_id: LegalCaseId,
) -> Result<LegalSettlementQuote, CommandError> {
    let legal_case = state
        .legal_cases
        .get(&case_id)
        .ok_or(CommandError::MissingLegalCase { case_id })?;
    if legal_case.defendant_dynasty_id != state.player_dynasty_id
        || !matches!(
            legal_case.status,
            LegalCaseStatus::Filed | LegalCaseStatus::Hearing
        )
        || legal_case.claim_source.is_none()
    {
        return Err(CommandError::LegalSettlementUnavailable { case_id });
    }
    let exposure = crate::systems::strategic::recoverable_legal_damages(
        state,
        legal_case.claim_source,
        legal_case.damages,
    );
    if exposure <= Money::ZERO {
        return Err(CommandError::LegalSettlementNothingToSettle { case_id });
    }
    let settlement_basis_points = 5_000_i64
        .saturating_add(i64::from(legal_case.evidence_basis_points) / 2)
        .clamp(5_000, 10_000);
    let amount = exposure.saturating_mul_ratio_ceil_nonnegative(settlement_basis_points, 10_000);
    Ok(LegalSettlementQuote {
        case_id,
        plaintiff_dynasty_id: legal_case.plaintiff_dynasty_id,
        kind: legal_case.kind,
        amount,
    })
}

pub(crate) fn apply_legal_case(
    registry: &Registry,
    state: &mut AppState,
    defendant_dynasty_id: DynastyId,
    kind: LegalCaseKind,
    evidence_basis_points: u16,
    damages: Money,
) -> Result<CommandOutcome, CommandError> {
    if evidence_basis_points > 10_000 || damages <= Money::ZERO {
        // Zero damages would file an unresolvable case: nothing to settle, no
        // judgment award, and a claim source occupied for its whole life.
        return Err(CommandError::InvalidLegalTerms);
    }
    // Party validity and existence are the quote's own preconditions, so they
    // are checked once inside `quote_player_legal_claim`.
    if state.legal_cases.values().any(|legal_case| {
        legal_case.plaintiff_dynasty_id == state.player_dynasty_id
            && legal_case.defendant_dynasty_id == defendant_dynasty_id
            && legal_case.kind == kind
            && matches!(
                legal_case.status,
                LegalCaseStatus::Filed | LegalCaseStatus::Hearing
            )
    }) {
        return Err(CommandError::DuplicateActiveLegalCase {
            defendant_dynasty_id,
            kind,
        });
    }
    if let Some(last_filing_day) = state
        .legal_cases
        .values()
        .filter(|legal_case| legal_case.plaintiff_dynasty_id == state.player_dynasty_id)
        .map(|legal_case| legal_case.filed_day)
        .max()
    {
        let next_filing_day = checked_future_day(last_filing_day, LEGAL_CASE_FILING_INTERVAL_DAYS)?;
        if state.clock.day() < next_filing_day {
            return Err(CommandError::LegalCaseCooldown { next_filing_day });
        }
    }
    let claim = quote_player_legal_claim(state, defendant_dynasty_id, kind)?;
    if evidence_basis_points > claim.evidence_basis_points {
        return Err(CommandError::LegalEvidenceExceedsClaim {
            evidence_basis_points,
            maximum_basis_points: claim.evidence_basis_points,
        });
    }
    if damages > claim.maximum_damages {
        return Err(CommandError::LegalDamagesExceedClaim {
            damages,
            maximum_damages: claim.maximum_damages,
        });
    }
    let hearing_day = checked_future_day(state.clock.day(), LEGAL_CASE_HEARING_DELAY_DAYS)?;
    // Resolve every fallible step before the first mutation so a rejected
    // filing never leaves a debited plaintiff or a credited court behind.
    let id = state.next_ids.try_legal_case()?;
    crate::systems::court_filing_fee_headroom(registry, state)?;
    spend_player_treasury(state, LEGAL_CASE_FILING_COST)?;
    crate::systems::collect_court_filing_fee(registry, state);
    state.legal_cases.insert(
        id,
        LegalCase {
            id,
            plaintiff_dynasty_id: state.player_dynasty_id,
            defendant_dynasty_id,
            kind,
            claim_source: Some(claim.claim_source),
            evidence_basis_points,
            public_attention_basis_points: 1_500,
            filed_day: state.clock.day(),
            hearing_day,
            damages,
            status: LegalCaseStatus::Filed,
        },
    );
    crate::systems::strategic::adjust_dynasty_relationship(
        state,
        state.player_dynasty_id,
        defendant_dynasty_id,
        crate::systems::strategic::RelationshipDelta::new(-100, -30, 0, 150, 0),
    );
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Legal,
        format!("Legal case {id} filed"),
        format!(
            "A {kind:?} case was filed against dynasty {defendant_dynasty_id}: {}.",
            claim.description
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Filed legal case {id}."),
    })
}

pub(crate) fn apply_legal_settlement(
    state: &mut AppState,
    case_id: LegalCaseId,
) -> Result<CommandOutcome, CommandError> {
    let quote = quote_player_legal_settlement(state, case_id)?;
    let player_id = state.player_dynasty_id;
    // Resolve both sides' resulting balances before committing anything so a
    // rejected settlement never leaves a debited payer behind.
    let plaintiff_treasury = state
        .dynasties
        .get(&quote.plaintiff_dynasty_id)
        .expect("legal plaintiff dynasty must exist")
        .treasury();
    let plaintiff_after = plaintiff_treasury.checked_add(quote.amount).ok_or(
        CommandError::LegalSettlementTreasuryOverflow {
            plaintiff_dynasty_id: quote.plaintiff_dynasty_id,
            current: plaintiff_treasury,
            incoming: quote.amount,
        },
    )?;
    let player_treasury = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist")
        .treasury();
    if player_treasury < quote.amount {
        return Err(CommandError::InsufficientPlayerFunds {
            available: player_treasury,
            required: quote.amount,
        });
    }
    spend_player_treasury(state, quote.amount)?;
    let claim_source = state
        .legal_cases
        .get(&case_id)
        .expect("quoted legal case must exist")
        .claim_source;

    state
        .dynasties
        .get_mut(&quote.plaintiff_dynasty_id)
        .expect("legal plaintiff dynasty must exist")
        .resources
        .treasury = plaintiff_after;
    // A negotiated settlement closes the grounded obligation in full only
    // when the payment actually covers it; filing small damages and settling
    // them cannot erase a larger underlying debt.
    let settles_in_full = quote.amount
        >= crate::systems::strategic::outstanding_legal_claim_obligation(state, claim_source);
    crate::systems::strategic::settle_legal_claim_source(
        state,
        claim_source,
        quote.plaintiff_dynasty_id,
        player_id,
        quote.amount,
        settles_in_full,
        // A negotiated settlement is amicable: any remaining obligation
        // stands on its own terms instead of executing against pledged
        // collateral in the same breath as a relationship repair.
        false,
    );
    state
        .legal_cases
        .get_mut(&case_id)
        .expect("quoted legal case must exist")
        .status = LegalCaseStatus::Settled;
    crate::systems::strategic::adjust_dynasty_relationship(
        state,
        quote.plaintiff_dynasty_id,
        player_id,
        crate::systems::strategic::RelationshipDelta::new(80, 40, -20, -120, 0),
    );
    crate::systems::strategic::remember_dynasty_interaction(
        state,
        quote.plaintiff_dynasty_id,
        player_id,
        &format!(
            "Legal case {case_id} was settled by negotiated payment of {}.",
            quote.amount
        ),
    );
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Legal,
        format!("Legal case {case_id} settled"),
        if settles_in_full {
            format!(
                "The dynasty paid {} to settle the {:?} claim before judgment; the grounded obligation is closed.",
                quote.amount, quote.kind
            )
        } else {
            format!(
                "The dynasty paid {} toward the {:?} claim before judgment; the remaining grounded obligation stands.",
                quote.amount, quote.kind
            )
        },
    )?;
    Ok(CommandOutcome {
        summary: format!("Settled legal case {case_id} for {}.", quote.amount),
    })
}
