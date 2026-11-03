//! The dedicated container a host-cleared provider runs in (#594, Nano
//! 2026-11-03): Codex runs only inside it, through `podman exec`, never on
//! the runner's host.
//!
//! The operator names the container and its exact mounts in the
//! assertions file ([`super::assertions`]). Before every call, the runner
//! must find that container through its own rootless podman, with
//! exactly those mounts, and confirm that its `/etc/machine-id` is mounted
//! read-only from a file the runner cannot change and holds the named id.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orch_cli::{CommandRunner, ExecError, Exit};
use orch_dispatch::{AgentError, ClassifiedError};
use orch_lens::{Answer, Model, Question};
use rusty_json::Value;

use super::assertions::{guarded_read, Unread};

/// Where a container's machine id is mounted.
pub const MACHINE_ID_PATH: &str = "/etc/machine-id";

/// The podman binary, by absolute path so `PATH` cannot substitute one.
/// Run as the runner's own user, it sees only that user's rootless
/// containers: finding the container at all shows the runner owns it.
pub const PODMAN: &str = "/usr/bin/podman";

/// `podman container inspect` reads local state; a minute is generous.
pub const INSPECT_TIMEOUT: Duration = Duration::from_secs(60);

/// Environment variables the container must not carry: each would route
/// Codex off its subscription login (`orch_codex::SCRUBBED_ENV`).
const FORBIDDEN_ENV: &[&str] = orch_codex::SCRUBBED_ENV;

/// A container the operator cleared for one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSpec {
    /// The container's name in the runner's podman.
    pub name: String,
    /// What the container's `/etc/machine-id` must hold.
    pub machine_id: String,
    /// Every mount the container may have, and no others. Always includes
    /// the read-only bind of [`MACHINE_ID_PATH`].
    pub mounts: Vec<Mount>,
}

/// One mount the operator allows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mount {
    /// A host path bound into the container.
    Bind {
        source: PathBuf,
        destination: String,
        rw: bool,
    },
    /// A named podman volume.
    Volume { name: String, destination: String },
}

impl Mount {
    /// Where the mount appears inside the container.
    pub fn destination(&self) -> &str {
        match self {
            Self::Bind { destination, .. } | Self::Volume { destination, .. } => destination,
        }
    }
}

impl ContainerSpec {
    /// The host file holding the container's machine id: the source of
    /// the read-only bind at [`MACHINE_ID_PATH`].
    pub fn machine_id_source(&self) -> Option<&PathBuf> {
        self.mounts.iter().find_map(|m| match m {
            Mount::Bind {
                source,
                destination,
                rw: false,
            } if destination == MACHINE_ID_PATH => Some(source),
            _ => None,
        })
    }
}

