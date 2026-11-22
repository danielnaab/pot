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

/// The engine's command with the option `run` adds after `run`: `--tty` when stdin is a terminal (the engine then
/// relays through a pty of its own, never the host's terminal). No fd of the caller's reaches the container: the
/// variables go over a named pipe the plan binds.
pub fn run_argv(argv: &[String], tty: bool) -> Vec<String> {
    let mut v = argv.to_vec();
    if let (true, Some(i)) = (tty, v.iter().position(|a| a == "run")) {
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

/// docker's name for the run's container: `pot-` and 16 hex digits of the run folder's path (FNV-1a), so `run` and
/// `cleanup` find it whether or not its id reached the cidfile.
pub fn docker_name(rundir: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rundir.trim_end_matches('/').bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("pot-{h:016x}")
}

/// The command that removes the run's container, and waits for the engine to have removed it. podman: every
/// container in the run's own store (`--all`: the store is the run's alone, so a container whose id never reached
/// the cidfile goes too). docker: the one with the run's name, with its anonymous volumes.
pub fn remove_argv(engine: Engine, program: &str, rundir: &str) -> Vec<String> {
    let mut v = vec![program.to_string()];
    match engine {
        Engine::Podman => {
            v.extend(podman_globals(rundir));
            v.extend(["rm", "--all", "--force", "--time", "0"].map(String::from));
        }
        Engine::Docker => v.extend(["rm", "--force", "--volumes", &docker_name(rundir)].map(String::from)),
    }
    v
}

/// The command that stops the run's container (SIGTERM, then SIGKILL after 10 s), when `run` is asked to stop:
/// podman's by the id in its cidfile (none yet: `None`), docker's by its name.
pub fn stop_argv(engine: Engine, program: &str, rundir: &str, cid: Option<&str>) -> Option<Vec<String>> {
    let mut v = vec![program.to_string()];
    let target = match engine {
        Engine::Podman => {
            v.extend(podman_globals(rundir));
            cid.filter(|c| is_cid(c))?.to_string()
        }
        Engine::Docker => docker_name(rundir),
    };
    v.extend(["stop", "--time", "10", &target].map(String::from));
    Some(v)
}

/// docker image configuration that must be empty, null or unset (what an empty `docker import` has).
const IMAGE_EMPTY: &[&str] = &[
    "Entrypoint",
    "Cmd",
    "Env",
    "Volumes",
    "User",
    "WorkingDir",
    "OnBuild",
    "Labels",
    "ExposedPorts",
    "Healthcheck",
    "StopSignal",
    "Shell",
    "ArgsEscaped",
];

/// What docker's `image inspect` shows that is not pot's empty image: a layer other than the one empty layer, or a
/// configuration that adds to the run (an entrypoint, a command, variables, volumes, a user, a working directory,
/// triggers, labels, ports, a health check or a stop signal).
pub fn image_gaps(inspect: &serde_json::Value) -> Vec<String> {
    let mut gaps = vec![];
    if inspect["RootFS"]["Layers"] != serde_json::json!([EMPTY_LAYER]) {
        gaps.push("a layer that is not the one empty layer".to_string());
    }
    let Some(config) = inspect["Config"].as_object() else {
        gaps.push("no Config".into());
        return gaps;
    };
    let mut bad: Vec<&String> = config
        .iter()
        .filter(|(k, v)| {
            IMAGE_EMPTY.contains(&k.as_str())
                && !(v.is_null()
                    || v.as_str() == Some("")
                    || v.as_array().is_some_and(Vec::is_empty)
                    || v.as_object().is_some_and(serde_json::Map::is_empty)
                    || v.as_bool() == Some(false))
        })
        .map(|(k, _)| k)
        .collect();
    bad.sort();
    gaps.extend(bad.into_iter().cloned());
    gaps
}

/// The secret variables' records for the pipe: `NAME=value` and a NUL each, for the names that are set.
pub fn secret_records(names: &[String], get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut out = vec![];
    for n in names {
        if let Some(v) = get(n) {
            out.extend_from_slice(n.as_bytes());
            out.push(b'=');
            out.extend_from_slice(v.as_bytes());
            out.push(0);
        }
    }
    // the sentinel: the script runs nothing unless the stream ends with it (S1)
    out.extend_from_slice(b".\0");
    out
}

/// The `after` event for what this backend carries out, in place with read access or none: nothing the run can
/// leave (bwrap's event, empty: no commits, no host-run files, no sealed leftovers, no proxy). Clone mode and write
/// access are refused, as `prepare` refuses them.
pub fn after_event(p: &Policy, l: &Launch, proxy_log: &str) -> Result<serde_json::Value, String> {
    if p.mode == "clone" {
        return Err("clone mode is not in this slice".into());
    }
    if p.access == "write" {
        return Err("write access is not in this slice".into());
    }
    let (denials, loopback) = proxy_refusals(proxy_log);
    Ok(
        serde_json::json!({ "ev": "after", "by": "host", "commits": [], "unsigned": 0, "touches_host_run": [],
        "repo_kit_change": [], "root_repo_change": false, "new_git_metadata": [], "denials": denials,
        "loopback": loopback, "agent_config": [], "signs": l.host_signs, "sealed_left": [] }),
    )
}

/// What the proxy refused, from its log (bwrap's `after`): each `DENY host:port` (or `DENY (malformed)`) counted, and
/// each `LOOPBACK host:port address` (a host that resolved to the host's own loopback), sorted. Run-chosen text is
/// made safe for a terminal.
pub fn proxy_refusals(log: &str) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    use std::collections::BTreeMap;
    let mut denied: BTreeMap<String, u64> = BTreeMap::new();
    let mut local: BTreeMap<(String, String), u64> = BTreeMap::new();
    for line in log.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.first() {
            Some(&"DENY") => *denied.entry(parts[1..].join(" ")).or_default() += 1,
            Some(&"LOOPBACK") if parts.len() == 3 => {
                *local
                    .entry((parts[1].to_string(), parts[2].to_string()))
                    .or_default() += 1;
            }
            _ => {}
        }
    }
    (
        denied
            .into_iter()
            .map(|(h, n)| serde_json::json!({ "host": crate::visible(&h), "count": n }))
            .collect(),
        local
            .into_iter()
            .map(|((h, a), n)| serde_json::json!({ "host": crate::visible(&h), "address": crate::visible(&a), "count": n }))
            .collect(),
    )
}

