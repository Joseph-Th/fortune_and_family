//! Autonomous rival houses: objectives, monthly upkeep, credit participation, recovery.
//!
//! Purpose: give every non-player house deterministic objectives and monthly
//! stewardship (wealth upkeep, legitimacy ceiling, credit workout preference,
//! supply ordering at penalty-scaled value) without a second ruleset.
//! Owns: `AI_OBJECTIVE_REVIEW_DAYS`, `AI_BUSINESS_RECOVERY_TREASURY_RESERVE`,
//! monthly AI entry points (`advance_ai_objectives`, `apply_ai_dynasty_upkeep`,
//! `advance_ai_credit_participation`, `recover_ai_businesses`) and selection
//! helpers.
//! Reads: `Registry` + `AppState` (immutable for planning).
//! Mutates: AI dynasties, objectives, and businesses through canonical
//! strategic primitives (same validation as player paths).
//! Does not own: simulation daily loop or persistence.
//! Focused tests: `strategic_tests` AI objectives/upkeep, gameplay harness
//! persona diversity.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectiveProgress {
    Pending,
    Achieved,
}

pub(crate) const AI_OBJECTIVE_REVIEW_DAYS: i64 = 720;
pub(crate) const AI_BUSINESS_RECOVERY_TREASURY_RESERVE: Money = Money::from_copper(25_000);
/// Routine patronage buys rival legitimacy only up to this plateau: standing
/// above it must come from offices, works, and crisis stewardship rather than
/// a monthly stipend, so passive rivals cannot out-rank an actively governing
/// house indefinitely.
pub(crate) const AI_LEGITIMACY_OBJECTIVE_CEILING_BASIS_POINTS: u16 = 5_800;

/// The canonical rival-house monthly upkeep: household base, per-member and
/// per-business charges, plus great-house wealth stewardship — a percentage of
/// everything the house holds above the threshold, so hoards bleed toward
/// levels the house's real income can sustain instead of compounding forever.
pub(crate) fn ai_dynasty_monthly_upkeep(
    treasury: Money,
    family_members: usize,
    business_count: usize,
) -> Money {
    let family = AI_DYNASTY_UPKEEP_PER_FAMILY_MEMBER
        .saturating_mul(i64::try_from(family_members).unwrap_or(i64::MAX));
    let portfolio = AI_DYNASTY_UPKEEP_PER_BUSINESS
        .saturating_mul(i64::try_from(business_count).unwrap_or(i64::MAX));
    let excess_wealth = treasury.saturating_sub(AI_DYNASTY_WEALTH_UPKEEP_THRESHOLD);
    let wealth_stewardship =
        excess_wealth.saturating_mul_ratio(AI_DYNASTY_WEALTH_UPKEEP_BASIS_POINTS, 10_000);
    AI_DYNASTY_HOUSEHOLD_UPKEEP_MONTHLY
        .saturating_add(family)
        .saturating_add(portfolio)
        .saturating_add(wealth_stewardship)
}

impl ObjectiveProgress {
    const fn from_achieved(achieved: bool) -> Self {
        if achieved {
            Self::Achieved
        } else {
            Self::Pending
        }
    }
}

/// Applies a monthly household upkeep to every non-player dynasty so that rival houses
/// bear real recurring costs for their families and business portfolios.
///
/// A dynasty that cannot cover its upkeep loses standing instead of silently receiving a
/// free pass. This makes credit demand, loan defaults, and grounded legal claims reachable
/// in normal play. The player is exempt because the player's own discretionary spending
/// already taxes the dynasty treasury.
#[allow(clippy::too_many_lines)]
pub(crate) fn apply_ai_dynasty_upkeep(state: &mut AppState) -> Result<(), SimulationError> {
    let player_id = state.player_dynasty_id;
    let dynasties: Vec<_> = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != player_id)
        .map(|dynasty| {
            let family_members = state
                .family_councils
                .get(&dynasty.id())
                .map_or(0, |council| {
                    council
                        .members
                        .iter()
                        .filter(|member_id| {
                            state.characters.get(**member_id).is_some_and(|character| {
                                character.status() == CharacterStatus::Active
                            })
                        })
                        .count()
                });
            let business_count = state
                .businesses
                .ids_for_owner(dynasty.id())
                .into_iter()
                .flatten()
                .filter(|business_id| {
                    state.businesses.get(**business_id).is_some_and(|business| {
                        !matches!(
                            business.status(),
                            BusinessStatus::Insolvent | BusinessStatus::Closed
                        )
                    })
                })
                .count();
            (
                dynasty.id(),
                family_members,
                business_count,
                dynasty.treasury(),
            )
        })
        .collect();
    let mut total_upkeep = Money::ZERO;
    let mut total_shortfall = Money::ZERO;
    for (dynasty_id, family_members, business_count, treasury) in dynasties {
        let required = ai_dynasty_monthly_upkeep(treasury, family_members, business_count);
        if required == Money::ZERO {
            continue;
        }
        let paid = required.min(treasury);
        let shortfall = required.saturating_sub(paid);
        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("upkeep dynasty must exist");
        // `paid` is bounded by the treasury, so the subtraction cannot fail.
        dynasty.resources.treasury = dynasty
            .resources
            .treasury
            .checked_sub(paid)
            .expect("bounded upkeep payment must not exceed the treasury");
        total_upkeep =
            total_upkeep
                .checked_add(paid)
                .ok_or(SimulationError::DynastyTreasuryOverflow {
                    dynasty_id,
                    current: total_upkeep,
                    incoming: paid,
                })?;
        if shortfall > Money::ZERO {
            dynasty.resources.legitimacy_basis_points = dynasty
                .resources
                .legitimacy_basis_points
                .saturating_sub(AI_DYNASTY_UPKEEP_SHORTFALL_LEGITIMACY_PENALTY);
            dynasty.resources.reputation_reliability_basis_points = dynasty
                .resources
                .reputation_reliability_basis_points
                .saturating_sub(AI_DYNASTY_UPKEEP_SHORTFALL_RELIABILITY_PENALTY);
            total_shortfall = total_shortfall.checked_add(shortfall).ok_or(
                SimulationError::DynastyTreasuryOverflow {
                    dynasty_id,
                    current: total_shortfall,
                    incoming: shortfall,
                },
            )?;
        }
    }
    if total_upkeep > Money::ZERO {
        // Household upkeep buys goods, staff, and services from the city's
        // market sector rather than deleting copper: the pooled clearing
        // account is the credited counterparty, so AI maintenance no longer
        // deflates private money supplies month after month.
        credit_market_clearing_account(state, total_upkeep)?;
    }
    if total_upkeep > Money::ZERO || total_shortfall > Money::ZERO {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::HouseholdUpkeep,
            subject: "ai-dynasties".into(),
            detail: format!(
                "monthly_upkeep={};shortfall={}",
                total_upkeep.copper(),
                total_shortfall.copper()
            )
            .into(),
        });
    }
    Ok(())
}

