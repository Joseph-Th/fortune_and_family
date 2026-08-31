//! Typed persistent identifiers for registry definitions and runtime records.
//!
//! Purpose: give every cross-subsystem reference a distinct compile-time
//! type so `DistrictId` cannot be confused with `DynastyId`, and make
//! persistent relationships explicit `*Id` values rather than names or
//! positional indexes.
//! Owns: all `*Id` newtypes, their ordering/hashing/display/`serde` shape,
//! and the `IdentifierAllocationError` variants used when a state-owned
//! `NextIds` counter exhausts its `u32` space.
//! Reads: nothing.
//! Mutates: nothing directly (allocation mutates `NextIds` in
//! `src/core/state.rs`; IDs themselves are value types).
//! Does not own: state storage, registry definitions, or domain validation.
//! Canonical operations: `*Id::new(value)` ↔ `*Id::value()` (transparent
//! `u32` round-trip), display as decimal, ordered use as `BTreeMap` keys
//! and deterministic tie-breakers.
//! Relevant invariants: every `*Id` is `Copy + Ord + Hash + Eq` so it can
//! serve as a stable deterministic tie-breaker; serialized shape is
//! transparent `u32`; IDs never cross `NextIds` exhaustion sentinel
//! (`u32::MAX - 1` and `u32::MAX` are invalid allocator states).
//! Focused tests: `src/core/state_tests.rs` allocation, exhaustion, and
//! staleness; `src/persistence_tests.rs` typed-ID recovery.

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Defines one typed persistent identifier newtype (`*Id`).
///
/// Each expansion produces a transparent `u32` newtype that is `Copy + Ord +
/// Hash` so it can serve as a stable deterministic tie-breaker and
/// `BTreeMap` key. IDs are never reused; exhaustion is signaled via
/// `IdentifierAllocationError` and `NextIds` reserves the sentinel values
/// `u32::MAX - 1` and `u32::MAX`.
macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u32);

        impl $name {
            #[must_use]
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn value(self) -> u32 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

define_id!(DistrictId);
define_id!(GoodId);
define_id!(RecipeId);
define_id!(InstitutionId);
define_id!(DynastyId);
define_id!(CharacterId);
define_id!(HouseholdId);
define_id!(BusinessId);
define_id!(ContractId);
define_id!(PropertyId);
define_id!(LoanId);
define_id!(CivicDebtId);
define_id!(EmploymentId);
define_id!(FamilyLinkId);
define_id!(LawId);
define_id!(InformationReportId);
define_id!(ObjectiveId);
define_id!(PublicWorkId);
define_id!(LegalCaseId);
define_id!(ExternalRouteId);
define_id!(CrisisId);
define_id!(OutboxMessageId);
define_id!(ChronicleEntryId);

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum IdentifierAllocationError {
    #[error("dynasty identifier space is exhausted")]
    Dynasty,
    #[error("character identifier space is exhausted")]
    Character,
    #[error("household identifier space is exhausted")]
    Household,
    #[error("business identifier space is exhausted")]
    Business,
    #[error("contract identifier space is exhausted")]
    Contract,
    #[error("property identifier space is exhausted")]
    Property,
    #[error("loan identifier space is exhausted")]
    Loan,
    #[error("civic debt identifier space is exhausted")]
    CivicDebt,
    #[error("employment identifier space is exhausted")]
    Employment,
    #[error("family link identifier space is exhausted")]
    FamilyLink,
    #[error("law identifier space is exhausted")]
    Law,
    #[error("information report identifier space is exhausted")]
    InformationReport,
    #[error("objective identifier space is exhausted")]
    Objective,
    #[error("public work identifier space is exhausted")]
    PublicWork,
    #[error("legal case identifier space is exhausted")]
    LegalCase,
    #[error("external route identifier space is exhausted")]
    ExternalRoute,
    #[error("crisis identifier space is exhausted")]
    Crisis,
    #[error("outbox message identifier space is exhausted")]
    OutboxMessage,
    #[error("chronicle entry identifier space is exhausted")]
    ChronicleEntry,
}
