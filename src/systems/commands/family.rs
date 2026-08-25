//! House governance, councils, heirs, wards, and education commands.

#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn apply_governance(
    state: &mut AppState,
    governance: HouseGovernance,
) -> Result<CommandOutcome, CommandError> {
    let dynasty_id = state.player_dynasty_id;
    let council = state
        .family_councils
        .get(&dynasty_id)
        .ok_or(CommandError::MissingFamilyCouncil { dynasty_id })?;
    if council.governance == governance {
        return Err(CommandError::UnchangedHouseGovernance { governance });
    }
    let subject = format!("dynasty:{dynasty_id}");
    if let Some(last_change_day) = latest_cooldown_audit_day(
        state,
        AuditKind::HouseGovernanceChange,
        HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS,
        |record_subject| record_subject == subject,
    ) {
        let next_change_day =
            checked_future_day(last_change_day, HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS)?;
        if state.clock.day() < next_change_day {
            return Err(CommandError::HouseGovernanceCooldown { next_change_day });
        }
    }
    // Family unity is a real cost of forcing a governance change: a house
    // too divided to pay it cannot amend its own charter.
    if council.unity_basis_points < HOUSE_GOVERNANCE_UNITY_COST {
        return Err(CommandError::InsufficientFamilyUnity {
            available: council.unity_basis_points,
            required: HOUSE_GOVERNANCE_UNITY_COST,
        });
    }
    let next_charter_version = next_family_charter_version(dynasty_id, council.charter_version)?;
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.governance = governance;
    council.charter_version = next_charter_version;
    council.unity_basis_points = council
        .unity_basis_points
        .checked_sub(HOUSE_GOVERNANCE_UNITY_COST)
        .expect("validated family unity must cover the governance cost");
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::HouseGovernanceChange,
        subject: subject.into(),
        detail: format!("governance={governance:?}").into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        "House charter amended".to_owned(),
        format!(
            "The dynasty adopted {governance:?} governance, changing administrative coordination, family cohesion, and succession risk."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Changed house governance to {governance:?}."),
    })
}

pub(crate) fn apply_family_council_meeting(
    state: &mut AppState,
) -> Result<CommandOutcome, CommandError> {
    let dynasty_id = state.player_dynasty_id;
    let subject = format!("dynasty:{dynasty_id};council-meeting");
    let council = state
        .family_councils
        .get(&dynasty_id)
        .ok_or(CommandError::MissingFamilyCouncil { dynasty_id })?;
    if let Some(last_meeting_day) = latest_cooldown_audit_day(
        state,
        AuditKind::FamilyCouncilMeeting,
        FAMILY_COUNCIL_MEETING_INTERVAL_DAYS,
        |record_subject| record_subject == subject,
    ) {
        let next_meeting_day =
            checked_future_day(last_meeting_day, FAMILY_COUNCIL_MEETING_INTERVAL_DAYS)?;
        if state.clock.day() < next_meeting_day {
            return Err(CommandError::FamilyCouncilMeetingCooldown { next_meeting_day });
        }
    }
    let member_ids: Vec<_> = council.members.iter().copied().collect();
    let unity_before = council.unity_basis_points;
    spend_player_treasury_to_market(state, FAMILY_COUNCIL_MEETING_COST)?;
    for character_id in member_ids {
        if let Some(character) = state.characters.get_mut(character_id)
            && character.status() == CharacterStatus::Active
        {
            character.runtime.loyalty_basis_points = character
                .runtime
                .loyalty_basis_points
                .saturating_add(FAMILY_COUNCIL_MEETING_LOYALTY_GAIN)
                .min(10_000);
        }
    }
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    // A council already at full unity cannot gain more; report what actually
    // happened instead of claiming a rise that saturation absorbed.
    let unity_gain = FAMILY_COUNCIL_MEETING_UNITY_GAIN.min(10_000_u16.saturating_sub(unity_before));
    council.unity_basis_points = council
        .unity_basis_points
        .saturating_add(unity_gain)
        .min(10_000);
    let unity_after = council.unity_basis_points;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::FamilyCouncilMeeting,
        subject: subject.into(),
        detail: format!(
            "cost={};unity_before={unity_before};unity_after={unity_after};loyalty_gain={FAMILY_COUNCIL_MEETING_LOYALTY_GAIN}",
            FAMILY_COUNCIL_MEETING_COST.copper()
        ).into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        "Family council convened".to_owned(),
        if unity_gain > 0 {
            format!(
                "The dynasty spent {FAMILY_COUNCIL_MEETING_COST} on settlements, hospitality, and internal obligations. Family unity rose from {unity_before} to {unity_after} bp and active council members gained loyalty."
            )
        } else {
            format!(
                "The dynasty spent {FAMILY_COUNCIL_MEETING_COST} on settlements, hospitality, and internal obligations. Family unity held at {unity_before} bp and active council members gained loyalty."
            )
        },
    )?;
    Ok(CommandOutcome {
        summary: format!("Convened the family council; unity is now {unity_after} bp."),
    })
}