/// Monthly AI credit-market participation so rival houses borrow and lend through the
/// same canonical loan machinery the player uses.
///
/// Without dynamic credit activity the private-credit economy is static: bootstrap loans
/// amortize to repayment and the player's `BorrowFunds`/`ExtendCredit` routes have nothing
/// to respond to, which starves grounded debt-enforcement legal claims. This function makes
/// rival houses (a) lend a portion of idle treasury to a house whose businesses need
/// working capital, and (b) borrow when their own businesses need capital and their
/// treasury is thin, mirroring `ensure_non_player_loan_counterparty_accepts`.
pub(crate) fn advance_ai_credit_participation(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    // Live-book membership is read for every candidate house below; one fold
    // over the loan ledger per month replaces a full scan per candidate. The
    // book is updated at every commit inside this pass, so subsequent
    // candidates observe exactly what a fresh rescan would.
    let mut active_book = ActiveAiLoanBook::collect(state);
    advance_ai_default_restructuring(state, &mut active_book)?;
    advance_ai_credit_lending(registry, state, &mut active_book);
    advance_ai_credit_borrowing(registry, state, &mut active_book)?;
    Ok(())
}

const AI_DEFAULT_RESTRUCTURING_WEEKS: i64 = 208;
const AI_DEFAULT_RESTRUCTURING_INTEREST_BASIS_POINTS: u16 = 900;
const AI_DEFAULT_RESTRUCTURING_MIN_INTEREST_BASIS_POINTS: u16 = 400;

/// Which houses sit on either side of a repayment-active loan, folded once
/// from the loan ledger and kept current as this pass commits new loans.
#[derive(Default)]
pub(crate) struct ActiveAiLoanBook {
    borrowers: BTreeSet<DynastyId>,
    lenders: BTreeSet<DynastyId>,
}

impl ActiveAiLoanBook {
    pub(crate) fn collect(state: &AppState) -> Self {
        let mut book = Self::default();
        for loan in state.loans.values() {
            if loan.status.is_repayment_active() {
                book.record(loan.lender_dynasty_id, loan.borrower_dynasty_id);
            }
        }
        book
    }

    pub(crate) fn record(&mut self, lender_id: DynastyId, borrower_id: DynastyId) {
        self.lenders.insert(lender_id);
        self.borrowers.insert(borrower_id);
    }

    pub(crate) fn has_active_borrowing(&self, dynasty_id: DynastyId) -> bool {
        self.borrowers.contains(&dynasty_id)
    }

    pub(crate) fn holds_active_loan(&self, dynasty_id: DynastyId) -> bool {
        self.lenders.contains(&dynasty_id)
    }
}

/// Works one aged default per borrower back into an amortizing repayment plan
/// before the city considers fresh credit.
///
/// The workout advances no new cash. It converts the existing claim to a long
/// repayment schedule, caps punitive rates at a recovery rate, and leaves any
/// additional defaults untouched until this workout is resolved. That makes
/// default a persistent creditor relationship instead of a reset button that
/// sends a failed borrower shopping for a new lender each month.
pub(crate) fn advance_ai_default_restructuring(
    state: &mut AppState,
    active_book: &mut ActiveAiLoanBook,
) -> Result<(), SimulationError> {
    let player_id = state.player_dynasty_id;
    let mut eligible: Vec<_> = state
        .loans
        .values()
        .filter(|loan| {
            loan.lender_dynasty_id != player_id
                && loan.borrower_dynasty_id != player_id
                && defaulted_loan_restructuring_available(state, loan)
        })
        .map(|loan| (loan.next_due_day, loan.id))
        .collect();
    eligible.sort_unstable();

    for (_, loan_id) in eligible {
        let (lender_dynasty_id, borrower_dynasty_id, balance, prior_interest) = {
            let loan = state
                .loans
                .get(&loan_id)
                .expect("collected defaulted loan must exist");
            (
                loan.lender_dynasty_id,
                loan.borrower_dynasty_id,
                loan.balance,
                loan.interest_basis_points,
            )
        };
        if active_book.has_active_borrowing(borrower_dynasty_id) {
            continue;
        }
        let terms = LoanTerms {
            lender_dynasty_id,
            borrower_dynasty_id,
            principal: Money::ZERO,
            weekly_payment: ai_loan_weekly_payment(balance, AI_DEFAULT_RESTRUCTURING_WEEKS),
            interest_basis_points: prior_interest.clamp(
                AI_DEFAULT_RESTRUCTURING_MIN_INTEREST_BASIS_POINTS,
                AI_DEFAULT_RESTRUCTURING_INTEREST_BASIS_POINTS,
            ),
            collateral_property_id: None,
        };
        let committed = ai_strategic_attempt(
            &validate_loan(state, terms.clone()).and_then(|token| token.commit(state)),
        )?;
        if committed {
            active_book.record(lender_dynasty_id, borrower_dynasty_id);
        }
    }
    Ok(())
}

/// A business is worth external working capital only when its own operating
/// history does not mark it as structurally unprofitable: the recovery
/// machinery refuses lifetime-losing firms, so financing one through a loan
/// would just convert a rescue doctrine into default churn.
pub(crate) fn business_is_creditworthy(business: &crate::core::Business) -> bool {
    business.finance.lifetime_costs <= business.finance.lifetime_revenue
}

