//! Supply contracts: terms, validated commits, capacity, and weekly settlement.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Nominal weekly throughput assumed when sizing a supply contract against a
/// producer whose exact capacity is not the point of the calculation.
pub(crate) const STANDARD_CONTRACT_BATCHES_PER_WEEK: i64 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyContractTerms {
    pub buyer_business_id: BusinessId,
    pub seller_business_id: BusinessId,
    pub good_id: GoodId,
    pub quantity_per_week: Quantity,
    pub unit_price: Money,
    pub penalty: Money,
    pub duration_weeks: u16,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DueContract {
    id: crate::ids::ContractId,
    buyer_id: BusinessId,
    seller_id: BusinessId,
    good_id: GoodId,
    quantity: Quantity,
    unit_price: Money,
    penalty: Money,
    due_day: i64,
    end_day: i64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ContractPartySettlementState {
    owner_id: DynastyId,
    can_perform: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ContractSettlementState {
    buyer: ContractPartySettlementState,
    seller: ContractPartySettlementState,
}

impl ContractSettlementState {
    const fn is_fulfilled(self) -> bool {
        self.buyer.can_perform && self.seller.can_perform
    }

    const fn buyer_is_at_fault(self) -> bool {
        !self.buyer.can_perform
    }

    const fn seller_is_at_fault(self) -> bool {
        !self.seller.can_perform
    }

    const fn has_attributable_nonperformance(self) -> bool {
        self.buyer_is_at_fault() || self.seller_is_at_fault()
    }

    const fn breach_attribution(self) -> (Option<DynastyId>, Option<DynastyId>) {
        attribute_contract_breach(
            self.buyer.owner_id,
            self.seller.owner_id,
            self.buyer.can_perform,
            self.seller.can_perform,
        )
    }

    const fn breaching_dynasty_id(self) -> Option<DynastyId> {
        self.breach_attribution().0
    }

    const fn breach_victim_dynasty_id(self) -> Option<DynastyId> {
        self.breach_attribution().1
    }
}

#[derive(Debug)]
pub struct ValidatedSupplyContract {
    terms: SupplyContractTerms,
}

impl ValidatedSupplyContract {
    /// Revalidates and commits a supply contract exactly once.
    ///
    /// # Errors
    ///
    /// Returns the current validation error if state changed after the token was created, or an
    /// allocation or timeline error if durable contract feedback can no longer be recorded.
    pub fn commit(
        self,
        registry: &Registry,
        state: &mut AppState,
    ) -> Result<crate::ids::ContractId, StrategicError> {
        validate_supply_contract_terms(registry, state, &self.terms)?;
        let reserved = reserve_supply_contract_commit(state, &self.terms)?;
        Ok(commit_supply_contract_reserved(
            registry,
            state,
            &self.terms,
            reserved,
        ))
    }
}

/// Durable identifiers and schedule results a supply-contract commit consumes.
#[derive(Clone, Copy)]
pub(crate) struct ReservedSupplyContractCommit {
    contract_id: crate::ids::ContractId,
    next_due_day: i64,
    end_day: i64,
    outbox_id: crate::ids::OutboxMessageId,
    counterparty_report: Option<ReservedCounterpartyReport>,
}

pub(crate) fn reserve_supply_contract_commit(
    state: &mut AppState,
    terms: &SupplyContractTerms,
) -> Result<ReservedSupplyContractCommit, StrategicError> {
    let day = state.clock.day();
    let next_due_day = checked_future_day(day, 7)?;
    let end_day = checked_future_day(day, i64::from(terms.duration_weeks) * 7)?;
    let report_is_due = {
        let buyer_owner = state
            .businesses
            .get(terms.buyer_business_id)
            .expect("validated contract buyer must exist")
            .owner_dynasty_id();
        let seller_owner = state
            .businesses
            .get(terms.seller_business_id)
            .expect("validated contract seller must exist")
            .owner_dynasty_id();
        (buyer_owner == state.player_dynasty_id) != (seller_owner == state.player_dynasty_id)
    };
    let ids_before = state.next_ids.clone();
    let reservation = (|| -> Result<ReservedSupplyContractCommit, StrategicError> {
        let contract_id = state.next_ids.try_contract()?;
        let outbox_id = state.next_ids.try_outbox()?;
        let counterparty_report = if report_is_due {
            Some(ReservedCounterpartyReport {
                id: state.next_ids.try_information_report()?,
                expires_day: checked_future_day(day, COUNTERPARTY_REPORT_EXPIRY_DAYS)?,
            })
        } else {
            None
        };
        Ok(ReservedSupplyContractCommit {
            contract_id,
            next_due_day,
            end_day,
            outbox_id,
            counterparty_report,
        })
    })();
    match reservation {
        Ok(reserved) => Ok(reserved),
        Err(error) => {
            state.next_ids = ids_before;
            Err(error)
        }
    }
}

/// Applies a validated supply contract with every durable identifier
/// pre-reserved; infallible by construction.
pub(crate) fn commit_supply_contract_reserved(
    registry: &Registry,
    state: &mut AppState,
    terms: &SupplyContractTerms,
    reserved: ReservedSupplyContractCommit,
) -> crate::ids::ContractId {
    let ReservedSupplyContractCommit {
        contract_id: id,
        next_due_day,
        end_day,
        outbox_id,
        counterparty_report,
    } = reserved;
    let &SupplyContractTerms {
        buyer_business_id,
        seller_business_id,
        good_id,
        quantity_per_week,
        unit_price,
        penalty,
        ..
    } = terms;
    let buyer_owner_id = state
        .businesses
        .get(buyer_business_id)
        .expect("validated contract buyer must exist")
        .owner_dynasty_id();
    let seller_owner_id = state
        .businesses
        .get(seller_business_id)
        .expect("validated contract seller must exist")
        .owner_dynasty_id();
    let buyer_name = state
        .businesses
        .get(buyer_business_id)
        .expect("validated contract buyer must exist")
        .name()
        .to_owned();
    let seller_name = state
        .businesses
        .get(seller_business_id)
        .expect("validated contract seller must exist")
        .name()
        .to_owned();
    let good_name = registry
        .get_good(good_id)
        .expect("validated contract good must exist")
        .name()
        .to_owned();
    state.contracts.insert(
        id,
        SupplyContract {
            id,
            buyer_business_id,
            seller_business_id,
            good_id,
            quantity_per_week,
            unit_price,
            penalty,
            next_due_day,
            end_day,
            fulfilled_deliveries: 0,
            fulfilled_deliveries_by_dynasty: BTreeMap::default(),
            missed_deliveries: 0,
            breaching_dynasty_id: None,
            breach_victim_dynasty_id: None,
            unpaid_breach_penalty: Money::ZERO,
            collected_breach_penalty: Money::ZERO,
            status: ContractStatus::Active,
        },
    );
    state.outbox.push(OutboxMessage {
        id: outbox_id,
        day: state.clock.day(),
        kind: OutboxKind::Contract,
        subject: format!("Supply contract {id} signed"),
        body: format!(
            "{seller_name} (business {seller_business_id}) will deliver {quantity_per_week} of {good_name} to {buyer_name} (business {buyer_business_id}) each week."
        ),
        acknowledged: false,
    });
    adjust_dynasty_relationship(
        state,
        buyer_owner_id,
        seller_owner_id,
        RelationshipDelta::new(40, 20, 0, -10, 1),
    );
    remember_dynasty_interaction(
        state,
        buyer_owner_id,
        seller_owner_id,
        &format!("Supply contract {id} was signed."),
    );
    if let Some(report) = counterparty_report {
        emit_counterparty_report(
            state,
            report,
            buyer_owner_id,
            seller_owner_id,
            "Contract negotiation and delivery records",
        );
    }
    id
}

/// Validates a supply contract without mutating state.
///
/// # Errors
///
/// Returns an error for missing parties, invalid quantities, or incompatible production chains.
///
/// # Panics
///
/// Panics if an existing business contains an invalid recipe reference.
pub fn validate_supply_contract(
    registry: &Registry,
    state: &AppState,
    terms: SupplyContractTerms,
) -> Result<ValidatedSupplyContract, StrategicError> {
    validate_supply_contract_terms(registry, state, &terms)?;
    Ok(ValidatedSupplyContract { terms })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SupplyContractCapacity {
    pub(crate) buyer: Quantity,
    pub(crate) seller: Quantity,
}

pub(crate) fn available_supply_contract_capacity(
    registry: &Registry,
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    good_id: GoodId,
) -> Option<SupplyContractCapacity> {
    let buyer = state.businesses.get(buyer_business_id)?;
    let seller = state.businesses.get(seller_business_id)?;
    let buyer_recipe = registry.get_recipe(buyer.recipe_id())?;
    let seller_recipe = registry.get_recipe(seller.recipe_id())?;
    if seller_recipe.output_good_id() != good_id {
        return None;
    }
    let input_per_batch = buyer_recipe
        .inputs()
        .iter()
        .find(|input| input.good_id() == good_id)?
        .quantity();
    let seller_capacity = seller_recipe.output_quantity().saturating_mul_ratio(
        i64::from(seller.operations.capacity_batches_per_day)
            .saturating_mul(CONTRACT_CAPACITY_COMMITMENT_DAYS),
        1,
    );
    let buyer_capacity = input_per_batch.saturating_mul_ratio(
        i64::from(buyer.operations.capacity_batches_per_day)
            .saturating_mul(CONTRACT_CAPACITY_COMMITMENT_DAYS),
        1,
    );
    let committed_outgoing = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active
                && contract.seller_business_id == seller_business_id
                && contract.good_id == good_id
        })
        .fold(Quantity::ZERO, |total, contract| {
            total.saturating_add(contract.quantity_per_week)
        });
    let committed_incoming = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active
                && contract.buyer_business_id == buyer_business_id
                && contract.good_id == good_id
        })
        .fold(Quantity::ZERO, |total, contract| {
            total.saturating_add(contract.quantity_per_week)
        });
    Some(SupplyContractCapacity {
        buyer: buyer_capacity.saturating_sub(committed_incoming),
        seller: seller_capacity.saturating_sub(committed_outgoing),
    })
}

