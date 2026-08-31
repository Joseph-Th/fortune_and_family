//! Gameplay harness orchestration — the evaluation runtime.
//!
//! Purpose: run the deterministic `GameplayHarnessConfig` ×
//! `Registry` → `GameplayHarnessReport` loop (campaign construction,
//! jittered decision cycles, counterfactual branches, attribution).
//! Owns: `run_gameplay_harness`, campaign parallelism (immutable registry),
//! `CampaignAccumulator`, jittered-interval derivation, and horizon
//! calculation (`max_consequence_horizon_days`).
//! Reads: `Registry`, `AppState` via `build_new_game` / `advance_days`.
//! Mutates: per-campaign `AppState` clones; harness owns its accumulator
//! but not the authoritative dynasty stores.
//! Does not own: candidate generation (candidates) or findings (findings).
//! Invariants: every campaign owns its `AppState`; registry immutable,
//! ordering stable, random-state owned; daily rotation is adapter policy.
//! Focused tests: `src/gameplay_tests.rs` harness integration.

#[allow(clippy::wildcard_imports)] // the module tree re-exports one flat namespace
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub kind: GameplayCommandKind,
    pub command: PlayerCommand,
    pub description: String,
    pub score: i64,
}

pub(crate) fn classify_player_command(
    state: &AppState,
    command: &PlayerCommand,
) -> Option<GameplayCommandKind> {
    let player_id = state.player_dynasty_id;
    match command {
        PlayerCommand::TransferBusinessCash { .. } => {
            Some(GameplayCommandKind::TransferBusinessCash)
        }
        PlayerCommand::WithdrawBusinessCash { .. } => {
            Some(GameplayCommandKind::WithdrawBusinessCash)
        }
        PlayerCommand::AcquireBusiness { .. } => Some(GameplayCommandKind::AcquireBusiness),
        PlayerCommand::InvestInBusiness { .. } => Some(GameplayCommandKind::InvestInBusiness),
        PlayerCommand::SetBusinessPolicy { .. } => Some(GameplayCommandKind::SetBusinessPolicy),
        PlayerCommand::SetBusinessWages { .. } => Some(GameplayCommandKind::SetBusinessWages),
        PlayerCommand::CreateSupplyContract { terms } => {
            let buyer_is_player = state
                .businesses
                .get(terms.buyer_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == player_id);
            let seller_is_player = state
                .businesses
                .get(terms.seller_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == player_id);
            match (buyer_is_player, seller_is_player) {
                (true, false) => Some(GameplayCommandKind::SecureSupply),
                (false, true) => Some(GameplayCommandKind::SellOutput),
                (false, false) | (true, true) => None,
            }
        }
        PlayerCommand::IssueLoan { terms } => {
            match (
                terms.borrower_dynasty_id == player_id,
                terms.lender_dynasty_id == player_id,
            ) {
                (true, false) => Some(GameplayCommandKind::BorrowFunds),
                (false, true) => Some(GameplayCommandKind::ExtendCredit),
                (false, false) | (true, true) => None,
            }
        }
        PlayerCommand::BuyProperty { .. } => Some(GameplayCommandKind::BuyProperty),
        PlayerCommand::SellProperty { .. } => Some(GameplayCommandKind::SellProperty),
        PlayerCommand::EnactLaw { .. } => Some(GameplayCommandKind::EnactLaw),
        PlayerCommand::StartPublicWork { .. } => Some(GameplayCommandKind::StartPublicWork),
        PlayerCommand::FundPublicWork { .. } => Some(GameplayCommandKind::FundPublicWork),
        PlayerCommand::FileLegalCase { .. } => Some(GameplayCommandKind::FileLegalCase),
        PlayerCommand::SettleLegalCase { .. } => Some(GameplayCommandKind::SettleLegalCase),
        PlayerCommand::SetHouseGovernance { .. } => Some(GameplayCommandKind::SetHouseGovernance),
        PlayerCommand::ConveneFamilyCouncil => Some(GameplayCommandKind::ConveneFamilyCouncil),
        PlayerCommand::DesignateHeir { .. } => Some(GameplayCommandKind::DesignateHeir),
        PlayerCommand::AdoptWard { .. } => Some(GameplayCommandKind::AdoptWard),
        PlayerCommand::EducateFamilyMember { .. } => Some(GameplayCommandKind::EducateFamilyMember),
        PlayerCommand::CultivateInstitutionSupport { .. } => {
            Some(GameplayCommandKind::CultivateInstitutionSupport)
        }
        PlayerCommand::EndowInstitution { .. } => Some(GameplayCommandKind::EndowInstitution),
        PlayerCommand::NominateForOffice { .. } => Some(GameplayCommandKind::NominateForOffice),
        PlayerCommand::ExerciseOfficePower { .. } => Some(GameplayCommandKind::ExerciseOfficePower),
        PlayerCommand::WithdrawFromInstitution { .. } => {
            Some(GameplayCommandKind::WithdrawFromInstitution)
        }
        PlayerCommand::RespondToCrisis { .. } => Some(GameplayCommandKind::RespondToCrisis),
        PlayerCommand::ResolveLaborDispute { .. } => Some(GameplayCommandKind::ResolveLaborDispute),
        PlayerCommand::CommissionInformation { .. } => {
            Some(GameplayCommandKind::CommissionInformation)
        }
        PlayerCommand::LeverageInformation { .. } => Some(GameplayCommandKind::LeverageInformation),
        PlayerCommand::AcknowledgeNotification { .. } => {
            Some(GameplayCommandKind::AcknowledgeNotification)
        }
    }
}