/// The lending half of AI credit participation: a liquid house may fund a borrower's
/// working-capital shortfall through the canonical loan machinery. Sound firms are
/// financed first at standard terms; a house already carrying a live loan may
/// additionally diversify into speculative credit on the monthly risk-appetite draw,
/// and a house with no book at all may speculate once no sound firm needs capital.
/// Speculative credit — to distressed or lifetime-losing firms, at punitive secured
/// terms — is the world's controlled source of repayment failure:
/// some of those loans rescue the borrower, others miss installments, fall
/// delinquent, default, and ground the enforcement claims that keep courts,
/// collateral seizure, and banking panics reachable. Without it every obligation in
/// the campaign is serviced on schedule and the enforcement systems stay idle.
pub(crate) fn advance_ai_credit_lending(
    registry: &Registry,
    state: &mut AppState,
    active_book: &mut ActiveAiLoanBook,
) {
    let player_id = state.player_dynasty_id;
    let dynasties: Vec<_> = state.dynasties.keys().copied().collect();

    // Lending: a liquid house may fund a borrower's working-capital shortfall.
    for lender_id in dynasties.iter().copied().filter(|id| *id != player_id) {
        let lender_available = state
            .dynasties
            .get(&lender_id)
            .and_then(|lender| {
                lender
                    .treasury()
                    .checked_sub(crate::systems::PRIVATE_LOAN_COUNTERPARTY_RESERVE)
            })
            .unwrap_or(Money::ZERO);
        if lender_available < Money::from_copper(2_000) {
            continue;
        }
        let Some(offer) =
            ai_credit_lending_offer(registry, state, player_id, lender_id, active_book)
        else {
            continue;
        };
        let principal = offer.principal(lender_available);
        if principal < Money::from_copper(1_000) {
            continue;
        }
        let terms = LoanTerms {
            lender_dynasty_id: lender_id,
            borrower_dynasty_id: offer.borrower_dynasty_id,
            principal,
            weekly_payment: ai_loan_weekly_payment(principal, offer.term_weeks),
            interest_basis_points: offer.interest_basis_points,
            collateral_property_id: offer.collateral_property_id,
        };
        // `validate_loan(...).commit(...)` uses the canonical loan machinery and records
        // outbox feedback; a failed offer is simply skipped. The loan then capitalizes
        // the borrower's short business so the financing need actually resolves.
        match validate_loan(state, terms.clone()).and_then(|token| token.commit(state)) {
            Ok(_) => active_book.record(terms.lender_dynasty_id, terms.borrower_dynasty_id),
            Err(_) => continue,
        }
        if let Some(business) = state.businesses.get_mut(offer.business_id)
            && business.owner_dynasty_id() == offer.borrower_dynasty_id
            && business.status() != BusinessStatus::Closed
        {
            let _ = capitalize_owned_business(
                state,
                offer.borrower_dynasty_id,
                offer.business_id,
                principal.min(offer.shortfall),
            );
        }
    }
}

/// One concrete loan offer a liquid house could make this month: the most
/// undercapitalized operating business of an unleveraged rival house, priced by
/// the borrower's track record.
pub(crate) struct AiCreditLendingOffer {
    pub(crate) borrower_dynasty_id: DynastyId,
    pub(crate) business_id: BusinessId,
    pub(crate) shortfall: Money,
    pub(crate) principal_cap: Money,
    pub(crate) interest_basis_points: u16,
    pub(crate) term_weeks: i64,
    pub(crate) collateral_property_id: Option<crate::ids::PropertyId>,
}

impl AiCreditLendingOffer {
    pub(crate) fn principal(&self, lender_available: Money) -> Money {
        lender_available.min(self.shortfall).min(self.principal_cap)
    }
}

#[cfg(test)]
pub(crate) fn lending_offer_for_test(
    registry: &Registry,
    state: &mut AppState,
    player_id: DynastyId,
    lender_id: DynastyId,
) -> Option<AiCreditLendingOffer> {
    let active_book = ActiveAiLoanBook::collect(state);
    ai_credit_lending_offer(registry, state, player_id, lender_id, &active_book)
}

pub(crate) fn ai_credit_lending_offer(
    registry: &Registry,
    state: &mut AppState,
    player_id: DynastyId,
    lender_id: DynastyId,
    active_book: &ActiveAiLoanBook,
) -> Option<AiCreditLendingOffer> {
    let sound = state
        .dynasties
        .keys()
        .copied()
        .filter(|id| *id != lender_id && *id != player_id)
        .filter_map(|candidate_id| {
            if active_book.has_active_borrowing(candidate_id)
                || borrower_has_unresolved_default(state, candidate_id)
            {
                return None;
            }
            best_creditworthy_business(registry, state, candidate_id).map(
                |(business_id, shortfall)| AiCreditLendingOffer {
                    borrower_dynasty_id: candidate_id,
                    business_id,
                    shortfall,
                    principal_cap: Money::from_copper(12_000),
                    interest_basis_points: 700_u16,
                    term_weeks: 104_i64,
                    collateral_property_id: None,
                },
            )
        })
        .max_by_key(|offer| offer.shortfall);
    // Risk appetite: a liquid house whose safe working-capital book already
    // carries a live loan may diversify into a losing firm's recovery at
    // punitive, property-secured terms. Without this second route the
    // speculative tier only opens once *every* firm in the city is fully
    // capitalized, which on fresh campaigns takes years of accumulation —
    // so delinquency, default, seizure, and the grounded legal claims they
    // ground stay unreachable across a whole standard session. The draw stays
    // monthly and per-house, and unserved sound demand remains the fallback
    // answer when the draw fails.
    let sound_book_active = active_book.holds_active_loan(lender_id);
    if sound_book_active {
        if state
            .rng
            .is_chance_success(SPECULATIVE_LOAN_MONTHLY_CHANCE_BASIS_POINTS)
            && let Some(speculative) =
                speculative_lending_offer(registry, state, player_id, lender_id, active_book)
        {
            return Some(speculative);
        }
        return sound;
    }
    // With nothing on the book yet, the safe offer still comes first and the
    // original saturated-book speculation path applies behind it.
    if let Some(sound) = sound {
        return Some(sound);
    }
    if !state
        .rng
        .is_chance_success(SPECULATIVE_LOAN_MONTHLY_CHANCE_BASIS_POINTS)
    {
        return None;
    }
    speculative_lending_offer(registry, state, player_id, lender_id, active_book)
}

