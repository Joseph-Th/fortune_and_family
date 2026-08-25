//! Institutions, patronage, endowments, offices, nominations, and directives.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn apply_institution_withdrawal(
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<CommandOutcome, CommandError> {
    let character = state
        .characters
        .get(character_id)
        .ok_or(CommandError::MissingCharacter { character_id })?;
    if character.dynasty_id() != state.player_dynasty_id {
        return Err(CommandError::InvalidInstitutionWithdrawal {
            institution_id,
            character_id,
        });
    }
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if !institution.members.contains(&character_id) {
        return Err(CommandError::InvalidInstitutionWithdrawal {
            institution_id,
            character_id,
        });
    }
    let resigned_office = institution.office_holder_id == Some(character_id);
    let day = state.clock.day();
    let replacement_selection_day = resigned_office
        .then(|| checked_future_day(day, 30))
        .transpose()?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.members.remove(&character_id);
    if resigned_office {
        institution.office_holder_id = None;
        // The departed holder's policy dies with the resignation: a directive
        // nobody holds office for must not keep shaping the district.
        institution.active_directive = None;
        if let Some(replacement_selection_day) = replacement_selection_day {
            institution.next_selection_day = institution
                .next_selection_day
                .min(replacement_selection_day);
        }
    }
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::InstitutionWithdrawal,
        subject: institution_support_subject(institution_id, character_id).into(),
        detail: if resigned_office {
            crate::systems::OFFICE_RESIGNATION_AUDIT_DETAIL
                .to_owned()
                .into()
        } else {
            "resigned_office=false".to_owned().into()
        },
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Character {character_id} withdrew from institution {institution_id}"),
        if resigned_office {
            "The dynasty surrendered the office and its institutional membership; a replacement selection will be scheduled.".to_owned()
        } else {
            "The dynasty surrendered this institutional membership.".to_owned()
        },
    )?;
    Ok(CommandOutcome {
        summary: if resigned_office {
            format!(
                "Withdrew character {character_id} from institution {institution_id} and resigned the office."
            )
        } else {
            format!("Withdrew character {character_id} from institution {institution_id}.")
        },
    })
}

/// Whether the named office power is currently held by an active player-dynasty
/// character. This is the single canonical gate for exercising office powers:
/// incapacitation vacates offices, but this predicate does not depend on that
/// cross-system invariant holding.
pub(crate) fn office_power_is_player_held(
    state: &AppState,
    institution: &InstitutionRuntime,
    power: OfficePower,
) -> bool {
    institution.powers.contains(&power)
        && institution.office_holder_id.is_some_and(|character_id| {
            state.characters.get(character_id).is_some_and(|character| {
                character.status() == CharacterStatus::Active
                    && character.dynasty_id() == state.player_dynasty_id
            })
        })
}

pub(crate) fn has_player_office(state: &AppState) -> bool {
    state.institutions.values().any(|institution| {
        institution.office_holder_id.is_some_and(|character_id| {
            state.characters.get(character_id).is_some_and(|character| {
                character.status() == CharacterStatus::Active
                    && character.dynasty_id() == state.player_dynasty_id
            })
        })
    })
}

pub(crate) fn has_player_office_power(state: &AppState, power: OfficePower) -> bool {
    state
        .institutions
        .values()
        .any(|institution| office_power_is_player_held(state, institution, power))
}

pub(crate) fn player_office_power_available_day(
    state: &AppState,
    power: OfficePower,
) -> Option<i64> {
    checked_player_office_power_available_day(state, power).unwrap_or(Some(i64::MAX))
}

pub(crate) fn checked_player_office_power_available_day(
    state: &AppState,
    power: OfficePower,
) -> Result<Option<i64>, TimelineError> {
    let mut earliest = None;
    let mut range_error = None;
    for institution in state
        .institutions
        .values()
        .filter(|institution| office_power_is_player_held(state, institution, power))
    {
        match checked_future_day(
            institution.term_started_day,
            OFFICE_POWER_ESTABLISHMENT_DAYS,
        ) {
            Ok(available_day) => {
                earliest = Some(earliest.map_or(available_day, |day: i64| day.min(available_day)));
            }
            Err(error) if range_error.is_none() => range_error = Some(error),
            Err(_) => {}
        }
    }
    match earliest {
        Some(day) => Ok(Some(day)),
        None => range_error.map_or(Ok(None), Err),
    }
}