/// Check `inspect` (the output of `podman container inspect <name>`)
/// against `spec`. On success, the container's id, which every exec then
/// names, so a container recreated after the check is not used.
///
/// Refused unless the container is running, unprivileged, has no added
/// devices or capabilities, carries no [`FORBIDDEN_ENV`] variable, and has
/// exactly the mounts in `spec`, and unless `machine_id_at` reads
/// `spec.machine_id` from the source of its read-only machine-id bind.
pub fn check(
    spec: &ContainerSpec,
    inspect: &str,
    machine_id_at: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<String, String> {
    let v = Value::parse(inspect).map_err(|e| format!("inspect is not JSON: {e}"))?;
    let c = match v.as_array().map(Vec::as_slice) {
        Some([c]) => c,
        _ => return Err("inspect: exactly one container expected".to_owned()),
    };
    let id = c
        .get("Id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.chars().all(|ch| ch.is_ascii_hexdigit()))
        .ok_or("inspect: no container id")?;
    if c.pointer("/State/Running").and_then(Value::as_bool) != Some(true) {
        return Err("not running".to_owned());
    }
    if c.pointer("/HostConfig/Privileged").and_then(Value::as_bool) != Some(false) {
        return Err("privileged, or privilege not reported".to_owned());
    }
    for k in ["Devices", "CapAdd"] {
        if !empty_or_null(c.pointer(&format!("/HostConfig/{k}"))) {
            return Err(format!("HostConfig.{k} is not empty"));
        }
    }
    check_env(c.pointer("/Config/Env"))?;
    let seen = mounts(c.get("Mounts"))?;
    let wanted: BTreeSet<&Mount> = spec.mounts.iter().collect();
    if seen.iter().collect::<BTreeSet<_>>() != wanted {
        return Err("its mounts differ from the asserted list".to_owned());
    }
    let source = spec
        .machine_id_source()
        .ok_or("no read-only machine-id bind asserted")?;
    let found = machine_id_at(source)?;
    if found.trim() != spec.machine_id {
        return Err(format!(
            "{} does not hold the asserted machine id",
            source.display()
        ));
    }
    Ok(id.to_owned())
}

fn empty_or_null(v: Option<&Value>) -> bool {
    match v {
        None => true,
        Some(v) => v.is_null() || v.as_array().is_some_and(Vec::is_empty),
    }
}

fn check_env(env: Option<&Value>) -> Result<(), String> {
    let Some(env) = env else { return Ok(()) };
    let list = env.as_array().ok_or("inspect: Config.Env is not a list")?;
    for e in list {
        let e = e.as_str().ok_or("inspect: Config.Env holds a non-string")?;
        let key = e.split('=').next().unwrap_or_default();
        if FORBIDDEN_ENV.contains(&key) {
            return Err(format!("its environment sets {key}"));
        }
    }
    Ok(())
}

/// The inspected mounts as [`Mount`]s. Any type other than a bind or a
/// named volume is refused.
fn mounts(v: Option<&Value>) -> Result<Vec<Mount>, String> {
    let list = v
        .and_then(Value::as_array)
        .ok_or("inspect: Mounts is not a list")?;
    list.iter()
        .map(|m| {
            let s = |k: &str| {
                m.get(k)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("inspect: a mount without {k}"))
            };
            match m.get("Type").and_then(Value::as_str) {
                Some("bind") => Ok(Mount::Bind {
                    source: PathBuf::from(s("Source")?),
                    destination: s("Destination")?,
                    rw: m
                        .get("RW")
                        .and_then(Value::as_bool)
                        .ok_or("inspect: a bind mount without RW")?,
                }),
                Some("volume") => Ok(Mount::Volume {
                    name: s("Name")?,
                    destination: s("Destination")?,
                }),
                other => Err(format!("a mount of type {other:?}")),
            }
        })
        .collect()
}

/// The machine id held in `path`, an operator's file the runner cannot
/// change ([`guarded_read`]).
pub fn read_machine_id(path: &Path) -> Result<String, String> {
    guarded_read(path).map_err(|e| match e {
        Unread::Missing => format!("{}: missing", path.display()),
        Unread::Refused(e) => e,
    })
}

/// The id of the container checked for the call in progress, shared by
/// [`Contained`] (which sets it) and [`InContainer`] (which execs in it).
#[derive(Debug, Clone, Default)]
pub struct Verified(Arc<Mutex<Option<String>>>);

impl Verified {
    fn set(&self, id: Option<String>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = id;
        }
    }

    fn get(&self) -> Option<String> {
        self.0.lock().ok().and_then(|slot| slot.clone())
    }
}

/// A [`Model`] whose processes run only inside the operator's container:
/// before every question the container is checked ([`check`]); a failed
/// check is `Unavailable` and runs nothing in it.
pub struct Contained<C> {
    model: Box<dyn Model>,
    runner: C,
    spec: ContainerSpec,
    verified: Verified,
    machine_id_at: fn(&Path) -> Result<String, String>,
}

impl<C: CommandRunner> Contained<C> {
    /// `model` must run its processes through an [`InContainer`] sharing
    /// `verified`; `runner` runs the inspect on the host.
    pub fn new(
        model: Box<dyn Model>,
        runner: C,
        spec: ContainerSpec,
        verified: Verified,
        machine_id_at: fn(&Path) -> Result<String, String>,
    ) -> Self {
        Self {
            model,
            runner,
            spec,
            verified,
            machine_id_at,
        }
    }