/// The punitive recovery-loan pool: the most undercapitalized losing firm of
/// an otherwise unleveraged rival house.
///
/// The borrower must genuinely need the money: a house whose treasury could
/// fund the shortfall itself recapitalizes directly instead of paying punitive
/// interest, so speculative books land on houses whose own liquidity is too
/// thin to rescue the firm. That overextension is what makes some of these
/// loans miss installments, default, and ground enforcement claims instead of
/// being risk-free theater.
pub(crate) fn speculative_lending_offer(
    registry: &Registry,
    state: &mut AppState,
    player_id: DynastyId,
    lender_id: DynastyId,
    active_book: &ActiveAiLoanBook,
) -> Option<AiCreditLendingOffer> {
    let candidates: Vec<(AiCreditLendingOffer, Money)> = state
        .dynasties
        .keys()
        .copied()
        .filter(|id| *id != lender_id && *id != player_id)
        .filter_map(|candidate_id| {
            if active_book.has_active_borrowing(candidate_id)
                || borrower_has_unresolved_default(state, candidate_id)
            {
                return None;
            }
            let candidate = best_speculative_business(registry, state, candidate_id)?;
            let (_, shortfall) = candidate;
            let treasury = state
                .dynasties
                .get(&candidate_id)
                .map_or(Money::ZERO, crate::core::Dynasty::treasury);
            // Two borrower profiles justify punitive risk capital: a house
            // too thin to fund the recapitalization itself, or any house
            // whose firm is structurally losing rather than temporarily
            // short. Both are genuine recovery bets; a rich house rescuing a
            // merely undercapitalized profitable firm is not, and its certain
            // repayment would make the speculative book risk-free theater.
            let firm_is_structural_loser =
                state.businesses.get(candidate.0).is_some_and(|business| {
                    business.finance.lifetime_costs > business.finance.lifetime_revenue
                });
            let house_cannot_self_fund =
                treasury < shortfall.saturating_add(AI_BUSINESS_RECOVERY_TREASURY_RESERVE);
            if !firm_is_structural_loser && !house_cannot_self_fund {
                return None;
            }
            Some((
                AiCreditLendingOffer {
                    borrower_dynasty_id: candidate_id,
                    business_id: candidate.0,
                    shortfall,
                    principal_cap: SPECULATIVE_LOAN_MAX_PRINCIPAL,
                    interest_basis_points: SPECULATIVE_LOAN_INTEREST_BASIS_POINTS,
                    term_weeks: SPECULATIVE_LOAN_TERM_WEEKS,
                    collateral_property_id: unpledged_borrower_property(state, candidate_id),
                },
                treasury,
            ))
        })
        .collect();
    // Among equal needs, lend down: the thinnest treasury is both the house
    // most dependent on outside capital and the one whose repayment is a real
    // bet instead of a formality.
    candidates
        .into_iter()
        .max_by_key(|(offer, treasury)| (offer.shortfall, std::cmp::Reverse(*treasury)))
        .map(|(offer, _)| offer)
}

/// The most undercapitalized creditworthy business owned by a dynasty, when its
/// recapitalization gap is large enough to be worth financing.
pub(crate) fn best_creditworthy_business(
    registry: &Registry,
    state: &AppState,
    owner_id: DynastyId,
) -> Option<(BusinessId, Money)> {
    undercapitalized_business(
        registry,
        state,
        owner_id,
        BusinessStatusFilter::Creditworthy,
    )
}

/// The most undercapitalized structurally losing business owned by a dynasty:
/// the speculative lending pool. Operating firms only — insolvent and closed
/// businesses have nothing left to finance.
pub(crate) fn best_speculative_business(
    registry: &Registry,
    state: &AppState,
    owner_id: DynastyId,
) -> Option<(BusinessId, Money)> {
    undercapitalized_business(registry, state, owner_id, BusinessStatusFilter::Speculative)
}

/// Which side of the credit ledger a candidate business is drawn from.
#[derive(Clone, Copy)]
pub(crate) enum BusinessStatusFilter {
    /// Operating, lifetime-profitable firms: safe working-capital clients.
    Creditworthy,
    /// Speculative recovery bets: everything else — lifetime-losing firms and
    /// distressed firms generally. A firm that cannot keep itself operating
    /// this season is a risky borrower even with a profitable history, so the
    /// punitive secured pool has eligible candidates within a session instead
    /// of only after years of accumulated losses.
    Speculative,
}

pub(crate) fn undercapitalized_business(
    registry: &Registry,
    state: &AppState,
    owner_id: DynastyId,
    filter: BusinessStatusFilter,
) -> Option<(BusinessId, Money)> {
    state
        .businesses
        .ids_for_owner(owner_id)
        .into_iter()
        .flatten()
        .filter_map(|business_id| state.businesses.get(*business_id))
        .filter(|business| {
            !matches!(
                business.status(),
                BusinessStatus::Insolvent | BusinessStatus::Closed
            )
        })
        .filter(|business| {
            // A distressed firm is never a safe client even when its lifetime
            // history nets positive: the safe book must not rescue for cheap
            // the very firms whose operating trouble makes punitive, secured
            // recovery credit (and its eventual repayment failures) possible.
            let sound =
                business_is_creditworthy(business) && business.status() == BusinessStatus::Active;
            match filter {
                BusinessStatusFilter::Creditworthy => sound,
                BusinessStatusFilter::Speculative => !sound,
            }
        })
        .filter_map(|business| {
            let target = business_recapitalization_target(registry, state, business);
            let shortfall = target.saturating_sub(business.cash());
            (shortfall >= Money::from_copper(1_000)).then_some((business.id(), shortfall))
        })
        .max_by_key(|(_, shortfall)| *shortfall)
}

pub(crate) fn unpledged_borrower_property(
    state: &AppState,
    owner_id: DynastyId,
) -> Option<crate::ids::PropertyId> {
    state
        .properties
        .values()
        .find(|property| {
            property.owner_dynasty_id == Some(owner_id) && property.collateral_loan_id.is_none()
        })
        .map(|property| property.id)
}

