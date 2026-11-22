//! `probe`: which engine, and what it proves on this host. The checks are the phase 2 spike's: rootless podman
//! (never root), the caller's `NoNewPrivs` (rootless podman's setuid newuidmap fails under it), the engine's
//! seccomp filter, its version, and the cgroup controllers resource limits need.
//!
//! [`assess`] is pure, over what [`facts`] read from the engine and /proc, so its rules are unit-tested.

use std::process::Command;

use crate::render::Engine;

/// Guarantees this slice proves wherever its engine checks pass (bwrap's names, where they mean the same).
pub const GUARANTEES: &[&str] = &[
    "closure-store",
    "own-netns",
    "own-pidns",
    "no-tiocsti",
    "local-listen",
    "in-place",
    "root-files",
];

/// Oldest versions accepted: podman 4.4 (`--passwd`, `--rootfs DIR:O` with keep-id), docker 20.10 (cgroup v2).
const PODMAN_MIN: (u32, u32) = (4, 4);
const DOCKER_MIN: (u32, u32) = (20, 10);

#[derive(Debug)]
pub struct Facts {
    /// The engine's version, or why it could not be read.
    pub version: Result<String, String>,
    /// podman: `.Host.Security.Rootless`.
    pub rootless: bool,
    /// The engine's seccomp filter is on (podman `.Host.Security.SECCOMPEnabled`; docker `seccomp` in
    /// `.SecurityOptions`).
    pub seccomp: bool,
    /// podman: `.Host.CgroupControllers`; docker: `memory` and `pids` when `.MemoryLimit` and `.PidsLimit`.
    pub controllers: Vec<String>,
    /// The caller's `NoNewPrivs` (from /proc/self/status).
    pub no_new_privs: bool,
}

fn version_of(s: &str) -> Option<(u32, u32)> {
    let mut it = s.trim().trim_start_matches('v').split(['.', '-', '+']);
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// The guarantees, or why the engine can't run pot's runs here.
pub fn assess(engine: Engine, f: &Facts) -> Result<Vec<&'static str>, String> {
    let name = engine.name();
    let v = f.version.as_ref().map_err(|e| format!("{name}: {e}"))?;
    let min = if engine == Engine::Podman {
        PODMAN_MIN
    } else {
        DOCKER_MIN
    };
    match version_of(v) {
        Some(have) if have >= min => {}
        Some(_) => return Err(format!("{name} {v} is older than {}.{}", min.0, min.1)),
        None => return Err(format!("{name}: can't read its version from {v:?}")),
    }
    if engine == Engine::Podman {
        if !f.rootless {
            return Err("podman runs as root here: pot runs containers with rootless podman only".into());
        }
        if f.no_new_privs {
            return Err(
                "this process has NoNewPrivileges set, so rootless podman's newuidmap can't map its user \
                 namespace: run pot from a service without NoNewPrivileges (systemd DynamicUser sets it)"
                    .into(),
            );
        }
    }
    if !f.seccomp {
        return Err(format!("{name} has no seccomp filter here"));
    }
    let mut g = GUARANTEES.to_vec();
    if ["memory", "pids"]
        .iter()
        .all(|c| f.controllers.iter().any(|x| x == c))
    {
        g.push("resource-limits");
    }
    Ok(g)
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("can't run {program}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{program} {} failed: {}", args.join(" "), err.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn no_new_privs() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("NoNewPrivs:").map(|v| v.trim() != "0"))
        })
        .unwrap_or(false)
}