pub(crate) fn validate_candidate_classifications(
    state: &AppState,
    candidates: &[Candidate],
) -> Result<(), GameplayHarnessError> {
    for candidate in candidates {
        let Some(actual) = classify_player_command(state, &candidate.command) else {
            return Err(GameplayHarnessError::UnclassifiedCandidate {
                description: candidate.description.clone(),
            });
        };
        if actual != candidate.kind {
            return Err(GameplayHarnessError::CandidateKindMismatch {
                description: candidate.description.clone(),
                declared: candidate.kind,
                actual,
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct ProbeResult {
    pub selected: Option<Candidate>,
    pub viable_count: usize,
    pub substantive_viable_count: usize,
    pub viable_command_kinds: BTreeSet<GameplayCommandKind>,
    pub viable_options: Vec<GameplayViableOption>,
    pub close_choice_score_gap: Option<i64>,
    pub distinct_immediate_choice_profiles: usize,
    pub distinct_projected_choice_profiles: usize,
    pub family_close_choice_score_gap: Option<i64>,
    pub distinct_immediate_family_profiles: usize,
    pub distinct_projected_family_profiles: usize,
    pub rejections: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ChoiceCycleMetrics {
    pub substantive_candidate_count: usize,
    pub substantive_viable_count: usize,
    pub viable_command_kind_count: usize,
    pub family_quality: AlternativeQuality,
    pub option_quality: AlternativeQuality,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AlternativeQuality(u8);

impl AlternativeQuality {
    const MULTIPLE: u8 = 1;
    const CLOSE: u8 = 1 << 1;
    const DISTINCT_IMMEDIATE: u8 = 1 << 2;
    const DISTINCT_PROJECTED: u8 = 1 << 3;

    pub fn from_observations(
        alternative_count: usize,
        score_gap: Option<i64>,
        immediate_profile_count: usize,
        projected_profile_count: usize,
    ) -> Self {
        let mut flags = 0_u8;
        if alternative_count >= 2 {
            flags |= Self::MULTIPLE;
        }
        if score_gap.is_some_and(|gap| gap <= CLOSE_CHOICE_SCORE_GAP) {
            flags |= Self::CLOSE;
        }
        if immediate_profile_count >= 2 {
            flags |= Self::DISTINCT_IMMEDIATE;
        }
        if projected_profile_count >= 2 {
            flags |= Self::DISTINCT_PROJECTED;
        }
        Self(flags)
    }

    const fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PhaseCycleObservation {
    pub action: Option<GameplayCommandKind>,
    pub choices: ChoiceCycleMetrics,
    pub ambient_change: bool,
    pub quiet_cause: Option<QuietCause>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum QuietCause {
    GeneratorGap,
    PolicyGate,
    Restrained,
    ValidationGate,
    BudgetGate,
    Dormant,
}

#[derive(Debug)]
pub(crate) struct CampaignAccumulator {
    pub commands: BTreeMap<GameplayCommandKind, GameplayCommandStats>,
    pub phase_stats: BTreeMap<GameplayPhase, GameplayPhaseStats>,
    pub current_phase_quiet_streaks: BTreeMap<GameplayPhase, u32>,
    pub rejection_reasons: BTreeMap<String, u32>,
    pub domain_changes: BTreeMap<GameplayDomain, u32>,
    pub causal_domain_changes: BTreeMap<GameplayDomain, u32>,
    pub ambient_domain_changes: BTreeMap<GameplayDomain, u32>,
    pub interactions: BTreeMap<(GameplayCommandKind, GameplayDomain), u32>,
    pub trace: Vec<GameplayTraceStep>,
    pub decision_cycles: u32,
    pub cycles_with_viable_choices: u32,
    pub cycles_with_multiple_viable_command_kinds: u32,
    pub cycles_with_close_viable_command_kinds: u32,
    pub cycles_with_distinct_immediate_consequences: u32,
    pub cycles_with_distinct_projected_consequences: u32,
    pub cycles_with_multiple_viable_options: u32,
    pub cycles_with_close_viable_options: u32,
    pub cycles_with_distinct_immediate_option_consequences: u32,
    pub cycles_with_distinct_projected_option_consequences: u32,
    pub no_action_cycles: u32,
    pub quiet_cycles: u32,
    pub quiet_cycles_with_ambient_change: u32,
    pub blocked_cycles: u32,
    pub total_viable_choices: u32,
    pub total_viable_command_kinds: u32,
    pub minimum_food_satisfaction: u16,
    pub minimum_district_food_satisfaction: u16,
    pub minimum_operating_businesses: u16,
    pub peak_player_treasury: Money,
    pub minimum_unowned_property_value: Option<Money>,
    pub maximum_disputed_employment: u16,
    pub maximum_player_disputed_employment: u16,
    pub maximum_delinquent_loans: u16,
    pub maximum_defaulted_loans: u16,
    pub maximum_player_delinquent_lending: u16,
    pub maximum_player_defaulted_lending: u16,
    pub maximum_player_delinquent_borrowing: u16,
    pub maximum_player_defaulted_borrowing: u16,
    pub maximum_delinquent_civic_debts: u16,
    pub maximum_defaulted_civic_debts: u16,
    pub maximum_offices_held: u16,
    pub maximum_unfinished_public_works: u16,
    pub maximum_active_crises: u16,
    pub observed_crisis_kinds: BTreeSet<CrisisKind>,
    pub maximum_unread_notifications: u16,
    pub maximum_contract_relationship_pressure_basis_points: u16,
    pub minimum_post_succession_family_unity: Option<u16>,
    pub last_command: Option<GameplayCommandKind>,
    pub last_substantive_command: Option<GameplayCommandKind>,
    pub last_substantive_command_day: Option<i64>,
    pub current_substantive_command_streak: u16,
    pub longest_substantive_command_streak: u16,
    pub longest_substantive_streak_command: Option<GameplayCommandKind>,
    pub current_substantive_action_gap_days: u32,
    pub longest_substantive_action_gap_days: u32,
    pub current_asset_rich_quiet_gap_days: u32,
    pub longest_asset_rich_quiet_gap_days: u32,
    pub current_recovery_pressure_days: u32,
    pub longest_recovery_pressure_days: u32,
    pub commission_leverage_pairs: u16,
    pub player_debt_enforcement_cases: u16,
    pub peak_route_disruption_basis_points: u16,
    pub peak_city_distressed_businesses: u16,
    pub last_information_commission_day: Option<i64>,
    pub starting_generation: Option<u16>,
    pub fantasy_arc: GameplayFantasyArc,
    pub succession_transition: Option<GameplaySuccessionTransition>,
    pub quiet_diagnostic: GameplayQuietDiagnostic,
    pub last_observed_snapshot: Option<GameplaySnapshot>,
}

impl CampaignAccumulator {
    pub fn new() -> Self {
        Self {
            commands: initialized_command_stats(),
            phase_stats: initialized_phase_stats(),
            current_phase_quiet_streaks: initialized_phase_counts(),
            rejection_reasons: BTreeMap::new(),
            domain_changes: initialized_domain_counts(),
            causal_domain_changes: initialized_domain_counts(),
            ambient_domain_changes: initialized_domain_counts(),
            interactions: BTreeMap::new(),
            trace: Vec::new(),
            decision_cycles: 0,
            cycles_with_viable_choices: 0,
            cycles_with_multiple_viable_command_kinds: 0,
            cycles_with_close_viable_command_kinds: 0,
            cycles_with_distinct_immediate_consequences: 0,
            cycles_with_distinct_projected_consequences: 0,
            cycles_with_multiple_viable_options: 0,
            cycles_with_close_viable_options: 0,
            cycles_with_distinct_immediate_option_consequences: 0,
            cycles_with_distinct_projected_option_consequences: 0,
            no_action_cycles: 0,
            quiet_cycles: 0,
            quiet_cycles_with_ambient_change: 0,
            blocked_cycles: 0,
            total_viable_choices: 0,
            total_viable_command_kinds: 0,
            minimum_food_satisfaction: u16::MAX,
            minimum_district_food_satisfaction: u16::MAX,
            minimum_operating_businesses: u16::MAX,
            peak_player_treasury: Money::ZERO,
            minimum_unowned_property_value: None,
            maximum_disputed_employment: 0,
            maximum_player_disputed_employment: 0,
            maximum_delinquent_loans: 0,
            maximum_defaulted_loans: 0,
            maximum_player_delinquent_lending: 0,
            maximum_player_defaulted_lending: 0,
            maximum_player_delinquent_borrowing: 0,
            maximum_player_defaulted_borrowing: 0,
            maximum_delinquent_civic_debts: 0,
            maximum_defaulted_civic_debts: 0,
            maximum_offices_held: 0,
            maximum_unfinished_public_works: 0,
            maximum_active_crises: 0,
            observed_crisis_kinds: BTreeSet::new(),
            maximum_unread_notifications: 0,
            maximum_contract_relationship_pressure_basis_points: 0,
            minimum_post_succession_family_unity: None,
            last_command: None,
            last_substantive_command: None,
            last_substantive_command_day: None,
            current_substantive_command_streak: 0,
            longest_substantive_command_streak: 0,
            longest_substantive_streak_command: None,
            current_substantive_action_gap_days: 0,
            longest_substantive_action_gap_days: 0,
            current_asset_rich_quiet_gap_days: 0,
            longest_asset_rich_quiet_gap_days: 0,
            current_recovery_pressure_days: 0,
            longest_recovery_pressure_days: 0,
            commission_leverage_pairs: 0,
            player_debt_enforcement_cases: 0,
            peak_route_disruption_basis_points: 0,
            peak_city_distressed_businesses: 0,
            last_information_commission_day: None,
            starting_generation: None,
            fantasy_arc: GameplayFantasyArc::default(),
            succession_transition: None,
            quiet_diagnostic: GameplayQuietDiagnostic::default(),
            last_observed_snapshot: None,
        }
    }

    pub fn record_executed_command(&mut self, kind: GameplayCommandKind, day: i64) {
        self.last_command = Some(kind);
        if !is_substantive_command_kind(kind) {
            return;
        }
        if kind == GameplayCommandKind::NominateForOffice {
            self.fantasy_arc
                .first_office_campaign_day
                .get_or_insert(day);
        }
        if kind == GameplayCommandKind::CultivateInstitutionSupport {
            self.fantasy_arc
                .first_institution_support_day
                .get_or_insert(day);
        }
        if kind == GameplayCommandKind::DesignateHeir {
            self.fantasy_arc
                .first_heir_designation_day
                .get_or_insert(day);
        }
        // City-shaping means exercising authority or committing a dynasty-
        // sponsored civic project. Funding someone else's stalled work is
        // patronage, not governance, so it must not start the governance
        // phase by itself.
        if matches!(
            kind,
            GameplayCommandKind::EnactLaw
                | GameplayCommandKind::StartPublicWork
                | GameplayCommandKind::ExerciseOfficePower
        ) {
            self.fantasy_arc
                .first_city_shaping_action_day
                .get_or_insert(day);
            self.fantasy_arc
                .first_city_shaping_command
                .get_or_insert(kind);
        }
        if kind == GameplayCommandKind::CommissionInformation {
            self.last_information_commission_day = Some(day);
        } else if kind == GameplayCommandKind::LeverageInformation
            && self
                .last_information_commission_day
                .is_some_and(|commission_day| {
                    day.saturating_sub(commission_day) <= INFORMATION_ROUTINE_PAIR_WINDOW_DAYS
                })
        {
            self.commission_leverage_pairs = self.commission_leverage_pairs.saturating_add(1);
            self.last_information_commission_day = None;
        }
        let follows_recent_same_command = self.last_substantive_command == Some(kind)
            && self
                .last_substantive_command_day
                .is_some_and(|previous_day| {
                    day.saturating_sub(previous_day) <= SUBSTANTIVE_STREAK_MAX_GAP_DAYS
                });
        if follows_recent_same_command {
            self.current_substantive_command_streak =
                self.current_substantive_command_streak.saturating_add(1);
        } else {
            self.last_substantive_command = Some(kind);
            self.current_substantive_command_streak = 1;
        }
        self.last_substantive_command_day = Some(day);
        if self.current_substantive_command_streak > self.longest_substantive_command_streak {
            self.longest_substantive_command_streak = self.current_substantive_command_streak;
            self.longest_substantive_streak_command = Some(kind);
        }
    }

    pub fn record_executed_candidate(
        &mut self,
        kind: GameplayCommandKind,
        command: &PlayerCommand,
        day: i64,
    ) {
        self.record_executed_command(kind, day);
        match command {
            PlayerCommand::CultivateInstitutionSupport { institution_id, .. } => {
                self.fantasy_arc
                    .first_institution_support_target
                    .get_or_insert(*institution_id);
            }
            PlayerCommand::NominateForOffice { institution_id, .. } => {
                self.fantasy_arc
                    .first_office_campaign_target
                    .get_or_insert(*institution_id);
            }
            PlayerCommand::FileLegalCase {
                kind: LegalCaseKind::Debt,
                ..
            } => {
                self.player_debt_enforcement_cases =
                    self.player_debt_enforcement_cases.saturating_add(1);
            }
            PlayerCommand::TransferBusinessCash { .. }
            | PlayerCommand::WithdrawBusinessCash { .. }
            | PlayerCommand::AcquireBusiness { .. }
            | PlayerCommand::InvestInBusiness { .. }
            | PlayerCommand::SetBusinessPolicy { .. }
            | PlayerCommand::SetBusinessWages { .. }
            | PlayerCommand::CreateSupplyContract { .. }
            | PlayerCommand::IssueLoan { .. }
            | PlayerCommand::BuyProperty { .. }
            | PlayerCommand::SellProperty { .. }
            | PlayerCommand::EnactLaw { .. }
            | PlayerCommand::StartPublicWork { .. }
            | PlayerCommand::FundPublicWork { .. }
            | PlayerCommand::FileLegalCase { .. }
            | PlayerCommand::SettleLegalCase { .. }
            | PlayerCommand::SetHouseGovernance { .. }
            | PlayerCommand::ConveneFamilyCouncil
            | PlayerCommand::DesignateHeir { .. }
            | PlayerCommand::AdoptWard { .. }
            | PlayerCommand::EducateFamilyMember { .. }
            | PlayerCommand::EndowInstitution { .. }
            | PlayerCommand::ExerciseOfficePower { .. }
            | PlayerCommand::WithdrawFromInstitution { .. }
            | PlayerCommand::RespondToCrisis { .. }
            | PlayerCommand::ResolveLaborDispute { .. }
            | PlayerCommand::CommissionInformation { .. }
            | PlayerCommand::LeverageInformation { .. }
            | PlayerCommand::AcknowledgeNotification { .. } => {}
        }
    }

    pub fn record_action_gap(
        &mut self,
        action: Option<GameplayCommandKind>,
        step_days: u32,
        snapshot: &GameplaySnapshot,
    ) {
        if action.is_some_and(is_substantive_command_kind) {
            self.current_substantive_action_gap_days = 0;
            self.current_asset_rich_quiet_gap_days = 0;
            return;
        }
        self.current_substantive_action_gap_days = self
            .current_substantive_action_gap_days
            .saturating_add(step_days);
        self.longest_substantive_action_gap_days = self
            .longest_substantive_action_gap_days
            .max(self.current_substantive_action_gap_days);
        let has_locked_operating_wealth = snapshot.active_businesses > 0
            && snapshot.player_business_cash >= Money::from_copper(10_000);
        let asset_rich_and_cash_poor = snapshot.player_treasury < Money::from_copper(4_000)
            && (snapshot.player_properties >= 2 || has_locked_operating_wealth);
        if asset_rich_and_cash_poor {
            self.current_asset_rich_quiet_gap_days = self
                .current_asset_rich_quiet_gap_days
                .saturating_add(step_days);
            self.longest_asset_rich_quiet_gap_days = self
                .longest_asset_rich_quiet_gap_days
                .max(self.current_asset_rich_quiet_gap_days);
        } else {
            self.current_asset_rich_quiet_gap_days = 0;
        }
    }

    pub fn record_recovery_pressure(&mut self, step_days: u32, snapshot: &GameplaySnapshot) {
        let under_recovery_pressure = snapshot.player_treasury <= Money::ZERO
            && snapshot.active_businesses == 0
            && snapshot
                .distressed_businesses
                .saturating_add(snapshot.insolvent_businesses)
                > 0
            && snapshot.player_properties == 0
            && snapshot.defaulted_loans > 0;
        if under_recovery_pressure {
            self.current_recovery_pressure_days = self
                .current_recovery_pressure_days
                .saturating_add(step_days);
            self.longest_recovery_pressure_days = self
                .longest_recovery_pressure_days
                .max(self.current_recovery_pressure_days);
        } else {
            self.current_recovery_pressure_days = 0;
        }
    }

    pub fn record_phase_cycle(&mut self, phase: GameplayPhase, observation: PhaseCycleObservation) {
        let PhaseCycleObservation {
            action,
            choices,
            ambient_change,
            quiet_cause,
        } = observation;
        let quiet_cycle = action.is_none_or(|kind| !is_substantive_command_kind(kind))
            && choices.substantive_viable_count == 0
            && choices.substantive_candidate_count == 0;
        let current_quiet_streak = self
            .current_phase_quiet_streaks
            .get_mut(&phase)
            .expect("every gameplay phase must have quiet-streak state");
        if quiet_cycle {
            *current_quiet_streak = current_quiet_streak.saturating_add(1);
        } else {
            *current_quiet_streak = 0;
        }
        let stats = self
            .phase_stats
            .get_mut(&phase)
            .expect("every gameplay phase must have statistics");
        stats.decision_cycles = stats.decision_cycles.saturating_add(1);
        stats.total_viable_command_kinds = stats
            .total_viable_command_kinds
            .saturating_add(usize_to_u32(choices.viable_command_kind_count));
        stats.total_viable_choices = stats
            .total_viable_choices
            .saturating_add(usize_to_u32(choices.substantive_viable_count));
        if action.is_some_and(is_substantive_command_kind) {
            stats.substantive_actions = stats.substantive_actions.saturating_add(1);
            let kind = action.expect("substantive action must have a command kind");
            let count = stats.executed_commands.entry(kind).or_default();
            *count = count.saturating_add(1);
            if action.is_some_and(|kind| {
                matches!(
                    kind,
                    GameplayCommandKind::CultivateInstitutionSupport
                        | GameplayCommandKind::NominateForOffice
                )
            }) {
                stats.institutional_campaign_actions =
                    stats.institutional_campaign_actions.saturating_add(1);
            }
        } else if choices.substantive_viable_count == 0 {
            match quiet_cause {
                Some(QuietCause::GeneratorGap) => {
                    stats.generator_gap_cycles = stats.generator_gap_cycles.saturating_add(1);
                }
                Some(QuietCause::PolicyGate) => {
                    stats.policy_gate_cycles = stats.policy_gate_cycles.saturating_add(1);
                }
                Some(QuietCause::Restrained) => {
                    stats.restrained_cycles = stats.restrained_cycles.saturating_add(1);
                }
                Some(QuietCause::ValidationGate) => {
                    stats.validation_gate_cycles = stats.validation_gate_cycles.saturating_add(1);
                }
                Some(QuietCause::BudgetGate) => {
                    stats.budget_gate_cycles = stats.budget_gate_cycles.saturating_add(1);
                }
                Some(QuietCause::Dormant) => {
                    stats.dormant_cycles = stats.dormant_cycles.saturating_add(1);
                }
                None => {}
            }
            if choices.substantive_candidate_count == 0 {
                stats.quiet_cycles = stats.quiet_cycles.saturating_add(1);
                stats.longest_quiet_streak_cycles =
                    stats.longest_quiet_streak_cycles.max(*current_quiet_streak);
                if ambient_change {
                    stats.quiet_cycles_with_ambient_change =
                        stats.quiet_cycles_with_ambient_change.saturating_add(1);
                    self.quiet_cycles_with_ambient_change =
                        self.quiet_cycles_with_ambient_change.saturating_add(1);
                }
            } else {
                stats.blocked_cycles = stats.blocked_cycles.saturating_add(1);
            }
        }
        record_phase_alternative_quality(stats, choices);
    }

    pub fn observe_initial_snapshot(&mut self, snapshot: &GameplaySnapshot) {
        self.starting_generation = Some(snapshot.generation);
        self.observe_fantasy_arc(snapshot);
        self.observe_non_food_snapshot(snapshot);
        self.last_observed_snapshot = Some(snapshot.clone());
    }

    pub fn observe_crisis_kinds(&mut self, state: &AppState) {
        self.observed_crisis_kinds
            .extend(state.crises.values().map(|crisis| crisis.kind));
    }

    pub fn observe_snapshot(&mut self, snapshot: &GameplaySnapshot) {
        self.minimum_food_satisfaction = self
            .minimum_food_satisfaction
            .min(snapshot.average_food_satisfaction);
        self.minimum_district_food_satisfaction = self
            .minimum_district_food_satisfaction
            .min(snapshot.minimum_district_food_satisfaction);
        self.observe_fantasy_arc(snapshot);
        self.observe_non_food_snapshot(snapshot);
        self.last_observed_snapshot = Some(snapshot.clone());
    }

    /// Replaces an observation-granularity succession day with the exact event
    /// day the chronicle recorded, when available. Decision windows can straddle
    /// a year boundary, and milestone timing should reflect when succession
    /// actually executed rather than when the next observation noticed it.
    pub fn refine_succession_day(&mut self, state: &AppState) {
        let Some(recorded) = self.fantasy_arc.first_succession_day else {
            return;
        };
        let Some(observed) = self.last_observed_snapshot.as_ref().map(|s| s.generation) else {
            return;
        };
        if self
            .starting_generation
            .is_none_or(|start| observed <= start)
        {
            return;
        }
        if let Some(entry) = state.chronicle.iter().rev().find(|entry| {
            entry.kind == crate::core::ChronicleKind::Succession
                && entry
                    .summary
                    .starts_with(&format!("Dynasty {} passed", state.player_dynasty_id))
        }) && entry.day < recorded
        {
            self.fantasy_arc.first_succession_day = Some(entry.day);
            if let Some(transition) = self.succession_transition.as_mut()
                && transition.day > entry.day
            {
                transition.day = entry.day;
            }
        }
    }

    pub fn observe_fantasy_arc(&mut self, snapshot: &GameplaySnapshot) {
        let has_reputation = snapshot
            .quality_reputation
            .max(snapshot.reliability_reputation)
            >= COMMERCIAL_STANDING_REPUTATION_REQUIREMENT;
        if has_reputation {
            self.fantasy_arc
                .first_reputation_standing_day
                .get_or_insert(snapshot.day);
        }
        if has_reputation
            && snapshot.player_contract_deliveries >= OFFICE_NOMINATION_DELIVERY_REQUIREMENT
        {
            self.fantasy_arc
                .first_commercial_standing_day
                .get_or_insert(snapshot.day);
        }
        if snapshot.offices_held > 0 {
            self.fantasy_arc
                .first_office_day
                .get_or_insert(snapshot.day);
        }
        if snapshot.player_disputed_employment > 0 {
            self.fantasy_arc
                .first_player_labor_dispute_day
                .get_or_insert(snapshot.day);
        }
        if self
            .starting_generation
            .is_some_and(|generation| snapshot.generation > generation)
        {
            if self.succession_transition.is_none()
                && let Some(previous) = self.last_observed_snapshot.as_ref()
                && snapshot.generation > previous.generation
            {
                self.succession_transition = Some(GameplaySuccessionTransition {
                    day: snapshot.day,
                    family_unity_before: previous.family_unity,
                    family_unity_after: snapshot.family_unity,
                    legitimacy_before: previous.legitimacy,
                    legitimacy_after: snapshot.legitimacy,
                    offices_before: previous.offices_held,
                    offices_after: snapshot.offices_held,
                    institution_memberships_before: previous.institution_memberships,
                    institution_memberships_after: snapshot.institution_memberships,
                    represented_institutions_before: previous.player_institutions_represented,
                    represented_institutions_after: snapshot.player_institutions_represented,
                });
            }
            self.fantasy_arc
                .first_succession_day
                .get_or_insert(snapshot.day);
            self.minimum_post_succession_family_unity = Some(
                self.minimum_post_succession_family_unity
                    .map_or(snapshot.family_unity, |minimum| {
                        minimum.min(snapshot.family_unity)
                    }),
            );
        }
    }

    pub fn observe_non_food_snapshot(&mut self, snapshot: &GameplaySnapshot) {
        self.peak_player_treasury = self.peak_player_treasury.max(snapshot.player_treasury);
        self.minimum_unowned_property_value = snapshot
            .minimum_unowned_property_value
            .map(|value| {
                self.minimum_unowned_property_value
                    .map_or(value, |current: Money| current.min(value))
            })
            .or(self.minimum_unowned_property_value);
        self.minimum_operating_businesses = self.minimum_operating_businesses.min(
            snapshot
                .active_businesses
                .saturating_add(snapshot.distressed_businesses),
        );
        self.maximum_disputed_employment = self
            .maximum_disputed_employment
            .max(snapshot.disputed_employment);
        self.maximum_player_disputed_employment = self
            .maximum_player_disputed_employment
            .max(snapshot.player_disputed_employment);
        self.maximum_delinquent_loans =
            self.maximum_delinquent_loans.max(snapshot.delinquent_loans);
        self.maximum_defaulted_loans = self.maximum_defaulted_loans.max(snapshot.defaulted_loans);
        self.maximum_player_delinquent_lending = self
            .maximum_player_delinquent_lending
            .max(snapshot.player_delinquent_lending);
        self.maximum_player_defaulted_lending = self
            .maximum_player_defaulted_lending
            .max(snapshot.player_defaulted_lending);
        self.maximum_player_delinquent_borrowing = self
            .maximum_player_delinquent_borrowing
            .max(snapshot.player_delinquent_borrowing);
        self.maximum_player_defaulted_borrowing = self
            .maximum_player_defaulted_borrowing
            .max(snapshot.player_defaulted_borrowing);
        self.maximum_delinquent_civic_debts = self
            .maximum_delinquent_civic_debts
            .max(snapshot.delinquent_civic_debts);
        self.maximum_defaulted_civic_debts = self
            .maximum_defaulted_civic_debts
            .max(snapshot.defaulted_civic_debts);
        self.maximum_offices_held = self.maximum_offices_held.max(snapshot.offices_held);
        self.maximum_unfinished_public_works = self.maximum_unfinished_public_works.max(
            snapshot
                .building_public_works
                .saturating_add(snapshot.suspended_public_works),
        );
        self.maximum_active_crises = self.maximum_active_crises.max(snapshot.active_crises);
        self.maximum_unread_notifications = self
            .maximum_unread_notifications
            .max(snapshot.unread_notifications);
        self.maximum_contract_relationship_pressure_basis_points = self
            .maximum_contract_relationship_pressure_basis_points
            .max(snapshot.maximum_contract_relationship_pressure_basis_points);
        self.peak_route_disruption_basis_points = self
            .peak_route_disruption_basis_points
            .max(snapshot.maximum_route_disruption_basis_points);
        self.peak_city_distressed_businesses = self
            .peak_city_distressed_businesses
            .max(snapshot.city_distressed_businesses);
    }
}

/// Runs deterministic player agents across configured backgrounds, personas, and seeds.
///
/// Each agent derives legal candidates from visible campaign state, probes those candidates through
/// the canonical command API on cloned state, commits the highest-ranked viable action, advances
/// through the canonical simulation pipeline, and records immediate and delayed system reactions.
///
/// # Errors
///
/// Returns an error for invalid configuration, campaign creation failure, simulation failure, or a
/// command that unexpectedly fails after succeeding against an identical probe state.
pub fn run_gameplay_harness(
    registry: &Registry,
    config: GameplayHarnessConfig,
) -> Result<GameplayHarnessReport, GameplayHarnessError> {
    config.validate()?;
    let work = campaign_work_items(&config)?;
    let campaigns = execute_campaigns(registry, &config, &work)?;
    let aggregate = aggregate_campaigns(&campaigns);
    let persona_aggregates = aggregate_campaigns_by_persona(&campaigns);
    let findings = derive_findings(&aggregate, &campaigns);
    Ok(GameplayHarnessReport {
        schema_version: GAMEPLAY_REPORT_SCHEMA_VERSION,
        config,
        aggregate,
        persona_aggregates,
        campaigns,
        findings,
        limitations: gameplay_harness_limitations(),
    })
}

pub(crate) struct CampaignWorkItem {
    pub seed: u64,
    pub background: StartingBackground,
    pub persona: GameplayPersona,
}

pub(crate) fn campaign_work_items(
    config: &GameplayHarnessConfig,
) -> Result<Vec<CampaignWorkItem>, GameplayHarnessError> {
    let mut work = Vec::with_capacity(config.campaign_count());
    for seed_offset in 0..config.seed_count {
        let seed = config
            .start_seed
            .checked_add(u64::from(seed_offset))
            .ok_or_else(|| GameplayHarnessError::InvalidConfig {
                reason: "configured seed range exceeds u64::MAX".to_owned(),
            })?;
        for background in &config.backgrounds {
            for persona in &config.personas {
                work.push(CampaignWorkItem {
                    seed,
                    background: *background,
                    persona: *persona,
                });
            }
        }
    }
    Ok(work)
}

pub(crate) fn execute_campaigns(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    work: &[CampaignWorkItem],
) -> Result<Vec<GameplayCampaignReport>, GameplayHarnessError> {
    if work.len() <= 1 {
        return work
            .iter()
            .map(|item| {
                run_campaign(
                    registry,
                    config,
                    item.seed,
                    item.background,
                    item.persona,
                    true,
                )
            })
            .collect();
    }
    let available = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let capped = std::env::var("CIVIC_DYNASTY_JOBS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value >= 1)
        .unwrap_or(available);
    let parallelism = capped.min(available).min(work.len());
    let chunks: Vec<&[CampaignWorkItem]> = work.chunks(work.len().div_ceil(parallelism)).collect();
    let results: Result<
        Vec<Result<GameplayCampaignReport, GameplayHarnessError>>,
        GameplayHarnessError,
    > = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|item| {
                            run_campaign(
                                registry,
                                config,
                                item.seed,
                                item.background,
                                item.persona,
                                false,
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut campaigns = Vec::with_capacity(work.len());
        for handle in handles {
            campaigns.extend(
                handle
                    .join()
                    .map_err(|_| GameplayHarnessError::CampaignWorkerPanicked)?,
            );
        }
        Ok(campaigns)
    });
    results?.into_iter().collect()
}

pub(crate) fn gameplay_harness_limitations() -> Vec<String> {
    vec![
        "Automated agents measure reachability and systemic outcomes, not whether a human understands the interface or enjoys the decisions.".to_owned(),
        "The report cannot measure emotional investment, narrative quality, or the cognitive burden of comparing choices.".to_owned(),
        "Agents inspect authoritative simulation state when choosing what to investigate; commissioned reports can unlock traceable follow-up actions, but the harness does not measure whether a human can interpret the report or identify the best use.".to_owned(),
        "Alternative-choice profiles retain every successfully probed concrete target and distinguish measured impact from persistent target identity. Every viable option is projected over a shared horizon of three decision intervals (bounded by max_consequence_horizon_days), so delayed tradeoffs are compared consistently; this remains a bounded counterfactual rather than a full alternate campaign.".to_owned(),
        "A distinct target fingerprint proves that two branches preserve different strategic state, not that a human will value the difference or that the difference becomes important within the campaign.".to_owned(),
        "Deterministic personas follow fixed priorities with a small state-derived exploration variation. The variation is reproducible, cannot consume the game RNG, and only changes close calls; it does not model misunderstanding, changing preferences, or interface friction.".to_owned(),
        "Choice breadth measures the options emitted by the configured persona policy, not every legal command a human could discover in the same state. Cross-persona matrices are required before treating a narrow candidate set as a hard game-system ceiling.".to_owned(),
        "Stress personas can prove that risky legal, labor, and financial routes exist, but they cannot prove that those risks are legible or attractive to a human player.".to_owned(),
        "AI-objective progress measures rival activity, but the harness cannot prove that a human recognizes which house caused a setback or understands that rival's intent.".to_owned(),
        "Counterfactual attribution can only detect consequences represented by the report snapshot and configured consequence horizon.".to_owned(),
        "Material civic endpoints include per-district employment, sanitation, safety, and unrest, but the harness does not judge whether those neighborhood differences are fair, narratively legible, or understandable to a human player.".to_owned(),
        "Persistent state and chronicle changes approximate historical imprint; the harness cannot judge whether the game presents that legacy as a coherent remembered story.".to_owned(),
    ]
}

pub(crate) fn record_phase_alternative_quality(
    stats: &mut GameplayPhaseStats,
    choices: ChoiceCycleMetrics,
) {
    if choices
        .family_quality
        .contains(AlternativeQuality::MULTIPLE)
    {
        stats.cycles_with_multiple_viable_command_kinds = stats
            .cycles_with_multiple_viable_command_kinds
            .saturating_add(1);
    }
    if choices.family_quality.contains(AlternativeQuality::CLOSE) {
        stats.cycles_with_close_viable_command_kinds = stats
            .cycles_with_close_viable_command_kinds
            .saturating_add(1);
    }
    if choices
        .family_quality
        .contains(AlternativeQuality::DISTINCT_IMMEDIATE)
    {
        stats.cycles_with_distinct_immediate_consequences = stats
            .cycles_with_distinct_immediate_consequences
            .saturating_add(1);
    }
    if choices
        .family_quality
        .contains(AlternativeQuality::DISTINCT_PROJECTED)
    {
        stats.cycles_with_distinct_projected_consequences = stats
            .cycles_with_distinct_projected_consequences
            .saturating_add(1);
    }
    if choices
        .option_quality
        .contains(AlternativeQuality::MULTIPLE)
    {
        stats.cycles_with_multiple_viable_options =
            stats.cycles_with_multiple_viable_options.saturating_add(1);
    }
    if choices.option_quality.contains(AlternativeQuality::CLOSE) {
        stats.cycles_with_close_viable_options =
            stats.cycles_with_close_viable_options.saturating_add(1);
    }
    if choices
        .option_quality
        .contains(AlternativeQuality::DISTINCT_IMMEDIATE)
    {
        stats.cycles_with_distinct_immediate_option_consequences = stats
            .cycles_with_distinct_immediate_option_consequences
            .saturating_add(1);
    }
    if choices
        .option_quality
        .contains(AlternativeQuality::DISTINCT_PROJECTED)
    {
        stats.cycles_with_distinct_projected_option_consequences = stats
            .cycles_with_distinct_projected_option_consequences
            .saturating_add(1);
    }
}

pub(crate) fn run_campaign(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    seed: u64,
    background: StartingBackground,
    persona: GameplayPersona,
    parallel_counterfactuals: bool,
) -> Result<GameplayCampaignReport, GameplayHarnessError> {
    let mut state = build_new_game(
        registry,
        NewGameConfig {
            seed,
            dynasty_name: format!("Harness-{}-{seed}", persona.label()),
            founder_name: format!("Agent {}", persona.label()),
            background,
        },
    )?;
    let start = GameplaySnapshot::capture(&state);
    let mut accumulator = CampaignAccumulator::new();
    accumulator.observe_initial_snapshot(&start);
    accumulator.observe_crisis_kinds(&state);
    let mut remaining = config.days_per_campaign;
    while remaining > 0 {
        let jittered_interval = jittered_decision_interval(&state, config, persona, &accumulator);
        let configured_step = jittered_interval.min(remaining);
        let step_days = next_campaign_step_days(&state, configured_step).min(remaining);
        run_decision_cycle(
            registry,
            config,
            persona,
            &mut state,
            step_days,
            parallel_counterfactuals,
            &mut accumulator,
        )?;
        remaining = remaining.saturating_sub(step_days);
    }
    run_terminal_phase_if_needed(
        registry,
        config,
        persona,
        &mut state,
        parallel_counterfactuals,
        &mut accumulator,
    )?;
    validate_invariants(registry, &state);
    Ok(finish_campaign_report(
        config,
        seed,
        persona,
        background,
        &state,
        start,
        accumulator,
    ))
}

/// Small deterministic jitter for the ordinary decision cadence so the harness
/// does not sample the same calendar offsets every campaign. The variation is
/// derived from the campaign RNG and current state, stays within +/-12 days,
/// never consumes the game RNG, and is clamped to at least 7 days to keep
/// urgent sub-week steps meaningful.
pub(crate) fn jittered_decision_interval(
    state: &AppState,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    accumulator: &CampaignAccumulator,
) -> u32 {
    let base = u32::from(config.decision_interval_days);
    if base <= 7 {
        return base;
    }
    let mut rng = state.rng;
    let mut sample = rng.next_u64();
    sample = sample.wrapping_add(state.clock.day().cast_unsigned());
    sample = sample.wrapping_add(u64::from(accumulator.decision_cycles));
    sample = sample.wrapping_add(u64::from(config.seed_count));
    sample = sample.wrapping_add(config.start_seed & 0xFFFF_FFFF);
    sample = sample.wrapping_add(u64::from(state.player_dynasty_id.value()));
    // Mix persona so campaigns that share a world seed but follow different
    // diagnostic roles sample distinct calendar offsets instead of replaying
    // one rigid schedule. Mirrors the candidate-level variation which already
    // keys on persona, extending the same organic divergence to timing.
    for byte in persona.label().bytes() {
        sample ^= u64::from(byte).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        sample = sample.rotate_left(7);
    }
    sample ^= u64::from(accumulator.total_viable_choices).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    sample ^= u64::from(accumulator.quiet_cycles).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    // Widen to +/-12 days so campaigns sample different calendar offsets even
    // when personas share a world seed. The variation stays inside one decision
    // cycle so the 30-day cadence remains legible while each world and persona
    // samples slightly different observation windows across campaigns.
    let delta = i64::try_from(sample % 25).unwrap_or(0) - 12;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        i64::from(base).saturating_add(delta).max(7) as u32
    }
}

pub(crate) fn next_campaign_step_days(state: &AppState, configured_step: u32) -> u32 {
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let urgent_funding_window = state
        .legal_cases
        .values()
        .filter_map(|legal_case| {
            let quote = quote_player_legal_settlement(state, legal_case.id).ok()?;
            if treasury >= quote.amount {
                return None;
            }
            let days_to_hearing = legal_case.hearing_day.saturating_sub(state.clock.day());
            // A hearing tomorrow is exactly the case that most needs an
            // accelerated funding step; only same-day judgments are past
            // saving. The `.max(1)` floor below keeps the step positive.
            (days_to_hearing >= 1).then_some(days_to_hearing)
        })
        .min();
    let legal_step = urgent_funding_window.map_or(u32::MAX, |days_to_hearing| {
        u32::try_from(days_to_hearing)
            .unwrap_or(u32::MAX)
            .div_ceil(2)
            .max(1)
    });
    let urgent_crisis_or_labor = state.crises.values().any(|crisis| {
        crisis.status.is_active() && !crisis_has_containment_response(state, crisis.id)
    }) || state.employment.values().any(|agreement| {
        agreement.status == EmploymentStatus::Disputed
            && state
                .businesses
                .get(agreement.business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
    });
    let urgent_step = if urgent_crisis_or_labor { 7 } else { u32::MAX };
    configured_step.min(legal_step).min(urgent_step).max(1)
}

pub(crate) fn finish_campaign_report(
    config: &GameplayHarnessConfig,
    seed: u64,
    persona: GameplayPersona,
    background: StartingBackground,
    state: &AppState,
    start: GameplaySnapshot,
    mut accumulator: CampaignAccumulator,
) -> GameplayCampaignReport {
    let end = GameplaySnapshot::capture(state);
    let scores = score_campaign(&accumulator, &start, &end);
    let interactions = interaction_vec(&accumulator.interactions);
    let trace = select_trace(
        std::mem::take(&mut accumulator.trace),
        usize::from(config.trace_limit_per_campaign),
    );
    GameplayCampaignReport {
        seed,
        persona,
        background,
        simulated_days: config.days_per_campaign,
        decision_cycles: accumulator.decision_cycles,
        cycles_with_viable_choices: accumulator.cycles_with_viable_choices,
        cycles_with_multiple_viable_command_kinds: accumulator
            .cycles_with_multiple_viable_command_kinds,
        cycles_with_close_viable_command_kinds: accumulator.cycles_with_close_viable_command_kinds,
        cycles_with_distinct_immediate_consequences: accumulator
            .cycles_with_distinct_immediate_consequences,
        cycles_with_distinct_projected_consequences: accumulator
            .cycles_with_distinct_projected_consequences,
        cycles_with_multiple_viable_options: accumulator.cycles_with_multiple_viable_options,
        cycles_with_close_viable_options: accumulator.cycles_with_close_viable_options,
        cycles_with_distinct_immediate_option_consequences: accumulator
            .cycles_with_distinct_immediate_option_consequences,
        cycles_with_distinct_projected_option_consequences: accumulator
            .cycles_with_distinct_projected_option_consequences,
        no_action_cycles: accumulator.no_action_cycles,
        quiet_cycles: accumulator.quiet_cycles,
        quiet_cycles_with_ambient_change: accumulator.quiet_cycles_with_ambient_change,
        blocked_cycles: accumulator.blocked_cycles,
        total_viable_choices: accumulator.total_viable_choices,
        total_viable_command_kinds: accumulator.total_viable_command_kinds,
        phase_stats: accumulator.phase_stats,
        commands: accumulator.commands,
        rejection_reasons: accumulator.rejection_reasons,
        domain_changes: accumulator.domain_changes,
        causal_domain_changes: accumulator.causal_domain_changes,
        ambient_domain_changes: accumulator.ambient_domain_changes,
        interactions,
        start,
        end,
        scores,
        minimum_food_satisfaction: accumulator.minimum_food_satisfaction,
        minimum_district_food_satisfaction: accumulator.minimum_district_food_satisfaction,
        minimum_operating_businesses: accumulator.minimum_operating_businesses,
        peak_player_treasury: accumulator.peak_player_treasury,
        minimum_unowned_property_value: accumulator.minimum_unowned_property_value,
        maximum_disputed_employment: accumulator.maximum_disputed_employment,
        maximum_player_disputed_employment: accumulator.maximum_player_disputed_employment,
        maximum_delinquent_loans: accumulator.maximum_delinquent_loans,
        maximum_defaulted_loans: accumulator.maximum_defaulted_loans,
        maximum_player_delinquent_lending: accumulator.maximum_player_delinquent_lending,
        maximum_player_defaulted_lending: accumulator.maximum_player_defaulted_lending,
        maximum_player_delinquent_borrowing: accumulator.maximum_player_delinquent_borrowing,
        maximum_player_defaulted_borrowing: accumulator.maximum_player_defaulted_borrowing,
        maximum_delinquent_civic_debts: accumulator.maximum_delinquent_civic_debts,
        maximum_defaulted_civic_debts: accumulator.maximum_defaulted_civic_debts,
        maximum_offices_held: accumulator.maximum_offices_held,
        maximum_unfinished_public_works: accumulator.maximum_unfinished_public_works,
        maximum_active_crises: accumulator.maximum_active_crises,
        observed_crisis_kinds: accumulator.observed_crisis_kinds,
        maximum_unread_notifications: accumulator.maximum_unread_notifications,
        maximum_contract_relationship_pressure_basis_points: accumulator
            .maximum_contract_relationship_pressure_basis_points,
        minimum_post_succession_family_unity: accumulator.minimum_post_succession_family_unity,
        longest_substantive_command_streak: accumulator.longest_substantive_command_streak,
        longest_substantive_streak_command: accumulator.longest_substantive_streak_command,
        longest_substantive_action_gap_days: accumulator.longest_substantive_action_gap_days,
        longest_asset_rich_quiet_gap_days: accumulator.longest_asset_rich_quiet_gap_days,
        longest_recovery_pressure_days: accumulator.longest_recovery_pressure_days,
        terminal_recovery_pressure_days: accumulator.current_recovery_pressure_days,
        commission_leverage_pairs: accumulator.commission_leverage_pairs,
        player_debt_enforcement_cases: accumulator.player_debt_enforcement_cases,
        peak_route_disruption_basis_points: accumulator.peak_route_disruption_basis_points,
        peak_city_distressed_businesses: accumulator.peak_city_distressed_businesses,
        rival_context: build_rival_context(state),
        fantasy_arc: accumulator.fantasy_arc,
        succession_transition: accumulator.succession_transition,
        quiet_diagnostic: accumulator.quiet_diagnostic,
        trace,
    }
}

/// Ranks every house in the city at campaign end so the report shows whether
/// the player is actually competing for standing, not just accumulating.
pub(crate) fn build_rival_context(state: &AppState) -> GameplayRivalContext {
    let offices_by_dynasty = |dynasty_id: DynastyId| -> u16 {
        usize_to_u16(
            state
                .institutions
                .values()
                .filter(|institution| {
                    institution
                        .office_holder_id
                        .and_then(|character_id| state.characters.get(character_id))
                        .is_some_and(|character| character.dynasty_id() == dynasty_id)
                })
                .count(),
        )
    };
    let standings: Vec<GameplayRivalStanding> = state
        .dynasties
        .values()
        .map(|dynasty| {
            let operating_businesses = state
                .businesses
                .ids_for_owner(dynasty.id())
                .into_iter()
                .flatten()
                .filter_map(|business_id| state.businesses.get(*business_id))
                .filter(|business| {
                    !matches!(
                        business.status(),
                        BusinessStatus::Insolvent | BusinessStatus::Closed
                    )
                })
                .count();
            GameplayRivalStanding {
                dynasty_id: dynasty.id(),
                name: dynasty.name().to_owned(),
                is_player: dynasty.id() == state.player_dynasty_id,
                treasury: dynasty.treasury(),
                legitimacy_basis_points: dynasty.resources.legitimacy_basis_points,
                offices_held: offices_by_dynasty(dynasty.id()),
                operating_businesses: usize_to_u16(operating_businesses),
            }
        })
        .collect();
    let mut by_treasury = standings.clone();
    // Stable descending wealth order; the dynasty ID breaks ties so parallel
    // runs and repeated renders cannot reorder the leaderboard.
    by_treasury.sort_by(|a, b| {
        b.treasury
            .cmp(&a.treasury)
            .then(a.dynasty_id.cmp(&b.dynasty_id))
    });
    let player_treasury_rank = by_treasury
        .iter()
        .position(|standing| standing.is_player)
        .map_or(0, |index| usize_to_u16(index + 1));
    let mut by_legitimacy = standings;
    by_legitimacy.sort_by(|a, b| {
        b.legitimacy_basis_points
            .cmp(&a.legitimacy_basis_points)
            .then(a.dynasty_id.cmp(&b.dynasty_id))
    });
    let player_legitimacy_rank = by_legitimacy
        .iter()
        .position(|standing| standing.is_player)
        .map_or(0, |index| usize_to_u16(index + 1));
    GameplayRivalContext {
        dynasty_count: usize_to_u16(state.dynasties.len()),
        player_treasury_rank,
        player_legitimacy_rank,
        leaders_by_treasury: by_treasury
            .into_iter()
            .take(RIVAL_LEADERBOARD_SIZE)
            .collect(),
    }
}

/// How many houses the campaign summary leaderboard shows.
const RIVAL_LEADERBOARD_SIZE: usize = 4;

pub(crate) fn run_terminal_phase_if_needed(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    state: &mut AppState,
    parallel_counterfactuals: bool,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    if terminal_phase_needs_decision(accumulator) {
        run_terminal_decision_cycle(
            registry,
            config,
            persona,
            state,
            parallel_counterfactuals,
            accumulator,
        )?;
    }
    Ok(())
}

pub(crate) fn terminal_phase_needs_decision(accumulator: &CampaignAccumulator) -> bool {
    accumulator
        .phase_stats
        .get(&gameplay_phase(&accumulator.fantasy_arc))
        .is_some_and(|stats| stats.decision_cycles == 0)
}

#[derive(Clone, Copy)]
pub(crate) enum DecisionCycleMode {
    AdvanceCampaign { step_days: u32 },
    Terminal,
}

pub(crate) fn run_terminal_decision_cycle(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    state: &mut AppState,
    parallel_counterfactuals: bool,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    run_decision_cycle_internal(
        registry,
        config,
        persona,
        state,
        DecisionCycleMode::Terminal,
        parallel_counterfactuals,
        accumulator,
    )
}

pub(crate) fn run_decision_cycle(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    state: &mut AppState,
    step_days: u32,
    parallel_counterfactuals: bool,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    run_decision_cycle_internal(
        registry,
        config,
        persona,
        state,
        DecisionCycleMode::AdvanceCampaign { step_days },
        parallel_counterfactuals,
        accumulator,
    )
}

// This is the intentionally linear harness pipeline: observe, generate, probe,
// commit, advance, attribute, and record. Keeping it visible preserves the
// canonical gameplay order while feedback capture adds a few bookkeeping lines.
#[expect(
    clippy::too_many_lines,
    reason = "the dispatch keeps the full decision path in one auditable function"
)]
pub(crate) fn run_decision_cycle_internal(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    state: &mut AppState,
    mode: DecisionCycleMode,
    parallel_counterfactuals: bool,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    accumulator.decision_cycles = accumulator.decision_cycles.saturating_add(1);
    apply_notification_housekeeping(registry, state, accumulator)?;
    let phase = gameplay_phase(&accumulator.fantasy_arc);
    let before = GameplaySnapshot::capture(state);
    let feedback_before = feedback_cursor(state);
    let (candidates, raw_generated_kinds) =
        ranked_candidates(registry, state, persona, accumulator);
    validate_candidate_classifications(state, &candidates)?;
    let activation_before = activation_opportunity_snapshot(accumulator);
    record_activation_opportunities(registry, state, persona, accumulator, &raw_generated_kinds);
    let activation_delta = activation_opportunity_delta(accumulator, &activation_before);
    let ranked_candidates = summarize_ranked_candidates(&candidates);
    let substantive_candidate_count = candidates
        .iter()
        .filter(|candidate| is_substantive_command_kind(candidate.kind))
        .count();
    record_offered_command_kinds(&candidates, accumulator);
    record_generated_candidates(&candidates, accumulator);
    let retained_kinds: BTreeSet<_> = candidates.iter().map(|candidate| candidate.kind).collect();
    let mut retained_counts_by_kind = BTreeMap::new();
    for candidate in &candidates {
        *retained_counts_by_kind.entry(candidate.kind).or_default() += 1;
    }
    let candidates_to_probe =
        select_probe_candidates(candidates, usize::from(config.max_candidate_probes));
    let mut probed_counts_by_kind = BTreeMap::new();
    for candidate in &candidates_to_probe {
        *probed_counts_by_kind.entry(candidate.kind).or_default() += 1;
    }
    let probe_limit = candidates_to_probe.len();
    let projection_step_days = match mode {
        DecisionCycleMode::AdvanceCampaign { step_days } => step_days,
        DecisionCycleMode::Terminal => u32::from(config.decision_interval_days),
    };
    let probe = probe_candidates_with_parallelism(
        registry,
        state,
        candidates_to_probe.into_iter(),
        projection_step_days,
        config.max_consequence_horizon_days,
        parallel_counterfactuals,
        accumulator,
    )?;
    // Staleness tripwire: a canonically viable probe proves the world offered
    // that route, so its activation predicate must have fired. A miss means a
    // predicate drifted from its canonical validation route and quiet-cycle
    // diagnosis would misclassify the family as dormant.
    let world_activations = pure_world_activation_set(registry, state, persona);
    let drifted: Vec<_> = probe
        .viable_command_kinds
        .iter()
        .filter(|kind| !world_activations.contains(kind))
        .copied()
        .collect();
    if !drifted.is_empty() {
        return Err(GameplayHarnessError::ActivationPredicateDrift { kinds: drifted });
    }
    let no_action_reason = record_quiet_diagnostic(
        accumulator,
        &probe,
        &raw_generated_kinds,
        &retained_kinds,
        &retained_counts_by_kind,
        &probed_counts_by_kind,
        &activation_delta,
    );
    let choice_metrics =
        record_choice_cycle_metrics(accumulator, substantive_candidate_count, &probe);
    // The ambient baseline branch advances a frozen pre-cycle clone, but only
    // cycles that actually commit an action have a separate no-action branch
    // to observe. A quiet cycle never branches (the main advance IS the
    // no-action path), so the expensive campaign clone is deferred until the
    // probe result shows an action will be committed.
    let baseline_state = probe.selected.is_some().then(|| state.clone());
    let action = apply_selected_candidate(registry, state, probe.selected.clone(), accumulator)?;
    let action_kind = action.as_ref().map(|action| action.kind);
    let command_feedback =
        collect_feedback(state, feedback_before, GameplayFeedbackSource::Command);
    let feedback_after_command = feedback_cursor(state);
    let action_gap_days = match mode {
        DecisionCycleMode::AdvanceCampaign { step_days } => step_days,
        DecisionCycleMode::Terminal => 0,
    };
    accumulator.record_action_gap(action_kind, action_gap_days, &before);
    let after_command = GameplaySnapshot::capture(state);
    let consequence_horizon = consequence_horizon_days(
        action.as_ref().map(|action| action.kind),
        projection_step_days,
        config.max_consequence_horizon_days,
    );
    let (after_time, simulation_feedback) = advance_and_collect_feedback(
        registry,
        state,
        mode,
        consequence_horizon,
        &after_command,
        feedback_after_command,
        accumulator,
    )?;
    let (baseline_after_time, ambient_change, baseline_feedback) = baseline_observation(
        registry,
        action.as_ref(),
        consequence_horizon,
        &after_time,
        baseline_state,
        &before,
        state,
        feedback_after_command,
    )?;
    accumulator.record_phase_cycle(
        phase,
        PhaseCycleObservation {
            action: action_kind,
            choices: choice_metrics,
            ambient_change,
            quiet_cause: quiet_cause(no_action_reason.as_deref()),
        },
    );
    let simulation_window = cycle_simulation_window_days(mode);
    let ambient_window = cycle_ambient_window_days(
        mode,
        action.as_ref().map(|action| action.kind),
        consequence_horizon,
    );
    record_decision_cycle(
        DecisionCycleSnapshots {
            before: &before,
            after_command: &after_command,
            after_time: &after_time,
            baseline_after_time: &baseline_after_time,
        },
        DecisionCycleRecord {
            considered: probe_limit,
            probe: &probe,
            ranked_candidates,
            phase,
            action,
            no_action_reason,
            command_feedback,
            simulation_window_days: simulation_window,
            simulation_feedback,
            ambient_window_days: ambient_window,
            ambient_feedback: baseline_feedback,
        },
        accumulator,
    );
    Ok(())
}

/// Days the action branch advanced after the commit when collecting its
/// feedback. Terminal cycles never advance the campaign, so their window is
/// empty and only command-time feedback is retained.
pub(crate) fn cycle_simulation_window_days(mode: DecisionCycleMode) -> u32 {
    match mode {
        DecisionCycleMode::AdvanceCampaign { step_days } => step_days,
        DecisionCycleMode::Terminal => 0,
    }
}

/// Days the no-action branch advanced when collecting ambient feedback.
/// Substantive cycles attribute over the consequence horizon; quiet cycles
/// never branch, so their ambient window is the ordinary advance itself.
pub(crate) fn cycle_ambient_window_days(
    mode: DecisionCycleMode,
    action: Option<GameplayCommandKind>,
    consequence_horizon: u32,
) -> u32 {
    match action {
        Some(_) => consequence_horizon,
        None => cycle_simulation_window_days(mode),
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DecisionCycleSnapshots<'a> {
    pub before: &'a GameplaySnapshot,
    pub after_command: &'a GameplaySnapshot,
    pub after_time: &'a GameplaySnapshot,
    pub baseline_after_time: &'a GameplaySnapshot,
}

pub(crate) struct DecisionCycleRecord<'a> {
    pub considered: usize,
    pub probe: &'a ProbeResult,
    pub ranked_candidates: Vec<GameplayCandidateRanking>,
    pub phase: GameplayPhase,
    pub action: Option<ExecutedAction>,
    pub no_action_reason: Option<String>,
    pub command_feedback: Vec<GameplayFeedbackEvent>,
    pub simulation_window_days: u32,
    pub simulation_feedback: Vec<GameplayFeedbackEvent>,
    pub ambient_window_days: u32,
    pub ambient_feedback: Vec<GameplayFeedbackEvent>,
}

pub(crate) fn record_decision_cycle(
    snapshots: DecisionCycleSnapshots<'_>,
    record: DecisionCycleRecord<'_>,
    accumulator: &mut CampaignAccumulator,
) {
    let DecisionCycleRecord {
        considered,
        probe,
        ranked_candidates,
        phase,
        action,
        no_action_reason,
        command_feedback,
        simulation_window_days,
        simulation_feedback,
        ambient_window_days,
        ambient_feedback,
    } = record;
    record_cycle(
        CycleObservation {
            before: snapshots.before,
            after_command: snapshots.after_command,
            after_time: snapshots.after_time,
            baseline_after_time: snapshots.baseline_after_time,
            considered,
            viable: probe.viable_count,
            substantive_viable: probe.substantive_viable_count,
            viable_command_kinds: probe.viable_command_kinds.clone(),
            ranked_candidates,
            phase,
            viable_options: probe.viable_options.clone(),
            close_choice_score_gap: probe.close_choice_score_gap,
            distinct_immediate_choice_profiles: probe.distinct_immediate_choice_profiles,
            distinct_projected_choice_profiles: probe.distinct_projected_choice_profiles,
            rejections: probe.rejections.clone(),
            action,
            no_action_reason,
            command_feedback,
            simulation_window_days,
            simulation_feedback,
            ambient_window_days,
            ambient_feedback,
        },
        accumulator,
    );
}

pub(crate) fn advance_decision_time(
    registry: &Registry,
    state: &mut AppState,
    mode: DecisionCycleMode,
    consequence_horizon: u32,
    after_command: &GameplaySnapshot,
    accumulator: &mut CampaignAccumulator,
) -> Result<GameplaySnapshot, GameplayHarnessError> {
    Ok(match mode {
        DecisionCycleMode::AdvanceCampaign { step_days } => {
            let mut consequence_state = (consequence_horizon > step_days).then(|| state.clone());
            // The harness owns this campaign branch outright and discards it
            // wholesale when a day fails, so the defensive copy the public
            // entry makes is redundant here.
            advance_days_scratch(registry, state, step_days)?;
            let campaign_after_time = GameplaySnapshot::capture(state);
            accumulator.observe_snapshot(&campaign_after_time);
            accumulator.refine_succession_day(state);
            accumulator.record_recovery_pressure(step_days, &campaign_after_time);
            if let Some(consequence_state) = consequence_state.as_mut() {
                advance_days_scratch(registry, consequence_state, consequence_horizon)?;
                GameplaySnapshot::capture(consequence_state)
            } else {
                campaign_after_time
            }
        }
        DecisionCycleMode::Terminal => {
            accumulator.observe_snapshot(after_command);
            let mut consequence_state = state.clone();
            advance_days_scratch(registry, &mut consequence_state, consequence_horizon)?;
            GameplaySnapshot::capture(&consequence_state)
        }
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "each parameter is a distinct branch input of the attribution pipeline"
)]
pub(crate) fn baseline_observation(
    registry: &Registry,
    // Kept in the signature so the branch contract stays explicit at call
    // sites: an executed action always pairs with `Some(baseline_state)`,
    // because both derive from the probe selecting a candidate.
    _action: Option<&ExecutedAction>,
    consequence_horizon: u32,
    after_time: &GameplaySnapshot,
    baseline_state: Option<AppState>,
    before: &GameplaySnapshot,
    advanced_state: &AppState,
    advanced_feedback_cursor: FeedbackCursor,
) -> Result<(GameplaySnapshot, bool, Vec<GameplayFeedbackEvent>), GameplayHarnessError> {
    let (baseline_after_time, ambient_feedback) = if let Some(mut baseline_state) = baseline_state {
        let feedback_before = feedback_cursor(&baseline_state);
        advance_days_scratch(registry, &mut baseline_state, consequence_horizon)?;
        let snapshot = GameplaySnapshot::capture(&baseline_state);
        let feedback = collect_feedback(
            &baseline_state,
            feedback_before,
            GameplayFeedbackSource::Ambient,
        );
        (snapshot, feedback)
    } else {
        // A quiet cycle never branches: the main advance already is the
        // no-action path, so its own post-advance events are the ambient
        // feedback. Collecting from an untouched clone here would always be
        // empty and hide exactly the world change quiet diagnostics measure.
        (
            after_time.clone(),
            collect_feedback(
                advanced_state,
                advanced_feedback_cursor,
                GameplayFeedbackSource::Ambient,
            ),
        )
    };
    let ambient_change = !before.changed_domains(&baseline_after_time).is_empty()
        || baseline_after_time.outbox_messages > before.outbox_messages
        || baseline_after_time.chronicle_entries > before.chronicle_entries;
    Ok((baseline_after_time, ambient_change, ambient_feedback))
}

pub(crate) fn advance_and_collect_feedback(
    registry: &Registry,
    state: &mut AppState,
    mode: DecisionCycleMode,
    consequence_horizon: u32,
    after_command: &GameplaySnapshot,
    feedback_after_command: FeedbackCursor,
    accumulator: &mut CampaignAccumulator,
) -> Result<(GameplaySnapshot, Vec<GameplayFeedbackEvent>), GameplayHarnessError> {
    let after_time = advance_decision_time(
        registry,
        state,
        mode,
        consequence_horizon,
        after_command,
        accumulator,
    )?;
    accumulator.observe_crisis_kinds(state);
    let feedback = collect_feedback(
        state,
        feedback_after_command,
        GameplayFeedbackSource::Simulation,
    );
    Ok((after_time, feedback))
}

#[derive(Clone, Copy)]
pub(crate) struct FeedbackCursor {
    pub outbox_len: usize,
    pub chronicle_len: usize,
}

pub(crate) fn feedback_cursor(state: &AppState) -> FeedbackCursor {
    FeedbackCursor {
        outbox_len: state.outbox.len(),
        chronicle_len: state.chronicle.len(),
    }
}

pub(crate) fn collect_feedback(
    state: &AppState,
    cursor: FeedbackCursor,
    source: GameplayFeedbackSource,
) -> Vec<GameplayFeedbackEvent> {
    const MAX_EVENTS: usize = 6;
    let mut events = state
        .outbox
        .iter()
        .skip(cursor.outbox_len)
        .map(|message| GameplayFeedbackEvent {
            source,
            day: message.day,
            channel: "outbox".to_owned(),
            kind: format!("{:?}", message.kind),
            subject: message.subject.clone(),
            text: message.body.clone(),
        })
        .chain(
            state
                .chronicle
                .iter()
                .skip(cursor.chronicle_len)
                .map(|entry| GameplayFeedbackEvent {
                    source,
                    day: entry.day,
                    channel: "chronicle".to_owned(),
                    kind: format!("{:?}", entry.kind),
                    subject: String::new(),
                    text: entry.summary.clone(),
                }),
        )
        .collect::<Vec<_>>();
    events.sort_by(|left, right| {
        (
            left.day,
            &left.channel,
            &left.kind,
            &left.subject,
            &left.text,
        )
            .cmp(&(
                right.day,
                &right.channel,
                &right.kind,
                &right.subject,
                &right.text,
            ))
    });
    events.truncate(MAX_EVENTS);
    events
}

pub(crate) fn record_choice_cycle_metrics(
    accumulator: &mut CampaignAccumulator,
    substantive_candidate_count: usize,
    probe: &ProbeResult,
) -> ChoiceCycleMetrics {
    let family_quality = AlternativeQuality::from_observations(
        probe.viable_command_kinds.len(),
        probe.family_close_choice_score_gap,
        probe.distinct_immediate_family_profiles,
        probe.distinct_projected_family_profiles,
    );
    let option_quality = AlternativeQuality::from_observations(
        probe.substantive_viable_count,
        probe.close_choice_score_gap,
        probe.distinct_immediate_choice_profiles,
        probe.distinct_projected_choice_profiles,
    );
    accumulator.total_viable_choices = accumulator
        .total_viable_choices
        .saturating_add(usize_to_u32(probe.substantive_viable_count));
    accumulator.total_viable_command_kinds = accumulator
        .total_viable_command_kinds
        .saturating_add(usize_to_u32(probe.viable_command_kinds.len()));
    if probe.substantive_viable_count > 0 {
        accumulator.cycles_with_viable_choices =
            accumulator.cycles_with_viable_choices.saturating_add(1);
    } else {
        accumulator.no_action_cycles = accumulator.no_action_cycles.saturating_add(1);
        if substantive_candidate_count == 0 {
            accumulator.quiet_cycles = accumulator.quiet_cycles.saturating_add(1);
        } else {
            accumulator.blocked_cycles = accumulator.blocked_cycles.saturating_add(1);
        }
    }
    if probe.viable_command_kinds.len() >= 2 {
        accumulator.cycles_with_multiple_viable_command_kinds = accumulator
            .cycles_with_multiple_viable_command_kinds
            .saturating_add(1);
    }
    if family_quality.contains(AlternativeQuality::CLOSE) {
        accumulator.cycles_with_close_viable_command_kinds = accumulator
            .cycles_with_close_viable_command_kinds
            .saturating_add(1);
    }
    if family_quality.contains(AlternativeQuality::DISTINCT_IMMEDIATE) {
        accumulator.cycles_with_distinct_immediate_consequences = accumulator
            .cycles_with_distinct_immediate_consequences
            .saturating_add(1);
    }
    if family_quality.contains(AlternativeQuality::DISTINCT_PROJECTED) {
        accumulator.cycles_with_distinct_projected_consequences = accumulator
            .cycles_with_distinct_projected_consequences
            .saturating_add(1);
    }
    if option_quality.contains(AlternativeQuality::MULTIPLE) {
        accumulator.cycles_with_multiple_viable_options = accumulator
            .cycles_with_multiple_viable_options
            .saturating_add(1);
    }
    if option_quality.contains(AlternativeQuality::CLOSE) {
        accumulator.cycles_with_close_viable_options = accumulator
            .cycles_with_close_viable_options
            .saturating_add(1);
    }
    if option_quality.contains(AlternativeQuality::DISTINCT_IMMEDIATE) {
        accumulator.cycles_with_distinct_immediate_option_consequences = accumulator
            .cycles_with_distinct_immediate_option_consequences
            .saturating_add(1);
    }
    if option_quality.contains(AlternativeQuality::DISTINCT_PROJECTED) {
        accumulator.cycles_with_distinct_projected_option_consequences = accumulator
            .cycles_with_distinct_projected_option_consequences
            .saturating_add(1);
    }
    ChoiceCycleMetrics {
        substantive_candidate_count,
        substantive_viable_count: probe.substantive_viable_count,
        viable_command_kind_count: probe.viable_command_kinds.len(),
        family_quality,
        option_quality,
    }
}

pub(crate) fn apply_notification_housekeeping(
    registry: &Registry,
    state: &mut AppState,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    let unread = state
        .outbox
        .iter()
        .filter(|message| !message.acknowledged)
        .count();
    if unread < NOTIFICATION_BATCH_THRESHOLD {
        return Ok(());
    }
    // Acknowledge the whole backlog in one housekeeping pass: acknowledging a
    // single message per cycle lets unread mail grow without bound between
    // decisions, which buries player-facing signals under stale notices.
    let mut acknowledged = 0_u32;
    while let Some(message_id) = state
        .outbox
        .iter()
        .rev()
        .find(|message| !message.acknowledged)
        .map(|message| message.id)
    {
        apply_player_command(
            registry,
            state,
            PlayerCommand::AcknowledgeNotification { message_id },
        )
        .map_err(|source| GameplayHarnessError::SelectedCommandRejected {
            description: format!(
                "acknowledge {unread} notifications through notification {message_id}"
            ),
            source,
        })?;
        acknowledged = acknowledged.saturating_add(1);
    }
    let command_stats = accumulator
        .commands
        .get_mut(&GameplayCommandKind::AcknowledgeNotification)
        .expect("acknowledgement statistics must exist");
    // Housekeeping acknowledgements execute mechanically before any decision
    // cycle, so only execution is credited here. Feedback and persistence
    // counters stay reserved for commands measured against an action/no-action
    // baseline like every other family.
    command_stats.offered_cycles = command_stats.offered_cycles.saturating_add(1);
    command_stats.generated = command_stats.generated.saturating_add(1);
    command_stats.considered = command_stats.considered.saturating_add(1);
    command_stats.viable = command_stats.viable.saturating_add(1);
    command_stats.executed = command_stats.executed.saturating_add(acknowledged);
    command_stats
        .changed_domains
        .insert(GameplayDomain::Feedback);
    accumulator.record_executed_command(
        GameplayCommandKind::AcknowledgeNotification,
        state.clock.day(),
    );
    Ok(())
}

pub(crate) fn gameplay_phase(arc: &GameplayFantasyArc) -> GameplayPhase {
    // The phase ladder follows the dynasty's durable milestones rather than
    // capping at the first succession: a house whose head dies before it ever
    // shaped the city is still climbing the establishment-to-ascent arc under
    // its heir, and only a governing dynasty navigating life after succession
    // has entered the legacy era the design reserves for testing whether the
    // organization outlives its founder.
    if arc.first_succession_day.is_some() && arc.first_city_shaping_action_day.is_some() {
        GameplayPhase::SuccessionLegacy
    } else if arc.first_city_shaping_action_day.is_some() {
        GameplayPhase::DynasticGovernance
    } else if arc.first_commercial_standing_day.is_some() {
        GameplayPhase::InstitutionalAscent
    } else if arc.first_reputation_standing_day.is_some() {
        GameplayPhase::Establishment
    } else {
        GameplayPhase::Foundation
    }
}

pub(crate) fn consequence_horizon_days(
    command: Option<GameplayCommandKind>,
    step_days: u32,
    maximum: u16,
) -> u32 {
    let desired = match command {
        Some(
            GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::ConveneFamilyCouncil
            | GameplayCommandKind::DesignateHeir
            | GameplayCommandKind::AdoptWard
            | GameplayCommandKind::EducateFamilyMember
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FundPublicWork,
        ) => 360,
        Some(GameplayCommandKind::NominateForOffice) => 120,
        Some(
            GameplayCommandKind::CultivateInstitutionSupport
            | GameplayCommandKind::EndowInstitution
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SettleLegalCase
            | GameplayCommandKind::EnactLaw
            | GameplayCommandKind::ExerciseOfficePower
            | GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::SetBusinessWages
            | GameplayCommandKind::WithdrawFromInstitution,
        ) => 180,
        Some(
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::WithdrawBusinessCash
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SecureSupply
            | GameplayCommandKind::SellOutput
            | GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::ExtendCredit
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::LeverageInformation,
        ) => 30,
        Some(
            GameplayCommandKind::ResolveLaborDispute | GameplayCommandKind::AcknowledgeNotification,
        )
        | None => step_days,
    };
    desired.min(u32::from(maximum)).max(step_days)
}

pub(crate) fn select_probe_candidates(candidates: Vec<Candidate>, limit: usize) -> Vec<Candidate> {
    if limit == 0 {
        return Vec::new();
    }
    let mut seen_kinds = BTreeSet::new();
    let mut family_leaders = Vec::new();
    let mut additional_variants = Vec::new();
    for candidate in candidates {
        if seen_kinds.insert(candidate.kind) {
            family_leaders.push(candidate);
        } else {
            additional_variants.push(candidate);
        }
    }
    family_leaders
        .into_iter()
        .chain(additional_variants)
        .take(limit)
        .collect()
}

pub(crate) fn summarize_ranked_candidates(
    candidates: &[Candidate],
) -> Vec<GameplayCandidateRanking> {
    let mut seen = BTreeSet::new();
    candidates
        .iter()
        .filter(|candidate| seen.insert(candidate.kind))
        .take(5)
        .map(|candidate| GameplayCandidateRanking {
            command: candidate.kind,
            score: candidate.score,
            description: candidate.description.clone(),
        })
        .collect()
}

#[derive(Debug)]
pub(crate) struct ExecutedAction {
    pub kind: GameplayCommandKind,
    pub description: String,
    pub outcome: String,
}

pub(crate) fn record_generated_candidates(
    candidates: &[Candidate],
    accumulator: &mut CampaignAccumulator,
) {
    for candidate in candidates {
        let command_stats = accumulator
            .commands
            .get_mut(&candidate.kind)
            .expect("every command kind must have statistics");
        command_stats.generated = command_stats.generated.saturating_add(1);
    }
}

/// Pure world-state activation set: every command kind whose canonical
/// validation route would accept some concrete action in this state. Reactive
/// predicates include the command's executable resource and cooldown gates;
/// world predicates mirror each family's canonical validation. Agent policy
/// (reserves, portfolio caps, persona targeting) never appears here.
pub(crate) fn pure_world_activation_set(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
) -> BTreeSet<GameplayCommandKind> {
    let mut active = BTreeSet::new();
    let crisis_opportunity = state.crises.values().any(|crisis| {
        crisis.status.is_active()
            && !crisis_has_containment_response(state, crisis.id)
            && crisis_responses(persona).into_iter().any(|response| {
                (response != CrisisResponse::Exploit || !crisis_was_exploited(state, crisis.id))
                    && can_afford_crisis_response(state, crisis, response)
            })
    });
    if crisis_opportunity {
        active.insert(GameplayCommandKind::RespondToCrisis);
    }
    let labor_opportunity = state.employment.values().any(|agreement| {
        agreement.status == EmploymentStatus::Disputed
            && state
                .businesses
                .get(agreement.business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
            && preferred_labor_response(state, agreement, persona).is_some()
    });
    if labor_opportunity {
        active.insert(GameplayCommandKind::ResolveLaborDispute);
    }
    for (kind, available) in [
        (
            GameplayCommandKind::FileLegalCase,
            has_legal_filing_opportunity(state),
        ),
        (
            GameplayCommandKind::SettleLegalCase,
            has_legal_settlement_opportunity(state),
        ),
        (
            GameplayCommandKind::SellProperty,
            has_property_liquidation_opportunity(registry, state),
        ),
        (
            GameplayCommandKind::WithdrawFromInstitution,
            has_institution_withdrawal_opportunity(state),
        ),
        (
            GameplayCommandKind::ExtendCredit,
            has_extend_credit_opportunity(registry, state),
        ),
        (
            GameplayCommandKind::TransferBusinessCash,
            has_transfer_cash_opportunity(state),
        ),
        (
            GameplayCommandKind::WithdrawBusinessCash,
            has_withdrawal_cash_opportunity(registry, state),
        ),
    ] {
        if available {
            active.insert(kind);
        }
    }
    for kind in ALL_COMMAND_KINDS.iter().copied() {
        if !matches!(
            kind,
            GameplayCommandKind::RespondToCrisis
                | GameplayCommandKind::ResolveLaborDispute
                | GameplayCommandKind::FileLegalCase
                | GameplayCommandKind::SettleLegalCase
                | GameplayCommandKind::SellProperty
                | GameplayCommandKind::WithdrawFromInstitution
                | GameplayCommandKind::ExtendCredit
                | GameplayCommandKind::TransferBusinessCash
                | GameplayCommandKind::WithdrawBusinessCash
        ) && has_world_opportunity(registry, state, persona, kind)
        {
            active.insert(kind);
        }
    }
    active
}

pub(crate) fn record_activation_opportunities(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    accumulator: &mut CampaignAccumulator,
    generated_kinds: &BTreeSet<GameplayCommandKind>,
) {
    // A generated candidate also counts as an offered action: the generator is
    // part of the agent, so its construction proves an actionable route even
    // where a predicate's executable-resource gate reads narrower.
    let mut world = pure_world_activation_set(registry, state, persona);
    for kind in generated_kinds {
        world.insert(*kind);
    }
    for kind in ALL_COMMAND_KINDS.iter().copied() {
        record_activation_opportunity(accumulator, kind, world.contains(&kind));
    }
}

/// World-state activation predicate for every command family that does not have
/// a dedicated reactive predicate above. Each branch is an independent read-only
/// check of whether the canonical game would accept some concrete action of that
/// kind -- so a quiet cycle is labelled a generator gap when the world offers an
/// action the candidate generator failed to build, rather than being misread as
/// a dormant world with nothing to do.
pub(crate) fn has_world_opportunity(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    kind: GameplayCommandKind,
) -> bool {
    match kind {
        GameplayCommandKind::SecureSupply => has_secure_supply_opportunity(registry, state),
        GameplayCommandKind::SetBusinessWages => has_business_wage_opportunity(state),
        GameplayCommandKind::SellOutput => has_sell_output_opportunity(registry, state),
        GameplayCommandKind::BuyProperty => has_buy_property_opportunity(state),
        GameplayCommandKind::EnactLaw => has_enact_law_opportunity(registry, state),
        GameplayCommandKind::StartPublicWork => has_start_public_work_opportunity(registry, state),
        GameplayCommandKind::FundPublicWork => has_fund_public_work_opportunity(state),
        GameplayCommandKind::SetHouseGovernance => has_governance_opportunity(state),
        GameplayCommandKind::ConveneFamilyCouncil => has_family_council_opportunity(state),
        GameplayCommandKind::DesignateHeir => has_heir_designation_opportunity(state),
        GameplayCommandKind::AdoptWard => has_ward_adoption_opportunity(state),
        GameplayCommandKind::EducateFamilyMember => has_family_education_opportunity(state),
        GameplayCommandKind::CultivateInstitutionSupport => {
            has_institution_support_opportunity(registry, state)
        }
        GameplayCommandKind::EndowInstitution => has_institution_endowment_opportunity(state),
        GameplayCommandKind::NominateForOffice => {
            has_office_nomination_opportunity(registry, state)
        }
        GameplayCommandKind::ExerciseOfficePower => has_office_power_opportunity(state),
        GameplayCommandKind::CommissionInformation => {
            has_information_commission_opportunity(registry, state, persona)
        }
        GameplayCommandKind::LeverageInformation => {
            has_information_leverage_opportunity(registry, state)
        }
        GameplayCommandKind::BorrowFunds => has_borrow_opportunity(state),
        GameplayCommandKind::AcknowledgeNotification => {
            has_notification_acknowledgement_opportunity(state)
        }
        GameplayCommandKind::RespondToCrisis
        | GameplayCommandKind::ResolveLaborDispute
        | GameplayCommandKind::FileLegalCase
        | GameplayCommandKind::SettleLegalCase
        | GameplayCommandKind::SellProperty
        | GameplayCommandKind::WithdrawFromInstitution
        | GameplayCommandKind::ExtendCredit
        | GameplayCommandKind::TransferBusinessCash
        | GameplayCommandKind::WithdrawBusinessCash => false,
        GameplayCommandKind::AcquireBusiness
        | GameplayCommandKind::InvestInBusiness
        | GameplayCommandKind::SetBusinessPolicy => has_business_opportunity(registry, state, kind),
    }
}

pub(crate) fn is_open_business(business: &crate::core::Business) -> bool {
    !matches!(
        business.status(),
        BusinessStatus::Closed | BusinessStatus::Insolvent
    )
}

/// Iterates audit records from newest to oldest, stopping before the first
/// record older than `earliest_day`.
///
/// Audit entries are append-only with chronologically nondecreasing days (an
/// enforced invariant), so a record older than the cutoff cannot be followed
/// by a newer one. Cooldown questions of the form "does a matching record
/// exist with `day + interval > today`?" are therefore decided entirely
/// inside the window, and scans stay proportional to the window instead of
/// the campaign's full history. The window must include the boundary: a
/// cooldown of `interval` days covers `day >= today - (interval - 1)`.
pub(crate) fn audit_records_from(
    state: &AppState,
    earliest_day: i64,
) -> impl Iterator<Item = &AuditRecord> {
    state
        .audit_log
        .iter()
        .rev()
        .take_while(move |record| record.day() >= earliest_day)
}

/// Newest-to-oldest audit records still inside an `interval`-day cooldown,
/// i.e. exactly those records for which `today < day + interval` holds.
pub(crate) fn audit_records_within_cooldown(
    state: &AppState,
    interval_days: i64,
) -> impl Iterator<Item = &AuditRecord> {
    audit_records_from(state, state.clock.day().saturating_sub(interval_days - 1))
}

/// Mirrors the canonical route (`apply_business_wages`): an owned, open
/// business with a workforce is off cooldown and accepts a wage change.
/// The agent's wage posture and material-change threshold live in the
/// candidate generator, so a changeable workforce is never misread as dormant.
pub(crate) fn has_business_wage_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    state
        .businesses
        .ids_for_owner(player_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .filter(|business| is_open_business(business))
        .any(|business| {
            let subject = format!("business:{}", business.id());
            audit_records_within_cooldown(state, BUSINESS_WAGE_CHANGE_INTERVAL_DAYS)
                .find(|record| {
                    record.kind() == crate::core::AuditKind::BusinessWageChange
                        && record.subject() == subject
                })
                .is_none()
                && state
                    .employment
                    .values()
                    .any(|agreement| agreement.business_id() == business.id())
        })
}

pub(crate) fn has_secure_supply_opportunity(registry: &Registry, state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() == player_id && is_open_business(business))
        .any(|business| {
            let recipe = registry
                .get_recipe(business.recipe_id())
                .expect("business recipe must resolve");
            recipe.inputs().iter().any(|input| {
                let Some(quote) = state.market.quotes.get(&input.good_id()) else {
                    return false;
                };
                contract_sellers(registry, state, input.good_id(), player_id).any(|seller| {
                    // Mirror the executable route: the generator dedupes active
                    // contracts per buyer-seller-good pair and offers the same
                    // weekly batch quantity, so a contracted pair or a pair
                    // without capacity is not an executable opportunity.
                    if state.contracts.values().any(|contract| {
                        contract.status == ContractStatus::Active
                            && contract.buyer_business_id == business.id()
                            && contract.seller_business_id == seller
                            && contract.good_id == input.good_id()
                    }) {
                        return false;
                    }
                    let unit_price =
                        contract_candidate_unit_price(state, business.id(), seller, quote.price);
                    game_accepts_contract_terms(
                        registry,
                        state,
                        business.id(),
                        seller,
                        input.good_id(),
                        input
                            .quantity()
                            .saturating_mul_ratio(secure_supply_batches(business), 1),
                        unit_price,
                    )
                })
            })
        })
}

pub(crate) fn has_sell_output_opportunity(registry: &Registry, state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() == player_id && is_open_business(business))
        .any(|business| {
            let recipe = registry
                .get_recipe(business.recipe_id())
                .expect("business recipe must resolve");
            let Some(quote) = state.market.quotes.get(&recipe.output_good_id()) else {
                return false;
            };
            contract_buyers(registry, state, recipe.output_good_id(), player_id).any(|buyer| {
                // Mirror the executable route (see `has_secure_supply_opportunity`).
                if state.contracts.values().any(|contract| {
                    contract.status == ContractStatus::Active
                        && contract.buyer_business_id == buyer
                        && contract.seller_business_id == business.id()
                        && contract.good_id == recipe.output_good_id()
                }) {
                    return false;
                }
                let unit_price =
                    contract_candidate_unit_price(state, buyer, business.id(), quote.price);
                game_accepts_contract_terms(
                    registry,
                    state,
                    buyer,
                    business.id(),
                    recipe.output_good_id(),
                    recipe
                        .output_quantity()
                        .saturating_mul_ratio(STANDARD_CONTRACT_BATCHES_PER_WEEK, 1),
                    unit_price,
                )
            })
        })
}

pub(crate) fn has_buy_property_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let Some(player) = state.dynasties.get(&player_id) else {
        return false;
    };
    // The canonical route (`buy_unowned_property`) accepts the purchase of any
    // unowned property the treasury can cover; the yield hurdle and liquidity
    // floor are the agent's investment policy and live in the generator. The
    // activation predicate mirrors the game so an affordable property is never
    // misread as dormant just because the persona declined a low-yield asset.
    let treasury = player.treasury();
    treasury > Money::ZERO
        && state
            .properties
            .values()
            .any(|property| property.owner_dynasty_id.is_none() && property.value <= treasury)
}

pub(crate) fn has_enact_law_opportunity(registry: &Registry, state: &AppState) -> bool {
    if !has_player_office(state) {
        return false;
    }
    let player_id = state.player_dynasty_id;
    let sponsorship_available = state
        .laws
        .values()
        .filter(|law| law.sponsor_dynasty_id == Some(player_id))
        .map(|law| law.enacted_day)
        .max()
        .is_none_or(|day| state.clock.day() >= day.saturating_add(LAW_SPONSORSHIP_INTERVAL_DAYS));
    let has_legitimacy = state.dynasties.get(&player_id).is_some_and(|dynasty| {
        dynasty.resources.legitimacy_basis_points >= LAW_LEGITIMACY_REQUIREMENT
    });
    let treasury_ok = state
        .dynasties
        .get(&player_id)
        .is_some_and(|dynasty| dynasty.treasury() >= LAW_SPONSORSHIP_COST);
    if !sponsorship_available || !has_legitimacy || !treasury_ok {
        return false;
    }
    law_candidates(registry, state).iter().any(|(kind, value)| {
        has_established_player_office_power(state, required_office_power_for_law(*kind))
            && !state
                .laws
                .values()
                .any(|law| law.active && law.kind == *kind && law.value == *value)
            && (*kind != LawKind::PublicDebtAuthorization
                || civic_debt_creditor_available(state, *value))
    })
}

pub(crate) fn civic_debt_creditor_available(state: &AppState, principal_copper: i64) -> bool {
    let principal = Money::from_copper(principal_copper);
    state.dynasties.values().any(|dynasty| {
        dynasty.id() != state.player_dynasty_id
            && dynasty
                .treasury()
                .saturating_sub(CIVIC_DEBT_CREDITOR_RESERVE)
                >= principal
    })
}

pub(crate) fn has_start_public_work_opportunity(registry: &Registry, state: &AppState) -> bool {
    if !has_established_player_office_power(state, OfficePower::PublicWorks) {
        return false;
    }
    let player_id = state.player_dynasty_id;
    let active_sponsored = state
        .public_works
        .values()
        .filter(|work| work.sponsor_dynasty_id == Some(player_id) && work.status.is_unfinished())
        .count();
    if active_sponsored >= MAX_ACTIVE_SPONSORED_PUBLIC_WORKS {
        return false;
    }
    let work_kinds = [
        PublicWorkKind::Road,
        PublicWorkKind::Bridge,
        PublicWorkKind::Market,
        PublicWorkKind::Granary,
        PublicWorkKind::Drainage,
        PublicWorkKind::WatchStation,
        PublicWorkKind::Hospital,
        PublicWorkKind::School,
    ];
    let has_open_slot = registry.districts().iter().any(|district| {
        work_kinds.iter().any(|kind| {
            !state.public_works.values().any(|work| {
                work.district_id == district.id()
                    && work.kind == *kind
                    && work.status.is_unfinished()
            })
        })
    });
    if !has_open_slot {
        return false;
    }
    let subject = format!("dynasty:{player_id}");
    let sponsorship_available =
        audit_records_within_cooldown(state, PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS)
            .find(|record| {
                record.kind() == AuditKind::PublicWorkStarted && record.subject() == subject
            })
            .is_none();
    sponsorship_available
        && state.dynasties.get(&player_id).is_some_and(|dynasty| {
            dynasty.treasury() >= public_work_initial_contribution(CANDIDATE_PUBLIC_WORK_BUDGET)
        })
}

pub(crate) fn has_fund_public_work_opportunity(state: &AppState) -> bool {
    // The canonical funding route (`quote_public_work_funding`) accepts any
    // positive contribution to any unfinished public work up to its remaining
    // budget when the dynasty can afford it — the dynasty's own projects, a
    // rival house's project, or a city-sponsored one. Whether the agent
    // *chooses* to accelerate or rescue a work is a policy decision kept in the
    // candidate generator; the activation predicate must not hide a world in
    // which the game accepts funding.
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .map_or(Money::ZERO, crate::core::Dynasty::treasury);
    state.public_works.values().any(|work| {
        work.status.is_unfinished()
            && work.budget.saturating_sub(work.spent) > Money::ZERO
            && treasury > Money::ZERO
    })
}

pub(crate) fn has_governance_opportunity(state: &AppState) -> bool {
    let Some(council) = state.family_councils.get(&state.player_dynasty_id) else {
        return false;
    };
    let governance_subject = format!("dynasty:{}", state.player_dynasty_id);
    let governance_available =
        audit_records_within_cooldown(state, HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS)
            .find(|record| {
                record.kind() == AuditKind::HouseGovernanceChange
                    && record.subject() == governance_subject
            })
            .is_none();
    // The canonical route (`apply_governance`) accepts any change to a
    // different governance model while the house can pay its unity price and
    // is off cooldown; which model the agent *prefers* is generator policy,
    // so the activation predicate stays policy-free.
    governance_available && council.unity_basis_points >= HOUSE_GOVERNANCE_UNITY_COST
}

pub(crate) fn has_family_council_opportunity(state: &AppState) -> bool {
    // The canonical route (`apply_family_council_meeting`) accepts a meeting
    // whenever the dynasty can pay and is off the council cooldown; it has no
    // unity gate. The agent's low-unity intervention policy lives in the
    // candidate generator; the activation predicate mirrors the game so a
    // healthy but affordable council is never misread as dormant.
    let player_id = state.player_dynasty_id;
    if !state.family_councils.contains_key(&player_id) {
        return false;
    }
    if state
        .dynasties
        .get(&player_id)
        .is_none_or(|dynasty| dynasty.treasury() < FAMILY_COUNCIL_MEETING_COST)
    {
        return false;
    }
    let subject = format!("dynasty:{player_id};council-meeting");
    audit_records_within_cooldown(state, FAMILY_COUNCIL_MEETING_INTERVAL_DAYS)
        .find(|record| {
            record.kind() == AuditKind::FamilyCouncilMeeting && record.subject() == subject
        })
        .is_none()
}

pub(crate) fn has_heir_designation_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let Some(dynasty) = state.dynasties.get(&player_id) else {
        return false;
    };
    if dynasty.resources.legitimacy_basis_points < HEIR_DESIGNATION_LEGITIMACY_COST {
        return false;
    }
    let designation_subject = format!("dynasty:{player_id}");
    let designation_available =
        audit_records_within_cooldown(state, HEIR_DESIGNATION_INTERVAL_DAYS)
            .find(|record| {
                record.kind() == AuditKind::HeirDesignation
                    && record.subject() == designation_subject
            })
            .is_none();
    if !designation_available {
        return false;
    }
    let council = state
        .family_councils
        .get(&player_id)
        .expect("player dynasty must own a family council");
    let head_id = dynasty.head_id();
    // Mirror the canonical route (`validate_heir_designation`): an eligible
    // council member who is not the head, is active, and is at least eighteen
    // can be designated when the house can pay the legitimacy and unity costs.
    // Confirming the existing heir is only possible once, so a prior
    // designation removes the confirmation route but not a genuine replacement
    // designation; the agent's head-age and succession-risk gates are policy
    // and live in the candidate generator.
    council.unity_basis_points >= HEIR_DESIGNATION_UNITY_COST
        && council.members.iter().any(|character_id| {
            state
                .characters
                .get(*character_id)
                .is_some_and(|character| {
                    *character_id != head_id
                        && character.dynasty_id() == player_id
                        && character.status() == CharacterStatus::Active
                        && state.clock.day().saturating_sub(character.birth_day()) >= 18 * 360
                })
        })
}

pub(crate) fn has_ward_adoption_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let Some(player) = state.dynasties.get(&player_id) else {
        return false;
    };
    player.treasury() >= WARD_ADOPTION_COST
        && player.resources.legitimacy_basis_points >= WARD_ADOPTION_LEGITIMACY_REQUIREMENT
        && state
            .family_councils
            .get(&player_id)
            .is_some_and(|council| council.unity_basis_points >= WARD_ADOPTION_UNITY_COST)
        && player
            .resources
            .reputation_quality_basis_points
            .max(player.resources.reputation_reliability_basis_points)
            >= WARD_ADOPTION_REPUTATION_REQUIREMENT
        && player_contract_deliveries(state) >= WARD_ADOPTION_DELIVERY_REQUIREMENT
        && active_player_ward_count(state) < MAX_ACTIVE_WARDS
        && {
            // Hoisted so the windowed scan never allocates per visited record.
            let subject_prefix = format!("dynasty:{player_id}:");
            audit_records_within_cooldown(state, WARD_ADOPTION_INTERVAL_DAYS)
                .find(|record| {
                    record.kind() == AuditKind::WardAdoption
                        && record.subject().starts_with(&subject_prefix)
                })
                .is_none()
        }
}

pub(crate) fn has_family_education_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let affordable = state
        .dynasties
        .get(&player_id)
        .is_some_and(|player| player.treasury() >= FAMILY_EDUCATION_COST);
    if !affordable {
        return false;
    }
    state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == player_id && character.status() == CharacterStatus::Active
        })
        .any(|character| {
            // Any trainable capability below 100 qualifies; persona-specific
            // focus ordering only matters when generating candidates.
            ALL_EDUCATION_FOCUSSES.into_iter().any(|focus| {
                character_focus_value(character, focus) < 100
                    && family_education_next_day(state, character.id())
                        .is_none_or(|day| state.clock.day() >= day)
            })
        })
}

pub(crate) fn has_institution_support_opportunity(registry: &Registry, state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let Some(player) = state.dynasties.get(&player_id) else {
        return false;
    };
    // Mirror the canonical route (`apply_institution_support`): an active
    // player character can join an institution when the house passes the
    // standing and commercial-record gates, the character is not already a
    // member, membership capacity is open, the pair is off cooldown, and the
    // treasury covers the guild-restriction-surcharged contribution. Which
    // institution the agent targets is generator policy.
    let entry_restriction = crate::systems::active_law_value(state, LawKind::GuildEntryRestriction)
        .unwrap_or(0)
        .clamp(0, 10_000);
    let support_cost =
        INSTITUTION_SUPPORT_COST.saturating_mul_ratio(10_000 + entry_restriction / 2, 10_000);
    if player.treasury() < support_cost {
        return false;
    }
    let best_reputation = player
        .resources
        .reputation_quality_basis_points
        .max(player.resources.reputation_reliability_basis_points);
    if best_reputation < INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT {
        return false;
    }
    let delivered = player_contract_deliveries(state);
    state.institutions.values().any(|institution| {
        let institution_id = institution.institution_id;
        state.characters.iter().any(|character| {
            character.dynasty_id() == player_id
                && character.status() == CharacterStatus::Active
                && !institution.members.contains(&character.id())
                && institution_membership_count(state, character.id())
                    < MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER
                && institution_support_next_day(state, institution_id, character.id())
                    .is_none_or(|day| state.clock.day() >= day)
                && delivered
                    >= institution_support_delivery_requirement(
                        registry,
                        state,
                        institution_id,
                        character.id(),
                    )
        })
    })
}

pub(crate) fn has_institution_endowment_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let affordable = state
        .dynasties
        .get(&player_id)
        .is_some_and(|player| player.treasury() >= INSTITUTION_ENDOWMENT_MIN);
    if !affordable {
        return false;
    }
    let has_membership = state.institutions.values().any(|institution| {
        has_established_player_institution_membership(state, institution.institution_id)
    });
    has_membership
        && institution_endowment_next_day(state).is_none_or(|day| state.clock.day() >= day)
}

pub(crate) fn has_office_nomination_opportunity(registry: &Registry, state: &AppState) -> bool {
    let characters = eligible_office_characters(state);
    state.institutions.values().any(|institution| {
        let institution_kind = registry
            .get_institution(institution.institution_id)
            .expect("runtime institution must have a registry definition")
            .kind();
        strongest_office_nominee(registry, state, institution, &characters, institution_kind)
            .is_some()
    })
}

pub(crate) fn has_office_power_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let has_legitimacy = state.dynasties.get(&player_id).is_some_and(|player| {
        player.resources.legitimacy_basis_points >= OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST
    });
    if !has_legitimacy {
        return false;
    }
    state.institutions.values().any(|institution| {
        let held_by_player = institution.office_holder_id.is_some_and(|character_id| {
            state.characters.get(character_id).is_some_and(|character| {
                character.status() == CharacterStatus::Active && character.dynasty_id() == player_id
            })
        });
        // The canonical route (`validate_office_power_directive`) requires the
        // exercised power to belong to the institution, so an office with no
        // powers cannot offer a directive.
        held_by_player
            && !institution.powers.is_empty()
            && office_power_directive_available(state, institution.institution_id)
            && state.clock.day()
                >= institution
                    .term_started_day
                    .saturating_add(OFFICE_POWER_ESTABLISHMENT_DAYS)
    })
}

