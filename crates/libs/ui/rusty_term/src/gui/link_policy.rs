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
    /// A host that is missing, has userinfo (`user@host` spoofing), or is
    /// not plain ASCII letters, digits, dots and hyphens (or a bracketed IPv6).
    Host,
    /// A `file:` link to another computer (a UNC share leaks the login hash
    /// or runs a remote program) or to a program the opener would run.
    File,
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
        || url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || is_invisible(c))
    {
        return Err(Deny::Malformed);
    }
    let (scheme, rest) = split_scheme(url).ok_or(Deny::Scheme)?;
    let origin = match scheme {
        Scheme::Http | Scheme::Https => origin_of(scheme, rest)?,
        Scheme::Ftp => {
            origin_of(Scheme::Https, rest)?;
            String::new()
        }
        Scheme::File => {
            check_file(rest)?;
            String::new()
        }
        Scheme::Mailto => {
            check_mailto(rest)?;
            String::new()
        }
    };
    Ok(Link {
        scheme,
        url: url.to_owned(),
        origin,
    })
}

/// A web link to a port of the pane's own host (`localhost`, `127.0.0.1`
/// or `[::1]`), which on a remote pane means the remote machine (C81).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Loopback {
    pub scheme: &'static str,
    pub port: u16,
    /// What follows the authority: path, query and fragment.
    pub rest: String,
    /// The host was `[::1]`: the tunnel dials IPv4 only, so such a link is
    /// refused rather than reaching whatever listens on 127.0.0.1.
    pub v6: bool,
}

impl Loopback {
    /// The same link at `local` port of this machine.
    pub(crate) fn at(&self, local: u16) -> String {
        format!("{}://127.0.0.1:{local}{}", self.scheme, self.rest)
    }
}

/// `url` as a [`Loopback`] link, if it is one: http or https, no userinfo,
/// a loopback host by name or literal, and a port (the scheme's own when
/// absent). Anything else, including `0.0.0.0`, is left to [`parse`].
pub(crate) fn loopback(url: &str) -> Option<Loopback> {
    if parse(url).is_err() {
        return None;
    }
    let (scheme, rest) = match split_scheme(url)? {
        (Scheme::Http, rest) => ("http", rest),
        (Scheme::Https, rest) => ("https", rest),
        _ => return None,
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    // A bracketed IPv6 literal holds colons itself: the port follows `]`.
    let (host, port) = match authority.strip_prefix('[') {
        Some(_) => match authority.split_once(']') {
            Some((inner, "")) => (format!("{inner}]"), None),
            Some((inner, tail)) => (format!("{inner}]"), Some(tail.strip_prefix(':')?)),
            None => return None,
        },
        None => match authority.split_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port)),
            None => (authority.to_owned(), None),
        },
    };
    let port = match port {
        Some(p) => Some(p.parse::<u16>().ok().filter(|p| *p != 0)?),
        None => None,
    };
    if !matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "[::1]"
    ) {
        return None;
    }
    let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some(Loopback {
        scheme,
        port,
        rest: tail.to_owned(),
        v6: host == "[::1]",
    })
}

/// Characters that draw as nothing or reorder text (bidi controls, zero
/// width, soft hyphen, BOM): a link built from them reads as another one.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{AD}' | '\u{61C}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
        | '\u{115F}' | '\u{1160}' | '\u{2800}' | '\u{3164}' | '\u{FFA0}')
}

/// Extensions the OS opener runs or installs instead of showing.
const PROGRAMS: [&str; 48] = [
    "exe",
    "com",
    "bat",
    "cmd",
    "lnk",
    "scr",
    "msi",
    "msp",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "cpl",
    "dll",
    "reg",
    "pif",
    "jar",
    "app",
    "command",
    "sh",
    "bash",
    "zsh",
    "pkg",
    "dmg",
    "appimage",
    "desktop",
    "url",
    "scf",
    "chm",
    "inf",
    "gadget",
    "msix",
    "appx",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "appref-ms",
    "application",
    "settingcontent-ms",
    "xll",
    "msc",
    "jnlp",
];

