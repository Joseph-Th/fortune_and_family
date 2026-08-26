//! Intelligence commissioning, leverage, and notification acknowledgement.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Debug)]
pub(crate) struct InformationCommissionPlan {
    target: InformationTarget,
    subject: String,
    summary: String,
}

pub(crate) fn commission_information(
    registry: &Registry,
    state: &mut AppState,
    focus: InformationFocus,
) -> Result<CommandOutcome, CommandError> {
    let plan = resolve_information_commission(registry, state, focus)?;
    let day = state.clock.day();
    let expires_day = checked_future_day(day, INFORMATION_REPORT_LIFETIME_DAYS)?;
    // Resolve every fallible step before the first mutation.
    let id = state.next_ids.try_information_report()?;
    spend_player_treasury_to_market(state, INFORMATION_COMMISSION_COST)?;
    state.information_reports.retain(|_, report| {
        report.owner_dynasty_id != state.player_dynasty_id || report.target != Some(plan.target)
    });
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: state.player_dynasty_id,
            target: Some(plan.target),
            subject: plan.subject.clone(),
            confidence: InformationConfidence::Confirmed,
            created_day: day,
            expires_day,
            source: COMMISSIONED_INFORMATION_SOURCE.to_owned(),
            summary: plan.summary,
        },
    );
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::InformationCommission,
        subject: format!("dynasty:{}", state.player_dynasty_id).into(),
        detail: format!("report={id};subject={}", plan.subject).into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Information,
        "Commissioned intelligence delivered".to_owned(),
        format!("{} is now available to the dynasty.", plan.subject),
    )?;
    Ok(CommandOutcome {
        summary: format!("Commissioned intelligence report {id}: {}.", plan.subject),
    })
}

pub(crate) fn resolve_information_commission(
    registry: &Registry,
    state: &AppState,
    focus: InformationFocus,
) -> Result<InformationCommissionPlan, CommandError> {
    let report_commission_day = state
        .information_reports
        .values()
        .filter(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == COMMISSIONED_INFORMATION_SOURCE
        })
        .map(|report| report.created_day)
        .max();
    let audit_subject = format!("dynasty:{}", state.player_dynasty_id);
    let audit_commission_day = state
        .audit_log
        .iter()
        .filter(|record| {
            record.kind() == AuditKind::InformationCommission && record.subject() == audit_subject
        })
        .map(AuditRecord::day)
        .max();
    if let Some(last_commission_day) = report_commission_day.max(audit_commission_day) {
        let next_commission_day =
            checked_future_day(last_commission_day, INFORMATION_COMMISSION_INTERVAL_DAYS)?;
        if state.clock.day() < next_commission_day {
            return Err(CommandError::InformationCommissionCooldown {
                next_commission_day,
            });
        }
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    if treasury < INFORMATION_COMMISSION_COST {
        return Err(CommandError::InsufficientPlayerFunds {
            available: treasury,
            required: INFORMATION_COMMISSION_COST,
        });
    }
    match focus {
        InformationFocus::Market { good_id } => {
            resolve_market_information(registry, state, good_id)
        }
        InformationFocus::Counterparty { dynasty_id } => {
            resolve_counterparty_information(state, dynasty_id)
        }
        InformationFocus::District { district_id } => {
            resolve_district_information(registry, state, district_id)
        }
    }
}

pub(crate) fn resolve_market_information(
    registry: &Registry,
    state: &AppState,
    good_id: GoodId,
) -> Result<InformationCommissionPlan, CommandError> {
    let good = registry
        .get_good(good_id)
        .ok_or(CommandError::MissingGood { good_id })?;
    let quote = state
        .market
        .quotes
        .get(&good_id)
        .ok_or(CommandError::MissingMarketQuote { good_id })?;
    Ok(InformationCommissionPlan {
        target: InformationTarget::Market { good_id },
        subject: format!("Commissioned market brief: {}", good.name()),
        summary: format!(
            "Price {}; previous price {}; stock {}; target stock {}; today's demand {}; today's supply {}; recorded causes {:?}.",
            quote.price,
            quote.previous_price,
            quote.stock,
            quote.target_stock,
            quote.demand_today,
            quote.supply_today,
            quote.causes
        ),
    })
}