pub(crate) fn has_information_commission_opportunity(
    _registry: &Registry,
    state: &AppState,
    _persona: GameplayPersona,
) -> bool {
    let player_id = state.player_dynasty_id;
    // The canonical commission route gates on affordability and the fixed
    // `INFORMATION_COMMISSION_INTERVAL_DAYS` cooldown only; it accepts any
    // syntactically valid focus (market good, district, or counterparty).
    // The agent's information thesis and persona-specific cadence live in the
    // generator, so the activation predicate must not require the thesis to be
    // material — otherwise a calm campaign would be misread as dormant just
    // because the agent chose not to commission.
    let affordable = state
        .dynasties
        .get(&player_id)
        .is_some_and(|player| player.treasury() >= INFORMATION_COMMISSION_COST);
    if !affordable {
        return false;
    }
    let report_commission_day = state
        .information_reports
        .values()
        .filter(|report| {
            report.owner_dynasty_id == player_id && report.source == COMMISSIONED_INFORMATION_SOURCE
        })
        .map(|report| report.created_day)
        .max();
    let audit_subject = format!("dynasty:{player_id}");
    let audit_commission_day = state
        .audit_log
        .iter()
        .filter(|record| {
            record.kind() == AuditKind::InformationCommission && record.subject() == audit_subject
        })
        .map(AuditRecord::day)
        .max();
    report_commission_day
        .max(audit_commission_day)
        .is_none_or(|day| {
            state.clock.day() >= day.saturating_add(INFORMATION_COMMISSION_INTERVAL_DAYS)
        })
}