pub(crate) fn validate_supply_contract_terms(
    registry: &Registry,
    state: &AppState,
    terms: &SupplyContractTerms,
) -> Result<(), StrategicError> {
    ensure_registry_matches(registry, state)?;
    if terms.buyer_business_id == terms.seller_business_id {
        return Err(StrategicError::SameContractParty);
    }
    if terms.quantity_per_week <= Quantity::ZERO {
        return Err(StrategicError::NonPositiveQuantity);
    }
    if terms.unit_price <= Money::ZERO || terms.penalty < Money::ZERO {
        return Err(StrategicError::NonPositiveAmount);
    }
    if terms.duration_weeks == 0 {
        return Err(StrategicError::EmptyContractDuration);
    }
    checked_future_day(state.clock.day(), 7)?;
    checked_future_day(state.clock.day(), i64::from(terms.duration_weeks) * 7)?;
    checked_cost_for(terms.quantity_per_week, terms.unit_price).ok_or(
        StrategicError::ContractPaymentOverflow {
            quantity: terms.quantity_per_week,
            unit_price: terms.unit_price,
        },
    )?;
    let buyer =
        state
            .businesses
            .get(terms.buyer_business_id)
            .ok_or(StrategicError::MissingBusiness {
                business_id: terms.buyer_business_id,
            })?;
    let seller =
        state
            .businesses
            .get(terms.seller_business_id)
            .ok_or(StrategicError::MissingBusiness {
                business_id: terms.seller_business_id,
            })?;
    if buyer.owner_dynasty_id() == seller.owner_dynasty_id() {
        return Err(StrategicError::SameContractOwner {
            dynasty_id: buyer.owner_dynasty_id(),
        });
    }
    for business in [buyer, seller] {
        if matches!(
            business.status(),
            crate::core::BusinessStatus::Insolvent | crate::core::BusinessStatus::Closed
        ) {
            return Err(StrategicError::BusinessInactive {
                business_id: business.id(),
            });
        }
    }
    let seller_recipe = registry
        .get_recipe(seller.recipe_id())
        .expect("business recipe references must be validated");
    if seller_recipe.output_good_id() != terms.good_id {
        return Err(StrategicError::SellerCannotProduce {
            seller_business_id: terms.seller_business_id,
            good_id: terms.good_id,
        });
    }
    let buyer_recipe = registry
        .get_recipe(buyer.recipe_id())
        .expect("business recipe references must be validated");
    if !buyer_recipe
        .inputs()
        .iter()
        .any(|input| input.good_id() == terms.good_id)
    {
        return Err(StrategicError::BuyerDoesNotConsume {
            buyer_business_id: terms.buyer_business_id,
            good_id: terms.good_id,
        });
    }
    Ok(())
}

