//! Link safety (C63, gui-design D69-D72): decide what a clicked link may do.
//!
//! Pure, no I/O. Terminal output is untrusted, so a URL is parsed once into
//! a [`Link`] (or refused with a [`Deny`], never to open whatever the user
//! clicks), and [`Policy::verdict`] answers [`Verdict::Open`] (trusted
//! origin) or [`Verdict::Ask`] (the user decides). The window acts on the verdict; nothing
//! else may launch an opener.

use std::collections::BTreeSet;

/// URLs longer than this are denied: they hide their tail in any prompt.
pub(crate) const MAX_LEN: usize = 2048;

/// Why a link can never open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Deny {
    /// Empty, over [`MAX_LEN`], or holds a control character or whitespace.
    Malformed,
    /// A scheme we never hand to the OS (`javascript:`, `data:`, app schemes).
    Scheme,
    /// An http(s) URL with no host, or with userinfo (`user@host` spoofing).
    Host,
}

/// The schemes we know. Anything else is [`Deny::Scheme`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scheme {
    Http,
    Https,
    Mailto,
    Ftp,
    File,
}

/// A URL that passed parsing: safe to show and to hand to the opener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    pub scheme: Scheme,
    /// The URL as clicked.
    pub url: String,
    /// `scheme://host[:port]`, lower-case, default port dropped. Empty for
    /// `mailto:` and `file:` (no origin to trust).
    pub origin: String,
}

/// What to do with a [`Link`] that parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Open,
    /// Ask. `can_trust` says whether "Always trust" may be offered.
    Ask {
        can_trust: bool,
    },
}

/// Parse `url`, or say why it can never open.
pub(crate) fn parse(url: &str) -> Result<Link, Deny> {
    if url.is_empty()
        || url.len() > MAX_LEN
        || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(Deny::Malformed);
    }
    let (scheme, rest) = split_scheme(url).ok_or(Deny::Scheme)?;
    let origin = match scheme {
        Scheme::Http | Scheme::Https => origin_of(scheme, rest)?,
        Scheme::Ftp => String::new(),
        Scheme::Mailto | Scheme::File => String::new(),
    };
    Ok(Link {
        scheme,
        url: url.to_owned(),
        origin,
    })
}

fn split_scheme(url: &str) -> Option<(Scheme, &str)> {
    let (name, rest) = url.split_once(':')?;
    let scheme = match name.to_ascii_lowercase().as_str() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        "mailto" => Scheme::Mailto,
        "ftp" => Scheme::Ftp,
        "file" => Scheme::File,
        _ => return None,
    };
    let needs_slashes = !matches!(scheme, Scheme::Mailto);
    if needs_slashes {
        return rest.strip_prefix("//").map(|r| (scheme, r));
    }
    (!rest.is_empty()).then_some((scheme, rest))
}

/// `scheme://host[:port]` from the authority at the start of `rest`.
fn origin_of(scheme: Scheme, rest: &str) -> Result<String, Deny> {
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return Err(Deny::Host);
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !p.contains(']') => (h, Some(p)),
        _ => (authority, None),
    };
    if host.is_empty()
        || port.is_some_and(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Deny::Host);
    }
    let default = match scheme {
        Scheme::Http => "80",
        _ => "443",
    };
    let name = match scheme {
        Scheme::Http => "http",
        _ => "https",
    };
    let host = host.to_ascii_lowercase();
    Ok(match port.filter(|p| *p != default) {
        Some(p) => format!("{name}://{host}:{p}"),
        None => format!("{name}://{host}"),
    })
}

/// The user's trusted origins and the rules on them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Policy {
    trusted: BTreeSet<String>,
}

impl Policy {
    pub(crate) fn new(trusted: impl IntoIterator<Item = String>) -> Self {
        Self {
            trusted: trusted.into_iter().collect(),
        }
    }

    /// What `link` may do. Only http(s) to a trusted origin opens unasked.
    pub(crate) fn verdict(&self, link: &Link) -> Verdict {
        match link.scheme {
            Scheme::Http | Scheme::Https if self.trusted.contains(&link.origin) => Verdict::Open,
            Scheme::Http | Scheme::Https => Verdict::Ask { can_trust: true },
            // Other apps open: ask every time, and never remember it.
            Scheme::Mailto | Scheme::Ftp | Scheme::File => Verdict::Ask { can_trust: false },
        }
    }

    /// Trust `link`'s origin. False (and no change) if it has none.
    pub(crate) fn trust(&mut self, link: &Link) -> bool {
        !link.origin.is_empty() && self.trusted.insert(link.origin.clone())
    }

    /// The trusted origins, sorted, for saving.
    pub(crate) fn origins(&self) -> impl Iterator<Item = &str> {
        self.trusted.iter().map(String::as_str)
    }

    /// The file text: `origins = ["https://a.com", ...]`.
    pub(crate) fn to_toml(&self) -> String {
        let items: Vec<String> = self.origins().map(|o| format!("\"{o}\"")).collect();
        format!("origins = [{}]\n", items.join(", "))
    }

