//! The operator's assertions file: the promises about billing and hosts
//! that no adapter can verify (#594, Nano 2026-11-03).
//!
//! Only the operator sets them, in a file outside any repository:
//!
//! ```json
//! {
//!   "no_overage": [
//!     {"provider": "claude", "account": "nano@example.com", "checked": "2026-11-03"}
//!   ],
//!   "host_cleared": [
//!     {"provider": "codex", "machine_id": "4c4c4544...", "checked": "2026-11-03"}
//!   ]
//! }
//! ```
//!
//! - `no_overage`: credits and extra usage are off on this provider's
//!   account, checked by the operator on `checked`. It lapses
//!   [`NO_OVERAGE_DAYS`] days later.
//! - `host_cleared`: everything this host lets the provider read is cleared
//!   for it, which in practice means a dedicated container or VM holding
//!   only the experiment repository and synthetic data. It holds only on the
//!   machine whose `/etc/machine-id` it names.
//!
//! Everything fails closed: no file, an unreadable or malformed file, a file
//! the runner could change, a lapsed or future date, or another machine
//! means the assertion is off. The ownership check protects the operator
//! only when the runner runs as a different user from the file's owner.

use std::collections::BTreeMap;
use std::path::Path;

use orch_core::AgentId;
use rusty_json::{Map, Value};
use rusty_time::{Date, DateTime, Time};

use super::registry::Assertions;

/// How long a no-overage check holds, in days.
pub const NO_OVERAGE_DAYS: i64 = 30;

/// What the operator asserted, per provider, as it holds today on this host.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    by_provider: BTreeMap<AgentId, Assertions>,
}

impl Grants {
    /// No assertions: every CLI provider stays `Unavailable`.
    pub fn none() -> Self {
        Self::default()
    }

    /// What holds for provider `id`; nothing when the file does not name it.
    pub fn for_provider(&self, id: AgentId) -> Assertions {
        self.by_provider.get(&id).copied().unwrap_or_default()
    }
}

/// Grants made directly, e.g. in tests; the runner reads them with [`load`].
impl FromIterator<(AgentId, Assertions)> for Grants {
    fn from_iter<I: IntoIterator<Item = (AgentId, Assertions)>>(iter: I) -> Self {
        Self {
            by_provider: iter.into_iter().collect(),
        }
    }
}

/// Decide which assertions in `text` hold on day `today` (days since the
/// Unix epoch) on the machine `machine_id`. Malformed entries are refused
/// with the field named; lapsed ones and other machines' are simply off.
pub fn grants(text: &str, today: i64, machine_id: Option<&str>) -> Result<Grants, String> {
    let v = Value::parse(text).map_err(|e| format!("assertions are not JSON: {e}"))?;
    let top = v.as_object().ok_or("assertions: object required")?;
    if let Some(k) = top
        .keys()
        .find(|k| !["no_overage", "host_cleared"].contains(&k.as_str()))
    {
        return Err(format!("assertions.{k}: not an assertion"));
    }
    let mut g = Grants::none();
    for (i, e) in entries(top, "no_overage")?.iter().enumerate() {
        let at = |m: String| format!("no_overage[{i}].{m}");
        let (id, checked) = entry(e, &["provider", "account", "checked"]).map_err(at)?;
        text_field(e, "account").map_err(at)?;
        if holds(checked, today, NO_OVERAGE_DAYS).map_err(at)? {
            g.by_provider.entry(id).or_default().no_overage = true;
        }
    }
    for (i, e) in entries(top, "host_cleared")?.iter().enumerate() {
        let at = |m: String| format!("host_cleared[{i}].{m}");
        let (id, checked) = entry(e, &["provider", "machine_id", "checked"]).map_err(at)?;
        let named = text_field(e, "machine_id").map_err(at)?;
        holds(checked, today, i64::MAX).map_err(at)?;
        if machine_id == Some(named) {
            g.by_provider.entry(id).or_default().host_cleared = true;
        }
    }
    Ok(g)
}

