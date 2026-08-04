//! Typed identifiers for registry definitions and persistent runtime records.

use serde::{Deserialize, Serialize};
use std::fmt;

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