    /// Read [`Policy::to_toml`]'s text. Entries that are not a normalised
    /// http(s) origin are dropped, so a hand-edited or damaged file can
    /// only ever trust fewer things, never a malformed one.
    pub(crate) fn from_toml(text: &str) -> Self {
        let Some((_, list)) = text.split_once('=') else {
            return Self::default();
        };
        let list = list.trim().trim_start_matches('[').trim_end_matches(']');
        let valid = list.split(',').filter_map(|item| {
            let o = item.trim().strip_prefix('"')?.strip_suffix('"')?;
            let l = parse(o).ok()?;
            (l.origin == o).then(|| o.to_owned())
        });
        Self::new(valid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(u: &str) -> Link {
        parse(u).unwrap()
    }

    #[test]
    fn origin_is_normalised() {
        assert_eq!(
            link("HTTPS://Example.COM/a?b#c").origin,
            "https://example.com"
        );
        assert_eq!(
            link("https://example.com:443/").origin,
            "https://example.com"
        );
        assert_eq!(link("http://example.com:80").origin, "http://example.com");
        assert_eq!(
            link("http://example.com:8080/x").origin,
            "http://example.com:8080"
        );
        assert_eq!(link("https://example.com?q").origin, "https://example.com");
    }

    #[test]
    fn unknown_schemes_are_denied() {
        for u in [
            "javascript:alert(1)",
            "data:text/html,x",
            "ssh://h",
            "vscode://x",
            "x",
        ] {
            assert_eq!(parse(u), Err(Deny::Scheme), "{u}");
        }
        assert_eq!(parse("http:example.com"), Err(Deny::Scheme));
    }

    #[test]
    fn malformed_is_denied() {
        let long = format!("https://a.com/{}", "a".repeat(MAX_LEN));
        for u in [
            "",
            "https://a.com/ b",
            "https://a.com/\n",
            "https://a.com/\u{1b}[",
            &long,
        ] {
            assert_eq!(parse(u), Err(Deny::Malformed), "{u:?}");
        }
    }

    #[test]
    fn shell_metacharacters_stay_inert_but_spaces_do_not() {
        // `&` is legal in a URL; the opener must not use a shell (see window.rs).
        assert!(parse("https://a.com/?x=1&y=2").is_ok());
        assert_eq!(parse("https://a.com/ & calc"), Err(Deny::Malformed));
    }

    #[test]
    fn spoofed_hosts_are_denied() {
        for u in [
            "https://google.com@evil.com/",
            "https://good.com\\@evil.com/",
            "https:///x",
            "https://:80/",
            "https://a.com:/",
            "https://a.com:80a/",
        ] {
            assert_eq!(parse(u), Err(Deny::Host), "{u}");
        }
    }

    #[test]
    fn only_trusted_web_origins_open() {
        let mut p = Policy::default();
        let l = link("https://example.com/x");
        assert_eq!(p.verdict(&l), Verdict::Ask { can_trust: true });
        assert!(p.trust(&l));
        assert_eq!(p.verdict(&link("https://example.com/other")), Verdict::Open);
        // A different scheme, port or host is a different origin.
        for u in [
            "http://example.com/",
            "https://example.com:8443/",
            "https://sub.example.com/",
        ] {
            assert_eq!(p.verdict(&link(u)), Verdict::Ask { can_trust: true }, "{u}");
        }
    }

    #[test]
    fn other_apps_always_ask_and_cannot_be_trusted() {
        let mut p = Policy::default();
        for u in ["mailto:a@b.c", "ftp://h/f", "file:///etc/passwd"] {
            let l = link(u);
            assert!(!p.trust(&l), "{u}");
            assert_eq!(p.verdict(&l), Verdict::Ask { can_trust: false }, "{u}");
        }
        assert_eq!(p.origins().count(), 0);
    }

    #[test]
    fn the_file_round_trips_and_drops_bad_entries() {
        let mut p = Policy::default();
        p.trust(&link("https://a.com/"));
        p.trust(&link("http://b.com:8080/"));
        assert_eq!(Policy::from_toml(&p.to_toml()), p);
        let hand = r#"origins = ["https://ok.com", "https://x.com/path", "javascript:1", "https://Up.com", "ftp://h"]"#;
        assert_eq!(
            Policy::from_toml(hand),
            Policy::new(["https://ok.com".to_owned()])
        );
        assert_eq!(Policy::from_toml("garbage"), Policy::default());
        assert_eq!(Policy::from_toml(""), Policy::default());
    }

    #[test]
    fn trust_is_idempotent_and_sorted() {
        let mut p = Policy::new(["https://b.com".to_owned()]);
        assert!(p.trust(&link("https://a.com/")));
        assert!(!p.trust(&link("https://a.com/z")));
        assert_eq!(
            p.origins().collect::<Vec<_>>(),
            ["https://a.com", "https://b.com"]
        );
    }
}
