//! [`Position`]: slicing a [`Url`] by component boundary.

use std::ops::{Index, Range, RangeFrom, RangeFull, RangeTo};

use crate::Url;

/// A position within a URL's serialized string, based on its components.
///
/// Slice a [`Url`] with a range of positions to get a sub-string:
///
/// ```
/// # use rusty_url::{Url, Position};
/// # fn example(url: Url) {
/// let serialization: &str = &url[..];
/// let without_fragment: &str = &url[..Position::AfterQuery];
/// let authority: &str = &url[Position::BeforeUsername..Position::AfterPort];
/// let scheme_relative: &str = &url[Position::BeforeUsername..];
/// # }
/// ```
///
/// In pseudo-grammar (`[`…`]?` marks an optional sub-sequence):
///
/// ```text
/// url =
///     scheme ":"
///     [ "//" [ username [ ":" password ]? "@" ]? host [ ":" port ]? ]?
///     path [ "?" query ]? [ "#" fragment ]?
/// ```
///
/// When a component is absent, its `Before*`/`After*` positions coincide
/// (so `&url[BeforeFoo..AfterFoo]` is `""`) while still preserving
/// component order (a missing query sits "between" the path and the
/// fragment). The initial `/` of a path is part of the path, not a
/// delimiter — so `&url[..BeforeFragment]` includes a `#` if present;
/// `&url[..AfterQuery]` is usually what you want instead.
#[derive(Debug, Clone, Copy)]
pub enum Position {
    BeforeScheme,
    AfterScheme,
    BeforeUsername,
    AfterUsername,
    BeforePassword,
    AfterPassword,
    BeforeHost,
    AfterHost,
    BeforePort,
    AfterPort,
    BeforePath,
    AfterPath,
    BeforeQuery,
    AfterQuery,
    BeforeFragment,
    AfterFragment,
}

fn count_digits(mut n: u16) -> usize {
    let mut digits = 1;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits
}

impl Url {
    fn position_index(&self, position: Position) -> usize {
        match position {
            Position::BeforeScheme => 0,
            Position::AfterScheme => self.scheme_end as usize,
            Position::BeforeUsername => {
                if self.has_authority() {
                    self.scheme_end as usize + "://".len()
                } else {
                    self.scheme_end as usize + ":".len()
                }
            }
            Position::AfterUsername => self.username_end as usize,
            Position::BeforePassword => {
                if self.has_authority()
                    && self.serialization.as_bytes()[self.username_end as usize] == b':'
                {
                    self.username_end as usize + ":".len()
                } else {
                    self.username_end as usize
                }
            }
            Position::AfterPassword => {
                if self.has_authority()
                    && self.serialization.as_bytes()[self.username_end as usize] == b':'
                {
                    self.host_start as usize - "@".len()
                } else {
                    self.host_start as usize
                }
            }
            Position::BeforeHost => self.host_start as usize,
            Position::AfterHost => self.host_end as usize,
            Position::BeforePort => {
                if self.port.is_some() {
                    self.host_end as usize + ":".len()
                } else {
                    self.host_end as usize
                }
            }
            Position::AfterPort => match self.port {
                Some(port) => self.host_end as usize + ":".len() + count_digits(port),
                None => self.host_end as usize,
            },
            Position::BeforePath => self.path_start as usize,
            Position::AfterPath => match (self.query_start, self.fragment_start) {
                (Some(q), _) => q as usize,
                (None, Some(f)) => f as usize,
                (None, None) => self.serialization.len(),
            },
            Position::BeforeQuery => match (self.query_start, self.fragment_start) {
                (Some(q), _) => q as usize + "?".len(),
                (None, Some(f)) => f as usize,
                (None, None) => self.serialization.len(),
            },
            Position::AfterQuery => match self.fragment_start {
                Some(f) => f as usize,
                None => self.serialization.len(),
            },
            Position::BeforeFragment => match self.fragment_start {
                Some(f) => f as usize + "#".len(),
                None => self.serialization.len(),
            },
            Position::AfterFragment => self.serialization.len(),
        }
    }
}

impl Index<RangeFull> for Url {
    type Output = str;
    fn index(&self, _: RangeFull) -> &str {
        &self.serialization
    }
}

impl Index<RangeFrom<Position>> for Url {
    type Output = str;
    fn index(&self, range: RangeFrom<Position>) -> &str {
        &self.serialization[self.position_index(range.start)..]
    }
}

impl Index<RangeTo<Position>> for Url {
    type Output = str;
    fn index(&self, range: RangeTo<Position>) -> &str {
        &self.serialization[..self.position_index(range.end)]
    }
}

impl Index<Range<Position>> for Url {
    type Output = str;
    fn index(&self, range: Range<Position>) -> &str {
        &self.serialization[self.position_index(range.start)..self.position_index(range.end)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_range_is_the_whole_serialization() {
        let url = Url::parse("https://example.com/path?q#f").unwrap();
        assert_eq!(&url[..], url.as_str());
    }

    #[test]
    fn before_query_to_end_excludes_fragment_delimiter() {
        let url = Url::parse("https://example.com/path?q#f").unwrap();
        assert_eq!(&url[..Position::AfterQuery], "https://example.com/path?q");
    }

    #[test]
    fn authority_range() {
        let url = Url::parse("https://user:pass@example.com:8080/path").unwrap();
        assert_eq!(
            &url[Position::BeforeUsername..Position::AfterPort],
            "user:pass@example.com:8080"
        );
    }

    #[test]
    fn scheme_relative_range() {
        let url = Url::parse("https://example.com/path").unwrap();
        assert_eq!(&url[Position::BeforeUsername..], "example.com/path");
    }

    #[test]
    fn missing_component_is_empty_range() {
        let url = Url::parse("https://example.com/path").unwrap();
        assert_eq!(&url[Position::BeforeQuery..Position::AfterQuery], "");
        assert_eq!(&url[Position::BeforeFragment..Position::AfterFragment], "");
        assert_eq!(&url[Position::BeforePort..Position::AfterPort], "");
    }

    #[test]
    fn data_url_payload_via_before_path_to_after_query() {
        let url = Url::parse("data:text/plain,hello?not-a-query").unwrap();
        assert_eq!(
            &url[Position::BeforePath..Position::AfterQuery],
            "text/plain,hello?not-a-query"
        );
    }

    #[test]
    fn path_range_matches_path_accessor() {
        let url = Url::parse("https://example.com/a/b?q#f").unwrap();
        assert_eq!(&url[Position::BeforePath..Position::AfterPath], url.path());
    }
}
