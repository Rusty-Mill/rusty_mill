//! Confining a frozen pool to the hosting assignment's permission lease.
//!
//! The lease never widens a pool member's sandbox: each confined member keeps
//! only the parts of its declared paths that the lease also reaches, with
//! both sides resolved through symlinks first. BBP still adds the system
//! roots, the turn workspace and, with `checkout`, the repository's object
//! store; a checkout therefore needs the repository inside the lease.
//!
//! Lease paths must already be canonical: `orch-core` compares a child's lease
//! with its parent's by spelling, which is sound only when no component of a
//! leased path is a symlink. No kept path of any member may lie beneath
//! another kept write root, of the same member or another, since that
//! harness could replace it with a symlink before its rule is made; BBP
//! refuses the same, with its own roots, when the moderator starts.
use crate::config::resolved;
use crate::{Config, Sandbox};
use orch_core::factory::{contains, Lease};
use std::path::Path;

/// The pool of `config` with every assigned member's sandbox bounded by
/// `lease`. An assigned member without a sandbox cannot be bounded, so it is
/// refused rather than run as configured.
pub(crate) fn confine(config: &Config, lease: &Lease) -> Result<Config, String> {
    let read = exact(&lease.read)?;
    let write = exact(&lease.write)?;
    let reach: Vec<String> = read.iter().chain(&write).cloned().collect();
    let repo = canonical(&[path(&config.repo)?])?;
    let mut confined = config.clone();
    for assignment in config.assignments.values() {
        let name = &assignment.agent;
        let agent = confined
            .pool
            .get_mut(name)
            .ok_or("assignment names absent pool member")?;
        let Some(sandbox) = agent.sandbox.as_mut() else {
            return Err(format!(
                "pool member {name} is unconfined; a lease needs every assigned member sandboxed"
            ));
        };
        if sandbox.checkout && !repo.iter().all(|r| reaches(&reach, r)) {
            return Err(format!(
                "pool member {name} checks out the repository, which the lease does not reach"
            ));
        }
        *sandbox = bounded(sandbox, &read, &write)?;
    }
    unswappable(&confined)?;
    Ok(confined)
}

/// Each lease path exactly as the kernel resolves it: existing, with no
/// symlink anywhere along it.
fn exact(paths: &[String]) -> Result<Vec<String>, String> {
    for p in paths {
        let real = std::fs::canonicalize(p).map_err(|e| format!("lease path {p}: {e}"))?;
        if real != Path::new(p) {
            return Err(format!(
                "lease path {p} resolves to {}; leases name canonical paths",
                real.display()
            ));
        }
    }
    Ok(paths.to_vec())
}

/// Refuse a confined pool in which some member's path lies strictly beneath
/// a write root of any member.
fn unswappable(config: &Config) -> Result<(), String> {
    let sandboxes: Vec<&Sandbox> = config
        .assignments
        .values()
        .filter_map(|a| config.pool.get(&a.agent)?.sandbox.as_ref())
        .collect();
    for write in sandboxes.iter().flat_map(|s| &s.write) {
        let beneath = sandboxes
            .iter()
            .flat_map(|s| s.read.iter().chain(&s.write))
            .find(|p| *p != write && contains(write, p));
        if let Some(path) = beneath {
            return Err(format!(
                "{path} lies beneath write root {write}, which a harness could swap"
            ));
        }
    }
    Ok(())
}

/// `sandbox` with its paths met with the lease's. Nothing is kept beneath a
/// kept write root, so no harness can swap a path another rule names.
fn bounded(sandbox: &Sandbox, read: &[String], write: &[String]) -> Result<Sandbox, String> {
    let own_write = canonical(&sandbox.write)?;
    let own_reach: Vec<String> = canonical(&sandbox.read)?
        .into_iter()
        .chain(own_write.iter().cloned())
        .collect();
    let lease_reach: Vec<String> = read.iter().chain(write).cloned().collect();
    let kept_write = outermost(meet(&own_write, write));
    let kept_read = outermost(
        meet(&own_reach, &lease_reach)
            .into_iter()
            .filter(|p| !reaches(&kept_write, p))
            .collect(),
    );
    Ok(Sandbox {
        read: kept_read,
        write: kept_write,
        ..sandbox.clone()
    })
}