/// The borrowing half of AI credit participation: a house whose businesses need capital
/// and whose treasury is thin seeks a working-capital loan from a liquid rival house.
/// The borrowed principal then capitalizes the short business immediately — a house
/// must not pay interest to hold idle treasury cash while the need that motivated
/// the loan persists. Player lending is a deliberate player command, never an
/// autonomous AI counterparty.
pub(crate) fn advance_ai_credit_borrowing(
    registry: &Registry,
    state: &mut AppState,
    active_book: &mut ActiveAiLoanBook,
) -> Result<(), SimulationError> {
    let player_id = state.player_dynasty_id;
    let dynasties: Vec<_> = state.dynasties.keys().copied().collect();
    for borrower_id in dynasties.iter().copied().filter(|id| *id != player_id) {
        let Some(borrower) = state.dynasties.get(&borrower_id) else {
            continue;
        };
        let neediest_business = state
            .businesses
            .ids_for_owner(borrower_id)
            .into_iter()
            .flatten()
            .filter_map(|business_id| state.businesses.get(*business_id))
            .filter(|business| {
                !matches!(
                    business.status(),
                    BusinessStatus::Insolvent | BusinessStatus::Closed
                )
            })
            .filter(|business| business_is_creditworthy(business))
            .filter_map(|business| {
                let shortfall = business_recapitalization_target(registry, state, business)
                    .saturating_sub(business.cash());
                (shortfall > Money::ZERO).then_some((business.id(), shortfall))
            })
            .max_by_key(|(_, shortfall)| *shortfall);
        let Some((_, shortfall)) = neediest_business else {
            continue;
        };
        if borrower.treasury() >= Money::from_copper(20_000) {
            continue;
        }
        if active_book.has_active_borrowing(borrower_id)
            || borrower_has_unresolved_default(state, borrower_id)
        {
            continue;
        }
        let Some((lender_id, available)) = state
            .dynasties
            .keys()
            .copied()
            .filter(|id| *id != borrower_id && *id != player_id)
            .filter_map(|candidate_id| {
                // `validate_loan` itself rejects a second unsettled loan
                // between the same pair, so no duplicate pre-filter here.
                let available = state
                    .dynasties
                    .get(&candidate_id)
                    .and_then(|lender| {
                        lender
                            .treasury()
                            .checked_sub(crate::systems::PRIVATE_LOAN_COUNTERPARTY_RESERVE)
                    })
                    .unwrap_or(Money::ZERO);
                (available >= Money::from_copper(2_000)).then_some((candidate_id, available))
            })
            .max_by_key(|(candidate_id, available)| (*available, *candidate_id))
        else {
            continue;
        };
        // Borrow only what the motivating shortfall needs: a house must not
        // pay interest to hold idle treasury cash while the need that
        // motivated the loan persists.
        let principal = available.min(shortfall).min(Money::from_copper(10_000));
        if principal < Money::from_copper(1_000) {
            continue;
        }
        let terms = LoanTerms {
            lender_dynasty_id: lender_id,
            borrower_dynasty_id: borrower_id,
            principal,
            weekly_payment: ai_loan_weekly_payment(principal, 156_i64),
            interest_basis_points: 800_u16,
            collateral_property_id: state
                .properties
                .values()
                .find(|property| {
                    property.owner_dynasty_id == Some(borrower_id)
                        && property.collateral_loan_id.is_none()
                })
                .map(|property| property.id),
        };
        let committed = ai_strategic_attempt(
            &validate_loan(state, terms.clone()).and_then(|token| token.commit(state)),
        )?;
        if !committed {
            continue;
        }
        active_book.record(terms.lender_dynasty_id, terms.borrower_dynasty_id);
        // Deploy the borrowed working capital exactly where the shortfall that
        // motivated the loan lives.
        if let Some((business_id, shortfall)) = neediest_business
            && let Some(business) = state.businesses.get_mut(business_id)
            && business.owner_dynasty_id() == borrower_id
            && business.status() != BusinessStatus::Closed
        {
            let _ = capitalize_owned_business(
                state,
                borrower_id,
                business_id,
                principal.min(shortfall),
            );
        }
    }
    Ok(())
}

/// Ceil-divides a money amount into a per-week payment without overflow.
pub(crate) fn ai_loan_weekly_payment(principal: Money, weeks: i64) -> Money {
    debug_assert!(principal > Money::ZERO);
    debug_assert!(weeks > 0);
    principal.ceil_div_positive(weeks)
}

pub(crate) fn recover_ai_businesses(registry: &Registry, state: &mut AppState) {
    let mut business_ids: Vec<_> = state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() != state.player_dynasty_id)
        .filter(|business| {
            matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            )
        })
        .map(crate::core::Business::id)
        .collect();
    business_ids.sort_unstable();

    for business_id in business_ids {
        let business = state
            .businesses
            .get(business_id)
            .expect("indexed AI business must exist");
        if checked_next_business_finance_version(business).is_none() {
            continue;
        }
        if business.finance.lifetime_costs > business.finance.lifetime_revenue {
            continue;
        }
        let owner_dynasty_id = business.owner_dynasty_id();
        let target_cash = business_recapitalization_target(registry, state, business);
        let shortfall = target_cash.saturating_sub(business.cash()).max(Money::ZERO);
        if shortfall == Money::ZERO {
            continue;
        }
        let treasury = state
            .dynasties
            .get(&owner_dynasty_id)
            .expect("AI business owner dynasty must exist")
            .treasury();
        let available = treasury
            .saturating_sub(AI_BUSINESS_RECOVERY_TREASURY_RESERVE)
            .max(Money::ZERO);
        // A rescue must fund the full operating target in one commit. Trickle
        // capitalization that only crosses the daily lifecycle recovery bar
        // produces weekly distressed-to-recovered churn: the firm re-enters
        // distress as soon as the next wage or input settlement drains the
        // shallow cushion, and the owner's treasury bleeds without ever
        // resolving the underlying shortfall. An owner that cannot commit the
        // whole target lets the firm work through its distress instead.
        if available < shortfall {
            continue;
        }
        capitalize_owned_business(state, owner_dynasty_id, business_id, shortfall)
            .expect("prevalidated AI business capitalization must commit");
    }
}

pub(crate) fn ai_strategic_attempt<T>(
    result: &Result<T, StrategicError>,
) -> Result<bool, SimulationError> {
    match result {
        Ok(_) => Ok(true),
        Err(StrategicError::IdentifierAllocation(error)) => Err((*error).into()),
        Err(_) => Ok(false),
    }
}

pub(crate) fn advance_ai_objectives(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let objectives: Vec<_> = state
        .ai_objectives
        .values()
        .filter(|objective| objective.status == ObjectiveStatus::Pursuing)
        .map(|objective| {
            (
                objective.id,
                objective.dynasty_id,
                objective.kind,
                objective.created_day,
            )
        })
        .collect();
    for (objective_id, dynasty_id, kind, created_day) in objectives {
        let progress = match kind {
            ObjectiveKind::AcquireProperty => advance_ai_property_objective(state, dynasty_id)?,
            ObjectiveKind::WinOffice => advance_ai_office_objective(state, dynasty_id)?,
            ObjectiveKind::SecureSupply => {
                advance_ai_supply_objective(registry, state, dynasty_id)?
            }
            ObjectiveKind::ReduceDebt => advance_ai_debt_objective(state, dynasty_id)?,
            ObjectiveKind::ImproveLegitimacy => advance_ai_legitimacy_objective(state, dynasty_id)?,
            ObjectiveKind::AccumulateCash => {
                advance_ai_accumulation_objective(registry, state, dynasty_id)?
            }
            ObjectiveKind::ContainRival => advance_ai_rival_objective(state, dynasty_id)?,
        };
        let terminal = match progress {
            ObjectiveProgress::Achieved => Some((
                ObjectiveStatus::Achieved,
                "The prior objective was completed; the house selected the next strongest route to durable power.",
            )),
            ObjectiveProgress::Pending
                if day.saturating_sub(created_day) >= AI_OBJECTIVE_REVIEW_DAYS =>
            {
                Some((
                    ObjectiveStatus::Abandoned,
                    "The prior objective stalled; the house redirected resources toward a more viable route to durable power.",
                ))
            }
            ObjectiveProgress::Pending => None,
        };
        if let Some((terminal_status, rationale)) = terminal {
            let objective = state
                .ai_objectives
                .get_mut(&objective_id)
                .expect("AI objective must exist");
            objective.status = terminal_status;
            if terminal_status == ObjectiveStatus::Abandoned {
                objective.rationale.push_str(
                    " The house abandoned this route after two years without decisive progress.",
                );
            }
            let new_id = state.next_ids.try_objective()?;
            state.ai_objectives.insert(
                new_id,
                AiObjective {
                    id: new_id,
                    dynasty_id,
                    kind: next_objective_kind(kind),
                    priority: 50,
                    created_day: day,
                    status: ObjectiveStatus::Pursuing,
                    rationale: rationale.to_owned(),
                },
            );
        }
    }
    Ok(())
}