pub(crate) fn has_established_player_office_power(state: &AppState, power: OfficePower) -> bool {
    player_office_power_available_day(state, power)
        .is_some_and(|available_day| state.clock.day() >= available_day)
}

pub(crate) const fn required_office_power_for_law(kind: LawKind) -> OfficePower {
    match kind {
        LawKind::BreadPriceCeiling | LawKind::ForeignMerchantToll => OfficePower::MarketTolls,
        LawKind::InterestLimit => OfficePower::DebtEnforcement,
        LawKind::FireCode => OfficePower::Inspections,
        LawKind::RentRestriction | LawKind::PublicDebtAuthorization => OfficePower::Taxation,
        LawKind::GuildEntryRestriction => OfficePower::Licenses,
        LawKind::EmergencyImports => OfficePower::EmergencyImports,
    }
}

pub(crate) fn apply_institution_support(
    registry: &Registry,
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<CommandOutcome, CommandError> {
    let character = state
        .characters
        .get(character_id)
        .ok_or(CommandError::InvalidNominee { character_id })?;
    if character.dynasty_id() != state.player_dynasty_id
        || character.status() != CharacterStatus::Active
    {
        return Err(CommandError::InvalidNominee { character_id });
    }
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    validate_institution_support_standing(registry, state, institution_id, character_id)?;
    let subject = institution_support_subject(institution_id, character_id);
    if institution.members.contains(&character_id) {
        return Err(CommandError::InstitutionSupportAlreadyEstablished {
            institution_id,
            character_id,
        });
    }
    let membership_count = institution_membership_count(state, character_id);
    if membership_count >= MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER {
        return Err(CommandError::InstitutionMembershipCapacity {
            character_id,
            current: membership_count,
            maximum: MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER,
        });
    }
    if let Some(next_support_day) =
        institution_support_next_day(state, institution_id, character_id)
        && state.clock.day() < next_support_day
    {
        return Err(CommandError::InstitutionSupportCooldown { next_support_day });
    }
    // An active guild entry restriction prices outsiders out of the charter:
    // joining costs more as the restriction tightens, up to a fifty percent
    // surcharge on the standard patronage contribution.
    let entry_restriction =
        crate::systems::strategic::active_law_value(state, LawKind::GuildEntryRestriction)
            .unwrap_or(0)
            .clamp(0, 10_000);
    let support_cost =
        INSTITUTION_SUPPORT_COST.saturating_mul_ratio(10_000 + entry_restriction / 2, 10_000);
    let budget_after = institution.budget.checked_add(support_cost).ok_or(
        CommandError::InstitutionBudgetOverflow {
            institution_id,
            current: institution.budget,
            incoming: support_cost,
        },
    )?;
    let member_dynasties: BTreeSet<_> = institution
        .members
        .iter()
        .filter_map(|member_id| state.characters.get(*member_id))
        .map(Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != state.player_dynasty_id)
        .collect();
    let established_day =
        checked_future_day(state.clock.day(), INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS)?;
    spend_player_treasury(state, support_cost)?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.budget = budget_after;
    institution.members.insert(character_id);
    record_institution_patronage_relationships(
        state,
        institution_id,
        character_id,
        member_dynasties,
    );
    finish_institution_patronage(
        state,
        institution_id,
        character_id,
        subject,
        established_day,
        support_cost,
    )
}

pub(crate) fn record_institution_patronage_relationships(
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
    member_dynasties: BTreeSet<DynastyId>,
) {
    let player_dynasty_id = state.player_dynasty_id;
    for member_dynasty_id in member_dynasties {
        crate::systems::strategic::adjust_dynasty_relationship(
            state,
            player_dynasty_id,
            member_dynasty_id,
            crate::systems::strategic::RelationshipDelta::new(180, 260, 0, -60, 75),
        );
        crate::systems::strategic::remember_dynasty_interaction(
            state,
            player_dynasty_id,
            member_dynasty_id,
            &format!(
                "the player dynasty patronized institution {institution_id} for character {character_id}"
            ),
        );
    }
}

pub(crate) fn finish_institution_patronage(
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
    subject: String,
    established_day: i64,
    contribution: Money,
) -> Result<CommandOutcome, CommandError> {
    let day = state.clock.day();
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::InstitutionPatronage,
        subject: subject.into(),
        detail: format!("contribution={}", contribution.copper()).into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Institutional support cultivated for character {character_id}"),
        format!(
            "The dynasty patronized institution {institution_id}; character {character_id}'s support, and the legitimacy it earns the house, will be established by day {established_day}."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Cultivated support for character {character_id} in institution {institution_id}."
        ),
    })
}