/// Validates and creates a supply contract through its canonical commit token.
///
/// # Errors
///
/// Returns the same errors as [`validate_supply_contract`], plus allocation or timeline exhaustion
/// while committing the contract and its durable feedback.
pub fn sign_supply_contract(
    registry: &Registry,
    state: &mut AppState,
    terms: SupplyContractTerms,
) -> Result<crate::ids::ContractId, StrategicError> {
    validate_supply_contract(registry, state, terms)?.commit(registry, state)
}

pub(crate) fn settle_contracts(state: &mut AppState) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let due: Vec<_> = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active && contract.next_due_day <= day
        })
        .map(|contract| DueContract {
            id: contract.id,
            buyer_id: contract.buyer_business_id,
            seller_id: contract.seller_business_id,
            good_id: contract.good_id,
            quantity: contract.quantity_per_week,
            unit_price: contract.unit_price,
            penalty: contract.penalty,
            due_day: contract.next_due_day,
            end_day: contract.end_day,
        })
        .collect();
    for due_contract in due {
        settle_due_contract(state, due_contract)?;
    }
    Ok(())
}

pub(crate) fn settle_due_contract(
    state: &mut AppState,
    due: DueContract,
) -> Result<(), SimulationError> {
    let final_delivery = is_final_contract_delivery(due);
    let payment = cost_for(due.quantity, due.unit_price);
    let (seller_active, seller_owner_id, seller_can_deliver) = {
        let seller = state
            .businesses
            .get(due.seller_id)
            .expect("contract seller must exist");
        (
            !matches!(
                seller.status(),
                BusinessStatus::Insolvent | BusinessStatus::Closed
            ),
            seller.owner_dynasty_id(),
            seller.inventory_quantity(due.good_id) >= due.quantity,
        )
    };
    let (buyer_active, buyer_owner_id, buyer_can_pay) = {
        let buyer = state
            .businesses
            .get(due.buyer_id)
            .expect("contract buyer must exist");
        (
            !matches!(
                buyer.status(),
                BusinessStatus::Insolvent | BusinessStatus::Closed
            ),
            buyer.owner_dynasty_id(),
            buyer.cash() >= payment,
        )
    };
    if !seller_active || !buyer_active {
        terminate_inactive_contract(
            state,
            &due,
            buyer_owner_id,
            seller_owner_id,
            buyer_active,
            seller_active,
        )?;
        return Ok(());
    }
    let settlement = ContractSettlementState {
        buyer: ContractPartySettlementState {
            owner_id: buyer_owner_id,
            can_perform: buyer_can_pay,
        },
        seller: ContractPartySettlementState {
            owner_id: seller_owner_id,
            can_perform: seller_can_deliver,
        },
    };
    let fulfilled = settlement.is_fulfilled();
    let terminates_for_misses = !fulfilled
        && state
            .contracts
            .get(&due.id)
            .expect("contract must exist")
            .missed_deliveries
            >= 2;
    let next_due_day = if final_delivery || terminates_for_misses {
        None
    } else {
        Some(checked_future_day(due.due_day, 7)?)
    };
    if fulfilled {
        let seller_cash = state
            .businesses
            .get(due.seller_id)
            .expect("contract seller must exist")
            .cash();
        seller_cash
            .checked_add(payment)
            .ok_or(SimulationError::BusinessCashOverflow {
                business_id: due.seller_id,
                current: seller_cash,
                incoming: payment,
            })?;
        let buyer_inventory = state
            .businesses
            .get(due.buyer_id)
            .expect("contract buyer must exist")
            .inventory_quantity(due.good_id);
        buyer_inventory.checked_add(due.quantity).ok_or(
            SimulationError::BusinessInventoryOverflow {
                business_id: due.buyer_id,
                good_id: due.good_id,
                current: buyer_inventory,
                incoming: due.quantity,
            },
        )?;
        settle_fulfilled_contract(state, due, payment, settlement, next_due_day)?;
    } else {
        settle_failed_contract(state, due, settlement, next_due_day)?;
    }
    finalize_expired_contract(state, due, settlement, fulfilled, final_delivery)?;
    Ok(())
}