#[derive(Debug)]
pub(crate) struct HeirDesignationPlan {
    dynasty_id: DynastyId,
    prior_heir_id: Option<CharacterId>,
    legitimacy: u16,
    next_charter_version: u64,
    confirmation: bool,
    subject: String,
}

pub(crate) fn validate_heir_designation(
    state: &AppState,
    character_id: CharacterId,
) -> Result<HeirDesignationPlan, CommandError> {
    let dynasty_id = state.player_dynasty_id;
    let (head_id, prior_heir_id, legitimacy) = {
        let dynasty = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist");
        (
            dynasty.head_id(),
            dynasty.heir_id(),
            dynasty.resources.legitimacy_basis_points,
        )
    };
    let subject = format!("dynasty:{dynasty_id}");
    let latest_designation =
        state.audit_log.iter().rev().find(|record| {
            record.kind() == AuditKind::HeirDesignation && record.subject() == subject
        });
    let confirmation = prior_heir_id == Some(character_id);
    // Confirming the sitting heir is a no-op only when the most recent
    // designation already names them. A later designation of a different
    // heir must not lock the family out of re-preparing this one.
    if confirmation
        && latest_designation
            .is_some_and(|record| crate::systems::heir_audit_detail_matches(record, character_id))
    {
        return Err(CommandError::UnchangedHeir { character_id });
    }
    let candidate = state
        .characters
        .get(character_id)
        .ok_or(CommandError::InvalidHeirCandidate { character_id })?;
    let candidate_age = state.clock.day().saturating_sub(candidate.birth_day());
    let council = state
        .family_councils
        .get(&dynasty_id)
        .ok_or(CommandError::MissingFamilyCouncil { dynasty_id })?;
    if candidate.dynasty_id() != dynasty_id
        || candidate.status() != CharacterStatus::Active
        || character_id == head_id
        || candidate_age < HEIR_MINIMUM_AGE_DAYS
        || !council.members.contains(&character_id)
    {
        return Err(CommandError::InvalidHeirCandidate { character_id });
    }
    if legitimacy < HEIR_DESIGNATION_LEGITIMACY_COST {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: HEIR_DESIGNATION_LEGITIMACY_COST,
        });
    }
    // Redrawing the succession wounds family cohesion; a divided house cannot
    // pay the unity price of a new charter line.
    if council.unity_basis_points < HEIR_DESIGNATION_UNITY_COST {
        return Err(CommandError::InsufficientFamilyUnity {
            available: council.unity_basis_points,
            required: HEIR_DESIGNATION_UNITY_COST,
        });
    }
    if let Some(last_designation_day) = latest_designation.map(AuditRecord::day) {
        let next_designation_day =
            checked_future_day(last_designation_day, HEIR_DESIGNATION_INTERVAL_DAYS)?;
        if state.clock.day() < next_designation_day {
            return Err(CommandError::HeirDesignationCooldown {
                next_designation_day,
            });
        }
    }
    let next_charter_version = next_family_charter_version(dynasty_id, council.charter_version)?;

    Ok(HeirDesignationPlan {
        dynasty_id,
        prior_heir_id,
        legitimacy,
        next_charter_version,
        confirmation,
        subject,
    })
}

