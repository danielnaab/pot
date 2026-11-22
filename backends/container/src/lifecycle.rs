//! `run`, `after` and `cleanup`: what happens around the engine's command `prepare` wrote. The pure parts (the
//! commands, the exit status, the after event) are here and unit-tested; main.rs starts the processes.

use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use crate::launch::Launch;
use crate::policy::Policy;
use crate::render::{Engine, podman_globals};

/// The diff id of an empty layer (a tar of 1024 zero bytes, which `docker import` keeps as is): docker's run
/// image has exactly this one layer.
pub const EMPTY_LAYER: &str = "sha256:5f70bf18a086007016e948b04aed3b82103a36bea41755b6cddfaf10ace3c6ef";

/// The engine's command with `--tty` after `run` when stdin is a terminal (the engine then relays through a pty
/// of its own, never the host's terminal).
pub fn with_tty(argv: &[String], tty: bool) -> Vec<String> {
    let mut v = argv.to_vec();
    if let Some(i) = v.iter().position(|a| a == "run").filter(|_| tty) {
        v.insert(i + 1, "--tty".into());
    }
    v
}

/// The command's status as a shell, bwrap's `run` and pot record it: its exit code, or 128 + the signal that
/// killed the engine.
pub fn exit_code(s: ExitStatus) -> u8 {
    match (s.code(), s.signal()) {
        (Some(c), _) => u8::try_from(c & 0xff).unwrap_or(u8::MAX),
        (None, Some(sig)) => u8::try_from(128 + (sig & 0x7f)).unwrap_or(u8::MAX),
        (None, None) => u8::MAX,
    }
}

/// A container id as the engines write it to `--cidfile`: 64 lowercase hex digits.
pub fn is_cid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The command that removes the run's container, and waits for the engine to have removed it. podman: every
/// container in the run's own store (`--all`: the store is the run's alone, so a container whose id never reached
/// the cidfile goes too). docker: the one its cidfile names.
pub fn remove_argv(engine: Engine, program: &str, rundir: &str, cid: Option<&str>) -> Option<Vec<String>> {
    let mut v = vec![program.to_string()];
    match engine {
        Engine::Podman => {
            v.extend(podman_globals(rundir));
            v.extend(["rm", "--all", "--force", "--time", "0"].map(String::from));
        }
        Engine::Docker => {
            let cid = cid.filter(|c| is_cid(c))?;
            v.extend(["rm", "--force", cid].map(String::from));
        }
    }
    Some(v)
}

/// The command that stops a running container (SIGTERM, then SIGKILL after 10 s), when `run` is asked to stop.
pub fn stop_argv(engine: Engine, program: &str, rundir: &str, cid: &str) -> Vec<String> {
    let mut v = vec![program.to_string()];
    if engine == Engine::Podman {
        v.extend(podman_globals(rundir));
    }
    v.extend(["stop", "--time", "10", cid].map(String::from));
    v
}

/// The `after` event for what this backend carries out, in place with read access or none: nothing the run can
/// leave (bwrap's event, empty: no commits, no host-run files, no sealed leftovers, no proxy). Clone mode and write
/// access are refused, as `prepare` refuses them.
pub fn after_event(p: &Policy, l: &Launch) -> Result<serde_json::Value, String> {
    if p.mode == "clone" {
        return Err("clone mode is not in this slice".into());
    }
    if p.access == "write" {
        return Err("write access is not in this slice".into());
    }
    Ok(
        serde_json::json!({ "ev": "after", "by": "host", "commits": [], "unsigned": 0, "touches_host_run": [],
        "repo_kit_change": [], "root_repo_change": false, "new_git_metadata": [], "denials": [], "loopback": [],
        "agent_config": [], "signs": l.host_signs, "sealed_left": [] }),
    )
}