pub(crate) fn resolve_counterparty_information(
    state: &AppState,
    dynasty_id: DynastyId,
) -> Result<InformationCommissionPlan, CommandError> {
    if dynasty_id == state.player_dynasty_id {
        return Err(CommandError::InformationCannotTargetPlayer);
    }
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .ok_or(CommandError::MissingDynasty { dynasty_id })?;
    let relationship = state
        .relationships
        .get(&DynastyPair::new(state.player_dynasty_id, dynasty_id))
        .expect("every dynasty pair must have a relationship record");
    let unsettled_credit = state
        .loans
        .values()
        .filter(|loan| {
            ((loan.lender_dynasty_id == state.player_dynasty_id
                && loan.borrower_dynasty_id == dynasty_id)
                || (loan.lender_dynasty_id == dynasty_id
                    && loan.borrower_dynasty_id == state.player_dynasty_id))
                && !matches!(loan.status, crate::core::LoanStatus::Repaid)
        })
        .count();
    Ok(InformationCommissionPlan {
        target: InformationTarget::Counterparty { dynasty_id },
        subject: format!("Commissioned house brief: House {}", dynasty.name()),
        summary: format!(
            "Treasury {}; reliability {}; trust {}; respect {}; fear {}; resentment {}; obligation {}; unsettled bilateral credit {}.",
            dynasty.treasury(),
            basis_points_label(dynasty.resources.reputation_reliability_basis_points),
            basis_points_label(relationship.trust_basis_points),
            basis_points_label(relationship.respect_basis_points),
            basis_points_label(relationship.fear_basis_points),
            basis_points_label(relationship.resentment_basis_points),
            relationship.obligation,
            unsettled_credit
        ),
    })
}

/// Formats basis points as a percentage with one correctly rounded decimal
/// digit, so intelligence briefs never truncate their own figures.
pub(crate) fn basis_points_label(basis_points: u16) -> String {
    let tenths_of_percent = (basis_points + 5) / 10;
    format!("{}.{}%", tenths_of_percent / 10, tenths_of_percent % 10)
}