pub(crate) fn apply_heir(
    state: &mut AppState,
    character_id: CharacterId,
) -> Result<CommandOutcome, CommandError> {
    let HeirDesignationPlan {
        dynasty_id,
        prior_heir_id,
        legitimacy,
        next_charter_version,
        confirmation,
        subject,
    } = validate_heir_designation(state, character_id)?;

    if !confirmation {
        if let Some(prior_heir_id) = prior_heir_id {
            let prior_heir = state
                .characters
                .get_mut(prior_heir_id)
                .expect("designated heir must exist");
            if prior_heir.status() == CharacterStatus::Active
                && prior_heir.role() == CharacterRole::Heir
            {
                prior_heir.runtime.role = CharacterRole::Clerk;
            }
        }
        state
            .characters
            .get_mut(character_id)
            .expect("validated heir candidate must exist")
            .runtime
            .role = CharacterRole::Heir;
    }
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("player dynasty must exist");
    dynasty.relationships.heir_id = Some(character_id);
    dynasty.resources.legitimacy_basis_points = legitimacy
        .checked_sub(HEIR_DESIGNATION_LEGITIMACY_COST)
        .expect("validated heir designation legitimacy cost must fit");
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.unity_basis_points = council
        .unity_basis_points
        .checked_sub(HEIR_DESIGNATION_UNITY_COST)
        .expect("validated family unity must cover the heir designation cost");
    council.charter_version = next_charter_version;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::HeirDesignation,
        subject: subject.into(),
        detail: format!(
            "prior_heir={};{};confirmation={confirmation};legitimacy_cost={HEIR_DESIGNATION_LEGITIMACY_COST};unity_cost={HEIR_DESIGNATION_UNITY_COST}",
            prior_heir_id.map_or_else(|| "none".to_owned(), |id| id.to_string()),
            crate::systems::heir_designation_detail_component(character_id)
        ).into(),
    });
    let chronicle_id = state.next_ids.try_chronicle()?;
    let chronicle_summary = if confirmation {
        format!("Dynasty {dynasty_id} formally confirmed character {character_id} as heir.")
    } else {
        format!(
            "Dynasty {dynasty_id} designated character {character_id} as heir, replacing {}.",
            prior_heir_id.map_or_else(
                || "no prior heir".to_owned(),
                |id| format!("character {id}")
            )
        )
    };
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::SuccessionPrepared,
        summary: chronicle_summary,
    });
    let outcome_summary = if confirmation {
        format!("Formally confirmed character {character_id} as heir.")
    } else {
        format!("Designated character {character_id} as heir.")
    };
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        if confirmation {
            format!("Character {character_id} confirmed as heir")
        } else {
            format!("Character {character_id} designated as heir")
        },
        format!(
            "The family charter now names character {character_id} as successor. The change cost \
             {HEIR_DESIGNATION_LEGITIMACY_COST} legitimacy and {HEIR_DESIGNATION_UNITY_COST} \
             family unity, but a formally prepared heir will face a far gentler transition when \
             the head of house dies."
        ),
    )?;
    Ok(CommandOutcome {
        summary: outcome_summary,
    })
}