pub(crate) fn has_information_leverage_opportunity(registry: &Registry, state: &AppState) -> bool {
    // Mirror the canonical leverage route (`resolve_information_leverage`): the
    // report must be owned, commissioned, confirmed, and unexpired, the dynasty
    // must afford the leverage cost, and the leverage plan must actually resolve
    // (an active market contract, a counterparty brief, or a district initiative).
    let affordable = state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_some_and(|player| player.treasury() >= INFORMATION_LEVERAGE_COST);
    if !affordable {
        return false;
    }
    state.information_reports.values().any(|report| {
        report.owner_dynasty_id == state.player_dynasty_id
            && report.source == COMMISSIONED_INFORMATION_SOURCE
            && report.confidence == crate::core::InformationConfidence::Confirmed
            && state.clock.day() <= report.expires_day
            && quote_information_leverage(registry, state, report.id()).is_ok()
    })
}

pub(crate) fn has_borrow_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    if !state.dynasties.contains_key(&player_id) {
        return false;
    }
    if state
        .loans
        .values()
        .any(|loan| loan.borrower_dynasty_id == player_id && loan.status.is_repayment_active())
    {
        return false;
    }
    // An aged default creates a canonical workout opportunity with its
    // existing creditor even when no new cash can be advanced.
    if state.loans.values().any(|loan| {
        loan.borrower_dynasty_id == player_id && defaulted_loan_restructuring_available(state, loan)
    }) {
        return true;
    }
    // Fresh credit is evaluated per counterparty: a default to one house
    // blocks only that pair until the workout window opens, while
    // unrelated lenders remain canonically available. The agent's policy
    // keeps shopping a fresh advance while a default exists, but the
    // activation predicate must mirror the game's pair-scoped gate instead
    // of treating any default as a global borrowing ban.
    state.dynasties.values().any(|dynasty| {
        dynasty.id() != player_id
            && !credit_pair_blocks_new_loan(state, dynasty.id(), player_id)
            && dynasty
                .treasury()
                .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
                .is_some_and(|available| available >= Money::from_copper(1_000))
    })
}