/// What the engine says of itself, and the caller's `NoNewPrivs`.
pub fn facts(engine: Engine, program: &str) -> Facts {
    let mut f = Facts {
        version: Err("not read".into()),
        rootless: false,
        seccomp: false,
        controllers: vec![],
        no_new_privs: no_new_privs(),
    };
    match engine {
        Engine::Podman => {
            f.version = run(program, &["version", "--format", "{{.Client.Version}}"]);
            let info = run(
                program,
                &[
                    "info",
                    "--format",
                    "{{.Host.Security.Rootless}} {{.Host.Security.SECCOMPEnabled}} {{range .Host.CgroupControllers}}{{.}},{{end}}",
                ],
            );
            match info {
                Ok(s) => {
                    let mut w = s.split_whitespace();
                    f.rootless = w.next() == Some("true");
                    f.seccomp = w.next() == Some("true");
                    f.controllers = w
                        .next()
                        .unwrap_or("")
                        .split(',')
                        .filter(|c| !c.is_empty())
                        .map(String::from)
                        .collect();
                }
                Err(e) => f.version = Err(e),
            }
        }
        Engine::Docker => {
            f.version = run(program, &["version", "--format", "{{.Server.Version}}"]);
            match run(
                program,
                &[
                    "info",
                    "--format",
                    "{{.MemoryLimit}} {{.PidsLimit}} {{json .SecurityOptions}}",
                ],
            ) {
                Ok(s) => {
                    let mut w = s.splitn(3, ' ');
                    if w.next() == Some("true") {
                        f.controllers.push("memory".into());
                    }
                    if w.next() == Some("true") {
                        f.controllers.push("pids".into());
                    }
                    f.seccomp = w.next().is_some_and(|o| o.contains("name=seccomp"));
                }
                Err(e) => f.version = Err(e),
            }
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_podman() -> Facts {
        Facts {
            version: Ok("5.4.1".into()),
            rootless: true,
            seccomp: true,
            controllers: vec!["cpu".into(), "io".into(), "memory".into(), "pids".into()],
            no_new_privs: false,
        }
    }

    #[test]
    fn rootless_podman_proves_the_slice_and_limits_with_controllers() {
        let g = assess(Engine::Podman, &ok_podman()).unwrap();
        assert_eq!(g.last(), Some(&"resource-limits"));
        assert!(g.contains(&"closure-store") && g.contains(&"own-netns"));
        let f = Facts {
            controllers: vec!["cpu".into()],
            ..ok_podman()
        };
        assert!(!assess(Engine::Podman, &f).unwrap().contains(&"resource-limits"));
    }

    #[test]
    fn podman_refuses_root_no_new_privs_old_versions_and_no_seccomp() {
        let e = assess(
            Engine::Podman,
            &Facts {
                rootless: false,
                ..ok_podman()
            },
        )
        .unwrap_err();
        assert!(e.contains("rootless podman only"), "{e}");
        let e = assess(
            Engine::Podman,
            &Facts {
                no_new_privs: true,
                ..ok_podman()
            },
        )
        .unwrap_err();
        assert!(e.contains("NoNewPrivileges"), "{e}");
        let e = assess(
            Engine::Podman,
            &Facts {
                version: Ok("4.3.1".into()),
                ..ok_podman()
            },
        )
        .unwrap_err();
        assert_eq!(e, "podman 4.3.1 is older than 4.4");
        let e = assess(
            Engine::Podman,
            &Facts {
                seccomp: false,
                ..ok_podman()
            },
        )
        .unwrap_err();
        assert_eq!(e, "podman has no seccomp filter here");
        let e = assess(
            Engine::Podman,
            &Facts {
                version: Err("can't run podman: not found".into()),
                ..ok_podman()
            },
        );
        assert_eq!(e.unwrap_err(), "podman: can't run podman: not found");
    }

    #[test]
    fn docker_ignores_no_new_privs_and_rootless() {
        let f = Facts {
            version: Ok("27.5.1".into()),
            rootless: false,
            no_new_privs: true,
            ..ok_podman()
        };
        assert!(assess(Engine::Docker, &f).is_ok());
        assert_eq!(version_of("v28.0.4"), Some((28, 0)));
        assert_eq!(version_of("4.9.3-dev"), Some((4, 9)));
    }
}