/// Every path both sets reach: the deeper of each related pair.
fn meet(a: &[String], b: &[String]) -> Vec<String> {
    a.iter()
        .flat_map(|x| {
            b.iter().filter_map(move |y| {
                if contains(y, x) {
                    Some(x.clone())
                } else if contains(x, y) {
                    Some(y.clone())
                } else {
                    None
                }
            })
        })
        .collect()
}

/// `paths` without duplicates or any path beneath another of them.
fn outermost(mut paths: Vec<String>) -> Vec<String> {
    paths.sort();
    paths.dedup();
    let all = paths.clone();
    paths.retain(|p| !all.iter().any(|root| root != p && contains(root, p)));
    paths
}

fn reaches(roots: &[String], path: &str) -> bool {
    roots.iter().any(|root| contains(root, path))
}

/// Resolve each path through symlinks, so containment is about what the
/// kernel will open, not about spelling.
fn canonical(paths: &[String]) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|p| {
            if !Path::new(p).is_absolute() {
                return Err(format!("sandbox path {p} is not absolute"));
            }
            path(&resolved(Path::new(p))?)
        })
        .collect()
}

fn path(p: &Path) -> Result<String, String> {
    p.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "non-UTF8 host path".to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::tests::{config, temp};

    fn strings(paths: &[&std::path::Path]) -> Vec<String> {
        paths.iter().map(|p| path(p).unwrap()).collect()
    }
    fn lease(read: &[&std::path::Path], write: &[&std::path::Path]) -> Lease {
        Lease {
            read: strings(read),
            write: strings(write),
            expires: None,
        }
    }
    fn made(paths: &[&std::path::Path]) {
        for p in paths {
            std::fs::create_dir_all(p).unwrap();
        }
    }
    fn sandboxed(config: &mut Config, sandbox: Sandbox) {
        for agent in config.pool.values_mut() {
            agent.sandbox = Some(sandbox.clone());
        }
    }

    #[test]
    fn a_lease_keeps_only_the_paths_both_reach() {
        let dir = temp().canonicalize().unwrap();
        let (src, a, tool, other) = (
            dir.join("src"),
            dir.join("src/a"),
            dir.join("tool"),
            dir.join("other"),
        );
        made(&[&a, &tool]);
        let mut config = config(&dir);
        sandboxed(
            &mut config,
            Sandbox {
                read: strings(&[&dir.join("src/docs"), &tool, &other]),
                write: strings(&[&src]),
                env: vec!["PATH".into()],
                checkout: false,
            },
        );
        let confined = confine(&config, &lease(&[&dir.join("src"), &tool], &[&a])).unwrap();
        for agent in confined.pool.values() {
            let sandbox = agent.sandbox.as_ref().unwrap();
            // The member's write root narrows to the lease's; outside reads go.
            assert_eq!(sandbox.write, strings(&[&a]));
            assert_eq!(sandbox.read, strings(&[&src, &tool]));
            assert_eq!(sandbox.env, vec!["PATH".to_owned()]);
        }
    }

    #[test]
    fn nothing_is_kept_beneath_a_write_root_or_widened() {
        let dir = temp().canonicalize().unwrap();
        let src = dir.join("src");
        let sandbox = Sandbox {
            read: strings(&[&src.join("x")]),
            write: strings(&[&src.join("y")]),
            ..Sandbox::default()
        };
        // A wide lease keeps the member's own, narrower paths.
        let wide = bounded(&sandbox, &[], &strings(&[&dir])).unwrap();
        assert_eq!(
            (wide.read.clone(), wide.write.clone()),
            (sandbox.read, sandbox.write)
        );
        let writes = Sandbox {
            read: strings(&[&src.join("a/b")]),
            write: strings(&[&src, &src.join("a")]),
            ..Sandbox::default()
        };
        let kept = bounded(&writes, &[], &strings(&[&dir])).unwrap();
        assert_eq!((kept.read, kept.write), (vec![], strings(&[&src])));
    }

    #[test]
    fn symlinks_are_resolved_before_containment() {
        let dir = temp().canonicalize().unwrap();
        let (src, outside) = (dir.join("src"), dir.join("outside"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, src.join("escape")).unwrap();
        let sandbox = Sandbox {
            read: strings(&[&src.join("escape")]),
            ..Sandbox::default()
        };
        let kept = bounded(&sandbox, &strings(&[&src]), &[]).unwrap();
        assert!(kept.read.is_empty(), "{:?}", kept.read);
        let relative = Sandbox {
            read: vec!["relative".into()],
            ..Sandbox::default()
        };
        assert!(bounded(&relative, &[], &[]).is_err());
    }

    #[test]
    fn unconfined_members_and_unreached_checkouts_are_refused() {
        let dir = temp().canonicalize().unwrap();
        let config = config(&dir);
        let refused = confine(&config, &lease(&[&dir], &[])).unwrap_err();
        assert!(refused.contains("unconfined"), "{refused}");
        let mut checkout = config.clone();
        sandboxed(
            &mut checkout,
            Sandbox {
                checkout: true,
                ..Sandbox::default()
            },
        );
        made(&[&dir.join("elsewhere"), &dir.join("repo")]);
        let refused = confine(&checkout, &lease(&[&dir.join("elsewhere")], &[])).unwrap_err();
        assert!(refused.contains("repository"), "{refused}");
        assert!(confine(&checkout, &lease(&[&dir.join("repo")], &[])).is_ok());
    }

    #[test]
    fn a_lease_path_through_a_symlink_is_refused() {
        // A child leased `/src/escape` is lexically inside a parent's `/src`;
        // were `escape` a link to `/secret`, it would reach outside it.
        let dir = temp().canonicalize().unwrap();
        let (src, secret) = (dir.join("src"), dir.join("secret"));
        made(&[&src, &secret]);
        std::os::unix::fs::symlink(&secret, src.join("escape")).unwrap();
        let mut config = config(&dir);
        sandboxed(
            &mut config,
            Sandbox {
                read: strings(&[&secret, &src]),
                ..Sandbox::default()
            },
        );
        let refused = confine(&config, &lease(&[&src.join("escape")], &[])).unwrap_err();
        assert!(refused.contains("canonical"), "{refused}");
        let missing = confine(&config, &lease(&[&src.join("absent")], &[])).unwrap_err();
        assert!(missing.contains("absent"), "{missing}");
        assert!(confine(&config, &lease(&[&src], &[])).is_ok());
    }

    #[test]
    fn one_members_write_root_may_not_hold_anothers_path() {
        let dir = temp().canonicalize().unwrap();
        let (src, sub) = (dir.join("src"), dir.join("src/sub"));
        made(&[&sub]);
        let mut config = config(&dir);
        config.pool.get_mut("writer").unwrap().sandbox = Some(Sandbox {
            write: strings(&[&src]),
            ..Sandbox::default()
        });
        config.pool.get_mut("judge").unwrap().sandbox = Some(Sandbox {
            read: strings(&[&sub]),
            ..Sandbox::default()
        });
        let refused = confine(&config, &lease(&[], &[&src])).unwrap_err();
        assert!(refused.contains("swap"), "{refused}");
        // Narrowing the writer below the reader's path removes the hazard.
        made(&[&src.join("out")]);
        assert!(confine(&config, &lease(&[&sub], &[&src.join("out")])).is_ok());
    }
}