/// Whether CMDLINE is pot's proxy for this run: started in the run folder with its socket `proxy.sock` and the run's
/// own `--denials RUNDIR/denials` (main.rs checks its working folder too).
pub fn names_proxy(cmdline: &[u8], rundir: &str) -> bool {
    let r = rundir.trim_end_matches('/');
    let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
    let denials = format!("{r}/denials");
    args.iter().any(|a| *a == b"proxy.sock")
        && args
            .windows(2)
            .any(|w| w[0] == b"--denials" && w[1] == denials.as_bytes())
}

/// The names the configured engine program runs under: its own name, its real file's, and nix's wrapped name
/// (`.NAME-wrapped`, which conmon's exit command names).
pub fn engine_names(program: &str) -> Vec<String> {
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    let mut names = vec![base(program)];
    let found = if program.contains('/') {
        Some(std::path::PathBuf::from(program))
    } else {
        std::env::var_os("PATH").and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join(program))
                .find(|p| p.is_file())
        })
    };
    if let Some(real) = found.and_then(|p| std::fs::canonicalize(p).ok()) {
        names.push(base(&real.to_string_lossy()));
    }
    for n in names.clone() {
        names.push(format!(".{n}-wrapped"));
    }
    names.sort();
    names.dedup();
    names
}

/// Whether CMDLINE (a process's NUL-separated arguments) is one of the engine's processes for this run: podman (its
/// `run`, `rm`, `stop`, conmon's exit command) with the exact `--root RUNDIR/enginestore`, or conmon with its bundle
/// in the run's store or its exit folder in the run's. A process that merely names a path there is not.
pub fn names_engine_state(cmdline: &[u8], rundir: &str, names: &[String]) -> bool {
    let r = rundir.trim_end_matches('/');
    let args: Vec<&[u8]> = cmdline.split(|b| *b == 0).collect();
    let name = args
        .first()
        .map(|a| a.rsplit(|b| *b == b'/').next().unwrap_or(a))
        .unwrap_or_default();
    let is = |a: &[u8], s: &str| a == s.as_bytes();
    let under = |a: &[u8], d: &str| a.starts_with(format!("{r}/{d}/").as_bytes());
    let pair = |f: &dyn Fn(&[u8], &[u8]) -> bool| args.windows(2).any(|w| f(w[0], w[1]));
    if names.iter().any(|n| n.as_bytes() == name) {
        pair(&|k, v| is(k, "--root") && is(v, &format!("{r}/enginestore")))
    } else if name == b"conmon" {
        pair(&|k, v| {
            (is(k, "-b") && under(v, "enginestore"))
                || (is(k, "--exit-dir") && is(v, &format!("{r}/enginetmp/exits")))
                || ((is(k, "-p") || is(k, "--conmon-pidfile")) && under(v, "engineroot"))
        })
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn tty_goes_after_run_and_no_fd_is_passed() {
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
            run_argv(&argv, true),
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
        assert_eq!(run_argv(&argv, false), argv);
        assert!(!run_argv(&argv, true).iter().any(|a| a == "--preserve-fd"));
    }

    /// bwrap's after: DENY lines counted per host:port, LOOPBACK lines per host and address; other lines ignored;
    /// run-chosen text made safe for a terminal.
    #[test]
    fn the_proxy_log_gives_denials_and_loopback() {
        let log = "ALLOW example.com:443\nDENY example.org:443\nDENY example.org:443\nDENY (malformed)\n\
                   LOOPBACK localhost:8080 127.0.0.1\nLOOPBACK localhost:8080 127.0.0.1\nNOTOKEN\nDENY a\u{1b}b:1\n";
        let (d, l) = proxy_refusals(log);
        assert_eq!(
            d,
            [
                serde_json::json!({ "host": "(malformed)", "count": 1 }),
                serde_json::json!({ "host": "a\\x1bb:1", "count": 1 }),
                serde_json::json!({ "host": "example.org:443", "count": 2 }),
            ]
        );
        assert_eq!(
            l,
            [serde_json::json!({ "host": "localhost:8080", "address": "127.0.0.1", "count": 2 })]
        );
        assert_eq!(proxy_refusals(""), (vec![], vec![]));
    }

    #[test]
    fn the_runs_proxy_is_known_by_its_socket_and_denials() {
        let c = b"/nix/store/p/bin/python3\0-I\0-S\0/x/proxy.py\0proxy.sock\0--denials\0/s/runs/abc/denials\0--context\0w\0";
        assert!(names_proxy(c, "/s/runs/abc"));
        assert!(names_proxy(c, "/s/runs/abc/"));
        assert!(!names_proxy(c, "/s/runs/ab"));
        assert!(!names_proxy(
            b"tail\0-f\0/s/runs/abc/denials\0proxy.sock\0",
            "/s/runs/abc"
        ));
        assert!(!names_proxy(
            b"python3\0proxy.py\0x.sock\0--denials\0/s/runs/abc/denials\0",
            "/s/runs/abc"
        ));
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
        let g = [
            "--root",
            "/r/enginestore",
            "--runroot",
            "/r/engineroot",
            "--tmpdir",
            "/r/enginetmp",
        ];
        let with = |pre: &[&str], post: &[&str]| s(&[pre, &g[..], post].concat());
        assert_eq!(
            remove_argv(Engine::Podman, "podman", "/r"),
            with(&["podman"], &["rm", "--all", "--force", "--time", "0"])
        );
        let name = docker_name("/r");
        assert_eq!(
            remove_argv(Engine::Docker, "docker", "/r/"),
            s(&["docker", "rm", "--force", "--volumes", &name])
        );
        assert_eq!(
            stop_argv(Engine::Podman, "podman", "/r", Some(id)),
            Some(with(&["podman"], &["stop", "--time", "10", id]))
        );
        assert_eq!(stop_argv(Engine::Podman, "podman", "/r", None), None);
        assert_eq!(stop_argv(Engine::Podman, "podman", "/r", Some("--all")), None);
        assert_eq!(
            stop_argv(Engine::Docker, "docker", "/r", None),
            Some(s(&["docker", "stop", "--time", "10", &name]))
        );
    }

    #[test]
    fn dockers_name_is_the_run_folders() {
        let a = docker_name("/s/runs/abc");
        assert_eq!(a, docker_name("/s/runs/abc/"));
        assert_ne!(a, docker_name("/s/runs/abd"));
        assert!(a.starts_with("pot-") && a.len() == 20, "{a}");
        assert!(a[4..].bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn engine_processes_are_the_engines_for_this_run() {
        let r = "/s/runs/abc";
        let names = engine_names("/nix/store/p/bin/podman");
        let is = |c: &[u8]| names_engine_state(c, r, &names);
        let conmon =
            b"/nix/store/x/bin/conmon\0-c\0id\0-b\0/s/runs/abc/enginestore/overlay-containers/x/userdata\0";
        assert!(is(conmon));
        // nix's wrapper runs .podman-wrapped, which conmon's exit command names
        assert!(is(
            b"/nix/store/p/bin/.podman-wrapped\0--root\0/s/runs/abc/enginestore\0container\0cleanup\0"
        ));
        assert!(is(
            b"/nix/store/p/bin/podman\0--root\0/s/runs/abc/enginestore\0run\0"
        ));
        assert!(names_engine_state(
            b"podman\0--root\0/s/runs/abc/enginestore\0",
            "/s/runs/abc/",
            &names
        ));
        assert!(is(b"conmon\0--exit-dir\0/s/runs/abc/enginetmp/exits\0"));
        // a process that names a path there is not the engine's (R5), nor is another run's engine
        assert!(!is(b"tail\0-f\0/s/runs/abc/enginestore/x\0"));
        assert!(!is(b"sh\0-c\0sleep 20\0--root\0/s/runs/abc/enginestore\0"));
        assert!(!is(b"podman\0ps\0/s/runs/abc/enginestore/x\0"));
        assert!(!is(b"podman\0--root\0/s/runs/abcd/enginestore\0"));
        assert!(!is(b"pot-container\0run\0/s/runs/abc\0"));
        // S7: the configured program, not any name with podman in it
        let renamed = engine_names("/opt/x/eng");
        assert!(names_engine_state(
            b"/opt/x/eng\0--root\0/s/runs/abc/enginestore\0",
            r,
            &renamed
        ));
        assert!(!names_engine_state(
            b"/usr/bin/podman-x\0--root\0/s/runs/abc/enginestore\0",
            r,
            &renamed
        ));
        assert!(names_engine_state(conmon, r, &renamed));
    }

    #[test]
    fn only_the_empty_image_with_no_configuration_is_pots() {
        let empty = serde_json::json!({ "RootFS": { "Layers": [EMPTY_LAYER] }, "Config": { "Hostname": "", "User": "",
            "Env": null, "Cmd": null, "Volumes": null, "WorkingDir": "", "Entrypoint": null, "OnBuild": null, "Labels": null } });
        assert_eq!(image_gaps(&empty), Vec::<String>::new());
        let mut bad = empty.clone();
        bad["RootFS"]["Layers"] = serde_json::json!([EMPTY_LAYER, EMPTY_LAYER]);
        assert_eq!(image_gaps(&bad), ["a layer that is not the one empty layer"]);
        let mut bad = empty.clone();
        for (k, v) in [
            ("Entrypoint", serde_json::json!(["/bin/false"])),
            ("Env", serde_json::json!(["A=1"])),
            ("Volumes", serde_json::json!({ "/data": {} })),
            ("User", serde_json::json!("0")),
            ("Healthcheck", serde_json::json!({ "Test": ["CMD", "x"] })),
            ("StopSignal", serde_json::json!("SIGKILL")),
            ("ExposedPorts", serde_json::json!({ "80/tcp": {} })),
        ] {
            bad["Config"][k] = v;
        }
        assert_eq!(
            image_gaps(&bad),
            [
                "Entrypoint",
                "Env",
                "ExposedPorts",
                "Healthcheck",
                "StopSignal",
                "User",
                "Volumes"
            ]
        );
        assert_eq!(
            image_gaps(&serde_json::json!({})),
            ["a layer that is not the one empty layer", "no Config"]
        );
    }

    #[test]
    fn secrets_are_nul_separated_records_of_the_names_set() {
        let get = |n: &str| match n {
            "A" => Some("one\ntwo=three".into()),
            "B" => Some(String::new().into()),
            _ => None,
        };
        assert_eq!(
            secret_records(&s(&["A", "C", "B"]), get),
            b"A=one\ntwo=three\0B=\0.\0".to_vec()
        );
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
        let ev = after_event(&p, &l, "").unwrap();
        assert_eq!(
            ev,
            serde_json::json!({ "ev": "after", "by": "host", "commits": [], "unsigned": 0, "touches_host_run": [],
                "repo_kit_change": [], "root_repo_change": false, "new_git_metadata": [], "denials": [], "loopback": [],
                "agent_config": [], "signs": false, "sealed_left": [] })
        );
        l.host_signs = true;
        assert_eq!(after_event(&p, &l, "").unwrap()["signs"], true);
        let ev = after_event(&p, &l, "DENY example.org:443\nLOOPBACK localhost:1 ::1\n").unwrap();
        assert_eq!(
            ev["denials"],
            serde_json::json!([{ "host": "example.org:443", "count": 1 }])
        );
        assert_eq!(
            ev["loopback"],
            serde_json::json!([{ "host": "localhost:1", "address": "::1", "count": 1 }])
        );
        let (p, l) = fixture("access-none");
        assert!(after_event(&p, &l, "").is_ok());
        let (mut p, l) = fixture("write");
        assert_eq!(
            after_event(&p, &l, "").unwrap_err(),
            "write access is not in this slice"
        );
        p.mode = "clone".into();
        assert_eq!(
            after_event(&p, &l, "").unwrap_err(),
            "clone mode is not in this slice"
        );
    }
}