pub(crate) fn ai_net_liquid_position(state: &AppState, dynasty_id: DynastyId) -> i128 {
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .map_or(0_i128, |dynasty| i128::from(dynasty.treasury().copper()));
    let outstanding_debt = state
        .loans
        .values()
        .filter(|loan| loan.borrower_dynasty_id == dynasty_id && !loan.status.is_settled())
        .map(|loan| i128::from(loan.balance.copper()))
        .sum::<i128>();
    treasury - outstanding_debt
}

/// Accumulating cash is an active posture, not an ambient threshold: the house
/// skims each operating business's surplus above its distribution reserve into
/// the treasury, and completes once its net liquid position crosses a level
/// its income streams can actually reach within a review window.
pub(crate) fn advance_ai_accumulation_objective(
    registry: &Registry,
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    const ACCUMULATION_TARGET_COPPER: i128 = 60_000;
    const AI_ACCUMULATION_MONTHLY_SKIM_CAP: Money = Money::from_copper(2_000);
    if ai_net_liquid_position(state, dynasty_id) > ACCUMULATION_TARGET_COPPER {
        return Ok(ObjectiveProgress::Achieved);
    }
    let mut remaining_skim = AI_ACCUMULATION_MONTHLY_SKIM_CAP;
    let business_ids: Vec<_> = state
        .businesses
        .ids_for_owner(dynasty_id)
        .into_iter()
        .flatten()
        .copied()
        .filter(|business_id| {
            state.businesses.get(*business_id).is_some_and(|business| {
                !matches!(
                    business.status(),
                    BusinessStatus::Insolvent | BusinessStatus::Closed
                )
            })
        })
        .collect();
    for business_id in business_ids {
        if remaining_skim <= Money::ZERO {
            break;
        }
        let Some(business) = state.businesses.get(business_id) else {
            continue;
        };
        // The skim harvests genuinely idle cash only: the floor is the larger
        // of the distribution reserve and the recapitalization target, so an
        // accumulation drive cannot strip a firm below its own working-capital
        // needs and manufacture the very shortfall that interest-bearing
        // borrowing would later be recruited to refill.
        let skim_floor = business_owner_distribution_reserve(registry, business)
            .max(business_recapitalization_target(registry, state, business));
        let excess = business.cash().saturating_sub(skim_floor).max(Money::ZERO);
        // `distribute_owned_business_cash` revalidates ownership, lifecycle,
        // reserve, and overflow before committing; a failed skim is skipped.
        if excess <= Money::ZERO {
            continue;
        }
        let skim = excess.min(remaining_skim);
        if ai_strategic_attempt(&distribute_owned_business_cash(
            registry,
            state,
            dynasty_id,
            business_id,
            skim,
        ))? {
            remaining_skim = remaining_skim.saturating_sub(skim);
        }
    }
    Ok(ObjectiveProgress::Pending)
}

pub(crate) const fn next_objective_kind(kind: ObjectiveKind) -> ObjectiveKind {
    match kind {
        ObjectiveKind::AccumulateCash => ObjectiveKind::AcquireProperty,
        ObjectiveKind::AcquireProperty => ObjectiveKind::WinOffice,
        ObjectiveKind::WinOffice => ObjectiveKind::ImproveLegitimacy,
        ObjectiveKind::SecureSupply => ObjectiveKind::AccumulateCash,
        ObjectiveKind::ReduceDebt => ObjectiveKind::SecureSupply,
        ObjectiveKind::ImproveLegitimacy => ObjectiveKind::ContainRival,
        ObjectiveKind::ContainRival => ObjectiveKind::ReduceDebt,
    }
}

pub(crate) fn advance_ai_property_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .map_or(Money::ZERO, crate::core::Dynasty::treasury);
    let property_id = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id.is_none())
        // A purchase must leave the house's business-rescue reserve intact:
        // spending the whole treasury on real estate bankrupts the portfolio
        // and disables the recovery machinery this same house relies on.
        .filter(|property| {
            treasury
                .checked_sub(property.value)
                .is_some_and(|remaining| remaining >= AI_BUSINESS_RECOVERY_TREASURY_RESERVE)
        })
        .min_by_key(|property| (property.value, property.id))
        .map(|property| property.id);
    let achieved = match property_id {
        Some(property_id) => {
            ai_strategic_attempt(&buy_unowned_property(state, dynasty_id, property_id))?
        }
        None => false,
    };
    Ok(ObjectiveProgress::from_achieved(achieved))
}

pub(crate) fn advance_ai_office_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    let holds_office = state.institutions.values().any(|institution| {
        institution.office_holder_id.is_some_and(|character_id| {
            state
                .characters
                .get(character_id)
                .is_some_and(|character| character.dynasty_id() == dynasty_id)
        })
    });
    if holds_office {
        return Ok(ObjectiveProgress::Achieved);
    }
    let spend = state
        .dynasties
        .get(&dynasty_id)
        .map_or(Money::ZERO, |dynasty| {
            Money::from_copper(500).min(dynasty.resources.treasury)
        });
    if spend > Money::ZERO {
        // Campaigning buys food, favors, and visibility through the city's
        // market sector; the pooled clearing account is the counterparty.
        credit_market_clearing_account(state, spend)?;
        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("AI office-campaigning dynasty must exist");
        dynasty.resources.treasury = dynasty.resources.treasury.checked_sub(spend).expect(
            "bounded AI office spending must not exceed the treasury it was measured against",
        );
        let legitimacy_gain = u16::try_from(spend.saturating_mul_ratio(80, 500).copper())
            .unwrap_or(80)
            .min(80);
        dynasty.resources.legitimacy_basis_points = dynasty
            .resources
            .legitimacy_basis_points
            .saturating_add(legitimacy_gain)
            .min(10_000);
    }
    Ok(ObjectiveProgress::Pending)
}