pub(crate) fn has_notification_acknowledgement_opportunity(state: &AppState) -> bool {
    // The canonical route (`acknowledge`) accepts acknowledgement of any
    // existing message; the batch threshold is the agent's housekeeping policy
    // in the generator. The activation predicate mirrors the game so an unread
    // notification is never misread as dormant.
    state.outbox.iter().any(|message| !message.acknowledged)
}

pub(crate) fn has_business_opportunity(
    registry: &Registry,
    state: &AppState,
    kind: GameplayCommandKind,
) -> bool {
    match kind {
        GameplayCommandKind::InvestInBusiness => has_business_investment_opportunity(state),
        GameplayCommandKind::SetBusinessPolicy => has_business_policy_opportunity(state),
        GameplayCommandKind::AcquireBusiness => {
            has_business_acquisition_opportunity(registry, state)
        }
        _ => false,
    }
}

pub(crate) fn has_business_investment_opportunity(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let treasury = state
        .dynasties
        .get(&player_id)
        .map_or(Money::ZERO, crate::core::Dynasty::treasury);
    // The canonical route (`apply_business_investment`) accepts a positive
    // investment into any player-owned, non-closed business that the treasury
    // can cover, for every persona. Whether the agent *chooses* to modernize or
    // recapitalize is a policy decision in the candidate generator; the
    // activation predicate mirrors the game so an investable business is never
    // misread as dormant just because the persona did not build a candidate.
    treasury > Money::ZERO
        && state.businesses.iter().any(|business| {
            business.owner_dynasty_id() == player_id
                && business.status() != BusinessStatus::Closed
                && business.cash().checked_add(treasury).is_some()
        })
}