    fn verify(&self) -> Result<String, String> {
        let argv = [PODMAN, "container", "inspect", "--", &self.spec.name].map(str::to_owned);
        let exit = self
            .runner
            .run_scrubbed(&argv, b"", INSPECT_TIMEOUT, &[])
            .map_err(|e| format!("podman: {e}"))?;
        if exit.status != 0 {
            return Err("not found in the runner's podman".to_owned());
        }
        let text = std::str::from_utf8(&exit.stdout).map_err(|e| format!("inspect: {e}"))?;
        check(&self.spec, text, &self.machine_id_at)
    }
}

impl<C: CommandRunner> Model for Contained<C> {
    fn ask(&mut self, q: &Question) -> Result<Answer, ClassifiedError> {
        let id = self.verify().map_err(|e| {
            ClassifiedError::Unavailable(AgentError(format!("container {}: {e}", self.spec.name)))
        })?;
        self.verified.set(Some(id));
        let answer = self.model.ask(q);
        self.verified.set(None);
        answer
    }
}

/// A [`CommandRunner`] that runs every command inside the container
/// [`Contained`] verified for the call in progress, as `podman exec -i
/// --workdir <workdir> <id> <argv...>`. With no verified container it
/// starts nothing.
pub struct InContainer<C> {
    inner: C,
    workdir: String,
    verified: Verified,
}

impl<C> InContainer<C> {
    /// Commands run in `workdir`, a path inside the container.
    pub fn new(inner: C, workdir: impl Into<String>, verified: Verified) -> Self {
        Self {
            inner,
            workdir: workdir.into(),
            verified,
        }
    }
}