pub(crate) fn is_final_contract_delivery(due: DueContract) -> bool {
    due.due_day >= due.end_day
        || due
            .end_day
            .checked_sub(due.due_day)
            .is_some_and(|remaining_days| remaining_days < 7)
}

/// Attributes a contract breach to the single inactive party and records the
/// other side as victim; mutual inactivity or no inactivity attributes nobody.
/// The canonical rule behind `ContractSettlementState` attribution helpers.
/// Canonical breach attribution shared by the termination and settlement
/// paths: exactly one inactive party breaches; mutual inactivity or full
/// performance attributes nothing.
pub(crate) const fn attribute_contract_breach(
    buyer_owner_id: DynastyId,
    seller_owner_id: DynastyId,
    buyer_active: bool,
    seller_active: bool,
) -> (Option<DynastyId>, Option<DynastyId>) {
    match (buyer_active, seller_active) {
        (false, true) => (Some(buyer_owner_id), Some(seller_owner_id)),
        (true, false) => (Some(seller_owner_id), Some(buyer_owner_id)),
        (false, false) | (true, true) => (None, None),
    }
}

/// Collects the contractual penalty from the at-fault party up to its cash on
/// hand and returns whatever remains unpayable as recoverable breach debt.
pub(crate) fn collect_partial_contract_penalty(
    state: &mut AppState,
    payer_id: BusinessId,
    recipient_id: BusinessId,
    penalty: Money,
) -> Result<Money, SimulationError> {
    let available = state
        .businesses
        .get(payer_id)
        .expect("contract penalty payer must exist")
        .cash();
    let transferred =
        transfer_contract_money(state, payer_id, recipient_id, penalty.min(available))?;
    Ok(penalty
        .checked_sub(transferred)
        .expect("bounded contract penalty transfer cannot exceed the contractual penalty"))
}