pub(crate) fn advance_ai_supply_objective(
    registry: &Registry,
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    let owner_businesses: Vec<_> = state
        .businesses
        .ids_for_owner(dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|business_id| {
            state
                .businesses
                .get(*business_id)
                .filter(|business| {
                    !matches!(
                        business.status(),
                        BusinessStatus::Insolvent | BusinessStatus::Closed
                    )
                })
                .map(crate::core::Business::id)
        })
        .collect();
    for buyer_id in owner_businesses {
        let buyer_capacity_batches_per_day = state
            .businesses
            .get(buyer_id)
            .map_or(0, |buyer| buyer.operations.capacity_batches_per_day);
        let recipe = registry
            .get_recipe(
                state
                    .businesses
                    .get(buyer_id)
                    .expect("indexed business must exist")
                    .recipe_id(),
            )
            .expect("business recipe must resolve");
        for input in recipe.inputs() {
            let already = state.contracts.values().any(|contract| {
                contract.status == ContractStatus::Active
                    && contract.buyer_business_id == buyer_id
                    && contract.good_id == input.good_id()
            });
            if already {
                return Ok(ObjectiveProgress::Achieved);
            }
            let seller_id = state.businesses.iter().find_map(|seller| {
                let seller_recipe = registry.get_recipe(seller.recipe_id())?;
                (seller.owner_dynasty_id() != dynasty_id
                    && !matches!(
                        seller.status(),
                        crate::core::BusinessStatus::Insolvent
                            | crate::core::BusinessStatus::Closed
                    )
                    && seller_recipe.output_good_id() == input.good_id())
                .then_some(seller.id())
            });
            let Some(seller_id) = seller_id else {
                continue;
            };
            let price = state
                .market
                .get_quote(input.good_id())
                .expect("market quote must exist")
                .price();
            // Commit most of the buyer's real weekly consumption rather than
            // a token batch count: a commitment sized near genuine need makes
            // seller performance matter, so a bad month at the selling firm
            // can produce an attributable miss instead of every obligation
            // being serviced perfectly forever. The five-day basis matches
            // `CONTRACT_CAPACITY_COMMITMENT_DAYS`, keeping open-market trade
            // possible alongside the contract.
            let weekly_need = input.quantity().saturating_mul_ratio(
                i64::from(buyer_capacity_batches_per_day)
                    .saturating_mul(CONTRACT_CAPACITY_COMMITMENT_DAYS),
                1,
            );
            let capacity = available_supply_contract_capacity(
                registry,
                state,
                buyer_id,
                seller_id,
                input.good_id(),
            );
            let quantity_per_week = capacity.map_or_else(
                || input.quantity().saturating_mul_ratio(4, 1),
                |capacity| weekly_need.min(capacity.buyer).min(capacity.seller),
            );
            if quantity_per_week <= Quantity::ZERO {
                continue;
            }
            let weekly_payment = cost_for(quantity_per_week, price);
            let Some(penalty) = weekly_payment.checked_mul_ratio(2, 1) else {
                continue;
            };
            let terms = SupplyContractTerms {
                buyer_business_id: buyer_id,
                seller_business_id: seller_id,
                good_id: input.good_id(),
                quantity_per_week,
                unit_price: price,
                penalty,
                duration_weeks: 26,
            };
            if ai_strategic_attempt(&sign_supply_contract(registry, state, terms))? {
                return Ok(ObjectiveProgress::Achieved);
            }
        }
    }
    Ok(ObjectiveProgress::Pending)
}

pub(crate) fn advance_ai_debt_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    // Repay the most urgent obligation first: delinquent loans are one missed
    // installment away from default, so they outrank current ones, and higher
    // balances outrank lower ones within a status. Defaulted paper never
    // qualifies — it must cure through restructuring or the court, not
    // through this objective's quiet side-door payments.
    let loan_id = state
        .loans
        .values()
        .filter(|loan| loan.borrower_dynasty_id == dynasty_id && loan.status.is_repayment_active())
        .max_by_key(|loan| {
            (
                u8::from(loan.status == LoanStatus::Delinquent),
                loan.balance.copper(),
                u32::MAX - loan.id.value(),
            )
        })
        .map(|loan| loan.id);
    let Some(loan_id) = loan_id else {
        return Ok(ObjectiveProgress::Achieved);
    };
    let treasury = state
        .dynasties
        .get(&dynasty_id)
        .expect("AI dynasty must exist")
        .treasury();
    let balance = state
        .loans
        .get(&loan_id)
        .expect("AI loan must exist")
        .balance;
    let extra = Money::from_copper(1_000).min(treasury).min(balance);
    apply_loan_payment(state, loan_id, extra)?;
    Ok(ObjectiveProgress::from_achieved(
        state
            .loans
            .get(&loan_id)
            .is_some_and(|loan| loan.status == LoanStatus::Repaid),
    ))
}

pub(crate) fn advance_ai_legitimacy_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    let legitimacy_before = state
        .dynasties
        .get(&dynasty_id)
        .expect("AI dynasty must exist")
        .resources
        .legitimacy_basis_points;
    // Bootstrap houses start near 4,500 bp and this objective's own patronage
    // yields at most +120 bp per month. The ceiling stays deliberately below
    // the old 7,000: a rival house that merely pays routine patronage must not
    // tower over an actively governing player whose city-shaping commands
    // spend legitimacy, so idle prestige plateaus mid-scale while earned
    // standing (offices, works, crisis response) remains the way upward.
    if legitimacy_before >= AI_LEGITIMACY_OBJECTIVE_CEILING_BASIS_POINTS {
        return Ok(ObjectiveProgress::Achieved);
    }
    let spend = state
        .dynasties
        .get(&dynasty_id)
        .map_or(Money::ZERO, |dynasty| {
            Money::from_copper(750).min(dynasty.resources.treasury)
        });
    if spend > Money::ZERO {
        // Patronage and charity flow through the city's market sector; the
        // pooled clearing account is the credited counterparty for the copper.
        credit_market_clearing_account(state, spend)?;
    }
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("AI dynasty must exist");
    if spend > Money::ZERO {
        dynasty.resources.treasury = dynasty.resources.treasury.checked_sub(spend).expect(
            "bounded AI legitimacy spending must not exceed the treasury it was measured against",
        );
    }
    let legitimacy_gain = u16::try_from(spend.saturating_mul_ratio(120, 750).copper())
        .unwrap_or(120)
        .min(120);
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_add(legitimacy_gain)
        .min(10_000);
    Ok(ObjectiveProgress::Pending)
}

