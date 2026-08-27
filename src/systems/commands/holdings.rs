//! Player commands over owned businesses: transfers, capital, policy, wages.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BusinessPolicyInput {
    pub(crate) target_input_days: u16,
    pub(crate) target_output_days: u16,
    pub(crate) minimum_cash_reserve: Money,
    pub(crate) maintenance_basis_points: u16,
    pub(crate) quality_target_basis_points: u16,
}

pub(crate) fn apply_business_acquisition(
    registry: &Registry,
    state: &mut AppState,
    business_id: BusinessId,
    manager_id: CharacterId,
    recapitalization: Money,
) -> Result<CommandOutcome, CommandError> {
    let quote = acquire_business_scratch(
        registry,
        state,
        state.player_dynasty_id,
        business_id,
        manager_id,
        recapitalization,
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Acquired business {business_id} for {} with {} working capital.",
            quote.purchase_price, recapitalization
        ),
    })
}

pub(crate) fn apply_cash_transfer(
    state: &mut AppState,
    from_business_id: BusinessId,
    to_business_id: BusinessId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_operable_owned_business(state, from_business_id)?;
    // Receiving a cash infusion is allowed for any owned business still
    // operating, including one under distress the transfer is meant to
    // relieve; rescuing an insolvent or closed firm is InvestInBusiness's
    // role, so `transfer_business_cash` rejects a terminated destination and
    // only the source must be operable here.
    ensure_owned_business(state, to_business_id)?;
    // Portfolio transfers honor the same operating-reserve floor as every
    // other player-driven route out of the business, so a firm cannot be
    // hollowed out below the reserve its daily decisions rely on.
    let spendable = state
        .businesses
        .get(from_business_id)
        .map(business_operating_spendable_cash)
        .expect("owned business must exist");
    if spendable < amount {
        return Err(CommandError::InsufficientBusinessFunds {
            business_id: from_business_id,
            available: spendable,
            required: amount,
        });
    }
    transfer_business_cash(state, from_business_id, to_business_id, amount)?;
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Portfolio cash moved to business {to_business_id}"),
        format!(
            "The dynasty transferred {amount} from business {from_business_id} to business {to_business_id}."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Transferred {amount} from business {from_business_id} to business {to_business_id}."
        ),
    })
}

pub(crate) fn apply_business_cash_withdrawal(
    registry: &Registry,
    state: &mut AppState,
    business_id: BusinessId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_owned_business(state, business_id)?;
    distribute_owned_business_cash(
        registry,
        state,
        state.player_dynasty_id,
        business_id,
        amount,
    )?;
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} distributed cash to the dynasty"),
        format!(
            "The dynasty withdrew {amount} of surplus cash from business {business_id} while preserving its operating reserve."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Withdrew {amount} from business {business_id}."),
    })
}

pub(crate) fn apply_business_investment(
    state: &mut AppState,
    business_id: BusinessId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_owned_business(state, business_id)?;
    // The canonical capitalization path owns every rule (positive amount,
    // lifecycle, treasury, overflow); the command translates its typed
    // rejections instead of duplicating them under a second taxonomy.
    let rehabilitation = crate::systems::strategic::capitalize_owned_business(
        state,
        state.player_dynasty_id,
        business_id,
        amount,
    )
    .map_err(CommandError::Strategic)?;
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} capitalized"),
        format!(
            "The dynasty invested {amount} into the enterprise, restoring {rehabilitation} basis points of operating condition."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Invested {amount} in business {business_id} and restored {rehabilitation} basis points of condition."
        ),
    })
}

/// A policy reserve may hold at most one full year of the firm's operating
/// cover idle: defensive enough for any downturn, bounded against locking a
/// firm's economy behind an unreachable cash floor.
pub(crate) fn recipe_daily_operating_cover_ceiling(
    registry: &Registry,
    business: &crate::core::Business,
) -> Money {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe reference must be valid");
    recipe
        .daily_operating_cost()
        .saturating_mul(BUSINESS_RESERVE_MAX_OPERATING_DAYS)
}