#[derive(Clone, Debug)]
pub(crate) struct ValidatedInstitutionEndowment {
    player_id: DynastyId,
    institution_id: InstitutionId,
    amount: Money,
    treasury_after: Money,
    contributions_after: Money,
    budget_after: Money,
    legitimacy_gain: u16,
    relationship_scale: i16,
    member_dynasties: BTreeSet<DynastyId>,
}

pub(crate) fn apply_institution_endowment(
    state: &mut AppState,
    institution_id: InstitutionId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    let validated = validate_institution_endowment(state, institution_id, amount)?;
    commit_institution_endowment(state, &validated)?;
    Ok(CommandOutcome {
        summary: format!("Endowed institution {institution_id} with {amount}."),
    })
}

pub(crate) fn validate_institution_endowment(
    state: &AppState,
    institution_id: InstitutionId,
    amount: Money,
) -> Result<ValidatedInstitutionEndowment, CommandError> {
    if amount < INSTITUTION_ENDOWMENT_MIN || amount > INSTITUTION_ENDOWMENT_MAX {
        return Err(CommandError::InstitutionEndowmentOutOfRange {
            minimum: INSTITUTION_ENDOWMENT_MIN,
            maximum: INSTITUTION_ENDOWMENT_MAX,
            requested: amount,
        });
    }
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if !has_established_player_institution_membership(state, institution_id) {
        return Err(CommandError::InstitutionEndowmentRequiresMembership { institution_id });
    }
    if let Some(next_endowment_day) = institution_endowment_next_day(state)
        && state.clock.day() < next_endowment_day
    {
        return Err(CommandError::InstitutionEndowmentCooldown { next_endowment_day });
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
        .expect("validated endowment must fit player treasury");
    let contributions_after = player.civic_contributions().checked_add(amount).ok_or(
        crate::systems::SimulationError::DynastyCivicContributionsOverflow {
            dynasty_id: player_id,
            current: player.civic_contributions(),
            incoming: amount,
        },
    )?;
    let budget_after =
        institution
            .budget
            .checked_add(amount)
            .ok_or(CommandError::InstitutionBudgetOverflow {
                institution_id,
                current: institution.budget,
                incoming: amount,
            })?;
    let member_dynasties: BTreeSet<_> = institution
        .members
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .map(Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != player_id)
        .collect();
    let legitimacy_gain = u16::try_from((amount.copper() / 200).clamp(25, 250))
        .expect("bounded endowment legitimacy gain must fit u16");
    let relationship_scale =
        i16::try_from((amount.copper() / INSTITUTION_ENDOWMENT_MIN.copper()).clamp(1, 10))
            .expect("bounded endowment relationship scale must fit i16");
    Ok(ValidatedInstitutionEndowment {
        player_id,
        institution_id,
        amount,
        treasury_after,
        contributions_after,
        budget_after,
        legitimacy_gain,
        relationship_scale,
        member_dynasties,
    })
}

pub(crate) fn commit_institution_endowment(
    state: &mut AppState,
    endowment: &ValidatedInstitutionEndowment,
) -> Result<(), CommandError> {
    let player = state
        .dynasties
        .get_mut(&endowment.player_id)
        .expect("validated player dynasty must exist");
    player.resources.treasury = endowment.treasury_after;
    player.resources.civic_contributions = endowment.contributions_after;
    let institution = state
        .institutions
        .get_mut(&endowment.institution_id)
        .expect("validated institution must exist");
    institution.budget = endowment.budget_after;
    // Report the legitimacy the endowment actually bought: a gain absorbed by
    // the cap must not be recorded as if it had been delivered.
    let legitimacy_before = institution.legitimacy_basis_points;
    institution.legitimacy_basis_points = institution
        .legitimacy_basis_points
        .saturating_add(endowment.legitimacy_gain)
        .min(10_000);
    let applied_legitimacy_gain = institution.legitimacy_basis_points - legitimacy_before;
    for member_dynasty_id in &endowment.member_dynasties {
        crate::systems::strategic::adjust_dynasty_relationship(
            state,
            endowment.player_id,
            *member_dynasty_id,
            crate::systems::strategic::RelationshipDelta::new(
                endowment.relationship_scale.saturating_mul(8),
                endowment.relationship_scale.saturating_mul(15),
                0,
                -endowment.relationship_scale.saturating_mul(5),
                i32::from((endowment.relationship_scale.saturating_add(1)) / 2),
            ),
        );
        crate::systems::strategic::remember_dynasty_interaction(
            state,
            endowment.player_id,
            *member_dynasty_id,
            &format!(
                "the player dynasty endowed institution {} with {}, strengthening its standing among the membership",
                endowment.institution_id, endowment.amount
            ),
        );
    }
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::InstitutionEndowment,
        subject: format!(
            "institution:{};dynasty:{}",
            endowment.institution_id, endowment.player_id
        )
        .into(),
        detail: format!(
            "amount={};institution_legitimacy_gain={}",
            endowment.amount.copper(),
            applied_legitimacy_gain
        )
        .into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Institution {} endowed", endowment.institution_id),
        format!(
            "The dynasty endowed institution {} with {}, strengthening its budget, civic legitimacy, and standing among member houses.",
            endowment.institution_id, endowment.amount
        ),
    )?;
    Ok(())
}

