//! Data classes (#598 2.2): what may reach which provider.

use std::fmt;

/// Ordered classes: a provider cleared for one sees it and everything below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataClass {
    Public,
    Internal,
    Sensitive,
}

impl DataClass {
    /// Parse a config word.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "public" => Some(Self::Public),
            "internal" => Some(Self::Internal),
            "sensitive" => Some(Self::Sensitive),
            _ => None,
        }
    }

    /// The config word.
    pub fn name(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Sensitive => "sensitive",
        }
    }
}

impl fmt::Display for DataClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// May an item of class `item` go to a provider cleared to `clearance`?
pub fn admit(item: DataClass, clearance: DataClass) -> bool {
    item <= clearance
}

#[cfg(test)]
mod tests {
    use super::*;
    use DataClass::*;

    #[test]
    fn admit_is_at_or_below_clearance() {
        assert!(admit(Public, Internal));
        assert!(admit(Internal, Internal));
        assert!(!admit(Sensitive, Internal));
        assert!(admit(Sensitive, Sensitive));
    }

    #[test]
    fn words_round_trip() {
        for c in [Public, Internal, Sensitive] {
            assert_eq!(DataClass::parse(c.name()), Some(c));
        }
        assert_eq!(DataClass::parse("secret"), None);
    }
}