pub(crate) fn resolve_district_information(
    registry: &Registry,
    state: &AppState,
    district_id: DistrictId,
) -> Result<InformationCommissionPlan, CommandError> {
    let district = registry
        .get_district(district_id)
        .ok_or(CommandError::MissingDistrict { district_id })?;
    let runtime = state
        .districts
        .get(&district_id)
        .ok_or(CommandError::MissingDistrict { district_id })?;
    Ok(InformationCommissionPlan {
        target: InformationTarget::District { district_id },
        subject: format!("Commissioned district brief: {}", district.name()),
        summary: format!(
            "Rent index {}; employment {}; sanitation {}; safety {}; unrest {}; population {}.",
            basis_points_label(runtime.rent_index_basis_points),
            basis_points_label(runtime.employment_basis_points),
            basis_points_label(runtime.sanitation_basis_points),
            basis_points_label(runtime.safety_basis_points),
            basis_points_label(runtime.unrest_basis_points),
            district.population()
        ),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InformationLeverageQuote {
    pub report_id: InformationReportId,
    pub description: String,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DistrictInformationInitiative {
    Employment,
    Sanitation,
    Safety,
}

impl DistrictInformationInitiative {
    const fn label(self) -> &'static str {
        match self {
            Self::Employment => "employment",
            Self::Sanitation => "sanitation",
            Self::Safety => "safety",
        }
    }
}

#[derive(Debug)]
pub(crate) enum InformationLeverageEffect {
    Contract {
        contract_id: ContractId,
        counterparty_id: DynastyId,
        previous_price: Money,
        new_price: Money,
    },
    Counterparty {
        dynasty_id: DynastyId,
    },
    CounterpartyContract {
        dynasty_id: DynastyId,
        contract_id: ContractId,
        previous_price: Money,
        new_price: Money,
    },
    District {
        district_id: DistrictId,
        initiative: DistrictInformationInitiative,
    },
}

#[derive(Debug)]
pub(crate) struct InformationLeveragePlan {
    pub(crate) quote: InformationLeverageQuote,
    pub(crate) effect: InformationLeverageEffect,
}

pub(crate) fn quote_information_leverage(
    registry: &Registry,
    state: &AppState,
    report_id: InformationReportId,
) -> Result<InformationLeverageQuote, CommandError> {
    resolve_information_leverage(registry, state, report_id).map(|plan| plan.quote)
}

pub(crate) fn resolve_information_leverage(
    registry: &Registry,
    state: &AppState,
    report_id: InformationReportId,
) -> Result<InformationLeveragePlan, CommandError> {
    let report = state
        .information_reports
        .get(&report_id)
        .ok_or(CommandError::MissingInformationReport { report_id })?;
    if report.owner_dynasty_id != state.player_dynasty_id {
        return Err(CommandError::InformationReportNotOwned { report_id });
    }
    if report.source != COMMISSIONED_INFORMATION_SOURCE
        || report.confidence != InformationConfidence::Confirmed
    {
        return Err(CommandError::InformationReportNotCommissioned { report_id });
    }
    if state.clock.day() > report.expires_day {
        return Err(CommandError::InformationReportExpired {
            report_id,
            expired_day: report.expires_day,
        });
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    if treasury < INFORMATION_LEVERAGE_COST {
        return Err(CommandError::InsufficientPlayerFunds {
            available: treasury,
            required: INFORMATION_LEVERAGE_COST,
        });
    }

    match report.target {
        Some(InformationTarget::Market { good_id }) => {
            resolve_market_information_leverage(registry, state, report_id, good_id)
        }
        Some(InformationTarget::Counterparty { dynasty_id }) => {
            resolve_counterparty_information_leverage(state, report_id, dynasty_id)
        }
        Some(InformationTarget::District { district_id }) => {
            resolve_district_information_leverage(registry, state, report_id, district_id)
        }
        None => Err(CommandError::InformationReportHasNoLeverage { report_id }),
    }
}

pub(crate) fn resolve_market_information_leverage(
    registry: &Registry,
    state: &AppState,
    report_id: InformationReportId,
    good_id: GoodId,
) -> Result<InformationLeveragePlan, CommandError> {
    let good = registry
        .get_good(good_id)
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let player_id = state.player_dynasty_id;
    let (contract, counterparty_id, new_price) = state
        .contracts
        .values()
        .filter(|contract| contract.status == ContractStatus::Active && contract.good_id == good_id)
        .find_map(|contract| {
            market_contract_leverage_terms(state, player_id, contract)
                .map(|(counterparty_id, new_price)| (contract, counterparty_id, new_price))
        })
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let description = format!(
        "use report {report_id} to renegotiate {} contract {} from {} to {} per unit",
        good.name(),
        contract.id,
        contract.unit_price,
        new_price
    );
    Ok(InformationLeveragePlan {
        quote: InformationLeverageQuote {
            report_id,
            description,
        },
        effect: InformationLeverageEffect::Contract {
            contract_id: contract.id,
            counterparty_id,
            previous_price: contract.unit_price,
            new_price,
        },
    })
}

pub(crate) fn market_contract_leverage_terms(
    state: &AppState,
    player_id: DynastyId,
    contract: &crate::core::SupplyContract,
) -> Option<(DynastyId, Money)> {
    let buyer_owner = state
        .businesses
        .get(contract.buyer_business_id)?
        .owner_dynasty_id();
    let seller_owner = state
        .businesses
        .get(contract.seller_business_id)?
        .owner_dynasty_id();
    let one_copper = Money::from_copper(1);
    let (counterparty_id, new_price) = if buyer_owner == player_id && seller_owner != player_id {
        let discounted = contract.unit_price.checked_mul_ratio(95, 100)?;
        let one_copper_less = contract.unit_price.checked_sub(one_copper)?;
        (
            seller_owner,
            discounted.min(one_copper_less).max(one_copper),
        )
    } else if seller_owner == player_id && buyer_owner != player_id {
        let increased = contract.unit_price.checked_mul_ratio(105, 100)?;
        let one_copper_more = contract.unit_price.checked_add(one_copper)?;
        (buyer_owner, increased.max(one_copper_more))
    } else {
        return None;
    };
    if new_price == contract.unit_price
        || crate::money::checked_cost_for(contract.quantity_per_week, new_price).is_none()
    {
        return None;
    }
    Some((counterparty_id, new_price))
}

pub(crate) fn resolve_counterparty_information_leverage(
    state: &AppState,
    report_id: InformationReportId,
    dynasty_id: DynastyId,
) -> Result<InformationLeveragePlan, CommandError> {
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let pair = DynastyPair::new(state.player_dynasty_id, dynasty_id);
    if !state.relationships.contains_key(&pair) {
        return Err(CommandError::InformationReportHasNoLeverage { report_id });
    }
    if let Some((contract, new_price)) = state
        .contracts
        .values()
        .filter(|contract| contract.status == ContractStatus::Active)
        .find_map(|contract| {
            market_contract_leverage_terms(state, state.player_dynasty_id, contract)
                .filter(|(counterparty_id, _)| *counterparty_id == dynasty_id)
                .map(|(_, new_price)| (contract, new_price))
        })
    {
        return Ok(InformationLeveragePlan {
            quote: InformationLeverageQuote {
                report_id,
                description: format!(
                    "use report {report_id} to negotiate contract {} with House {} from {} to {} per unit",
                    contract.id,
                    dynasty.name(),
                    contract.unit_price,
                    new_price
                ),
            },
            effect: InformationLeverageEffect::CounterpartyContract {
                dynasty_id,
                contract_id: contract.id,
                previous_price: contract.unit_price,
                new_price,
            },
        });
    }
    Ok(InformationLeveragePlan {
        quote: InformationLeverageQuote {
            report_id,
            description: format!(
                "use report {report_id} for targeted outreach to House {}",
                dynasty.name()
            ),
        },
        effect: InformationLeverageEffect::Counterparty { dynasty_id },
    })
}

pub(crate) fn resolve_district_information_leverage(
    registry: &Registry,
    state: &AppState,
    report_id: InformationReportId,
    district_id: DistrictId,
) -> Result<InformationLeveragePlan, CommandError> {
    let district_definition = registry
        .get_district(district_id)
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let district = state
        .districts
        .get(&district_id)
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let initiative = [
        (
            district.employment_basis_points,
            DistrictInformationInitiative::Employment,
        ),
        (
            district.sanitation_basis_points,
            DistrictInformationInitiative::Sanitation,
        ),
        (
            district.safety_basis_points,
            DistrictInformationInitiative::Safety,
        ),
    ]
    .into_iter()
    .min_by_key(|(value, _)| *value)
    .map(|(_, initiative)| initiative)
    .expect("district initiative list must be nonempty");
    Ok(InformationLeveragePlan {
        quote: InformationLeverageQuote {
            report_id,
            description: format!(
                "use report {report_id} to fund a targeted {} initiative in {}",
                initiative.label(),
                district_definition.name()
            ),
        },
        effect: InformationLeverageEffect::District {
            district_id,
            initiative,
        },
    })
}

pub(crate) fn leverage_information(
    registry: &Registry,
    state: &mut AppState,
    report_id: InformationReportId,
) -> Result<CommandOutcome, CommandError> {
    let plan = resolve_information_leverage(registry, state, report_id)?;
    spend_player_treasury_to_market(state, INFORMATION_LEVERAGE_COST)?;
    apply_information_leverage_effect(state, &plan.effect);
    state
        .information_reports
        .remove(&report_id)
        .expect("validated information report must exist");
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::InformationLeverage,
        subject: format!("information-report:{report_id}").into(),
        detail: plan.quote.description.clone().into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Information,
        "Commissioned intelligence converted into action".to_owned(),
        format!(
            "{} at a cost of {INFORMATION_LEVERAGE_COST}.",
            plan.quote.description
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Leveraged intelligence report {report_id}: {}.",
            plan.quote.description
        ),
    })
}

pub(crate) fn apply_information_leverage_effect(
    state: &mut AppState,
    effect: &InformationLeverageEffect,
) {
    match *effect {
        InformationLeverageEffect::Contract {
            contract_id,
            counterparty_id,
            previous_price,
            new_price,
        } => {
            state
                .contracts
                .get_mut(&contract_id)
                .expect("validated contract must exist")
                .unit_price = new_price;
            let memory = format!(
                "intelligence-backed contract renegotiation changed unit price from {previous_price} to {new_price}"
            );
            adjust_information_relationship(state, counterparty_id, -75, -50, 150, 125, 0, &memory);
        }
        InformationLeverageEffect::Counterparty { dynasty_id } => {
            adjust_information_relationship(
                state,
                dynasty_id,
                300,
                200,
                0,
                -200,
                2,
                "targeted outreach based on a commissioned house brief",
            );
        }
        InformationLeverageEffect::CounterpartyContract {
            dynasty_id,
            contract_id,
            previous_price,
            new_price,
        } => {
            state
                .contracts
                .get_mut(&contract_id)
                .expect("validated counterparty contract must exist")
                .unit_price = new_price;
            // Forcing a price concession with commissioned intelligence reads
            // to the target exactly like the market-brief squeeze: distrust,
            // lost esteem, intimidation, and resentment. Positive trust and
            // respect are reserved for outreach that extracts no price
            // change.
            adjust_information_relationship(
                state,
                dynasty_id,
                -75,
                -50,
                150,
                125,
                0,
                &format!(
                    "a commissioned house brief supported a negotiated contract adjustment from {previous_price} to {new_price}"
                ),
            );
        }
        InformationLeverageEffect::District {
            district_id,
            initiative,
        } => {
            let district = state
                .districts
                .get_mut(&district_id)
                .expect("validated district must exist");
            match initiative {
                DistrictInformationInitiative::Employment => {
                    district.employment_basis_points = district
                        .employment_basis_points
                        .saturating_add(250)
                        .min(10_000);
                }
                DistrictInformationInitiative::Sanitation => {
                    district.sanitation_basis_points = district
                        .sanitation_basis_points
                        .saturating_add(250)
                        .min(10_000);
                }
                DistrictInformationInitiative::Safety => {
                    district.safety_basis_points =
                        district.safety_basis_points.saturating_add(250).min(10_000);
                }
            }
            district.unrest_basis_points = district.unrest_basis_points.saturating_sub(100);
            improve_player_reputation(state, 75, 75);
        }
    }
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn adjust_information_relationship(
    state: &mut AppState,
    counterparty_id: DynastyId,
    trust_change: i16,
    respect_change: i16,
    fear_change: i16,
    resentment_change: i16,
    obligation_change: i32,
    memory: &str,
) {
    let player_id = state.player_dynasty_id;
    crate::systems::strategic::adjust_dynasty_relationship(
        state,
        player_id,
        counterparty_id,
        crate::systems::strategic::RelationshipDelta::new(
            trust_change,
            respect_change,
            fear_change,
            resentment_change,
            obligation_change,
        ),
    );
    crate::systems::strategic::remember_dynasty_interaction(
        state,
        player_id,
        counterparty_id,
        memory,
    );
}

pub(crate) fn acknowledge(
    state: &mut AppState,
    message_id: OutboxMessageId,
) -> Result<CommandOutcome, CommandError> {
    if !state.outbox.iter().any(|message| message.id == message_id) {
        return Err(CommandError::MissingNotification { message_id });
    }
    let mut acknowledged = 0_u32;
    for message in state
        .outbox
        .iter_mut()
        .filter(|message| message.id <= message_id && !message.acknowledged)
    {
        message.acknowledged = true;
        acknowledged = acknowledged.saturating_add(1);
    }
    if acknowledged == 0 {
        // Every notification through `message_id` was already acknowledged:
        // like other unchanged-state requests this is a typed no-op failure,
        // not a successful command that mutated nothing.
        return Err(CommandError::NotificationAlreadyAcknowledged { message_id });
    }
    Ok(CommandOutcome {
        summary: format!(
            "Acknowledged {acknowledged} notifications through notification {message_id}."
        ),
    })
}