pub(crate) fn has_established_player_institution_membership(
    state: &AppState,
    institution_id: InstitutionId,
) -> bool {
    let Some(institution) = state.institutions.get(&institution_id) else {
        return false;
    };
    institution.members.iter().copied().any(|character_id| {
        let active_player_member = state.characters.get(character_id).is_some_and(|character| {
            character.dynasty_id() == state.player_dynasty_id
                && character.status() == CharacterStatus::Active
        });
        active_player_member
            && (institution.office_holder_id == Some(character_id)
                || institution_support_day(state, institution_id, character_id).is_some_and(
                    |support_day| {
                        state.clock.day()
                            >= future_day_or_terminal(
                                support_day,
                                INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS,
                            )
                    },
                ))
    })
}

pub(crate) fn institution_endowment_next_day(state: &AppState) -> Option<i64> {
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::InstitutionEndowment
                && record.audit_subject().dynasty_id() == Some(state.player_dynasty_id)
        })
        .map(|record| future_day_or_terminal(record.day(), INSTITUTION_ENDOWMENT_INTERVAL_DAYS))
}

/// Reputation standing gate shared by every command that requires an
/// established house: the better of quality and reliability must clear the
/// requirement. The error payload carries the observed values so each caller
/// can surface its own typed variant.
pub(crate) fn ensure_reputation_standing(
    state: &AppState,
    required: u16,
) -> Result<(), (u16, u16, u16)> {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let quality = player.resources.reputation_quality_basis_points;
    let reliability = player.resources.reputation_reliability_basis_points;
    if quality.max(reliability) < required {
        Err((quality, reliability, required))
    } else {
        Ok(())
    }
}

/// Shared standing gate for privileged commands: the house needs both the
/// required reputation standing and a durable record of fulfilled contract
/// deliveries before it may act.
pub(crate) fn ensure_standing(
    state: &AppState,
    reputation_required: u16,
    deliveries_required: u32,
    insufficient_reputation: impl FnOnce(u16, u16, u16) -> CommandError,
    insufficient_deliveries: impl FnOnce(u32, u32) -> CommandError,
) -> Result<(), CommandError> {
    ensure_reputation_standing(state, reputation_required).map_err(
        |(quality, reliability, required)| insufficient_reputation(quality, reliability, required),
    )?;
    let delivered = player_contract_deliveries(state);
    if delivered < deliveries_required {
        return Err(insufficient_deliveries(delivered, deliveries_required));
    }
    Ok(())
}