/// `file:` links open only on this computer and never a program. `rest` is
/// what follows `file://`: `[host]/path`.
fn check_file(rest: &str) -> Result<(), Deny> {
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return Err(Deny::File);
    }
    let path = path.split(['?', '#']).next().unwrap_or("");
    let path = percent_decode(path).ok_or(Deny::File)?;
    // Past an empty host, a leading `//` is a UNC share (`\\\\server\\share`).
    if path.starts_with("//") {
        return Err(Deny::File);
    }
    // A drive prefix (`/C:/`) is the one colon a path may hold; any other
    // is a Windows alternate data stream.
    let body = match path.as_bytes() {
        [b'/', d, b':', ..] if d.is_ascii_alphabetic() => &path[3..],
        _ => &path[..],
    };
    if path.contains('\\') || body.contains(':') || path.chars().any(char::is_control) {
        return Err(Deny::File);
    }
    let name = body.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    // Windows drops a trailing space or dot, so `a.exe ` is `a.exe`.
    if name.ends_with([' ', '.']) {
        return Err(Deny::File);
    }
    let ext = name
        .rsplit_once('.')
        .map_or("", |(_, e)| e)
        .to_ascii_lowercase();
    match PROGRAMS.contains(&ext.as_str()) {
        true => Err(Deny::File),
        false => Ok(()),
    }
}

/// `%XX` escapes decoded; `None` if one is malformed or the result is not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let hex = s.get(i + 1..i + 3)?;
        out.push(u8::from_str_radix(hex, 16).ok()?);
        i += 3;
    }
    String::from_utf8(out).ok()
}

/// A `mailto:` that asks the mail app to attach a file is refused.
fn check_mailto(rest: &str) -> Result<(), Deny> {
    let query = rest.split_once('?').map_or("", |(_, q)| q);
    let attaches = query.split('&').any(|kv| {
        let key = kv.split('=').next().unwrap_or("");
        percent_decode(key).is_some_and(|k| k.to_ascii_lowercase().starts_with("attach"))
    });
    match attaches {
        true => Err(Deny::Malformed),
        false => Ok(()),
    }
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
    let plain = host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let v6 = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .is_some_and(|h| {
            !h.is_empty()
                && h.bytes()
                    .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        });
    if host.is_empty()
        || !(plain || v6)
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

    /// The trusted origins, sorted (tests compare what was loaded).
    #[cfg(test)]
    pub(crate) fn origins(&self) -> impl Iterator<Item = &str> {
        self.trusted.iter().map(String::as_str)
    }
}