pub(crate) fn has_business_policy_opportunity(state: &AppState) -> bool {
    state
        .businesses
        .iter()
        .filter(|business| {
            business.owner_dynasty_id() == state.player_dynasty_id && is_open_business(business)
        })
        .any(|business| {
            let policy_subject = format!("business:{}", business.id());
            // Windowed scan: a record outside the cooldown interval can never
            // change whether a change is available, so the newest-to-oldest
            // walk stops at the window boundary instead of the whole log.
            let policy_change_available =
                audit_records_within_cooldown(state, BUSINESS_POLICY_CHANGE_INTERVAL_DAYS).all(
                    |record| {
                        !(record.kind() == AuditKind::BusinessPolicyChange
                            && record.subject() == policy_subject)
                    },
                );
            // Mirror the canonical route (`apply_business_policy`): any policy
            // tuple distinct from the current one is accepted off cooldown.
            // Which tuple the agent prefers is generator policy, so the
            // predicate scans the template space without persona narrowing.
            policy_change_available
                && policy_templates(GameplayPersona::Steward)
                    .into_iter()
                    .any(|template| {
                        business.policy.target_input_days != template.target_input_days
                            || business.policy.target_output_days != template.target_output_days
                            || business.policy.minimum_cash_reserve != template.minimum_cash_reserve
                            || business.policy.maintenance_basis_points
                                != template.maintenance_basis_points
                            || business.policy.quality_target_basis_points
                                != template.quality_target_basis_points
                    })
        })
}

