use std::fmt;

macro_rules! id {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            /// Wrap a raw id, e.g. when rehydrating from storage.
            pub fn from_raw(raw: u64) -> Self {
                Self(raw)
            }

            /// The raw numeric id.
            pub fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "-{}"), self.0)
            }
        }
    };
}

id!(GoalId, "G", "Goal identifier, assigned by the store.");
id!(
    TaskId,
    "T",
    "Task identifier, unique within one plan (1-based)."
);
id!(
    EntryId,
    "E",
    "Blackboard entry identifier, unique within one board (1-based)."
);