pub(crate) fn terminate_inactive_contract(
    state: &mut AppState,
    due: &DueContract,
    buyer_owner_id: DynastyId,
    seller_owner_id: DynastyId,
    buyer_active: bool,
    seller_active: bool,
) -> Result<(), SimulationError> {
    let (breaching_dynasty_id, breach_victim_dynasty_id) =
        attribute_contract_breach(buyer_owner_id, seller_owner_id, buyer_active, seller_active);
    // Total breach exposure is capped at the contractual penalty across the
    // contract's whole life: collected cash and recoverable breach debt
    // partition that one penalty, so termination collects only what the
    // contract still owes.
    let remaining_collectible = state
        .contracts
        .get(&due.id)
        .map_or(Money::ZERO, |contract| {
            contract
                .penalty
                .saturating_sub(contract.collected_breach_penalty)
        });
    // The victim collects whatever the inactive party can still pay before the
    // termination is recorded; only the genuinely unpayable remainder
    // accumulates as recoverable breach debt.
    let penalty_parties = match (!buyer_active, !seller_active) {
        (true, false) => Some((due.buyer_id, due.seller_id)),
        (false, true) => Some((due.seller_id, due.buyer_id)),
        _ => None,
    };
    let collection_attempt = due.penalty.min(remaining_collectible);
    let unpaid_of_attempt = if let Some((payer_id, recipient_id)) = penalty_parties {
        collect_partial_contract_penalty(state, payer_id, recipient_id, collection_attempt)?
    } else {
        Money::ZERO
    };
    let contract = state
        .contracts
        .get_mut(&due.id)
        .expect("contract must exist");
    contract.missed_deliveries = contract.missed_deliveries.saturating_add(1);
    if let Some((breacher, victim)) = breaching_dynasty_id.zip(breach_victim_dynasty_id) {
        // Preserve any earlier attribution: the defendant for accrued
        // recoverable debt must remain identifiable even when this
        // termination is mutually inactive.
        contract.breaching_dynasty_id = Some(breacher);
        contract.breach_victim_dynasty_id = Some(victim);
    }
    // Collected cash and recoverable breach debt jointly partition the
    // contractual penalty: the cash reaches the victim first and only the
    // unpayable remainder accrues as recoverable debt.
    contract.collected_breach_penalty = contract
        .collected_breach_penalty
        .checked_add(collection_attempt.saturating_sub(unpaid_of_attempt))
        .expect("capped penalty collection cannot exceed the contractual penalty");
    contract.unpaid_breach_penalty = contract
        .penalty
        .saturating_sub(contract.collected_breach_penalty);
    contract.status = ContractStatus::Breached;
    if buyer_owner_id != seller_owner_id {
        if !seller_active {
            adjust_reliability_reputation(state, seller_owner_id, -120);
        }
        if !buyer_active {
            adjust_reliability_reputation(state, buyer_owner_id, -120);
        }
        adjust_dynasty_relationship(
            state,
            buyer_owner_id,
            seller_owner_id,
            RelationshipDelta::new(-100, -40, 0, 120, 0),
        );
        remember_dynasty_interaction(
            state,
            buyer_owner_id,
            seller_owner_id,
            &format!(
                "Supply contract {} ended because a party became inactive.",
                due.id
            ),
        );
        try_record_counterparty_information(
            state,
            buyer_owner_id,
            seller_owner_id,
            "Contract termination and business-status records",
        )?;
    }
    try_push_outbox(
        state,
        OutboxKind::Contract,
        format!("Contract {} terminated", due.id),
        "An inactive contract party could no longer perform the scheduled obligation.".to_owned(),
    )?;
    Ok(())
}