pub(crate) fn has_business_acquisition_opportunity(registry: &Registry, state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let treasury = state
        .dynasties
        .get(&player_id)
        .map_or(Money::ZERO, crate::core::Dynasty::treasury);
    // Mirror the canonical route (`quote_business_acquisition` plus the
    // purchase validation): any non-player firm is quotable — failing trades
    // at a discount, going concerns at the controlling premium — and the
    // canonical gate is affordability of price plus minimum recapitalization plus
    // an eligible manager. Portfolio caps, readiness screens, and expansion
    // reserves are agent policy and stay in the candidate generator.
    let has_eligible_manager = state.characters.iter().any(|character| {
        character.dynasty_id() == player_id
            && character.status() == CharacterStatus::Active
            && character.id()
                != state
                    .dynasties
                    .get(&player_id)
                    .map_or(character.id(), crate::core::Dynasty::head_id)
            && state.clock.day().saturating_sub(character.birth_day()) >= 18 * 360
    });
    if !has_eligible_manager {
        return false;
    }
    state.businesses.iter().any(|business| {
        business.owner_dynasty_id() != player_id
            && quote_business_acquisition(registry, state, player_id, business.id()).is_ok_and(
                |quote| {
                    let required = quote
                        .purchase_price
                        .saturating_add(quote.minimum_recapitalization);
                    treasury >= required
                },
            )
    })
}