pub(crate) fn advance_ai_rival_objective(
    state: &mut AppState,
    dynasty_id: DynastyId,
) -> Result<ObjectiveProgress, SimulationError> {
    if dynasty_id == state.player_dynasty_id {
        return Ok(ObjectiveProgress::Achieved);
    }
    let pair = DynastyPair::new(dynasty_id, state.player_dynasty_id);
    let Some(relationship) = state.relationships.get_mut(&pair) else {
        return Ok(ObjectiveProgress::Achieved);
    };
    relationship.trust_basis_points = relationship.trust_basis_points.saturating_sub(75);
    relationship.fear_basis_points = relationship
        .fear_basis_points
        .saturating_add(100)
        .min(10_000);
    relationship.resentment_basis_points = relationship
        .resentment_basis_points
        .saturating_add(100)
        .min(10_000);
    // The objective is reviewed and abandoned after `AI_OBJECTIVE_REVIEW_DAYS`
    // (at most 24 monthly increments of +100), so the maximum fear a dynasty can
    // reach on its own is bootstrap_max(2_500) + 24 * 100 = 4_900. Requiring the
    // current 5_000 threshold made the objective mathematically impossible to
    // complete. 4_500 keeps the milestone meaningful (well above the bootstrap
    // range) while staying reachable for houses that start with substantial fear.
    let achieved = relationship.fear_basis_points >= 4_500;
    if achieved {
        let rival_name = state.dynasties.get(&dynasty_id).map_or_else(
            || dynasty_id.to_string(),
            |dynasty| dynasty.name().to_owned(),
        );
        remember_dynasty_interaction(
            state,
            dynasty_id,
            state.player_dynasty_id,
            &format!(
                "House {rival_name} completed a containment campaign that hardened bilateral commercial relations."
            ),
        );
        try_record_counterparty_information(
            state,
            dynasty_id,
            state.player_dynasty_id,
            "Rival patronage, guild correspondence, and commercial refusals",
        )?;
        try_push_outbox(
            state,
            OutboxKind::Information,
            format!("House {rival_name} is containing the dynasty"),
            "A sustained rival campaign has reduced trust and increased resentment. Future contracts with that house may require a premium or discount until relations improve."
                .to_owned(),
        )?;
    }
    Ok(ObjectiveProgress::from_achieved(achieved))
}

pub(crate) fn file_grounded_ai_legal_cases(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let player_id = state.player_dynasty_id;
    let plaintiff_ids: Vec<_> = state
        .dynasties
        .keys()
        .copied()
        .filter(|dynasty_id| *dynasty_id != player_id)
        .collect();
    for plaintiff_id in plaintiff_ids {
        let can_fund_filing =
            state.dynasties.get(&plaintiff_id).is_some_and(|dynasty| {
                dynasty.treasury() >= crate::systems::LEGAL_CASE_FILING_COST
            }) && crate::systems::court_filing_fee_headroom(registry, state).is_ok();
        if !can_fund_filing || !legal_filing_interval_available(state, plaintiff_id, day) {
            continue;
        }
        let Some(claim) = next_grounded_ai_legal_claim(state, plaintiff_id) else {
            continue;
        };
        let hearing_day = checked_future_day(day, crate::systems::LEGAL_CASE_HEARING_DELAY_DAYS)?;
        let legal_case_id = state.next_ids.try_legal_case()?;
        let plaintiff_treasury = state
            .dynasties
            .get(&plaintiff_id)
            .expect("legal plaintiff dynasty must exist")
            .treasury();
        state
            .dynasties
            .get_mut(&plaintiff_id)
            .expect("legal plaintiff dynasty must exist")
            .resources
            .treasury = plaintiff_treasury
            .checked_sub(crate::systems::LEGAL_CASE_FILING_COST)
            .expect("prevalidated legal filing cost must fit plaintiff treasury");
        crate::systems::collect_court_filing_fee(registry, state);
        state.legal_cases.insert(
            legal_case_id,
            LegalCase {
                id: legal_case_id,
                plaintiff_dynasty_id: plaintiff_id,
                defendant_dynasty_id: claim.defendant_dynasty_id,
                kind: claim.kind,
                claim_source: Some(claim.claim_source),
                evidence_basis_points: claim.evidence_basis_points,
                public_attention_basis_points: 1_500,
                filed_day: day,
                hearing_day,
                damages: claim.maximum_damages,
                status: LegalCaseStatus::Filed,
            },
        );
        adjust_dynasty_relationship(
            state,
            plaintiff_id,
            claim.defendant_dynasty_id,
            RelationshipDelta::new(-100, -30, 0, 150, 0),
        );
        remember_dynasty_interaction(
            state,
            plaintiff_id,
            claim.defendant_dynasty_id,
            &format!(
                "House {} filed a {:?} case against house {} over {}.",
                plaintiff_id, claim.kind, claim.defendant_dynasty_id, claim.description
            ),
        );
        if claim.defendant_dynasty_id == player_id {
            try_push_outbox(
                state,
                OutboxKind::Legal,
                format!("Legal case {legal_case_id} filed against the dynasty"),
                format!(
                    "Dynasty {plaintiff_id} filed a {:?} claim for up to {}. The hearing is scheduled for day {hearing_day}: {}.",
                    claim.kind, claim.maximum_damages, claim.description
                ),
            )?;
        }
    }
    Ok(())
}

pub(crate) fn legal_filing_interval_available(
    state: &AppState,
    plaintiff_id: DynastyId,
    day: i64,
) -> bool {
    state
        .legal_cases
        .values()
        .filter(|legal_case| legal_case.plaintiff_dynasty_id == plaintiff_id)
        .map(|legal_case| legal_case.filed_day)
        .max()
        .is_none_or(|last_filing_day| {
            last_filing_day
                .checked_add(crate::systems::LEGAL_CASE_FILING_INTERVAL_DAYS)
                .is_some_and(|next_filing_day| day >= next_filing_day)
        })
}

pub(crate) fn next_grounded_ai_legal_claim(
    state: &AppState,
    plaintiff_id: DynastyId,
) -> Option<crate::systems::LegalClaimQuote> {
    state
        .dynasties
        .keys()
        .copied()
        .filter(|defendant_id| *defendant_id != plaintiff_id)
        .flat_map(|defendant_id| {
            [LegalCaseKind::Debt, LegalCaseKind::ContractBreach]
                .into_iter()
                .filter_map(move |kind| {
                    crate::systems::quote_grounded_legal_claim(
                        state,
                        plaintiff_id,
                        defendant_id,
                        kind,
                    )
                })
        })
        .filter(|claim| {
            !state.legal_cases.values().any(|legal_case| {
                legal_case.plaintiff_dynasty_id == plaintiff_id
                    && legal_case.defendant_dynasty_id == claim.defendant_dynasty_id
                    && legal_case.kind == claim.kind
                    && matches!(
                        legal_case.status,
                        LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                    )
            })
        })
        .max_by_key(|claim| {
            (
                claim.evidence_basis_points,
                claim.maximum_damages,
                std::cmp::Reverse(claim.defendant_dynasty_id),
                std::cmp::Reverse(claim.kind),
            )
        })
}