/// Immediately terminates every active supply contract naming an insolvent or
/// closed business.
///
/// The daily lifecycle pass flips businesses to inactive statuses as soon as
/// their cash and inventory run out; leaving their contracts `Active` until
/// the next weekly settlement would rest the simulation in a state its own
/// lifecycle invariant forbids. Each termination follows the canonical
/// inactive-party path: breach attribution, penalty collection up to the
/// payer's cash with the remainder accruing as recoverable breach debt, and
/// durable relationship and information records.
pub(crate) fn terminate_active_contracts_for_business(
    state: &mut AppState,
    business_id: BusinessId,
) -> Result<(), SimulationError> {
    let affected: Vec<crate::ids::ContractId> = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active
                && (contract.buyer_business_id == business_id
                    || contract.seller_business_id == business_id)
        })
        .map(|contract| contract.id)
        .collect();
    for contract_id in affected {
        let due = {
            let contract = state
                .contracts
                .get(&contract_id)
                .expect("collected contract must exist");
            DueContract {
                id: contract.id,
                buyer_id: contract.buyer_business_id,
                seller_id: contract.seller_business_id,
                good_id: contract.good_id,
                quantity: contract.quantity_per_week,
                unit_price: contract.unit_price,
                penalty: contract.penalty,
                due_day: state.clock.day(),
                end_day: contract.end_day,
            }
        };
        let buyer_active = state.businesses.get(due.buyer_id).is_some_and(|business| {
            !matches!(
                business.status(),
                BusinessStatus::Insolvent | BusinessStatus::Closed
            )
        });
        let seller_active = state.businesses.get(due.seller_id).is_some_and(|business| {
            !matches!(
                business.status(),
                BusinessStatus::Insolvent | BusinessStatus::Closed
            )
        });
        debug_assert!(
            !buyer_active || !seller_active,
            "termination sweep must target a contract with an inactive party"
        );
        let buyer_owner_id = state
            .businesses
            .get(due.buyer_id)
            .expect("contract buyer must exist")
            .owner_dynasty_id();
        let seller_owner_id = state
            .businesses
            .get(due.seller_id)
            .expect("contract seller must exist")
            .owner_dynasty_id();
        terminate_inactive_contract(
            state,
            &due,
            buyer_owner_id,
            seller_owner_id,
            buyer_active,
            seller_active,
        )?;
    }
    Ok(())
}

pub(crate) fn finalize_expired_contract(
    state: &mut AppState,
    due: DueContract,
    settlement: ContractSettlementState,
    fulfilled: bool,
    final_delivery: bool,
) -> Result<(), SimulationError> {
    let expired_active = final_delivery
        && state
            .contracts
            .get(&due.id)
            .is_some_and(|contract| contract.status == ContractStatus::Active);
    if !expired_active {
        return Ok(());
    }
    let contract = state
        .contracts
        .get_mut(&due.id)
        .expect("contract must exist");
    contract.status = if fulfilled {
        ContractStatus::Fulfilled
    } else {
        ContractStatus::Breached
    };
    if let Some((breacher, victim)) = settlement
        .breaching_dynasty_id()
        .zip(settlement.breach_victim_dynasty_id())
    {
        // A final-delivery miss attributes its breach without erasing any
        // earlier attributable miss; fulfillment preserves accrued recoverable
        // debt because the victim already lost the copper.
        contract.breaching_dynasty_id = Some(breacher);
        contract.breach_victim_dynasty_id = Some(victim);
    }
    if settlement.buyer.owner_id != settlement.seller.owner_id {
        let memory = if fulfilled {
            format!("Supply contract {} completed successfully.", due.id)
        } else {
            format!("Supply contract {} expired in breach.", due.id)
        };
        remember_dynasty_interaction(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            &memory,
        );
        try_record_counterparty_information(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            "Completed contract performance records",
        )?;
    }
    if !fulfilled {
        try_push_outbox(
            state,
            OutboxKind::Contract,
            format!("Contract {} expired in breach", due.id),
            "The final scheduled delivery was not completed before the contract ended.".to_owned(),
        )?;
    }
    Ok(())
}