pub(crate) fn validate_institution_support_standing(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<(), CommandError> {
    let required =
        institution_support_delivery_requirement(registry, state, institution_id, character_id);
    ensure_standing(
        state,
        INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT,
        required,
        |quality, reliability, required| CommandError::InsufficientInstitutionSupportReputation {
            quality,
            reliability,
            required,
        },
        |delivered, required| CommandError::InsufficientInstitutionSupportCommercialRecord {
            delivered,
            required,
        },
    )
}

pub(crate) fn institution_support_delivery_requirement(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> u32 {
    capability_adjusted_delivery_requirement(
        registry,
        state,
        institution_id,
        character_id,
        &StandingGateTuning {
            context: "institution support",
            base_requirement: INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT,
            capability_target_score: INSTITUTION_SUPPORT_CAPABILITY_TARGET_SCORE,
            capability_delivery_step: INSTITUTION_SUPPORT_CAPABILITY_DELIVERY_STEP,
            max_preparation_deliveries: INSTITUTION_SUPPORT_MAX_PREPARATION_DELIVERIES,
        },
    )
}

/// Tuning for a standing-gated action's commercial-record requirement.
#[derive(Clone, Copy)]
pub(crate) struct StandingGateTuning {
    /// Names the gate in validation panics.
    context: &'static str,
    base_requirement: u32,
    capability_target_score: u32,
    capability_delivery_step: u32,
    max_preparation_deliveries: u32,
}

/// Deliveries a standing-gated action requires: its base commercial record plus
/// preparation deliveries that cover the nominee's capability deficit for the
/// institution's line of work.
pub(crate) fn capability_adjusted_delivery_requirement(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
    tuning: &StandingGateTuning,
) -> u32 {
    let StandingGateTuning {
        context,
        base_requirement,
        capability_target_score,
        capability_delivery_step,
        max_preparation_deliveries,
    } = *tuning;
    let character = state
        .characters
        .get(character_id)
        .unwrap_or_else(|| panic!("{context} character must exist"));
    let institution_kind = registry
        .get_institution(institution_id)
        .unwrap_or_else(|| panic!("{context} institution must exist in the registry"))
        .kind();
    let capability_score =
        crate::systems::strategic::institution_capability_score(character, institution_kind);
    let deficit = capability_target_score.saturating_sub(capability_score);
    let extra_deliveries =
        deficit.saturating_add(capability_delivery_step - 1) / capability_delivery_step;
    base_requirement.saturating_add(extra_deliveries.min(max_preparation_deliveries))
}

pub(crate) fn apply_office_nomination(
    registry: &Registry,
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<CommandOutcome, CommandError> {
    let character = state
        .characters
        .get(character_id)
        .ok_or(CommandError::InvalidNominee { character_id })?;
    if character.dynasty_id() != state.player_dynasty_id
        || character.status() != crate::core::CharacterStatus::Active
    {
        return Err(CommandError::InvalidNominee { character_id });
    }
    if let Some(existing_institution_id) = state
        .institutions
        .values()
        .find(|institution| institution.office_holder_id == Some(character_id))
        .map(|institution| institution.institution_id)
    {
        return Err(CommandError::NomineeAlreadyHoldsOffice {
            character_id,
            institution_id: existing_institution_id,
        });
    }
    validate_office_nomination_standing(registry, state, institution_id, character_id)?;
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if !institution.members.contains(&character_id) {
        return Err(CommandError::MissingInstitutionSupport {
            institution_id,
            character_id,
        });
    }
    let support_day = institution_support_day(state, institution_id, character_id).ok_or(
        CommandError::MissingInstitutionSupport {
            institution_id,
            character_id,
        },
    )?;
    let available_day = checked_future_day(support_day, INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS)?;
    if state.clock.day() < available_day {
        return Err(CommandError::InstitutionSupportNotEstablished {
            institution_id,
            character_id,
            available_day,
        });
    }
    if let Some(next_nomination_day) = office_nomination_next_day(state, character_id)
        && state.clock.day() < next_nomination_day
    {
        return Err(CommandError::OfficeNominationCooldown {
            next_nomination_day,
        });
    }
    let selection_day = checked_future_day(state.clock.day(), OFFICE_NOMINATION_RESOLUTION_DAYS)?;
    spend_player_treasury_to_market(state, OFFICE_NOMINATION_CAMPAIGN_COST)?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.next_selection_day = institution.next_selection_day.min(selection_day);
    let subject = office_nomination_subject(institution_id, character_id);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::OfficeNomination,
        subject: subject.into(),
        detail: format!("campaign_cost={}", OFFICE_NOMINATION_CAMPAIGN_COST.copper()).into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Office campaign launched for character {character_id}"),
        format!(
            "The dynasty nominated character {character_id} to institution {institution_id}; selection is scheduled by day {selection_day}."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Nominated character {character_id} for institution {institution_id}."),
    })
}

#[derive(Debug)]
pub(crate) struct OfficePowerDirectivePlan {
    institution_id: InstitutionId,
    district_id: DistrictId,
    power: OfficePower,
    legitimacy: u16,
    subject: String,
}

pub(crate) fn validate_office_power_directive(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    power: OfficePower,
) -> Result<OfficePowerDirectivePlan, CommandError> {
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if !office_power_is_player_held(state, institution, power) {
        return Err(CommandError::OfficePowerUnavailable {
            institution_id,
            power,
        });
    }
    let available_day = checked_future_day(
        institution.term_started_day,
        OFFICE_POWER_ESTABLISHMENT_DAYS,
    )?;
    if state.clock.day() < available_day {
        return Err(CommandError::OfficePowerDirectiveNotEstablished {
            institution_id,
            power,
            available_day,
        });
    }
    let legitimacy = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points;
    if legitimacy < OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST,
        });
    }
    // The directive cadence belongs to the issuing officeholder's dynasty, not
    // to the institution: an elected successor must not inherit the previous
    // holder's cooldown on their first directive.
    if let Some(last_directive_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::OfficeDirective
                && record.audit_subject().institution_id() == Some(institution_id)
                && record.audit_subject().dynasty_id() == Some(state.player_dynasty_id)
        })
        .map(AuditRecord::day)
    {
        let next_directive_day =
            checked_future_day(last_directive_day, OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS)?;
        if state.clock.day() < next_directive_day {
            return Err(CommandError::OfficePowerDirectiveCooldown {
                institution_id,
                power,
                next_directive_day,
            });
        }
    }
    let district_id = registry
        .get_institution(institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?
        .district_id();
    let subject = format!(
        "institution:{institution_id};dynasty:{}",
        state.player_dynasty_id
    );
    Ok(OfficePowerDirectivePlan {
        institution_id,
        district_id,
        power,
        legitimacy,
        subject,
    })
}