/// `s` if it is exactly a normalised http(s) origin (what [`parse`] makes of
/// a web URL), else `None`: how stored origins are checked on load.
pub(crate) fn valid_origin(s: &str) -> Option<&str> {
    parse(s).ok().filter(|l| l.origin == s).map(|_| s)
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
    fn hosts_are_plain_ascii_or_bracketed_ipv6() {
        for u in [
            "https://ex\u{430}mple.com/",
            "https://exa\"mple.com/",
            "https://a%2eb.com/",
            "https://a|b.com/",
            "ftp://good.com@evil.com/",
            "ftp://ev\u{456}l.com/",
            "https://[]/",
            "https://[g::1]/",
        ] {
            assert_eq!(parse(u), Err(Deny::Host), "{u}");
        }
        assert_eq!(link("https://[::1]:8443/x").origin, "https://[::1]:8443");
        assert_eq!(
            link("https://xn--80ak6aa92e.com/").origin,
            "https://xn--80ak6aa92e.com"
        );
        assert!(parse("ftp://files.example.com/pub").is_ok());
    }

    #[test]
    fn invisible_and_bidi_characters_are_malformed() {
        for c in [
            '\u{202E}', '\u{200B}', '\u{2066}', '\u{2069}', '\u{FEFF}', '\u{AD}',
        ] {
            let u = format!("https://example.com/a{c}b");
            assert_eq!(parse(&u), Err(Deny::Malformed), "{c:?}");
        }
        assert!(
            parse("https://example.com/caf\u{e9}").is_ok(),
            "ordinary non-ASCII paths stay"
        );
    }

    #[test]
    fn file_links_stay_on_this_computer_and_never_run_a_program() {
        for u in [
            "file://attacker/share/x.txt",
            "file://evil.com/a.txt",
            "file:///tmp/setup.exe",
            "file:///tmp/Setup.EXE",
            "file:///tmp/setup.exe.",
            "file:///tmp/setup%2Eexe",
            "file:///tmp/s%65tup.exe",
            "file:///Applications/Tool.app/",
            "file:///tmp/run.command",
            "file:///C:/a/b.lnk",
            "file:///C:/a/b.txt:evil.exe",
            "file:///tmp/a%00.txt",
            "file:///tmp/a%zz.txt",
            "file:///tmp/a\\b.txt",
            "file:///tmp/x.bat?x=1",
            "file:////attacker/share/x.txt",
            "file://localhost//attacker/share/",
            "file:///%2F%2Fattacker/share/x.txt",
            "file:///C:/x/setup.exe%20",
            "file:///C:/x/setup.exe%20.",
            "file:///C:/x/setup.exe%20%20",
            "file:///tmp/a.msix",
            "file:///tmp/a.ISO",
            "file:///tmp/a.jnlp",
        ] {
            assert_eq!(parse(u), Err(Deny::File), "{u}");
        }
        for u in [
            "file:///home/user/notes/todo.txt",
            "file://localhost/etc/hosts",
            "file:///C:/Users/me/doc.pdf",
            "file:///tmp/archive.tar.gz",
            "file:///tmp/a%20b.txt",
        ] {
            assert!(parse(u).is_ok(), "{u}");
        }
    }

    #[test]
    fn mailto_cannot_attach_a_file() {
        assert!(parse("mailto:a@b.c?subject=hi&body=x").is_ok());
        for u in [
            "mailto:a@b.c?attach=/etc/passwd",
            "mailto:a@b.c?x=1&Attachment=/x",
            "mailto:a@b.c?%61ttach=/x",
        ] {
            assert_eq!(parse(u), Err(Deny::Malformed), "{u}");
        }
    }

    #[test]
    fn only_normalised_web_origins_are_valid_origins() {
        assert_eq!(valid_origin("https://ok.com"), Some("https://ok.com"));
        assert_eq!(
            valid_origin("http://ok.com:8080"),
            Some("http://ok.com:8080")
        );
        for o in [
            "https://x.com/path",
            "javascript:1",
            "https://Up.com",
            "ftp://h",
            "",
            "file:///a",
        ] {
            assert_eq!(valid_origin(o), None, "{o}");
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
    fn loopback_links_name_their_port_and_rest() {
        let l = loopback("http://localhost:5173/app?x=1#top").unwrap();
        assert_eq!(
            (l.scheme, l.port, l.rest.as_str()),
            ("http", 5173, "/app?x=1#top")
        );
        assert_eq!(l.at(40000), "http://127.0.0.1:40000/app?x=1#top");
        assert_eq!(loopback("https://LOCALHOST").unwrap().port, 443);
        assert_eq!(loopback("http://127.0.0.1/").unwrap().port, 80);
        assert_eq!(loopback("http://[::1]:3000").unwrap().port, 3000);
        assert!(loopback("http://[::1]:3000").unwrap().v6);
        assert!(!loopback("http://localhost:3000").unwrap().v6);
    }

    #[test]
    fn other_hosts_and_odd_links_are_not_loopback() {
        for url in [
            "http://example.com:3000/",
            "http://0.0.0.0:3000/",
            "http://localhost.evil.com:3000/",
            "http://user@localhost:3000/",
            "http://localhost:0/",
            "http://localhost:99999/",
            "ftp://localhost:21/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://localhost:3000/a b",
        ] {
            assert_eq!(loopback(url), None, "{url}");
        }
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