fn entries<'a>(top: &'a Map, k: &str) -> Result<Vec<&'a Map>, String> {
    let Some(v) = top.get(k) else {
        return Ok(Vec::new());
    };
    v.as_array()
        .ok_or_else(|| format!("{k}: array required"))?
        .iter()
        .enumerate()
        .map(|(i, e)| {
            e.as_object()
                .ok_or_else(|| format!("{k}[{i}]: object required"))
        })
        .collect()
}

/// The entry's provider and check day, after refusing unknown keys.
fn entry(e: &Map, keys: &[&str]) -> Result<(AgentId, i64), String> {
    if let Some(k) = e.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("{k}: not a field of this assertion"));
    }
    let id = AgentId::new(text_field(e, "provider")?).map_err(|e| format!("provider: {e}"))?;
    Ok((id, day(text_field(e, "checked")?)?))
}

fn text_field<'a>(e: &'a Map, k: &str) -> Result<&'a str, String> {
    e.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{k}: non-empty string required"))
}

/// `YYYY-MM-DD` as days since the Unix epoch.
fn day(s: &str) -> Result<i64, String> {
    let bad = || format!("checked: {s:?} is not a YYYY-MM-DD date");
    let mut parts = s.split('-');
    let (Some(y), Some(m), Some(d), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return Err(bad());
    }
    let (y, m, d) = (
        y.parse().map_err(|_| bad())?,
        m.parse().map_err(|_| bad())?,
        d.parse().map_err(|_| bad())?,
    );
    let date = Date::from_ymd(y, m, d).map_err(|_| bad())?;
    Ok(DateTime::new(date, Time::default(), 0)
        .timestamp()
        .div_euclid(86_400))
}

/// Does a check made on `checked` still hold on `today`? A date in the
/// future is refused: it can only be a mistake.
fn holds(checked: i64, today: i64, days: i64) -> Result<bool, String> {
    if checked > today {
        return Err("checked: a date in the future".to_owned());
    }
    Ok(today - checked < days)
}

/// Could the user `euid` change a file or directory owned by `owner` with
/// permission bits `mode`? The owner can always chmod it, root can write
/// anything, and group- or other-writable means someone else can, unless
/// it is a sticky directory, where only an entry's owner may replace it.
pub fn changeable(euid: u32, owner: u32, mode: u32, sticky_dir: bool) -> bool {
    euid == 0 || owner == euid || (mode & 0o022 != 0 && !sticky_dir)
}

/// Read the assertions at `path`, refusing a file the runner could change.
/// `Ok(Grants::none())` when there is no file.
#[cfg(unix)]
pub fn load(path: &Path) -> Result<Grants, String> {
    use std::os::unix::fs::MetadataExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    let full = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Grants::none()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let euid = euid()?;
    for p in full.ancestors() {
        let m = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let sticky = p != full && m.mode() & 0o1000 != 0;
        if changeable(euid, m.uid(), m.mode(), sticky) {
            return Err(format!(
                "{}: the runner's user could change it, so its assertions are ignored",
                p.display()
            ));
        }
    }
    let text = std::fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("clock: {e}"))?
        .as_secs();
    let today = i64::try_from(secs / 86_400).map_err(|_| "clock: out of range".to_owned())?;
    grants(&text, today, machine_id().as_deref()).map_err(|e| format!("{}: {e}", full.display()))
}

/// Without Unix ownership the file cannot be checked, so nothing holds.
#[cfg(not(unix))]
pub fn load(_path: &Path) -> Result<Grants, String> {
    Err("operator assertions need Unix file ownership; none hold".to_owned())
}

/// The runner's effective user, from `/proc/self/status` (Linux).
#[cfg(unix)]
fn euid() -> Result<u32, String> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|e| format!("cannot tell the runner's user: {e}"))?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| "cannot tell the runner's user".to_owned())
}

