//! Crisis-response and labor-dispute resolution commands.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn apply_crisis_response(
    registry: &Registry,
    state: &mut AppState,
    crisis_id: CrisisId,
    response: CrisisResponse,
) -> Result<CommandOutcome, CommandError> {
    let crisis = state
        .crises
        .get(&crisis_id)
        .ok_or(CommandError::MissingCrisis { crisis_id })?;
    if !crisis.status.is_active() {
        return Err(CommandError::InactiveCrisis { crisis_id });
    }
    let subject = validate_crisis_response_history(state, crisis_id, response)?;
    let severity = crisis.severity_basis_points;
    let district_id = crisis.district_id;
    let crisis_kind = crisis.kind;
    // Organized responses to a trade disruption send aid down the routes
    // themselves, so they heal route disruption by the same amount as crisis
    // severity; otherwise the tracked cause would outlive every response.
    // Profiteering heals nothing.
    let organized_response_severity_reduction = match response {
        CrisisResponse::Relief => 2_500,
        CrisisResponse::Reform => 1_800,
        CrisisResponse::Suppress => 2_000,
        CrisisResponse::Exploit => 0,
    };
    // Suppression spends standing like every other priced cost: a dynasty
    // without the legitimacy to pay rejects up front instead of silently
    // paying what little it has.
    if response == CrisisResponse::Suppress {
        let available_legitimacy = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points;
        if available_legitimacy < CRISIS_SUPPRESS_LEGITIMACY_COST {
            return Err(CommandError::InsufficientPlayerLegitimacy {
                available: available_legitimacy,
                required: CRISIS_SUPPRESS_LEGITIMACY_COST,
            });
        }
    }
    // Organized responses to a trade disruption heal the tracked routes first,
    // so the severity reduction is clamped against the post-healing route
    // condition and cannot declare victory over disruption that remains.
    if organized_response_severity_reduction > 0 && crisis_kind == CrisisKind::TradeDisruption {
        heal_disrupted_routes(state, organized_response_severity_reduction);
    }
    match response {
        CrisisResponse::Relief => {
            spend_player_treasury_to_market(state, crisis_relief_cost(severity))?;
            reduce_crisis(state, crisis_id, organized_response_severity_reduction);
            adjust_player_legitimacy(state, CRISIS_RELIEF_LEGITIMACY_GAIN, true);
            adjust_district_unrest(state, district_id, CRISIS_RELIEF_UNREST_REDUCTION, false);
        }
        CrisisResponse::Reform => {
            spend_player_treasury_to_market(state, CRISIS_REFORM_COST)?;
            reduce_crisis(state, crisis_id, organized_response_severity_reduction);
            adjust_player_legitimacy(state, CRISIS_REFORM_LEGITIMACY_GAIN, true);
            adjust_district_unrest(state, district_id, CRISIS_REFORM_UNREST_REDUCTION, false);
        }
        CrisisResponse::Suppress => {
            spend_player_treasury_to_market(state, CRISIS_SUPPRESS_COST)?;
            reduce_crisis(state, crisis_id, organized_response_severity_reduction);
            adjust_player_legitimacy(state, CRISIS_SUPPRESS_LEGITIMACY_COST, false);
            adjust_district_unrest(state, district_id, CRISIS_SUPPRESS_UNREST_INCREASE, true);
        }
        CrisisResponse::Exploit => {
            apply_crisis_exploitation(state, crisis_id, severity, district_id)?;
        }
    }
    // A guild revolt is a crisis of guild standing: the answer changes how
    // every chartered guild is seen, and that standing feeds back into future
    // revolt pressure.
    let mut guild_standing_note = String::new();
    if crisis_kind == CrisisKind::GuildRevolt {
        let standing_delta = crate::systems::strategic::apply_guild_revolt_standing_response(
            registry, state, response,
        );
        guild_standing_note = if standing_delta >= 0 {
            format!(
                " The chartered guilds' public standing improves by {standing_delta} basis points."
            )
        } else {
            format!(
                " The chartered guilds' public standing falls by {} basis points.",
                -standing_delta
            )
        };
    }
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Crisis,
        format!("Response applied to crisis {crisis_id}"),
        format!("The dynasty chose {response:?}.{guild_standing_note}"),
    )?;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::CrisisResponse,
        subject: subject.into(),
        detail: crate::systems::strategic::crisis_response_audit_detail(response).into(),
    });
    Ok(CommandOutcome {
        summary: format!("Applied {response:?} response to crisis {crisis_id}."),
    })
}