impl<C: CommandRunner> CommandRunner for InContainer<C> {
    fn run_in(
        &self,
        _cwd: Option<&Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        remove_env: &[&str],
    ) -> Result<Exit, ExecError> {
        let id = self
            .verified
            .get()
            .ok_or_else(|| ExecError::Spawn("no verified container".to_owned()))?;
        let mut full: Vec<String> = [PODMAN, "exec", "-i", "--workdir", &self.workdir, &id]
            .map(str::to_owned)
            .to_vec();
        full.extend_from_slice(argv);
        self.inner.run_in(None, &full, stdin, timeout, remove_env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_cli::fake::exit;
    use orch_codex::CodexAgent;
    use orch_lens::Allowed;

    const ID: &str = "4f1c0a9e";

    fn spec() -> ContainerSpec {
        ContainerSpec {
            name: "lel-codex".to_owned(),
            machine_id: "lel-1".to_owned(),
            mounts: vec![
                Mount::Bind {
                    source: PathBuf::from("/srv/lel/machine-id"),
                    destination: MACHINE_ID_PATH.to_owned(),
                    rw: false,
                },
                Mount::Bind {
                    source: PathBuf::from("/srv/lel/experiment"),
                    destination: "/work".to_owned(),
                    rw: true,
                },
                Mount::Volume {
                    name: "lel-codex-home".to_owned(),
                    destination: "/root".to_owned(),
                },
            ],
        }
    }

    const MOUNTS: &str = r#"[
      {"Type": "volume", "Name": "lel-codex-home", "Source": "/var/lib/x/_data",
       "Destination": "/root", "RW": true},
      {"Type": "bind", "Source": "/srv/lel/experiment", "Destination": "/work", "RW": true},
      {"Type": "bind", "Source": "/srv/lel/machine-id", "Destination": "/etc/machine-id", "RW": false}
    ]"#;

    /// `podman container inspect` output for a container with `mounts`,
    /// `host` as its HostConfig and `env` as its environment.
    fn inspect(mounts: &str, host: &str, env: &str) -> String {
        format!(
            r#"[{{"Id": "{ID}", "State": {{"Running": true}}, "Mounts": {mounts},
                 "HostConfig": {host}, "Config": {{"Env": {env}}}}}]"#
        )
    }

    const HOST: &str = r#"{"Privileged": false, "Devices": [], "CapAdd": null}"#;
    const ENV: &str = r#"["PATH=/usr/bin", "HOME=/root"]"#;

    fn holds(id: &'static str) -> impl Fn(&Path) -> Result<String, String> {
        move |p: &Path| {
            assert_eq!(p, Path::new("/srv/lel/machine-id"));
            Ok(format!("{id}\n"))
        }
    }

    #[test]
    fn the_asserted_container_passes_with_its_id() {
        let ok = check(&spec(), &inspect(MOUNTS, HOST, ENV), &holds("lel-1"));
        assert_eq!(ok, Ok(ID.to_owned()));
    }

    #[test]
    fn anything_else_is_refused() {
        let refused = |text: String, expected: &str| {
            let e = check(&spec(), &text, &holds("lel-1")).expect_err(expected);
            assert!(e.contains(expected), "{expected:?} not in {e:?}");
        };
        let extra = MOUNTS.replacen(
            "[",
            r#"[{"Type": "bind", "Source": "/run/user/1000/podman/podman.sock",
                 "Destination": "/run/podman.sock", "RW": true},"#,
            1,
        );
        refused(inspect(&extra, HOST, ENV), "mounts differ");
        let missing = r#"[{"Type": "bind", "Source": "/srv/lel/machine-id",
            "Destination": "/etc/machine-id", "RW": false}]"#;
        refused(inspect(missing, HOST, ENV), "mounts differ");
        let writable = MOUNTS.replace(
            r#""Destination": "/etc/machine-id", "RW": false"#,
            r#""Destination": "/etc/machine-id", "RW": true"#,
        );
        refused(inspect(&writable, HOST, ENV), "mounts differ");
        let moved = MOUNTS.replace("/srv/lel/experiment", "/home/nano");
        refused(inspect(&moved, HOST, ENV), "mounts differ");
        let tmpfs = MOUNTS.replacen(
            "[",
            r#"[{"Type": "tmpfs", "Source": "tmpfs", "Destination": "/tmp", "RW": true},"#,
            1,
        );
        refused(inspect(&tmpfs, HOST, ENV), "a mount of type");
        refused(
            inspect(MOUNTS, r#"{"Privileged": true}"#, ENV),
            "privileged",
        );
        refused(inspect(MOUNTS, "{}", ENV), "privilege not reported");
        refused(
            inspect(
                MOUNTS,
                r#"{"Privileged": false, "Devices": [{"PathOnHost": "/dev/sda"}]}"#,
                ENV,
            ),
            "HostConfig.Devices",
        );
        refused(
            inspect(
                MOUNTS,
                r#"{"Privileged": false, "CapAdd": ["CAP_SYS_ADMIN"]}"#,
                ENV,
            ),
            "HostConfig.CapAdd",
        );
        refused(
            inspect(MOUNTS, HOST, r#"["OPENAI_API_KEY=sk-x"]"#),
            "sets OPENAI_API_KEY",
        );
        refused(
            inspect(MOUNTS, HOST, ENV).replace(r#""Running": true"#, r#""Running": false"#),
            "not running",
        );
        refused(
            inspect(MOUNTS, HOST, ENV).replace(ID, "../x"),
            "no container id",
        );
        refused("[]".to_owned(), "exactly one container");
        let e = check(&spec(), &inspect(MOUNTS, HOST, ENV), &holds("other"))
            .expect_err("another machine id");
        assert!(e.contains("does not hold the asserted machine id"), "{e}");
        let e = check(&spec(), &inspect(MOUNTS, HOST, ENV), &|_: &Path| {
            Err("the runner's user could change it".to_owned())
        })
        .expect_err("changeable source");
        assert!(e.contains("could change it"), "{e}");
    }

    /// Inspect answers `inspect_out`; inside the container, the login
    /// probe answers a ChatGPT login and `codex exec` a valid reply.
    struct Podman {
        inspect_out: Exit,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl CommandRunner for Podman {
        fn run_in(
            &self,
            _cwd: Option<&Path>,
            argv: &[String],
            _stdin: &[u8],
            _timeout: Duration,
            _remove_env: &[&str],
        ) -> Result<Exit, ExecError> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(argv.to_vec());
            }
            if argv.get(1).map(String::as_str) == Some("container") {
                return Ok(self.inspect_out.clone());
            }
            if argv.iter().any(|a| a == "login") {
                return Ok(exit(0, "", "Logged in using ChatGPT\n"));
            }
            Ok(exit(
                0,
                concat!(
                    "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{\\\"posts\\\":[]}\"}}\n",
                    "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}\n",
                ),
                "",
            ))
        }
    }

    #[derive(Clone)]
    struct Shared(Arc<Podman>);

    impl CommandRunner for Shared {
        fn run_in(
            &self,
            cwd: Option<&Path>,
            argv: &[String],
            stdin: &[u8],
            timeout: Duration,
            remove_env: &[&str],
        ) -> Result<Exit, ExecError> {
            self.0.run_in(cwd, argv, stdin, timeout, remove_env)
        }
    }

    fn lel_1(_: &Path) -> Result<String, String> {
        Ok("lel-1\n".to_owned())
    }

    fn contained(inspect_out: Exit) -> (Contained<Shared>, Arc<Podman>) {
        let podman = Arc::new(Podman {
            inspect_out,
            calls: Mutex::new(Vec::new()),
        });
        let verified = Verified::default();
        let exec = InContainer::new(Shared(Arc::clone(&podman)), "/work", verified.clone());
        let agent = CodexAgent::with_runner("/work", exec)
            .lens_no_overage()
            .lens_host_cleared();
        let model = Contained::new(
            Box::new(agent),
            Shared(Arc::clone(&podman)),
            spec(),
            verified,
            lel_1,
        );
        (model, podman)
    }

    fn question() -> Question {
        Question {
            prefix: "p".to_owned(),
            body: "b".to_owned(),
            allowed: Allowed {
                evidence: None,
                claims: None,
            },
            max_output_tokens: 64,
        }
    }

    #[test]
    fn a_checked_container_runs_every_command_inside_it_by_id() {
        let (mut m, podman) = contained(exit(0, &inspect(MOUNTS, HOST, ENV), ""));
        let answer = m.ask(&question()).expect("answers");
        assert!(answer.posts.is_empty());
        let calls = podman.calls.lock().expect("lock").clone();
        assert_eq!(calls.len(), 3, "{calls:?}");
        assert_eq!(
            calls[0],
            ["/usr/bin/podman", "container", "inspect", "--", "lel-codex"]
        );
        for c in &calls[1..] {
            assert_eq!(
                c[..6],
                ["/usr/bin/podman", "exec", "-i", "--workdir", "/work", ID]
            );
            assert_eq!(c[6], "codex");
        }
        let exec = &calls[2];
        let at = exec.iter().position(|a| a == "-C").expect("-C");
        assert_eq!(exec[at + 1], "/work");
        // The id is cleared after the call: nothing can exec in it unchecked.
        assert_eq!(m.verified.get(), None);
    }

    #[test]
    fn a_failed_check_is_unavailable_and_nothing_runs_inside() {
        for out in [
            exit(125, "", "no such container"),
            exit(0, &inspect("[]", HOST, ENV), ""),
        ] {
            let (mut m, podman) = contained(out);
            let e = m.ask(&question()).expect_err("refused");
            assert!(matches!(e, ClassifiedError::Unavailable(_)), "{e:?}");
            assert_eq!(podman.calls.lock().expect("lock").len(), 1);
        }
    }

    #[test]
    fn without_a_checked_container_nothing_is_started() {
        let exec = InContainer::new(
            orch_cli::fake::FakeCommand::ok(""),
            "/work",
            Verified::default(),
        );
        let e = exec
            .run_scrubbed(&["codex".to_owned()], b"", Duration::from_secs(1), &[])
            .expect_err("no id");
        assert!(matches!(e, ExecError::Spawn(_)), "{e:?}");
        assert!(exec.inner.calls.lock().expect("lock").is_empty());
    }
}