pub(crate) fn settle_fulfilled_contract(
    state: &mut AppState,
    due: DueContract,
    payment: Money,
    settlement: ContractSettlementState,
    next_due_day: Option<i64>,
) -> Result<(), SimulationError> {
    let transferred = transfer_contract_money(state, due.buyer_id, due.seller_id, payment)?;
    debug_assert_eq!(
        transferred, payment,
        "prevalidated contract payment must transfer in full"
    );
    state
        .businesses
        .get_mut(due.seller_id)
        .expect("contract seller must exist")
        .remove_inventory(due.good_id, due.quantity);
    state
        .businesses
        .get_mut(due.buyer_id)
        .expect("contract buyer must exist")
        .add_inventory(due.good_id, due.quantity);
    let contract = state
        .contracts
        .get_mut(&due.id)
        .expect("contract must exist");
    contract.fulfilled_deliveries = contract.fulfilled_deliveries.saturating_add(1);
    // Breach requires repeated nonperformance, so a delivered week ends any
    // run of misses instead of letting isolated slips accumulate forever.
    contract.missed_deliveries = 0;
    let buyer_deliveries = contract
        .fulfilled_deliveries_by_dynasty
        .entry(settlement.buyer.owner_id)
        .or_default();
    *buyer_deliveries = buyer_deliveries.saturating_add(1);
    if settlement.seller.owner_id != settlement.buyer.owner_id {
        let seller_deliveries = contract
            .fulfilled_deliveries_by_dynasty
            .entry(settlement.seller.owner_id)
            .or_default();
        *seller_deliveries = seller_deliveries.saturating_add(1);
    }
    if let Some(next_due_day) = next_due_day {
        contract.next_due_day = next_due_day;
    }
    if settlement.buyer.owner_id != settlement.seller.owner_id {
        adjust_reliability_reputation(state, settlement.buyer.owner_id, 20);
        adjust_reliability_reputation(state, settlement.seller.owner_id, 20);
        adjust_dynasty_relationship(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            RelationshipDelta::new(5, 3, 0, -2, 0),
        );
    }
    Ok(())
}

pub(crate) fn settle_failed_contract(
    state: &mut AppState,
    due: DueContract,
    settlement: ContractSettlementState,
    next_due_day: Option<i64>,
) -> Result<(), SimulationError> {
    let penalty_parties = match (
        settlement.seller_is_at_fault(),
        settlement.buyer_is_at_fault(),
    ) {
        (false, true) => Some((due.buyer_id, due.seller_id)),
        (true, false) => Some((due.seller_id, due.buyer_id)),
        (false, false) | (true, true) => None,
    };
    // Total breach exposure is capped at the contractual penalty across the
    // contract's whole life: collected cash and recoverable breach debt
    // partition that one penalty, so a repeat miss can only collect what the
    // contract still owes, never a fresh penalty per missed delivery.
    let remaining_collectible = state
        .contracts
        .get(&due.id)
        .map_or(Money::ZERO, |contract| {
            contract
                .penalty
                .saturating_sub(contract.collected_breach_penalty)
        });
    let collection_attempt = due.penalty.min(remaining_collectible);
    let unpaid_of_attempt = if let Some((payer_id, recipient_id)) = penalty_parties {
        collect_partial_contract_penalty(state, payer_id, recipient_id, collection_attempt)?
    } else {
        Money::ZERO
    };
    let collected_now = collection_attempt.saturating_sub(unpaid_of_attempt);
    let breached = {
        let contract = state
            .contracts
            .get_mut(&due.id)
            .expect("contract must exist");
        contract.missed_deliveries = contract.missed_deliveries.saturating_add(1);
        if let Some(next_due_day) = next_due_day {
            contract.next_due_day = next_due_day;
        }
        if let Some((breacher, victim)) = settlement
            .breaching_dynasty_id()
            .zip(settlement.breach_victim_dynasty_id())
        {
            // Attribution records who owes the recoverable claim from the
            // first attributable miss and persists until the claim is
            // discharged, so later settlements cannot orphan accrued debt.
            contract.breaching_dynasty_id = Some(breacher);
            contract.breach_victim_dynasty_id = Some(victim);
        }
        if contract.missed_deliveries >= 3 {
            contract.status = ContractStatus::Breached;
        }
        if settlement.has_attributable_nonperformance() {
            // Collected cash and recoverable breach debt jointly partition the
            // contractual penalty: the cash reaches the victim first and only
            // the unpayable remainder accrues as recoverable debt.
            contract.collected_breach_penalty = contract
                .collected_breach_penalty
                .checked_add(collected_now)
                .expect("capped penalty collection cannot exceed the contractual penalty");
            contract.unpaid_breach_penalty = contract
                .penalty
                .saturating_sub(contract.collected_breach_penalty);
        }
        contract.status == ContractStatus::Breached
    };
    if breached {
        try_push_outbox(
            state,
            OutboxKind::Contract,
            format!("Contract {} breached", due.id),
            format!(
                "Repeated nonperformance caused supply contract {} to terminate.",
                due.id
            ),
        )?;
    }
    if settlement.has_attributable_nonperformance()
        && settlement.buyer.owner_id != settlement.seller.owner_id
    {
        if settlement.seller_is_at_fault() {
            adjust_reliability_reputation(state, settlement.seller.owner_id, -120);
        }
        if settlement.buyer_is_at_fault() {
            adjust_reliability_reputation(state, settlement.buyer.owner_id, -120);
        }
        adjust_dynasty_relationship(
            state,
            settlement.buyer.owner_id,
            settlement.seller.owner_id,
            RelationshipDelta::new(-30, -10, 0, 40, 0),
        );
        if breached {
            remember_dynasty_interaction(
                state,
                settlement.buyer.owner_id,
                settlement.seller.owner_id,
                &format!(
                    "Supply contract {} was terminated for repeated nonperformance.",
                    due.id
                ),
            );
            try_record_counterparty_information(
                state,
                settlement.buyer.owner_id,
                settlement.seller.owner_id,
                "Contract breach and penalty records",
            )?;
        }
    }
    Ok(())
}