/// Crisis profiteering: extracts wealth from the panicked market into the
/// player treasury. The extraction matches what relief would cost so
/// profiteering is a genuine liquidity-of-last-resort trade: real money now,
/// in exchange for a worsened crisis, legitimacy, and unrest. The whole
/// result resolves before any mutation: an empty or overdrawn clearing pool
/// offers nothing to take, so spending legitimacy on the attempt must reject
/// up front instead of paying full price for zero gain.
pub(crate) fn apply_crisis_exploitation(
    state: &mut AppState,
    crisis_id: CrisisId,
    severity: u16,
    district_id: Option<DistrictId>,
) -> Result<(), CommandError> {
    let available_legitimacy = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points;
    if available_legitimacy < CRISIS_EXPLOIT_LEGITIMACY_COST {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: available_legitimacy,
            required: CRISIS_EXPLOIT_LEGITIMACY_COST,
        });
    }
    let desired_gain = crisis_relief_cost(severity);
    let gain = desired_gain.min(state.market.clearing_account.max(Money::ZERO));
    if gain <= Money::ZERO {
        return Err(CommandError::MarketExtractionUnavailable {
            available: state.market.clearing_account,
        });
    }
    let current_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let resulting_treasury = current_treasury
        .checked_add(gain)
        .ok_or(CommandError::Strategic(
            StrategicError::DynastyTreasuryOverflow {
                dynasty_id: state.player_dynasty_id,
                current: current_treasury,
                incoming: gain,
            },
        ))?;
    debit_market_clearing_account(state, gain)?;
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.treasury = resulting_treasury;
    let crisis = state
        .crises
        .get_mut(&crisis_id)
        .expect("validated crisis must exist");
    crisis.severity_basis_points = severity
        .saturating_add(CRISIS_EXPLOIT_SEVERITY_INCREASE)
        .min(10_000);
    crisis.status = CrisisStatus::from_severity(crisis.severity_basis_points);
    adjust_player_legitimacy(state, CRISIS_EXPLOIT_LEGITIMACY_COST, false);
    adjust_district_unrest(state, district_id, CRISIS_EXPLOIT_UNREST_INCREASE, true);
    Ok(())
}

pub(crate) fn validate_crisis_response_history(
    state: &AppState,
    crisis_id: CrisisId,
    response: CrisisResponse,
) -> Result<String, CommandError> {
    let crisis_kind = state
        .crises
        .get(&crisis_id)
        .map(|crisis| crisis.kind)
        .ok_or(CommandError::MissingCrisis { crisis_id })?;
    let subject = format!("crisis:{crisis_id}");
    // A trade disruption is contained by healing its routes, and each response
    // heals a bounded amount of the tracked disruption. Repeated organized
    // responses are therefore a real strategy with real costs, not spam.
    //
    // Other crises accept one organized response per containment window: the
    // escalation engine counts a response as an ongoing effort for that same
    // bounded window, so once the window closes on a crisis that persists or
    // has re-escalated, another organized response must be legitimate instead
    // of locking the house out forever while the threat worsens.
    let window_cutoff = state
        .clock
        .day()
        .saturating_sub(crate::systems::strategic::CRISIS_RESPONSE_WINDOW_DAYS);
    // Audit days are chronologically nondecreasing, so reverse iteration stops
    // at the window boundary instead of sweeping the whole audit log.
    let has_recent_containment_response = state
        .audit_log
        .iter()
        .rev()
        .take_while(|record| record.day() >= window_cutoff)
        .filter(|record| record.kind() == AuditKind::CrisisResponse && record.subject() == subject)
        .any(crate::systems::strategic::crisis_response_contains_crisis);
    let has_exploitation_response = state.audit_log.iter().any(|record| {
        record.kind() == AuditKind::CrisisResponse
            && record.subject() == subject
            && crate::systems::strategic::audit_record_crisis_response(record)
                == Some(CrisisResponse::Exploit)
    });
    if (has_recent_containment_response && crisis_kind != CrisisKind::TradeDisruption)
        || (response == CrisisResponse::Exploit && has_exploitation_response)
    {
        return Err(CommandError::CrisisAlreadyAddressed { crisis_id });
    }
    Ok(subject)
}

