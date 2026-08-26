//! Tuning constants for every player command family.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Maximum idle cash a policy may hold: one year of operating cover.
pub(crate) const BUSINESS_RESERVE_MAX_OPERATING_DAYS: i64 = 360;
/// Days between operating-policy amendments for one business.
pub(crate) const BUSINESS_POLICY_CHANGE_INTERVAL_DAYS: i64 = 180;
/// Weeks between wage renegotiations for one business. Wage posture is a
/// standing labor commitment, so it changes on a slower cadence than policy.
pub(crate) const BUSINESS_WAGE_CHANGE_INTERVAL_DAYS: i64 = 90;
/// Upper bound on the weekly wage per worker the command accepts. It keeps
/// checked arithmetic unreachable while still allowing generous wages.
pub(crate) const MAX_WEEKLY_WAGE_PER_WORKER: Money = Money::from_copper(400);
pub(crate) const LAW_SPONSORSHIP_INTERVAL_DAYS: i64 = 360;
pub(crate) const LAW_SPONSORSHIP_COST: Money = Money::from_copper(2_000);
pub(crate) const LAW_LEGITIMACY_REQUIREMENT: u16 = 3_000;
pub(crate) const LAW_LEGITIMACY_COST: u16 = 250;
pub(crate) const CIVIC_DEBT_INTEREST_BASIS_POINTS: u16 = 600;
pub(crate) const CIVIC_DEBT_TERM_WEEKS: i64 = 104;
pub(crate) const CIVIC_DEBT_CREDITOR_RESERVE: Money = Money::from_copper(10_000);
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_RESERVE: Money = Money::from_copper(10_000);
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_BORROWER_LIQUIDITY_TARGET: Money =
    Money::from_copper(25_000);
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_MIN_INTEREST_BASIS_POINTS: u16 = 400;
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_MAX_INTEREST_BASIS_POINTS: u16 = 2_500;
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_MAX_AMORTIZATION_WEEKS: i64 = 260;
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_MIN_AMORTIZATION_WEEKS: i64 = 13;
pub(crate) const PRIVATE_LOAN_DISTRESSED_BORROWER_MIN_AMORTIZATION_WEEKS: i64 = 8;
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_MIN_COLLATERAL_LTV_BASIS_POINTS: i64 = 2_000;
pub(crate) const PROPERTY_COUNTERPARTY_BUYER_RESERVE: Money = Money::from_copper(10_000);
pub(crate) const PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS: i64 = 360;
pub(crate) const MAX_ACTIVE_SPONSORED_PUBLIC_WORKS: usize = 2;
pub(crate) const PUBLIC_WORK_MINIMUM_BUDGET: Money = Money::from_copper(1_000);
pub(crate) const LABOR_REPLACEMENT_COST: Money = Money::from_copper(750);
pub(crate) const LABOR_CONDITIONS_IMPROVEMENT_COST: Money = Money::from_copper(1_000);
pub(crate) const LABOR_NEGOTIATION_COST: Money = Money::from_copper(500);
pub(crate) const HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS: i64 = 1_080;
pub(crate) const HOUSE_GOVERNANCE_UNITY_COST: u16 = 250;
pub(crate) const FAMILY_COUNCIL_MEETING_INTERVAL_DAYS: i64 = 360;
pub(crate) const FAMILY_COUNCIL_MEETING_COST: Money = Money::from_copper(2_500);
pub(crate) const FAMILY_COUNCIL_MEETING_UNITY_GAIN: u16 = 1_500;
pub(crate) const FAMILY_COUNCIL_MEETING_LOYALTY_GAIN: u16 = 600;
pub(crate) const HEIR_DESIGNATION_INTERVAL_DAYS: i64 = 720;
pub(crate) const HEIR_DESIGNATION_LEGITIMACY_COST: u16 = 300;
pub(crate) const HEIR_DESIGNATION_UNITY_COST: u16 = 250;
pub(crate) const HEIR_MINIMUM_AGE_DAYS: i64 = 18 * 360;
pub(crate) const OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS: i64 = 180;
pub(crate) const OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST: u16 = 100;
pub(crate) const INSTITUTION_SUPPORT_INTERVAL_DAYS: i64 = 360;
pub(crate) const INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS: i64 = 90;
/// Recovery period after a character withdraws from an institution.
pub(crate) const INSTITUTION_WITHDRAWAL_RECOVERY_DAYS: i64 = 720;
pub(crate) const INSTITUTION_SUPPORT_COST: Money = Money::from_copper(1_200);
pub(crate) const INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT: u16 = 5_500;
pub(crate) const INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT: u32 = 52;
pub(crate) const INSTITUTION_ENDOWMENT_INTERVAL_DAYS: i64 = 360;
pub(crate) const INSTITUTION_ENDOWMENT_MIN: Money = Money::from_copper(5_000);
pub(crate) const INSTITUTION_ENDOWMENT_MAX: Money = Money::from_copper(50_000);
pub(crate) const INSTITUTION_SUPPORT_CAPABILITY_TARGET_SCORE: u32 = 10_000;
pub(crate) const INSTITUTION_SUPPORT_CAPABILITY_DELIVERY_STEP: u32 = 200;
pub(crate) const INSTITUTION_SUPPORT_MAX_PREPARATION_DELIVERIES: u32 = 13;
pub(crate) const MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER: usize = 2;
pub(crate) const OFFICE_NOMINATION_RECOVERY_DAYS: i64 = 720;
pub(crate) const OFFICE_NOMINATION_RESOLUTION_DAYS: i64 = 120;
pub(crate) const OFFICE_NOMINATION_CAMPAIGN_COST: Money = Money::from_copper(300);
pub(crate) const OFFICE_NOMINATION_REPUTATION_REQUIREMENT: u16 = 5_500;
pub(crate) const OFFICE_NOMINATION_DELIVERY_REQUIREMENT: u32 = 78;
pub(crate) const OFFICE_NOMINATION_CAPABILITY_TARGET_SCORE: u32 = 10_000;
pub(crate) const OFFICE_NOMINATION_CAPABILITY_DELIVERY_STEP: u32 = 100;
pub(crate) const OFFICE_NOMINATION_MAX_PREPARATION_DELIVERIES: u32 = 26;
pub(crate) const WARD_ADOPTION_INTERVAL_DAYS: i64 = 720;
pub(crate) const WARD_ADOPTION_COST: Money = Money::from_copper(6_000);
pub(crate) const WARD_ADOPTION_LEGITIMACY_REQUIREMENT: u16 = 3_500;
pub(crate) const WARD_ADOPTION_REPUTATION_REQUIREMENT: u16 = 5_200;
pub(crate) const WARD_ADOPTION_DELIVERY_REQUIREMENT: u32 = 52;
pub(crate) const WARD_ADOPTION_UNITY_COST: u16 = 100;
pub(crate) const WARD_ADOPTION_LEGITIMACY_COST: u16 = 250;
pub(crate) const MAX_ACTIVE_WARDS: usize = 4;
pub(crate) const FAMILY_EDUCATION_INTERVAL_DAYS: i64 = 360;
pub(crate) const FAMILY_EDUCATION_DYNASTY_INTERVAL_DAYS: i64 = 180;
pub(crate) const FAMILY_EDUCATION_COST: Money = Money::from_copper(2_000);
pub(crate) const INFORMATION_COMMISSION_INTERVAL_DAYS: i64 = 360;
pub(crate) const INFORMATION_COMMISSION_COST: Money = Money::from_copper(600);
pub(crate) const INFORMATION_LEVERAGE_COST: Money = Money::from_copper(600);
/// Relief mobilization base in copper; see [`crisis_relief_cost`].
pub(crate) const CRISIS_RELIEF_BASE_COST_COPPER: i64 = 1_200;
/// Severity points per copper of scaling relief grant; see [`crisis_relief_cost`].
pub(crate) const CRISIS_RELIEF_SEVERITY_DIVISOR: i64 = 3;
pub(crate) const CRISIS_REFORM_COST: Money = Money::from_copper(1_500);
pub(crate) const CRISIS_SUPPRESS_COST: Money = Money::from_copper(900);
pub(crate) const CRISIS_RELIEF_LEGITIMACY_GAIN: u16 = 500;
pub(crate) const CRISIS_REFORM_LEGITIMACY_GAIN: u16 = 300;
pub(crate) const CRISIS_SUPPRESS_LEGITIMACY_COST: u16 = 450;
pub(crate) const CRISIS_RELIEF_UNREST_REDUCTION: u16 = 800;
pub(crate) const CRISIS_REFORM_UNREST_REDUCTION: u16 = 500;
pub(crate) const CRISIS_SUPPRESS_UNREST_INCREASE: u16 = 700;
/// Exploit gates on and pays the same legitimacy: profiteering spends standing
/// outright rather than merely requiring it.
pub(crate) const CRISIS_EXPLOIT_LEGITIMACY_COST: u16 = 600;
pub(crate) const CRISIS_EXPLOIT_SEVERITY_INCREASE: u16 = 500;
pub(crate) const CRISIS_EXPLOIT_UNREST_INCREASE: u16 = 600;
pub(crate) const INFORMATION_REPORT_LIFETIME_DAYS: i64 = 540;
pub(crate) const COMMISSIONED_INFORMATION_SOURCE: &str = "Commissioned intelligence";