pub(crate) fn apply_adopt_ward(
    state: &mut AppState,
    focus: EducationFocus,
) -> Result<CommandOutcome, CommandError> {
    let context = validate_ward_adoption(state)?;
    let WardAdoptionContext {
        dynasty_id,
        head_id,
        dynasty_name,
    } = context;
    // Resolve every fallible allocation before the first mutation so a
    // rejected adoption never leaves a debited treasury or orphan record.
    let ward_id = state.next_ids.try_character()?;
    let family_link_id: crate::ids::FamilyLinkId = state.next_ids.try_family_link()?;
    let chronicle_id: crate::ids::ChronicleEntryId = state.next_ids.try_chronicle()?;
    spend_player_treasury_to_market(state, WARD_ADOPTION_COST)?;
    let ward_name = format!("{dynasty_name} Ward {ward_id}");
    insert_ward_character(state, dynasty_id, ward_id, ward_name.clone(), focus);
    insert_ward_family_link(state, family_link_id, head_id, ward_id);
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.members.insert(ward_id);
    council.unity_basis_points = council
        .unity_basis_points
        .checked_sub(WARD_ADOPTION_UNITY_COST)
        .expect("validated family unity must cover the ward adoption cost");
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_sub(WARD_ADOPTION_LEGITIMACY_COST);
    dynasty.resources.administrative_capacity =
        dynasty.resources.administrative_capacity.saturating_add(8);
    record_ward_adoption(state, dynasty_id, ward_id, &ward_name, focus, chronicle_id)?;
    Ok(CommandOutcome {
        summary: format!("Adopted ward {ward_id} with {focus:?} training."),
    })
}

#[derive(Debug)]
pub(crate) struct WardAdoptionContext {
    dynasty_id: DynastyId,
    head_id: CharacterId,
    dynasty_name: String,
}

pub(crate) fn validate_ward_adoption(
    state: &AppState,
) -> Result<WardAdoptionContext, CommandError> {
    let dynasty_id = state.player_dynasty_id;
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .expect("player dynasty must exist");
    let legitimacy = dynasty.resources.legitimacy_basis_points;
    if !state.family_councils.contains_key(&dynasty_id) {
        return Err(CommandError::MissingFamilyCouncil { dynasty_id });
    }
    let active = active_player_ward_count(state);
    if active >= MAX_ACTIVE_WARDS {
        return Err(CommandError::WardCapacity {
            active,
            maximum: MAX_ACTIVE_WARDS,
        });
    }
    ensure_standing(
        state,
        WARD_ADOPTION_REPUTATION_REQUIREMENT,
        WARD_ADOPTION_DELIVERY_REQUIREMENT,
        |quality, reliability, required| CommandError::InsufficientWardReputation {
            quality,
            reliability,
            required,
        },
        |delivered, required| CommandError::InsufficientWardCommercialRecord {
            delivered,
            required,
        },
    )?;
    if legitimacy < WARD_ADOPTION_LEGITIMACY_REQUIREMENT {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: WARD_ADOPTION_LEGITIMACY_REQUIREMENT,
        });
    }
    // Bringing a ward into the house strains family cohesion; a divided
    // council cannot absorb the adoption.
    let council = state
        .family_councils
        .get(&dynasty_id)
        .expect("validated family council must exist");
    if council.unity_basis_points < WARD_ADOPTION_UNITY_COST {
        return Err(CommandError::InsufficientFamilyUnity {
            available: council.unity_basis_points,
            required: WARD_ADOPTION_UNITY_COST,
        });
    }
    let adoption_subject_prefix = format!("dynasty:{dynasty_id}:");
    if let Some(last_adoption_day) = latest_cooldown_audit_day(
        state,
        AuditKind::WardAdoption,
        WARD_ADOPTION_INTERVAL_DAYS,
        |record_subject| record_subject.starts_with(&adoption_subject_prefix),
    ) {
        let next_adoption_day = checked_future_day(last_adoption_day, WARD_ADOPTION_INTERVAL_DAYS)?;
        if state.clock.day() < next_adoption_day {
            return Err(CommandError::WardAdoptionCooldown { next_adoption_day });
        }
    }
    Ok(WardAdoptionContext {
        dynasty_id,
        head_id: dynasty.head_id(),
        dynasty_name: dynasty.name().to_owned(),
    })
}