pub(crate) fn apply_business_policy(
    registry: &Registry,
    state: &mut AppState,
    business_id: BusinessId,
    input: BusinessPolicyInput,
) -> Result<CommandOutcome, CommandError> {
    let BusinessPolicyInput {
        target_input_days,
        target_output_days,
        minimum_cash_reserve,
        maintenance_basis_points,
        quality_target_basis_points,
    } = input;
    ensure_operable_owned_business(state, business_id)?;
    if target_input_days > 30
        || target_output_days > 30
        || minimum_cash_reserve.is_negative()
        || maintenance_basis_points > 10_000
        || quality_target_basis_points > 10_000
    {
        return Err(CommandError::InvalidBusinessPolicy);
    }
    let business = state
        .businesses
        .get(business_id)
        .expect("validated business must exist");
    // The reserve gates every spendable-copper route for the business, so an
    // unbounded floor would let one policy change permanently lock the firm's
    // payroll, purchasing, and rescue financing. A full year of operating
    // cover held idle is already an extreme defensive posture; beyond that
    // the request serves no operating purpose. The bound is deliberately
    // independent of the firm's current policy, so it never rejects a raise
    // merely because today's reserve happens to be small.
    if minimum_cash_reserve > recipe_daily_operating_cover_ceiling(registry, business) {
        return Err(CommandError::InvalidBusinessPolicy);
    }
    if business.policy.target_input_days == target_input_days
        && business.policy.target_output_days == target_output_days
        && business.policy.minimum_cash_reserve == minimum_cash_reserve
        && business.policy.maintenance_basis_points == maintenance_basis_points
        && business.policy.quality_target_basis_points == quality_target_basis_points
    {
        return Err(CommandError::UnchangedBusinessPolicy { business_id });
    }
    let subject = format!("business:{business_id}");
    if let Some(last_change_day) = latest_cooldown_audit_day(
        state,
        AuditKind::BusinessPolicyChange,
        BUSINESS_POLICY_CHANGE_INTERVAL_DAYS,
        |record_subject| record_subject == subject,
    ) {
        let next_change_day =
            checked_future_day(last_change_day, BUSINESS_POLICY_CHANGE_INTERVAL_DAYS)?;
        if state.clock.day() < next_change_day {
            return Err(CommandError::BusinessPolicyCooldown {
                business_id,
                next_change_day,
            });
        }
    }
    let next_finance_version = next_business_finance_version(business)?;
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.policy.target_input_days = target_input_days;
    business.policy.target_output_days = target_output_days;
    business.policy.minimum_cash_reserve = minimum_cash_reserve;
    business.policy.maintenance_basis_points = maintenance_basis_points;
    business.policy.quality_target_basis_points = quality_target_basis_points;
    business.finance.version = next_finance_version;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessPolicyChange,
        subject: subject.into(),
        detail: format!(
            "input_days={target_input_days}; output_days={target_output_days}; reserve={}; maintenance={maintenance_basis_points}; quality={quality_target_basis_points}",
            minimum_cash_reserve.copper()
        ).into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} operating policy updated"),
        format!(
            "The enterprise now targets {target_input_days} input days, {target_output_days} output days, a {minimum_cash_reserve} cash reserve, {maintenance_basis_points} maintenance basis points, and {quality_target_basis_points} quality basis points."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Updated operating policy for business {business_id}."),
    })
}

/// Sets the standing weekly wage for every workforce agreement of one owned
/// business. Wage posture is the player's proactive labor lever: wages below
/// the market reference erode workforce loyalty and eventually provoke
/// disputes, while generous wages build loyalty that absorbs operating strain.
pub(crate) fn apply_business_wages(
    state: &mut AppState,
    business_id: BusinessId,
    weekly_wage_per_worker: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_operable_owned_business(state, business_id)?;
    if weekly_wage_per_worker <= Money::ZERO || weekly_wage_per_worker > MAX_WEEKLY_WAGE_PER_WORKER
    {
        return Err(CommandError::InvalidBusinessWage {
            maximum: MAX_WEEKLY_WAGE_PER_WORKER,
        });
    }
    let subject = format!("business:{business_id}");
    if let Some(last_change_day) = latest_cooldown_audit_day(
        state,
        AuditKind::BusinessWageChange,
        BUSINESS_WAGE_CHANGE_INTERVAL_DAYS,
        |record_subject| record_subject == subject,
    ) {
        let next_change_day =
            checked_future_day(last_change_day, BUSINESS_WAGE_CHANGE_INTERVAL_DAYS)?;
        if state.clock.day() < next_change_day {
            return Err(CommandError::BusinessWageCooldown {
                business_id,
                next_change_day,
            });
        }
    }
    let agreements: Vec<EmploymentId> = state
        .employment
        .values()
        .filter(|agreement| {
            agreement.business_id() == business_id && agreement.status != EmploymentStatus::Ended
        })
        .map(EmploymentAgreement::id)
        .collect();
    if agreements.is_empty() {
        return Err(CommandError::BusinessHasNoWorkforce { business_id });
    }
    // Every agreement must already pay the requested per-worker wage for this
    // to be a genuine no-op: individual agreements drift apart when the
    // market wage system renegotiates them, so keying off one arbitrary
    // agreement would reject changes the workforce would still feel. The
    // comparison multiplies the requested per-worker wage back up to the
    // agreement total, so a renegotiated non-divisible payroll never reads as
    // an exact match and gets silently re-rounded by a "no-op" rewrite.
    let all_match_requested = agreements.iter().all(|agreement_id| {
        state.employment.get(agreement_id).is_some_and(|agreement| {
            weekly_wage_per_worker
                .checked_mul_ratio(i64::from(agreement.workers().max(1)), 1)
                .is_some_and(|total| total == agreement.weekly_wage())
        })
    });
    if all_match_requested {
        return Err(CommandError::UnchangedBusinessWage { business_id });
    }
    for agreement_id in &agreements {
        let agreement = state
            .employment
            .get_mut(agreement_id)
            .expect("collected employment agreement must exist");
        // The wage is already validated against MAX_WEEKLY_WAGE_PER_WORKER and
        // workers fit u16, so the product cannot overflow the fixed-point range.
        let total = weekly_wage_per_worker
            .checked_mul_ratio(i64::from(agreement.workers().max(1)), 1)
            .expect("validated wage times u16 workers cannot overflow");
        agreement.weekly_wage = total;
    }
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessWageChange,
        subject: subject.into(),
        detail: format!(
            "wage_per_worker={}; agreements={}",
            weekly_wage_per_worker.copper(),
            agreements.len()
        )
        .into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::District,
        format!("Business {business_id} wage terms updated"),
        format!(
            "The enterprise now pays {weekly_wage_per_worker} per worker each week across {} workforce agreement(s).",
            agreements.len()
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Set the weekly wage of business {business_id} to {weekly_wage_per_worker} per worker."
        ),
    })
}

/// Cash a player-driven business action may spend: everything above the
/// operating-reserve floor enforced by the business's own daily purchase and
/// production decisions.
#[must_use]
pub(crate) fn business_operating_spendable_cash(business: &crate::core::Business) -> Money {
    business
        .cash()
        .saturating_sub(business.policy.minimum_cash_reserve)
        .max(Money::ZERO)
}