pub(crate) fn improve_player_reputation(state: &mut AppState, quality: u16, reliability: u16) {
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.reputation_quality_basis_points = dynasty
        .resources
        .reputation_quality_basis_points
        .saturating_add(quality)
        .min(10_000);
    dynasty.resources.reputation_reliability_basis_points = dynasty
        .resources
        .reputation_reliability_basis_points
        .saturating_add(reliability)
        .min(10_000);
}

pub(crate) fn adjust_directive_district(
    state: &mut AppState,
    district_id: DistrictId,
    employment: u16,
    sanitation: u16,
    safety: u16,
    unrest: i16,
) {
    let district = state
        .districts
        .get_mut(&district_id)
        .expect("validated institution district must exist");
    district.employment_basis_points = district
        .employment_basis_points
        .saturating_add(employment)
        .min(10_000);
    district.sanitation_basis_points = district
        .sanitation_basis_points
        .saturating_add(sanitation)
        .min(10_000);
    district.safety_basis_points = district
        .safety_basis_points
        .saturating_add(safety)
        .min(10_000);
    district.unrest_basis_points = if unrest >= 0 {
        district
            .unrest_basis_points
            .saturating_add(unrest.unsigned_abs())
            .min(10_000)
    } else {
        district
            .unrest_basis_points
            .saturating_sub(unrest.unsigned_abs())
    };
}

pub(crate) fn apply_office_power_directive_effect(
    state: &mut AppState,
    institution_id: InstitutionId,
    district_id: DistrictId,
    power: OfficePower,
) {
    match power {
        OfficePower::Licenses => {
            adjust_directive_district(state, district_id, 250, 0, 0, 0);
            improve_player_reputation(state, 50, 0);
        }
        OfficePower::Inspections => {
            adjust_directive_district(state, district_id, 0, 300, 0, 50);
            improve_player_reputation(state, 100, 0);
        }
        OfficePower::MarketTolls => {
            adjust_directive_district(state, district_id, 0, 0, 0, 150);
            raise_institution_legitimacy(state, institution_id, 100);
        }
        OfficePower::DebtEnforcement => {
            adjust_directive_district(state, district_id, 0, 0, 0, 100);
            improve_player_reputation(state, 0, 100);
        }
        OfficePower::CityContracts => {
            adjust_directive_district(state, district_id, 250, 0, 0, 0);
            improve_player_reputation(state, 75, 75);
        }
        OfficePower::PublicWorks => adjust_directive_district(state, district_id, 200, 200, 0, 0),
        OfficePower::WatchPriorities => {
            adjust_directive_district(state, district_id, 0, 0, 350, -150);
        }
        OfficePower::Taxation => {
            adjust_directive_district(state, district_id, 0, 0, 0, 250);
            raise_institution_legitimacy(state, institution_id, 150);
        }
        OfficePower::EmergencyImports => {
            adjust_directive_district(state, district_id, 0, 0, 0, -200);
            for household in state
                .households
                .iter_mut()
                .filter(|household| household.district_id() == district_id)
            {
                household.food_satisfaction_basis_points = household
                    .food_satisfaction_basis_points
                    .saturating_add(300)
                    .min(10_000);
            }
        }
    }
}