pub(crate) fn insert_ward_character(
    state: &mut AppState,
    dynasty_id: DynastyId,
    ward_id: CharacterId,
    ward_name: String,
    focus: EducationFocus,
) {
    state.characters.insert(Character {
        identity: CharacterIdentity {
            id: ward_id,
            dynasty_id,
            name: ward_name,
            birth_day: state.clock.day().saturating_sub(18 * 360),
        },
        capabilities: ward_capabilities(focus),
        runtime: CharacterRuntime {
            status: CharacterStatus::Active,
            health_basis_points: 9_500,
            loyalty_basis_points: 8_500,
            role: CharacterRole::Clerk,
            incapacitated_day: None,
        },
    });
}

pub(crate) fn insert_ward_family_link(
    state: &mut AppState,
    family_link_id: crate::ids::FamilyLinkId,
    head_id: CharacterId,
    ward_id: CharacterId,
) {
    state.family_links.insert(
        family_link_id,
        FamilyLink {
            id: family_link_id,
            first_character_id: head_id,
            second_character_id: ward_id,
            kind: FamilyLinkKind::Ward,
            active: true,
        },
    );
}

pub(crate) fn record_ward_adoption(
    state: &mut AppState,
    dynasty_id: DynastyId,
    ward_id: CharacterId,
    ward_name: &str,
    focus: EducationFocus,
    chronicle_id: crate::ids::ChronicleEntryId,
) -> Result<(), CommandError> {
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::WardAdoption,
        subject: format!("dynasty:{dynasty_id}:character:{ward_id}").into(),
        detail: format!("focus={focus:?};cost={}", WARD_ADOPTION_COST.copper()).into(),
    });
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::FamilyExpanded,
        summary: format!("{ward_name} entered the dynasty as a ward focused on {focus:?}."),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        format!("Ward adopted: {ward_name}"),
        format!(
            "The dynasty spent {WARD_ADOPTION_COST} to adopt and train a new {focus:?}-focused household member."
        ),
    )?;
    Ok(())
}

/// Canonical active ward count for the player dynasty: a ward occupies a slot
/// only while both its guardian and the ward are active.
pub(crate) fn active_player_ward_count(state: &AppState) -> usize {
    state
        .family_links
        .values()
        .filter(|link| link.active && link.kind == FamilyLinkKind::Ward)
        .filter(|link| {
            let guardian_active = state
                .characters
                .get(link.first_character_id)
                .is_some_and(|character| character.status() == CharacterStatus::Active);
            let ward_active =
                state
                    .characters
                    .get(link.second_character_id)
                    .is_some_and(|character| {
                        character.dynasty_id() == state.player_dynasty_id
                            && character.status() == CharacterStatus::Active
                    });
            guardian_active && ward_active
        })
        .count()
}

pub(crate) const fn ward_capabilities(focus: EducationFocus) -> CharacterCapabilities {
    match focus {
        EducationFocus::Administration => CharacterCapabilities {
            administration: 62,
            commerce: 42,
            social: 45,
            craft: 35,
        },
        EducationFocus::Commerce => CharacterCapabilities {
            administration: 45,
            commerce: 62,
            social: 42,
            craft: 35,
        },
        EducationFocus::Social => CharacterCapabilities {
            administration: 45,
            commerce: 42,
            social: 62,
            craft: 35,
        },
        EducationFocus::Craft => CharacterCapabilities {
            administration: 40,
            commerce: 42,
            social: 40,
            craft: 62,
        },
    }
}