/// Whether CMDLINE (a process's NUL-separated arguments) names something in the run folder's engine state: the
/// engine's processes for this run (podman, conmon, conmon's exit command) all do.
pub fn names_engine_state(cmdline: &[u8], rundir: &str) -> bool {
    let needle = format!("{}/engine", rundir.trim_end_matches('/'));
    cmdline
        .split(|b| *b == 0)
        .any(|arg| arg.windows(needle.len()).any(|w| w == needle.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn tty_goes_after_run_only_for_a_terminal() {
        let argv = s(&[
            "podman",
            "--root",
            "/r/run",
            "run",
            "--interactive",
            "/usr/bin/env",
            "run",
        ]);
        assert_eq!(
            with_tty(&argv, true),
            s(&[
                "podman",
                "--root",
                "/r/run",
                "run",
                "--tty",
                "--interactive",
                "/usr/bin/env",
                "run"
            ])
        );
        assert_eq!(with_tty(&argv, false), argv);
    }

    #[test]
    fn a_signal_is_128_plus_its_number() {
        assert_eq!(exit_code(ExitStatus::from_raw(0)), 0);
        assert_eq!(exit_code(ExitStatus::from_raw(3 << 8)), 3);
        assert_eq!(exit_code(ExitStatus::from_raw(137 << 8)), 137);
        assert_eq!(exit_code(ExitStatus::from_raw(9)), 137);
        assert_eq!(exit_code(ExitStatus::from_raw(15)), 143);
    }

    #[test]
    fn a_cid_is_64_hex_digits() {
        let id = "4d5170dfef41aff33ccee5e96d51e457b59f3feb98337bc6328ae21e41695932";
        assert!(is_cid(id));
        assert!(!is_cid(&id[1..]));
        assert!(!is_cid(&format!("{}A", &id[1..])));
        assert!(!is_cid("--all"));
        assert!(!is_cid(""));
    }

    #[test]
    fn the_container_is_removed_through_the_engine() {
        let id = "4d5170dfef41aff33ccee5e96d51e457b59f3feb98337bc6328ae21e41695932";
        assert_eq!(
            remove_argv(Engine::Podman, "podman", "/r", None).unwrap(),
            s(&[
                "podman",
                "--root",
                "/r/enginestore",
                "--runroot",
                "/r/engineroot",
                "--tmpdir",
                "/r/enginetmp",
                "rm",
                "--all",
                "--force",
                "--time",
                "0"
            ])
        );
        assert_eq!(
            remove_argv(Engine::Docker, "docker", "/r", Some(id)).unwrap(),
            s(&["docker", "rm", "--force", id])
        );
        assert_eq!(remove_argv(Engine::Docker, "docker", "/r", None), None);
        assert_eq!(remove_argv(Engine::Docker, "docker", "/r", Some("--all")), None);
        assert_eq!(
            stop_argv(Engine::Podman, "podman", "/r", id),
            s(&[
                "podman",
                "--root",
                "/r/enginestore",
                "--runroot",
                "/r/engineroot",
                "--tmpdir",
                "/r/enginetmp",
                "stop",
                "--time",
                "10",
                id
            ])
        );
        assert_eq!(
            stop_argv(Engine::Docker, "docker", "/r", id),
            s(&["docker", "stop", "--time", "10", id])
        );
    }

    #[test]
    fn engine_processes_are_found_by_the_run_folder() {
        let conmon = b"conmon\0-b\0/s/runs/abc/enginestore/overlay-containers/x/userdata\0";
        assert!(names_engine_state(conmon, "/s/runs/abc"));
        assert!(names_engine_state(
            b"podman\0--runroot\0/s/runs/abc/engineroot\0",
            "/s/runs/abc/"
        ));
        // pot's own command line names the run folder, never its engine state; another run's folder is not this one
        assert!(!names_engine_state(
            b"pot-container\0run\0/s/runs/abc\0",
            "/s/runs/abc"
        ));
        assert!(!names_engine_state(
            b"podman\0--root\0/s/runs/abcd/enginestore\0",
            "/s/runs/abc"
        ));
    }

    fn fixture(name: &str) -> (Policy, Launch) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/reference/container/prepare.in");
        let read = |f: &str| std::fs::read_to_string(dir.join(name).join(f)).unwrap();
        (
            crate::policy::parse(&read("policy.json")).unwrap(),
            serde_json::from_str(&read("launch.json")).unwrap(),
        )
    }

    #[test]
    fn after_is_bwraps_empty_event_and_refuses_what_prepare_refuses() {
        let (p, mut l) = fixture("read-only");
        let ev = after_event(&p, &l).unwrap();
        assert_eq!(
            ev,
            serde_json::json!({ "ev": "after", "by": "host", "commits": [], "unsigned": 0, "touches_host_run": [],
                "repo_kit_change": [], "root_repo_change": false, "new_git_metadata": [], "denials": [], "loopback": [],
                "agent_config": [], "signs": false, "sealed_left": [] })
        );
        l.host_signs = true;
        assert_eq!(after_event(&p, &l).unwrap()["signs"], true);
        let (p, l) = fixture("access-none");
        assert!(after_event(&p, &l).is_ok());
        let (mut p, l) = fixture("write");
        assert_eq!(
            after_event(&p, &l).unwrap_err(),
            "write access is not in this slice"
        );
        p.mode = "clone".into();
        assert_eq!(
            after_event(&p, &l).unwrap_err(),
            "clone mode is not in this slice"
        );
    }
}
