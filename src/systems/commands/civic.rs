//! Public-work sponsorship and direct funding commands.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn apply_public_work(
    registry: &Registry,
    state: &mut AppState,
    district_id: DistrictId,
    kind: PublicWorkKind,
    budget: Money,
) -> Result<CommandOutcome, CommandError> {
    if registry.get_district(district_id).is_none() {
        return Err(CommandError::MissingDistrict { district_id });
    }
    if budget < PUBLIC_WORK_MINIMUM_BUDGET {
        return Err(CommandError::InvalidPublicWorkBudget {
            minimum: PUBLIC_WORK_MINIMUM_BUDGET,
        });
    }
    if state.public_works.values().any(|work| {
        work.district_id == district_id && work.kind == kind && work.status.is_unfinished()
    }) {
        return Err(CommandError::DuplicateActivePublicWork { district_id, kind });
    }
    let active_sponsored = state
        .public_works
        .values()
        .filter(|work| {
            work.sponsor_dynasty_id == Some(state.player_dynasty_id) && work.status.is_unfinished()
        })
        .count();
    if active_sponsored >= MAX_ACTIVE_SPONSORED_PUBLIC_WORKS {
        return Err(CommandError::PublicWorkCapacity {
            active: active_sponsored,
            maximum: MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
        });
    }
    let subject = format!("dynasty:{}", state.player_dynasty_id);
    validate_public_work_cooldown(state, &subject)?;
    if !has_player_office(state) {
        return Err(CommandError::PublicWorkSponsorshipRequiresOffice);
    }
    if !has_player_office_power(state, OfficePower::PublicWorks) {
        return Err(CommandError::PublicWorkSponsorshipRequiresPower);
    }
    let available_day = checked_player_office_power_available_day(state, OfficePower::PublicWorks)?
        .expect("validated public-works office must have an availability day");
    if state.clock.day() < available_day {
        return Err(CommandError::PublicWorkPowerNotEstablished { available_day });
    }
    let contribution = public_work_initial_contribution(budget);
    // Resolve every fallible step before the first mutation so a rejected
    // sponsorship never leaves a debited treasury behind.
    let id = state.next_ids.try_public_work()?;
    spend_player_treasury(state, contribution)?;
    // The sponsor pays construction labor and materials directly, so the
    // contribution keeps a credited counterparty in the market clearing pool
    // instead of vanishing from the economy.
    credit_market_clearing_account(state, contribution)?;
    let progress_basis_points =
        crate::systems::public_work_progress_basis_points(contribution, budget);
    state.public_works.insert(
        id,
        PublicWork {
            id,
            district_id,
            kind,
            sponsor_dynasty_id: Some(state.player_dynasty_id),
            budget,
            spent: contribution,
            progress_basis_points,
            status: PublicWorkStatus::Building,
        },
    );
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::PublicWorkStarted,
        subject: subject.into(),
        detail: format!(
            "district={};kind={kind:?};budget={};contribution={}",
            district_id.value(),
            budget.copper(),
            contribution.copper()
        )
        .into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Public work {id} started"),
        format!("Construction began on a {kind:?} project in district {district_id}."),
    )?;
    Ok(CommandOutcome {
        summary: format!("Started public work {id}."),
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PublicWorkFundingQuote {
    player_id: DynastyId,
    district_id: DistrictId,
    kind: PublicWorkKind,
    external_sponsor_dynasty_id: Option<DynastyId>,
    treasury_after: Money,
    contributions_after: Money,
    spent_after: Money,
    progress_basis_points: u16,
    legitimacy_gain: u16,
    completed: bool,
}

pub(crate) fn apply_public_work_funding(
    state: &mut AppState,
    public_work_id: PublicWorkId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    let quote = quote_public_work_funding(state, public_work_id, amount)?;
    let player = state
        .dynasties
        .get_mut(&quote.player_id)
        .expect("validated player dynasty must exist");
    player.resources.treasury = quote.treasury_after;
    player.resources.civic_contributions = quote.contributions_after;
    if quote.legitimacy_gain > 0 {
        player.resources.legitimacy_basis_points = player
            .resources
            .legitimacy_basis_points
            .saturating_add(quote.legitimacy_gain)
            .min(10_000);
    }
    // Direct funding pays construction costs immediately, so the amount keeps
    // a credited counterparty in the market clearing pool.
    credit_market_clearing_account(state, amount)?;
    let work = state
        .public_works
        .get_mut(&public_work_id)
        .expect("validated public work must exist");
    work.spent = quote.spent_after;
    work.progress_basis_points = quote.progress_basis_points;
    if quote.completed {
        work.status = PublicWorkStatus::Completed;
        crate::systems::strategic::apply_public_work_completion(
            state,
            quote.district_id,
            quote.kind,
        );
    }
    // A rival house whose project the dynasty bankrolls remembers the favor:
    // trust and standing rise and the sponsor carries a durable obligation.
    if let Some(sponsor_dynasty_id) = quote.external_sponsor_dynasty_id {
        crate::systems::strategic::adjust_dynasty_relationship(
            state,
            quote.player_id,
            sponsor_dynasty_id,
            crate::systems::strategic::RelationshipDelta::new(60, 80, 0, 0, 40),
        );
        crate::systems::strategic::remember_dynasty_interaction(
            state,
            quote.player_id,
            sponsor_dynasty_id,
            &format!(
                "the player dynasty contributed {amount} to the sponsor's {:?} project in district {}",
                quote.kind, quote.district_id
            ),
        );
    }
    let (title, detail) = if quote.external_sponsor_dynasty_id.is_some() {
        (
            format!("Public work {public_work_id} received dynasty funding"),
            format!(
                "The dynasty contributed {amount} to another house's {:?} project in district {}; its progress is now {} basis points and the city has taken notice.",
                quote.kind, quote.district_id, quote.progress_basis_points
            ),
        )
    } else if quote.completed {
        (
            format!("Public work {public_work_id} completed with dynasty funding"),
            format!(
                "The dynasty contributed {amount} directly to finish the {:?} project in district {}.",
                quote.kind, quote.district_id
            ),
        )
    } else {
        (
            format!("Public work {public_work_id} received dynasty funding"),
            format!(
                "The dynasty contributed {amount} directly to public work {public_work_id}; project progress is now {} basis points.",
                quote.progress_basis_points
            ),
        )
    };
    crate::systems::strategic::try_push_outbox(state, OutboxKind::Politics, title, detail)?;
    Ok(CommandOutcome {
        summary: if quote.completed {
            format!("Funded and completed public work {public_work_id} with {amount}.")
        } else {
            format!("Funded public work {public_work_id} with {amount}.")
        },
    })
}

pub(crate) fn quote_public_work_funding(
    state: &AppState,
    public_work_id: PublicWorkId,
    amount: Money,
) -> Result<PublicWorkFundingQuote, CommandError> {
    if amount <= Money::ZERO {
        return Err(PublicWorkFundingError::InvalidAmount.into());
    }
    let (sponsor_dynasty_id, status, budget, spent, district_id, kind) = {
        let work = state
            .public_works
            .get(&public_work_id)
            .ok_or(PublicWorkFundingError::Missing { public_work_id })?;
        (
            work.sponsor_dynasty_id,
            work.status,
            work.budget,
            work.spent,
            work.district_id,
            work.kind,
        )
    };
    if status == PublicWorkStatus::Completed {
        return Err(PublicWorkFundingError::AlreadyComplete { public_work_id }.into());
    }
    let remaining = budget
        .checked_sub(spent)
        .expect("validated public work spending must not exceed its budget");
    if amount > remaining {
        return Err(PublicWorkFundingError::ExceedsRemaining {
            public_work_id,
            remaining,
            requested: amount,
        }
        .into());
    }
    let player_id = state.player_dynasty_id;
    let player = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist");
    if player.treasury() < amount {
        return Err(CommandError::InsufficientPlayerFunds {
            available: player.treasury(),
            required: amount,
        });
    }
    let treasury_after = player
        .treasury()
        .checked_sub(amount)
        .expect("validated public-work funding must fit player treasury");
    let contributions_after = player.civic_contributions().checked_add(amount).ok_or(
        crate::systems::SimulationError::DynastyCivicContributionsOverflow {
            dynasty_id: player_id,
            current: player.civic_contributions(),
            incoming: amount,
        },
    )?;
    let spent_after = spent
        .checked_add(amount)
        .expect("bounded public-work funding must fit project budget");
    let progress_basis_points =
        crate::systems::public_work_progress_basis_points(spent_after, budget);
    // Contributing to a project the dynasty did not sponsor is public spirit
    // the city can see: it earns a bounded legitimacy gain in the same
    // proportion an institution endowment pays, scaled down because the
    // district benefit itself is part of the return.
    let external_sponsor_dynasty_id = sponsor_dynasty_id.filter(|sponsor| *sponsor != player_id);
    let legitimacy_gain = if external_sponsor_dynasty_id.is_some() {
        u16::try_from((amount.copper() / 400).clamp(10, 120))
            .expect("bounded civic-funding legitimacy gain must fit u16")
    } else {
        0
    };
    Ok(PublicWorkFundingQuote {
        player_id,
        district_id,
        kind,
        external_sponsor_dynasty_id,
        treasury_after,
        contributions_after,
        spent_after,
        progress_basis_points,
        legitimacy_gain,
        completed: spent_after == budget,
    })
}

pub(crate) fn validate_public_work_cooldown(
    state: &AppState,
    subject: &str,
) -> Result<(), CommandError> {
    if let Some(last_start_day) = latest_cooldown_audit_day(
        state,
        AuditKind::PublicWorkStarted,
        PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS,
        |record_subject| record_subject == subject,
    ) {
        let next_start_day =
            checked_future_day(last_start_day, PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS)?;
        if state.clock.day() < next_start_day {
            return Err(CommandError::PublicWorkCooldown { next_start_day });
        }
    }
    Ok(())
}

/// Initial sponsor contribution demanded for a public work of the given budget.
#[must_use]
pub(crate) fn public_work_initial_contribution(budget: Money) -> Money {
    Money::from_copper((budget.copper() / 10).max(1)).min(budget)
}