pub(crate) fn apply_family_education(
    state: &mut AppState,
    character_id: CharacterId,
    focus: EducationFocus,
) -> Result<CommandOutcome, CommandError> {
    let character = state
        .characters
        .get(character_id)
        .ok_or(CommandError::InvalidFamilyStudent { character_id })?;
    if character.dynasty_id() != state.player_dynasty_id
        || character.status() != CharacterStatus::Active
    {
        return Err(CommandError::InvalidFamilyStudent { character_id });
    }
    if education_focus_value(&character.capabilities, focus) >= 100 {
        return Err(CommandError::FamilyEducationAtMaximum {
            character_id,
            focus,
        });
    }
    if let Some(next_education_day) = family_education_next_day(state, character_id)
        && state.clock.day() < next_education_day
    {
        return Err(CommandError::FamilyEducationCooldown { next_education_day });
    }
    spend_player_treasury_to_market(state, FAMILY_EDUCATION_COST)?;
    let character = state
        .characters
        .get_mut(character_id)
        .expect("validated family student must exist");
    apply_education_focus(&mut character.capabilities, focus);
    if focus == EducationFocus::Administration {
        let dynasty = state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        dynasty.resources.administrative_capacity =
            dynasty.resources.administrative_capacity.saturating_add(2);
    }
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::FamilyEducation,
        subject: family_education_subject(state.player_dynasty_id, character_id).into(),
        detail: format!("focus={focus:?};cost={}", FAMILY_EDUCATION_COST.copper()).into(),
    });
    crate::systems::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        format!("Family education completed for character {character_id}"),
        format!("The dynasty spent {FAMILY_EDUCATION_COST} on advanced {focus:?} training."),
    )?;
    Ok(CommandOutcome {
        summary: format!("Educated character {character_id} in {focus:?}."),
    })
}

pub(crate) fn family_education_subject(dynasty_id: DynastyId, character_id: CharacterId) -> String {
    format!("dynasty:{dynasty_id}:character:{character_id}")
}

pub(crate) fn family_education_next_day(
    state: &AppState,
    character_id: CharacterId,
) -> Option<i64> {
    let dynasty_prefix = format!("dynasty:{}:", state.player_dynasty_id);
    let dynasty_next = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::FamilyEducation
                && record.subject().starts_with(&dynasty_prefix)
        })
        .map(|record| future_day_or_terminal(record.day(), FAMILY_EDUCATION_DYNASTY_INTERVAL_DAYS));
    let subject = family_education_subject(state.player_dynasty_id, character_id);
    let character_next = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::FamilyEducation && record.subject() == subject)
        .map(|record| future_day_or_terminal(record.day(), FAMILY_EDUCATION_INTERVAL_DAYS));
    dynasty_next.into_iter().chain(character_next).max()
}

pub(crate) const fn education_focus_value(
    capabilities: &CharacterCapabilities,
    focus: EducationFocus,
) -> u16 {
    match focus {
        EducationFocus::Administration => capabilities.administration,
        EducationFocus::Commerce => capabilities.commerce,
        EducationFocus::Social => capabilities.social,
        EducationFocus::Craft => capabilities.craft,
    }
}

pub(crate) fn apply_education_focus(
    capabilities: &mut CharacterCapabilities,
    focus: EducationFocus,
) {
    match focus {
        EducationFocus::Administration => {
            capabilities.administration = capabilities.administration.saturating_add(8).min(100);
            capabilities.social = capabilities.social.saturating_add(2).min(100);
        }
        EducationFocus::Commerce => {
            capabilities.commerce = capabilities.commerce.saturating_add(8).min(100);
            capabilities.administration = capabilities.administration.saturating_add(2).min(100);
        }
        EducationFocus::Social => {
            capabilities.social = capabilities.social.saturating_add(8).min(100);
            capabilities.commerce = capabilities.commerce.saturating_add(2).min(100);
        }
        EducationFocus::Craft => {
            capabilities.craft = capabilities.craft.saturating_add(8).min(100);
            capabilities.commerce = capabilities.commerce.saturating_add(2).min(100);
        }
    }
}