pub(crate) fn record_activation_opportunity(
    accumulator: &mut CampaignAccumulator,
    kind: GameplayCommandKind,
    available: bool,
) {
    if !available {
        return;
    }
    let command_stats = accumulator
        .commands
        .get_mut(&kind)
        .expect("every command kind must have statistics");
    command_stats.activation_opportunities =
        command_stats.activation_opportunities.saturating_add(1);
}

pub(crate) fn activation_opportunity_snapshot(
    accumulator: &CampaignAccumulator,
) -> BTreeMap<GameplayCommandKind, u32> {
    accumulator
        .commands
        .iter()
        .map(|(kind, stats)| (*kind, stats.activation_opportunities))
        .collect()
}

pub(crate) fn activation_opportunity_delta(
    accumulator: &CampaignAccumulator,
    before: &BTreeMap<GameplayCommandKind, u32>,
) -> BTreeMap<GameplayCommandKind, u32> {
    accumulator
        .commands
        .iter()
        .filter_map(|(kind, stats)| {
            let prior = before.get(kind).copied().unwrap_or(0);
            (stats.activation_opportunities > prior)
                .then_some((*kind, stats.activation_opportunities.saturating_sub(prior)))
        })
        .collect()
}

/// Explains why a no-action cycle had no substantive choice by separating
/// generator gaps (an activation opportunity existed but no candidate was
/// built), policy gates (the agent's own spending filters declined built
/// candidates), and validation gates (the game rejected every probed option).
/// Returns a human-readable reason for the retained trace when the cycle had
/// no actionable choice, and `None` when the agent could act.
#[expect(
    clippy::too_many_lines,
    reason = "the dispatch keeps the full decision path in one auditable function"
)]
pub(crate) fn record_quiet_diagnostic(
    accumulator: &mut CampaignAccumulator,
    probe: &ProbeResult,
    raw_generated_kinds: &BTreeSet<GameplayCommandKind>,
    retained_kinds: &BTreeSet<GameplayCommandKind>,
    retained_counts_by_kind: &BTreeMap<GameplayCommandKind, usize>,
    probed_counts_by_kind: &BTreeMap<GameplayCommandKind, usize>,
    activation_delta: &BTreeMap<GameplayCommandKind, u32>,
) -> Option<String> {
    let actionable = probe.substantive_viable_count > 0;
    if actionable {
        return None;
    }
    // Portfolio liquidity support (transfers/withdrawals) is operational
    // context for a quiet cycle, not itself a cause: classify the strategic
    // causes first and append the note, so every quiet cycle still resolves
    // into a phase-level cause.
    let operational_only = (raw_generated_kinds
        .contains(&GameplayCommandKind::TransferBusinessCash)
        || raw_generated_kinds.contains(&GameplayCommandKind::WithdrawBusinessCash))
        && raw_generated_kinds
            .iter()
            .all(|kind| !is_substantive_command_kind(*kind));
    let mut gap_kinds = Vec::new();
    let mut restrained_kinds = Vec::new();
    for (kind, delta) in activation_delta {
        if *delta > 0 && !raw_generated_kinds.contains(kind) {
            // The generator deliberately narrows these routes to strategic-need
            // conditions, so an unfired activation is the persona declining by
            // design, not a coverage hole. Keep them separate so a true
            // generator gap (an offered action with no construction logic)
            // stays visible in the diagnosis.
            if is_policy_gated_command_route(*kind) {
                restrained_kinds.push(*kind);
                *accumulator
                    .quiet_diagnostic
                    .restrained_routes
                    .entry(*kind)
                    .or_default() += 1;
            } else {
                gap_kinds.push(*kind);
                *accumulator
                    .quiet_diagnostic
                    .generator_gaps
                    .entry(*kind)
                    .or_default() += 1;
            }
        }
    }
    let mut gated_kinds = Vec::new();
    for kind in raw_generated_kinds {
        if !retained_kinds.contains(kind) {
            gated_kinds.push(*kind);
            *accumulator
                .quiet_diagnostic
                .policy_gates
                .entry(*kind)
                .or_default() += 1;
        }
    }
    let mut rejected_kinds = Vec::new();
    let mut budget_kinds = Vec::new();
    for kind in retained_kinds {
        // `viable_command_kinds` records substantive viability only; an
        // operational kind here was executed as the fallback action, not
        // rejected by validation.
        if !is_substantive_command_kind(*kind) {
            continue;
        }
        if !probe.viable_command_kinds.contains(kind) {
            let retained_count = retained_counts_by_kind.get(kind).copied().unwrap_or(0);
            let probed_count = probed_counts_by_kind.get(kind).copied().unwrap_or(0);
            if probed_count >= retained_count {
                rejected_kinds.push(*kind);
                *accumulator
                    .quiet_diagnostic
                    .validation_gates
                    .entry(*kind)
                    .or_default() += 1;
            } else {
                budget_kinds.push(*kind);
                *accumulator
                    .quiet_diagnostic
                    .budget_gates
                    .entry(*kind)
                    .or_default() += 1;
            }
        }
    }
    let mut causes = Vec::new();
    if !gap_kinds.is_empty() {
        causes.push(format!(
            "activation without candidate [{}]",
            kind_labels(&gap_kinds)
        ));
    }
    if !restrained_kinds.is_empty() {
        causes.push(format!(
            "reserved by agent policy [{}]",
            kind_labels(&restrained_kinds)
        ));
    }
    if !gated_kinds.is_empty() {
        causes.push(format!(
            "declined by agent policy [{}]",
            kind_labels(&gated_kinds)
        ));
    }
    if !rejected_kinds.is_empty() {
        causes.push(format!(
            "rejected by validation [{}]",
            kind_labels(&rejected_kinds)
        ));
    }
    if !budget_kinds.is_empty() {
        causes.push(format!(
            "unverified due to probe budget [{}]",
            kind_labels(&budget_kinds)
        ));
    }
    if causes.is_empty() {
        accumulator.quiet_diagnostic.dormant_cycles = accumulator
            .quiet_diagnostic
            .dormant_cycles
            .saturating_add(1);
        if operational_only {
            return Some(
                "dormant: operational-only liquidity support was available, but the world offered no strategic action"
                    .to_owned(),
            );
        }
        return Some(
            "dormant: no candidate was built and no activation opportunity fired; the world offered no detected action"
                .to_owned(),
        );
    }
    gap_kinds.sort();
    restrained_kinds.sort();
    gated_kinds.sort();
    rejected_kinds.sort();
    budget_kinds.sort();
    let mut reason = causes.join("; ");
    if operational_only {
        reason.push_str(
            "; operational-only: portfolio liquidity support was available, but no strategic commitment was viable",
        );
    }
    Some(reason)
}

pub(crate) fn quiet_cause(reason: Option<&str>) -> Option<QuietCause> {
    let reason = reason?;
    if reason.contains("activation without candidate") {
        Some(QuietCause::GeneratorGap)
    } else if reason.contains("declined by agent policy") {
        Some(QuietCause::PolicyGate)
    } else if reason.contains("reserved by agent policy") {
        Some(QuietCause::Restrained)
    } else if reason.contains("rejected by validation") {
        Some(QuietCause::ValidationGate)
    } else if reason.contains("unverified due to probe budget") {
        Some(QuietCause::BudgetGate)
    } else if reason.starts_with("dormant:") {
        Some(QuietCause::Dormant)
    } else {
        None
    }
}

pub(crate) fn kind_labels(kinds: &[GameplayCommandKind]) -> String {
    kinds
        .iter()
        .map(|kind| kind.label())
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn record_offered_command_kinds(
    candidates: &[Candidate],
    accumulator: &mut CampaignAccumulator,
) {
    let offered: BTreeSet<_> = candidates.iter().map(|candidate| candidate.kind).collect();
    for kind in offered {
        let command_stats = accumulator
            .commands
            .get_mut(&kind)
            .expect("every command kind must have statistics");
        command_stats.offered_cycles = command_stats.offered_cycles.saturating_add(1);
    }
}

pub(crate) fn apply_selected_candidate(
    registry: &Registry,
    state: &mut AppState,
    selected: Option<Candidate>,
    accumulator: &mut CampaignAccumulator,
) -> Result<Option<ExecutedAction>, GameplayHarnessError> {
    let Some(candidate) = selected else {
        return Ok(None);
    };
    let outcome =
        apply_player_command(registry, state, candidate.command.clone()).map_err(|source| {
            GameplayHarnessError::SelectedCommandRejected {
                description: candidate.description.clone(),
                source,
            }
        })?;
    accumulator
        .commands
        .get_mut(&candidate.kind)
        .expect("every command kind must have statistics")
        .executed = accumulator
        .commands
        .get(&candidate.kind)
        .expect("every command kind must have statistics")
        .executed
        .saturating_add(1);
    accumulator.record_executed_candidate(candidate.kind, &candidate.command, state.clock.day());
    Ok(Some(ExecutedAction {
        kind: candidate.kind,
        description: candidate.description,
        outcome: outcome.summary,
    }))
}