/// Returns the legitimacy actually applied after the 10,000 bp cap, so
/// callers can report what was delivered rather than what was requested.
pub(crate) fn raise_institution_legitimacy(
    state: &mut AppState,
    institution_id: InstitutionId,
    amount: u16,
) -> u16 {
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    let before = institution.legitimacy_basis_points;
    institution.legitimacy_basis_points = before.saturating_add(amount).min(10_000);
    institution.legitimacy_basis_points - before
}

pub(crate) fn apply_office_power_directive(
    registry: &Registry,
    state: &mut AppState,
    institution_id: InstitutionId,
    power: OfficePower,
) -> Result<CommandOutcome, CommandError> {
    let OfficePowerDirectivePlan {
        institution_id,
        district_id,
        power,
        legitimacy,
        subject,
    } = validate_office_power_directive(registry, state, institution_id, power)?;
    let directive_expires_day =
        checked_future_day(state.clock.day(), OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS)?;
    // Resolve every fallible step before the first mutation.
    let chronicle_id = state.next_ids.try_chronicle()?;
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points = legitimacy
        .checked_sub(OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST)
        .expect("validated office directive legitimacy cost must fit");
    apply_office_power_directive_effect(state, institution_id, district_id, power);
    state
        .institutions
        .get_mut(&institution_id)
        .expect("validated directive institution must exist")
        .active_directive = Some(OfficeDirectiveState {
        power,
        expires_day: directive_expires_day,
    });
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::OfficeDirective,
        subject: subject.into(),
        detail: format!(
            "district={district_id};power={power:?};legitimacy_cost={OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST}"
        ).into(),
    });
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::OfficeDirective,
        summary: format!(
            "The player dynasty directed institution {institution_id} to exercise {power:?} in district {district_id}."
        ),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("{power:?} directive issued through institution {institution_id}"),
        format!(
            "The dynasty spent {OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST} legitimacy to intensify {power:?} policy in district {district_id}."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Exercised {power:?} through institution {institution_id}."),
    })
}