/// A trade-disruption response sends aid down the disrupted routes. The aid
/// budget equals the response's severity reduction and is spent where
/// disruption runs deepest first, so one response heals a bounded amount of
/// total disruption instead of magically treating every route at full strength.
pub(crate) fn heal_disrupted_routes(state: &mut AppState, mut aid_basis_points: u16) {
    let mut disrupted: Vec<(std::cmp::Reverse<u16>, ExternalRouteId)> = state
        .external_routes
        .values()
        .filter(|route| route.disruption_basis_points > 0)
        .map(|route| (std::cmp::Reverse(route.disruption_basis_points), route.id))
        .collect();
    disrupted.sort_unstable();
    for (_, route_id) in disrupted {
        if aid_basis_points == 0 {
            break;
        }
        let Some(route) = state.external_routes.get_mut(&route_id) else {
            continue;
        };
        let healed = route.disruption_basis_points.min(aid_basis_points);
        route.disruption_basis_points = route.disruption_basis_points.saturating_sub(healed);
        aid_basis_points = aid_basis_points.saturating_sub(healed);
    }
}

pub(crate) fn reduce_crisis(state: &mut AppState, crisis_id: CrisisId, amount: u16) {
    let crisis = state
        .crises
        .get_mut(&crisis_id)
        .expect("validated crisis must exist");
    let severity_before_response = crisis.severity_basis_points;
    let reduced = severity_before_response.saturating_sub(amount);
    // A tracked trade disruption holds at the condition that spawned it: a
    // response cannot mark it resolved while any route remains disrupted at or
    // above the detection threshold, or the next monthly pass would immediately
    // re-detect an identical replacement crisis and orphan this audit trail.
    if crisis.kind == CrisisKind::TradeDisruption {
        let worst_route_disruption = state
            .external_routes
            .values()
            .map(|route| route.disruption_basis_points)
            .max()
            .unwrap_or(0);
        if worst_route_disruption
            >= crate::systems::strategic::TRADE_DISRUPTION_ROUTE_DISRUPTION_THRESHOLD
        {
            // The response re-anchors onto the tracked route condition without
            // ever raising the metric it responds to: routes that deepened past
            // this crisis's severity since detection are left for the monthly
            // pass to reflect, instead of a paid response silently worsening
            // the headline number.
            crisis.severity_basis_points =
                reduced.max(worst_route_disruption.min(severity_before_response));
        } else {
            crisis.severity_basis_points = reduced;
        }
    } else {
        crisis.severity_basis_points = reduced;
    }
    crisis.status = CrisisStatus::from_severity(crisis.severity_basis_points);
}

pub(crate) fn adjust_player_legitimacy(state: &mut AppState, amount: u16, increase: bool) {
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.legitimacy_basis_points = if increase {
        dynasty
            .resources
            .legitimacy_basis_points
            .saturating_add(amount)
            .min(10_000)
    } else {
        dynasty
            .resources
            .legitimacy_basis_points
            .saturating_sub(amount)
    };
}

pub(crate) fn adjust_district_unrest(
    state: &mut AppState,
    district_id: Option<DistrictId>,
    amount: u16,
    increase: bool,
) {
    let Some(district_id) = district_id else {
        return;
    };
    let Some(district) = state.districts.get_mut(&district_id) else {
        return;
    };
    district.unrest_basis_points = if increase {
        district
            .unrest_basis_points
            .saturating_add(amount)
            .min(10_000)
    } else {
        district.unrest_basis_points.saturating_sub(amount)
    };
}

pub(crate) fn validate_negotiated_weekly_wage(
    agreement: &crate::core::EmploymentAgreement,
    employment_id: EmploymentId,
    response: LaborResponse,
) -> Result<Option<Money>, CommandError> {
    match response {
        LaborResponse::Negotiate => {
            // Negotiation raises the wage by a tenth, but never past the same
            // per-worker ceiling the direct wage lever enforces, so repeated
            // dispute cycles cannot ratchet payroll past the supported range.
            let workers = i64::from(agreement.workers().max(1));
            let raise = agreement.weekly_wage.checked_mul_ratio(11, 10).ok_or(
                CommandError::LaborWageOverflow {
                    employment_id,
                    current: agreement.weekly_wage,
                },
            )?;
            let ceiling = MAX_WEEKLY_WAGE_PER_WORKER
                .checked_mul_ratio(workers, 1)
                .ok_or(CommandError::LaborWageOverflow {
                    employment_id,
                    current: agreement.weekly_wage,
                })?;
            // Negotiation is a concession, never a cut: a wage already above
            // the per-worker ceiling (reachable through market wage pressure)
            // stays where it is instead of being lowered onto disputing
            // workers.
            Ok(Some(raise.min(ceiling).max(agreement.weekly_wage)))
        }
        LaborResponse::ImproveConditions | LaborResponse::ReplaceWorkers => Ok(None),
    }
}