/// This host's machine id, when it has one.
#[cfg(unix)]
fn machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> AgentId {
        AgentId::new(s).expect("id")
    }

    /// 2026-11-03.
    const TODAY: i64 = 20_760;

    const FILE: &str = r#"{
      "no_overage": [
        {"provider": "claude", "account": "nano@example.com", "checked": "2026-10-05"},
        {"provider": "codex", "account": "nano@example.com", "checked": "2026-10-04"}
      ],
      "host_cleared": [
        {"provider": "codex", "machine_id": "box-1", "checked": "2026-11-01"}
      ]
    }"#;

    #[test]
    fn dates_count_days_from_the_epoch() {
        assert_eq!(day("1970-01-01"), Ok(0));
        assert_eq!(day("2026-11-03"), Ok(TODAY));
        for bad in [
            "2026-11-3",
            "2026-13-01",
            "2026-02-30",
            "26-11-03",
            "2026-11-03-1",
            "",
        ] {
            assert!(day(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn no_overage_lapses_after_thirty_days() {
        let g = grants(FILE, TODAY, None).expect("parses");
        // 29 days old: holds. 30 days old: lapsed.
        assert!(g.for_provider(id("claude")).no_overage);
        assert!(!g.for_provider(id("codex")).no_overage);
    }

    #[test]
    fn host_cleared_holds_only_on_the_named_machine() {
        let here = grants(FILE, TODAY, Some("box-1")).expect("parses");
        assert!(here.for_provider(id("codex")).host_cleared);
        assert!(!here.for_provider(id("claude")).host_cleared);
        for elsewhere in [Some("desktop"), None] {
            let g = grants(FILE, TODAY, elsewhere).expect("parses");
            assert!(!g.for_provider(id("codex")).host_cleared, "{elsewhere:?}");
        }
    }

    #[test]
    fn unnamed_providers_and_an_empty_file_assert_nothing() {
        let g = grants("{}", TODAY, Some("box-1")).expect("parses");
        assert_eq!(g, Grants::none());
        assert_eq!(g.for_provider(id("local")), Assertions::default());
    }

    #[test]
    fn malformed_entries_are_refused_with_the_field_named() {
        let refused = |text: &str, expected: &str| {
            let e = grants(text, TODAY, Some("box-1")).expect_err(expected);
            assert!(e.contains(expected), "{expected:?} not in {e:?}");
        };
        refused("[]", "assertions: object required");
        refused(r#"{"overage": []}"#, "assertions.overage: not an assertion");
        refused(r#"{"no_overage": {}}"#, "no_overage: array required");
        refused(
            r#"{"no_overage": [{"provider": "claude", "checked": "2026-11-01"}]}"#,
            "no_overage[0].account: non-empty string required",
        );
        refused(
            r#"{"no_overage": [{"provider": "claude", "account": "a", "checked": "2026-11-04"}]}"#,
            "no_overage[0].checked: a date in the future",
        );
        refused(
            r#"{"no_overage": [{"provider": "Claude!", "account": "a", "checked": "2026-11-01"}]}"#,
            "no_overage[0].provider",
        );
        refused(
            r#"{"host_cleared": [{"provider": "codex", "machine_id": "m", "checked": "2026-11-01", "by": "agent"}]}"#,
            "host_cleared[0].by: not a field",
        );
        refused(
            r#"{"host_cleared": [{"provider": "codex", "checked": "2026-11-01"}]}"#,
            "host_cleared[0].machine_id",
        );
    }

    #[test]
    fn a_file_the_runner_could_change_is_refused() {
        // Runner 1000, operator 1001.
        assert!(!changeable(1000, 1001, 0o644, false));
        assert!(!changeable(1000, 0, 0o755, false));
        assert!(changeable(1000, 1000, 0o444, false), "owner can chmod");
        assert!(changeable(0, 1001, 0o600, false), "root writes anything");
        assert!(changeable(1000, 1001, 0o664, false), "group-writable");
        assert!(changeable(1000, 1001, 0o646, false), "other-writable");
        assert!(
            !changeable(1000, 0, 0o1777, true),
            "sticky /tmp-like parent"
        );
    }

    #[test]
    fn a_missing_file_asserts_nothing() {
        let g = load(Path::new("/nonexistent/rusty_orch/assertions.json")).expect("no file");
        assert_eq!(g, Grants::none());
    }

    #[cfg(unix)]
    #[test]
    fn a_file_the_runner_owns_is_refused() {
        let dir = std::env::temp_dir().join(format!("orch-assert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("assertions.json");
        std::fs::write(&path, FILE).expect("write");
        let e = load(&path).expect_err("runner owns it");
        assert!(e.contains("the runner's user could change it"), "{e}");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