pub(crate) fn validate_office_nomination_standing(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<(), CommandError> {
    let required =
        office_nomination_delivery_requirement(registry, state, institution_id, character_id);
    ensure_standing(
        state,
        OFFICE_NOMINATION_REPUTATION_REQUIREMENT,
        required,
        |quality, reliability, required| CommandError::InsufficientOfficeReputation {
            quality,
            reliability,
            required,
        },
        |delivered, required| CommandError::InsufficientOfficeCommercialRecord {
            delivered,
            required,
        },
    )
}

pub(crate) fn office_nomination_delivery_requirement(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> u32 {
    capability_adjusted_delivery_requirement(
        registry,
        state,
        institution_id,
        character_id,
        &StandingGateTuning {
            context: "office nomination",
            base_requirement: OFFICE_NOMINATION_DELIVERY_REQUIREMENT,
            capability_target_score: OFFICE_NOMINATION_CAPABILITY_TARGET_SCORE,
            capability_delivery_step: OFFICE_NOMINATION_CAPABILITY_DELIVERY_STEP,
            max_preparation_deliveries: OFFICE_NOMINATION_MAX_PREPARATION_DELIVERIES,
        },
    )
}

pub(crate) fn office_nomination_subject(
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> String {
    format!("institution:{institution_id}:character:{character_id}")
}

pub(crate) fn office_nomination_next_day(
    state: &AppState,
    character_id: CharacterId,
) -> Option<i64> {
    // Every office campaign imposes its full recovery period from the
    // campaign day. Because the resolution window is shorter than the
    // ordinary interval, any retry at that interval would already land after
    // resolution, so a single stable schedule keeps the quoted retry day
    // honest and the accept/reject decisions unchanged.
    let campaign = latest_character_campaign_day_in_cooldown(
        state,
        AuditKind::OfficeNomination,
        OFFICE_NOMINATION_RECOVERY_DAYS,
        character_id,
    )
    .map(|day| future_day_or_terminal(day, OFFICE_NOMINATION_RECOVERY_DAYS));
    let dynasty_office_resignation = latest_player_office_resignation_day(state)
        .map(|day| future_day_or_terminal(day, INSTITUTION_WITHDRAWAL_RECOVERY_DAYS));
    campaign.into_iter().chain(dynasty_office_resignation).max()
}

pub(crate) fn institution_support_subject(
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> String {
    format!("institution:{institution_id}:character:{character_id}")
}

pub(crate) fn institution_support_next_day(
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Option<i64> {
    let patronage = latest_character_campaign_day_in_cooldown(
        state,
        AuditKind::InstitutionPatronage,
        INSTITUTION_SUPPORT_INTERVAL_DAYS,
        character_id,
    )
    .map(|day| future_day_or_terminal(day, INSTITUTION_SUPPORT_INTERVAL_DAYS));
    // Withdrawing from one institution costs standing with that house, not
    // with the whole city: the recovery cooldown is scoped to the institution
    // the character walked out of.
    let withdrawal = latest_character_campaign_day_in_cooldown(
        state,
        AuditKind::InstitutionWithdrawal,
        INSTITUTION_WITHDRAWAL_RECOVERY_DAYS,
        character_id,
    )
    .filter(|_| latest_withdrawal_institution(state, character_id) == Some(institution_id))
    .map(|day| future_day_or_terminal(day, INSTITUTION_WITHDRAWAL_RECOVERY_DAYS));
    // Resigning a held office, by contrast, is a city-visible act: it pauses
    // support cultivation everywhere until the same recovery period passes.
    let dynasty_office_resignation = latest_player_office_resignation_day(state)
        .map(|day| future_day_or_terminal(day, INSTITUTION_WITHDRAWAL_RECOVERY_DAYS));
    patronage
        .into_iter()
        .chain(withdrawal)
        .chain(dynasty_office_resignation)
        .max()
}

/// The institution of the character's most recent withdrawal still inside the
/// withdrawal recovery window.
///
/// Older withdrawals cannot restrict anything (their recovery has elapsed),
/// so the scan stops at the cooldown boundary; see
/// [`latest_cooldown_audit_day`] for the exactness argument.
pub(crate) fn latest_withdrawal_institution(
    state: &AppState,
    character_id: CharacterId,
) -> Option<InstitutionId> {
    let earliest_day = state
        .clock
        .day()
        .saturating_sub(INSTITUTION_WITHDRAWAL_RECOVERY_DAYS - 1);
    state
        .audit_log
        .iter()
        .rev()
        .take_while(|record| record.day() >= earliest_day)
        .find(|record| {
            record.kind() == AuditKind::InstitutionWithdrawal
                && record
                    .audit_subject()
                    .institution_character_ids()
                    .is_some_and(|(_, subject_character)| subject_character == character_id)
        })
        .and_then(|record| record.audit_subject().institution_character_ids())
        .map(|(institution_id, _)| institution_id)
}

pub(crate) fn latest_player_office_resignation_day(state: &AppState) -> Option<i64> {
    // Resignations only matter through their recovery cooldown, so the scan
    // is bounded by that window (audit days never decrease).
    let earliest_day = state
        .clock
        .day()
        .saturating_sub(INSTITUTION_WITHDRAWAL_RECOVERY_DAYS - 1);
    state
        .audit_log
        .iter()
        .rev()
        .take_while(|record| record.day() >= earliest_day)
        .find(|record| {
            record.kind() == AuditKind::InstitutionWithdrawal
                && record.detail() == crate::systems::OFFICE_RESIGNATION_AUDIT_DETAIL
                && record
                    .audit_subject()
                    .institution_character_ids()
                    .and_then(|(_, character_id)| state.characters.get(character_id))
                    .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
        })
        .map(AuditRecord::day)
}

pub(crate) fn institution_membership_count(state: &AppState, character_id: CharacterId) -> usize {
    state
        .institutions
        .values()
        .filter(|institution| institution.members.contains(&character_id))
        .count()
}

pub(crate) fn institution_support_day(
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Option<i64> {
    let subject = institution_support_subject(institution_id, character_id);
    latest_audit_day_for_subject(state, AuditKind::InstitutionPatronage, &subject)
}
