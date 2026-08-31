//! Persona policies — deterministic steward/entrepreneur/power-broker/opportunist rankings.
//!
//! Purpose: bias `Candidate` scoring by role without creating new domain
//! state or bypassing production validation.
//! Owns: `CLOSE_CHOICE_SCORE_GAP`, standing-floor and legitimacy-reserve
//! policy, commercial-standing thresholds, and per-persona ranking weights.
//! Reads: `AppState` via candidate scores and campaign accumulator.
//! Mutates: nothing durable (scores are transient; probes still canonical).
//! Does not own: candidate construction or findings thresholds.
//! Invariants: every persona uses the same discretionary floor (emergency
//! reserve + 2 months loan service); standing-burning actions add a
//! legitimacy reserve; ranking is deterministic and reproducible.
//! Focused tests: `src/gameplay_tests.rs` persona diversity.

use super::*;

pub(crate) const CLOSE_CHOICE_SCORE_GAP: i64 = 300;
pub(crate) const HEIR_CONFIRMATION_HEAD_AGE_YEARS: i64 = 52;
pub(crate) const HEIR_CONFIRMATION_HEALTH_THRESHOLD: u16 = 5_000;
pub(crate) const COMMERCIAL_STANDING_REPUTATION_REQUIREMENT: u16 =
    OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
pub(crate) const NOTIFICATION_BATCH_THRESHOLD: usize = 8;
pub(crate) const AGENT_LOAN_AMORTIZATION_WEEKS: i64 = 104;
pub(crate) const AGENT_OPPORTUNIST_LOAN_AMORTIZATION_WEEKS: i64 = 13;
pub(crate) const AGENT_OPPORTUNIST_STRESSED_LOAN_AMORTIZATION_WEEKS: i64 = 8;
pub(crate) const AGENT_OPPORTUNIST_LOAN_INTEREST_BASIS_POINTS: u16 = 2_500;
pub(crate) const AGENT_OFFICE_DUTY_RESERVE_MONTHS: i64 = 4;
pub(crate) const AGENT_OFFICE_LIQUIDITY_BUFFER: Money = Money::from_copper(5_000);
pub(crate) const AGENT_FAMILY_COUNCIL_DUTY_RESERVE_MONTHS: i64 = 6;
pub(crate) const AGENT_FAMILY_COUNCIL_LIQUIDITY_BUFFER: Money = Money::from_copper(2_500);
pub(crate) const FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD: u16 = 7_000;
pub(crate) const AGENT_CONTRACT_DURATION_WEEKS: u16 = 104;
pub(crate) const AGENT_CASH_REBALANCE_TRIGGER: Money = Money::from_copper(1_000);
pub(crate) const AGENT_CASH_REBALANCE_BUFFER: Money = Money::from_copper(2_000);
pub(crate) const AGENT_CASH_REBALANCE_INTERVAL_DAYS: i64 = 28;
pub(crate) const AGENT_OWNER_DISTRIBUTION_TRIGGER: Money = Money::from_copper(500);
pub(crate) const AGENT_OWNER_DISTRIBUTION_INTERVAL_DAYS: i64 = 90;
pub(crate) const AGENT_POLITICAL_RECOVERY_SUPPORT_BONUS: i64 = 1_500;
pub(crate) const AGENT_PLANNED_CAPITALIZATION_INTERVAL_DAYS: i64 = 360;
pub(crate) const AGENT_PLANNED_CAPITALIZATION_MAX: Money = Money::from_copper(8_000);
/// Monthly maintenance copper per resident family member the agent reserves
/// before committing treasury to property acquisitions.
pub(crate) const FAMILY_MAINTENANCE_MONTHLY_COPPER: i64 = 250;
pub(crate) const AGENT_CIVIC_ACCELERATION_TREASURY_TRIGGER: Money = Money::from_copper(50_000);
pub(crate) const AGENT_CIVIC_ACCELERATION_MAX_CONTRIBUTION: Money = Money::from_copper(12_000);
/// Minimum district need score (same scale as `public_work_need_score`)
/// before the agent considers bankrolling a project it does not sponsor:
/// external patronage answers visible deficits such as collapsing sanitation,
/// dangerous streets, or food-driven unrest, not routine civic upkeep. Typical
/// mid-condition districts score roughly 3,000-4,700 on their weakest
/// dimension, so a floor of 4,000 demands a genuinely deprived district while
/// staying reachable.
pub(crate) const EXTERNAL_PATRONAGE_MIN_NEED_SCORE: i64 = 4_000;
/// A project the city could not finish is already a public failure, so
/// rescuing one needs less provocation than accelerating a rival's work.
pub(crate) const STALLED_PATRONAGE_MIN_NEED_SCORE: i64 = 2_500;
pub(crate) const AGENT_ENDOWMENT_LIQUIDITY_FLOOR: Money = Money::from_copper(12_000);
pub(crate) const AGENT_ENDOWMENT_OFFICE_BUFFER: Money = Money::from_copper(10_000);
/// Minimum business-cash surplus above the operating target that justifies a
/// strategic owner withdrawal to meet a dynasty-level treasury commitment.
pub(crate) const AGENT_STRATEGIC_WITHDRAWAL_TRIGGER: Money = Money::from_copper(1_000);
/// An annual institution-patronage ceiling the strategic withdrawal path keeps
/// below the protected family floor so a single endowment never decapitalizes
/// the household.
pub(crate) const AGENT_STRATEGIC_WITHDRAWAL_MAX: Money = Money::from_copper(50_000);
/// Annual yield below which an owned property is considered underperforming and
/// candidates to reposition (sell) it for better capital use are considered.
pub(crate) const PROPERTY_PORTFOLIO_REPOSITIONING_YIELD_BASIS_POINTS: u16 = 800;
pub(crate) const AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS: u16 = 1_500;
pub(crate) const AGENT_INFORMATION_POLITICAL_VULNERABILITY_LEGITIMACY: u16 = 2_500;
pub(crate) const AGENT_INFORMATION_LEVERAGE_DELAY_DAYS: i64 = 60;
pub(crate) const AGENT_ROUTINE_COMMISSION_INTERVAL_DAYS: i64 = 720;
pub(crate) const INFORMATION_ROUTINE_PAIR_WINDOW_DAYS: i64 = 180;
pub(crate) const AGENT_INFORMATION_MARKET_PRICE_CHANGE_BASIS_POINTS: u64 = 2_000;
pub(crate) const AGENT_INFORMATION_MARKET_SHORTAGE_BASIS_POINTS: u64 = 4_000;
pub(crate) const AGENT_INFORMATION_MARKET_CONTRACT_GAP_BASIS_POINTS: u64 = 1_000;
pub(crate) const AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD: u16 = 4_500;
pub(crate) const AGENT_INFORMATION_DISTRICT_UNREST_THRESHOLD: u16 = 3_500;
pub(crate) const AGENT_INFORMATION_COUNTERPARTY_TRUST_THRESHOLD: u16 = 4_000;
pub(crate) const AGENT_INFORMATION_COUNTERPARTY_RESENTMENT_THRESHOLD: u16 = 2_500;
/// Same-kind commands chain into a streak only when they land within one
/// default decision cycle (30 days) of each other; a tighter window would
/// reset the streak on every ordinary cadence step and make the repetitive
/// command finding unreachable.
pub(crate) const SUBSTANTIVE_STREAK_MAX_GAP_DAYS: i64 = 30;
pub(crate) const ORGANIC_CANDIDATE_VARIATION_RANGE: i64 = 280;
/// Fixed budget used by agent-proposed public-work candidates.
pub(crate) const CANDIDATE_PUBLIC_WORK_BUDGET: Money = Money::from_copper(12_000);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GameplayPersona {
    Steward,
    Entrepreneur,
    PowerBroker,
    Opportunist,
}