pub(crate) fn transfer_contract_money(
    state: &mut AppState,
    payer_id: BusinessId,
    recipient_id: BusinessId,
    amount: Money,
) -> Result<Money, SimulationError> {
    if amount <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    let payer_cash = state
        .businesses
        .get(payer_id)
        .expect("contract payer must exist")
        .cash();
    let recipient_cash = state
        .businesses
        .get(recipient_id)
        .expect("contract recipient must exist")
        .cash();
    let transferred = amount.min(payer_cash);
    if transferred <= Money::ZERO {
        return Ok(Money::ZERO);
    }
    recipient_cash
        .checked_add(transferred)
        .ok_or(SimulationError::BusinessCashOverflow {
            business_id: recipient_id,
            current: recipient_cash,
            incoming: transferred,
        })?;
    let (payer_lifetime_costs, payer_finance_version) = {
        let payer = state
            .businesses
            .get(payer_id)
            .expect("contract payer must exist");
        (
            payer
                .finance
                .lifetime_costs
                .checked_add(transferred)
                .ok_or(SimulationError::BusinessLifetimeCostsOverflow {
                    business_id: payer_id,
                    current: payer.finance.lifetime_costs,
                    incoming: transferred,
                })?,
            next_business_finance_version(payer)?,
        )
    };
    let (recipient_lifetime_revenue, recipient_finance_version) = {
        let recipient = state
            .businesses
            .get(recipient_id)
            .expect("contract recipient must exist");
        (
            recipient
                .finance
                .lifetime_revenue
                .checked_add(transferred)
                .ok_or(SimulationError::BusinessLifetimeRevenueOverflow {
                    business_id: recipient_id,
                    current: recipient.finance.lifetime_revenue,
                    incoming: transferred,
                })?,
            next_business_finance_version(recipient)?,
        )
    };
    {
        let payer = state
            .businesses
            .get_mut(payer_id)
            .expect("contract payer must exist");
        payer.finance.cash = payer
            .finance
            .cash
            .checked_sub(transferred)
            .expect("bounded contract transfer must fit payer cash");
        payer.finance.lifetime_costs = payer_lifetime_costs;
        payer.finance.version = payer_finance_version;
    }
    {
        let recipient = state
            .businesses
            .get_mut(recipient_id)
            .expect("contract recipient must exist");
        recipient.finance.cash = recipient
            .finance
            .cash
            .checked_add(transferred)
            .expect("bounded contract transfer must fit recipient cash");
        recipient.finance.lifetime_revenue = recipient_lifetime_revenue;
        recipient.finance.version = recipient_finance_version;
    }
    Ok(transferred)
}