pub(crate) fn apply_labor_response(
    state: &mut AppState,
    employment_id: EmploymentId,
    response: LaborResponse,
) -> Result<CommandOutcome, CommandError> {
    let agreement = state
        .employment
        .get(&employment_id)
        .ok_or(CommandError::MissingEmployment { employment_id })?;
    let business_id = agreement.business_id;
    let workers = agreement.workers;
    ensure_operable_owned_business(state, business_id)?;
    if agreement.status != EmploymentStatus::Disputed {
        return Err(CommandError::InvalidLaborDispute { employment_id });
    }
    let negotiated_weekly_wage =
        validate_negotiated_weekly_wage(agreement, employment_id, response)?;
    match response {
        LaborResponse::ImproveConditions => {
            spend_business_cash(state, business_id, LABOR_CONDITIONS_IMPROVEMENT_COST)?;
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("validated employment must exist");
            agreement.conditions_basis_points = agreement
                .conditions_basis_points
                .saturating_add(2_000)
                .clamp(crate::systems::EMPLOYMENT_RECOVERY_BASIS_POINTS, 10_000);
            agreement.loyalty_basis_points = agreement
                .loyalty_basis_points
                .saturating_add(1_000)
                .clamp(crate::systems::EMPLOYMENT_RECOVERY_BASIS_POINTS, 10_000);
            agreement.status = EmploymentStatus::Active;
        }
        LaborResponse::Negotiate => {
            spend_business_cash(state, business_id, LABOR_NEGOTIATION_COST)?;
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("validated employment must exist");
            agreement.weekly_wage =
                negotiated_weekly_wage.expect("negotiated wage must be prevalidated");
            agreement.loyalty_basis_points = agreement.loyalty_basis_points.max(4_500);
            agreement.conditions_basis_points = agreement
                .conditions_basis_points
                .max(crate::systems::EMPLOYMENT_RECOVERY_BASIS_POINTS);
            agreement.status = EmploymentStatus::Active;
        }
        LaborResponse::ReplaceWorkers => {
            let district_id = state
                .businesses
                .get(business_id)
                .expect("validated business must exist")
                .district_id();
            let replacement = state
                .households
                .ids_for_district(district_id)
                .and_then(|ids| {
                    ids.iter().find(|id| {
                        **id != agreement.household_id
                            && crate::systems::available_household_workers(state, **id)
                                >= u32::from(workers)
                    })
                })
                .copied()
                .ok_or(CommandError::NoReplacementLaborAvailable {
                    employment_id,
                    district_id,
                    workers,
                })?;
            spend_business_cash(state, business_id, LABOR_REPLACEMENT_COST)?;
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("validated employment must exist");
            agreement.household_id = replacement;
            agreement.loyalty_basis_points = 6_000;
            agreement.conditions_basis_points = 6_000;
            agreement.status = EmploymentStatus::Active;
            adjust_district_unrest(state, Some(district_id), 400, true);
        }
    }
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::District,
        format!("Labor dispute {employment_id} resolved"),
        format!("The dynasty chose {response:?}."),
    )?;
    Ok(CommandOutcome {
        summary: format!("Resolved labor dispute {employment_id} with {response:?}."),
    })
}

/// Treasury cost of funding crisis relief for a crisis of the given severity.
///
/// Relief is the premium response: a fixed mobilization base plus a scaling
/// grant per severity point keeps it two to three times the cost of Reform
/// across the working severity range, instead of an unbounded per-point rate
/// that priced a single response above a small house's entire treasury.
#[must_use]
pub(crate) fn crisis_relief_cost(severity_basis_points: u16) -> Money {
    Money::from_copper(
        CRISIS_RELIEF_BASE_COST_COPPER
            + i64::from(severity_basis_points.max(1)) / CRISIS_RELIEF_SEVERITY_DIVISOR,
    )
}