impl GameplayPersona {
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::Steward,
            Self::Entrepreneur,
            Self::PowerBroker,
            Self::Opportunist,
        ]
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Steward => "steward",
            Self::Entrepreneur => "entrepreneur",
            Self::PowerBroker => "power-broker",
            Self::Opportunist => "opportunist",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayHarnessConfig {
    pub start_seed: u64,
    pub seed_count: u16,
    pub days_per_campaign: u32,
    pub decision_interval_days: u16,
    pub max_candidate_probes: u16,
    pub max_consequence_horizon_days: u16,
    pub trace_limit_per_campaign: u16,
    pub decision_log_campaigns: u8,
    pub personas: Vec<GameplayPersona>,
    pub backgrounds: Vec<StartingBackground>,
}

impl Default for GameplayHarnessConfig {
    fn default() -> Self {
        Self {
            start_seed: 1,
            // Three world seeds: personas share a world when the seed is
            // fixed, so world-content claims (crisis variety, breach rates,
            // civic drift) need several independent worlds to be meaningful.
            // Agent-choice claims still aggregate across every campaign.
            seed_count: 3,
            days_per_campaign: 1_080,
            decision_interval_days: 30,
            max_candidate_probes: 16,
            max_consequence_horizon_days: 360,
            trace_limit_per_campaign: 40,
            decision_log_campaigns: 3,
            personas: GameplayPersona::all().to_vec(),
            backgrounds: vec![
                StartingBackground::Baker,
                StartingBackground::ClothTrader,
                StartingBackground::Blacksmith,
            ],
        }
    }
}

impl GameplayHarnessConfig {
    /// Rejects configurations that cannot run a single campaign.
    ///
    /// # Errors
    ///
    /// Returns [`GameplayHarnessError::InvalidConfig`] when campaign or seed
    /// counts are non-positive.
    pub fn validate(&self) -> Result<(), GameplayHarnessError> {
        if self.seed_count == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "seed_count must be positive".to_owned(),
            });
        }
        if self
            .start_seed
            .checked_add(u64::from(self.seed_count.saturating_sub(1)))
            .is_none()
        {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "configured seed range exceeds u64::MAX".to_owned(),
            });
        }
        if self.days_per_campaign == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "days_per_campaign must be positive".to_owned(),
            });
        }
        if self.decision_interval_days == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "decision_interval_days must be positive".to_owned(),
            });
        }
        if self.max_candidate_probes == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "max_candidate_probes must be positive".to_owned(),
            });
        }
        if self.max_consequence_horizon_days == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "max_consequence_horizon_days must be positive".to_owned(),
            });
        }
        if self.personas.is_empty() || self.backgrounds.is_empty() {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "at least one persona and background are required".to_owned(),
            });
        }
        for (index, persona) in self.personas.iter().enumerate() {
            if self.personas[..index].contains(persona) {
                return Err(GameplayHarnessError::InvalidConfig {
                    reason: format!("persona {persona:?} was configured more than once"),
                });
            }
        }
        for (index, background) in self.backgrounds.iter().enumerate() {
            if self.backgrounds[..index].contains(background) {
                return Err(GameplayHarnessError::InvalidConfig {
                    reason: format!("background {background:?} was configured more than once"),
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn campaign_count(&self) -> usize {
        usize::from(self.seed_count)
            .saturating_mul(self.personas.len())
            .saturating_mul(self.backgrounds.len())
    }
}
