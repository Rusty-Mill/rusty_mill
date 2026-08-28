//! [`PathSegmentsMut`]: editing a [`Url`]'s path as a sequence of segments.

use crate::parser::{self, to_u32, Context, SchemeType};
use crate::Url;

/// Edits a [`Url`]'s path as a sequence of `/`-separated segments, via
/// [`Url::path_segments_mut`].
///
/// The path always starts with `/` and has at least one segment (which may
/// be empty). Changes take effect when this value is dropped.
///
/// ```
/// # use rusty_url::Url;
/// # fn example() -> Result<(), ()> {
/// let mut url = Url::parse("http://example.net/foo/index.html").unwrap();
/// url.path_segments_mut()?.pop().push("img").push("2/100%.png");
/// assert_eq!(url.as_str(), "http://example.net/foo/img/2%2F100%25.png");
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct PathSegmentsMut<'a> {
    url: &'a mut Url,
    after_first_slash: usize,
    after_path: String,
    old_after_path_position: u32,
}

pub(crate) fn new(url: &mut Url) -> PathSegmentsMut<'_> {
    let after_path = url.take_after_path();
    let old_after_path_position = to_u32(url.serialization.len()).unwrap();
    PathSegmentsMut {
        after_first_slash: url.path_start as usize + "/".len(),
        url,
        old_after_path_position,
        after_path,
    }
}

impl Drop for PathSegmentsMut<'_> {
    fn drop(&mut self) {
        self.url
            .restore_after_path(self.old_after_path_position, &self.after_path);
    }
}

impl PathSegmentsMut<'_> {
    /// Remove all segments, leaving `url.path() == "/"`.
    pub fn clear(&mut self) -> &mut Self {
        self.url.serialization.truncate(self.after_first_slash);
        self
    }

    /// Remove the last segment if it's empty (i.e. remove one trailing
    /// slash), unless that's also the path's only (initial) slash.
    pub fn pop_if_empty(&mut self) -> &mut Self {
        if self.after_first_slash >= self.url.serialization.len() {
            return self;
        }
        if self.url.serialization[self.after_first_slash..].ends_with('/') {
            self.url.serialization.pop();
        }
        self
    }

    /// Remove the last segment. If there was only one, it becomes empty
    /// (`url.path() == "/"`).
    pub fn pop(&mut self) -> &mut Self {
        if self.after_first_slash >= self.url.serialization.len() {
            return self;
        }
        let last_slash = self.url.serialization[self.after_first_slash..]
            .rfind('/')
            .unwrap_or(0);
        self.url
            .serialization
            .truncate(self.after_first_slash + last_slash);
        self
    }

    /// Append one segment. See [`PathSegmentsMut::extend`].
    pub fn push(&mut self, segment: &str) -> &mut Self {
        self.extend(Some(segment))
    }

    /// Append each segment from `segments`.
    ///
    /// Each segment is percent-encoded like a normal path, except `%` and
    /// `/` are also escaped (to `%25`/`%2F`) — unlike parsing, where `%` is
    /// left alone (it might already be an escape) and `/` separates
    /// segments. Here, a segment is one opaque unit.
    ///
    /// This always adds a `/` between the existing path and the new
    /// segments, except when the existing path is exactly `/`; segments
    /// are joined by `/` too. If the previous last segment was empty (a
    /// trailing slash), the result has two consecutive slashes — call
    /// [`PathSegmentsMut::pop_if_empty`] first to avoid that. For
    /// `Url::join`-like replacement of the last segment, call
    /// [`PathSegmentsMut::pop`] first.
    ///
    /// A segment that is exactly `"."` or `".."` is skipped, so that
    /// parsing the resulting serialization round-trips to the same URL.
    pub fn extend<I>(&mut self, segments: I) -> &mut Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let scheme_type = SchemeType::from(self.url.scheme());
        let path_start = self.url.path_start as usize;
        self.url.mutate(|parser| {
            parser.context = Context::PathSegmentSetter;
            for segment in segments {
                let segment = segment.as_ref();
                if matches!(segment, "." | "..") {
                    continue;
                }
                if parser.serialization.len() > path_start + 1
                    || parser.serialization.len() == path_start
                {
                    parser.serialization.push('/');
                }
                let mut has_host = true;
                parser.parse_path(
                    scheme_type,
                    &mut has_host,
                    path_start,
                    parser::Input::new_no_trim(segment),
                );
            }
        });
        self
    }
}
