//! `run`, `after` and `cleanup` on real run folders.
//!
//! The engine tests start real containers, so they run only when asked: `POT_TEST_PODMAN` names a podman program
//! (rootless), `POT_TEST_DOCKER` a docker one (a daemon the caller may use). Elsewhere (the flake's sandbox) they
//! pass without running. `cleanup`'s tests that need no engine always run.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_pot-container");

fn id(field: &str) -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with(field)).unwrap();
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn which(name: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap();
    let found = std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{name} is not on PATH"));
    fs::canonicalize(found).unwrap()
}

/// The coreutils program NAME, as a store path the run sees (its closure is bound).
fn coreutils(name: &str) -> String {
    which("env").with_file_name(name).display().to_string()
}

/// A run folder for ARGV: the read-only fixture with a real workspace and state directory, and the config's store
/// paths real ones (sh, env and their closure), so the command runs. ENGINE is `podman` or `docker`.
fn world(name: &str, engine: &str, program: &str, argv: &[&str]) -> (PathBuf, PathBuf) {
    let top = Path::new(env!("CARGO_TARGET_TMPDIR")).join("run").join(name);
    if top.exists() {
        // what an earlier run of this test left: through cleanup, which knows the engine's store
        let _ = Command::new(BIN)
            .arg("--config")
            .arg(top.join("config.json"))
            .arg("cleanup")
            .arg(top.join("run"))
            .output();
        fs::remove_dir_all(&top).unwrap();
    }
    let (ws, run, store) = (top.join("ws"), top.join("run"), top.join("store"));
    for d in [
        &ws.join(".git"),
        &run,
        &store,
        &top.join("state"),
        &store.join("terminfo"),
        &store.join("zoneinfo"),
    ] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(store.join("ca.crt"), "").unwrap();
    let (sh, env) = (which("sh"), which("env"));
    let closure = Command::new("nix-store")
        .arg("-qR")
        .arg(&sh)
        .arg(&env)
        .output()
        .unwrap();
    assert!(closure.status.success());
    fs::write(store.join("base.txt"), &closure.stdout).unwrap();
    let s = |f: &str| store.join(f).display().to_string();
    let config = serde_json::json!({ "engine": engine, engine: program, "sh": sh, "shell": sh, "env": env,
        "cacert": s("ca.crt"), "terminfo": s("terminfo"), "tzdir": s("zoneinfo"), "baseClosure": s("base.txt") });
    fs::write(top.join("config.json"), config.to_string()).unwrap();
    let docs =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/container/prepare.in/read-only");
    let mut p: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(docs.join("policy.json")).unwrap()).unwrap();
    let w = ws.display().to_string();
    p["workspace"][0]["path"] = w.clone().into();
    p["workspace"][0]["common"] = format!("{w}/.git").into();
    p["workspace"][0]["id"] = format!("repo:{w}").into();
    p["mounts"]["state"][0]["dir"] = top.join("state").display().to_string().into();
    let items: Vec<serde_json::Value> = p["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] != "file" && i["kind"] != "read")
        .cloned()
        .collect();
    p["items"] = items.into();
    // the test is not in pot's systemd scope
    let proved: Vec<serde_json::Value> = p["backend"]["proved"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| *g != "resource-limits")
        .cloned()
        .collect();
    p["backend"]["proved"] = proved.into();
    fs::write(run.join("policy.json"), p.to_string()).unwrap();
    let mut l: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(docs.join("launch.json")).unwrap()).unwrap();
    l["uid"] = id("Uid:").into();
    l["gid"] = id("Gid:").into();
    l["cwd"] = w.into();
    l["home"] = top.join("home").display().to_string().into();
    l["closure"] = serde_json::json!([]);
    l["potBin"] = serde_json::Value::Null;
    l["argv"] = serde_json::json!(argv);
    fs::write(run.join("launch.json"), l.to_string()).unwrap();
    (top, run)
}

fn verb(top: &Path, run: &Path, verb: &str) -> Command {
    let mut c = Command::new(BIN);
    c.arg("--config").arg(top.join("config.json")).arg(verb).arg(run);
    c
}

fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "exit {:?}: {}{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn prepare(top: &Path, run: &Path) {
    ok(&verb(top, run, "prepare").output().unwrap());
}

fn names(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn set(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| (*s).to_string()).collect()
}

/// The host's processes whose arguments name the run folder's engine state (podman, conmon, its exit command).
fn engine_processes(run: &Path) -> Vec<String> {
    let needle = format!(
        "{}/engine",
        fs::canonicalize(run).unwrap_or(run.to_path_buf()).display()
    );
    let mut out = vec![];
    for e in fs::read_dir("/proc").unwrap().flatten() {
        if let Ok(c) = fs::read(e.path().join("cmdline")) {
            let c = String::from_utf8_lossy(&c).replace('\0', " ");
            if c.contains(&needle) {
                out.push(format!("{}: {c}", e.file_name().to_string_lossy()));
            }
        }
    }
    out
}

/// podman with the run's own store and configuration (as run starts it), for the tests' own looks.
fn podman(program: &str, run: &Path, args: &[&str]) -> Output {
    let r = fs::canonicalize(run).unwrap().display().to_string();
    let mut c = Command::new(program);
    c.env_clear();
    for k in [
        "PATH",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "USER",
        "LOGNAME",
    ] {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    c.env("CONTAINERS_CONF", format!("{r}/containers.conf"))
        .env("CONTAINERS_STORAGE_CONF", format!("{r}/storage.conf"))
        .env("HOME", format!("{r}/enginehome"));
    c.args([
        "--root",
        &format!("{r}/enginestore"),
        "--runroot",
        &format!("{r}/engineroot"),
    ]);
    c.args(["--tmpdir", &format!("{r}/enginetmp")]).args(args);
    c.output().unwrap()
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// What a cleaned run folder holds: pot's files, and the status run records (bwrap's `status`).
const LEFT: &[&str] = &["denials", "launch.json", "policy.json"];

#[test]
fn podman_runs_the_command_and_returns_once_the_container_is_gone() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    // true, then an exit status 3; after either, no process of the engine's is left and the store has no container
    for (name, argv, code) in [
        ("true", vec!["/bin/sh", "-c", "true"], 0),
        ("three", vec!["/bin/sh", "-c", "exit 3"], 3),
    ] {
        let (top, run) = world(&format!("podman-{name}"), "podman", &program, &argv);
        prepare(&top, &run);
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(fs::read_to_string(run.join("status")).unwrap(), code.to_string());
        assert_eq!(
            engine_processes(&run),
            Vec::<String>::new(),
            "N7: run returned before the engine was done"
        );
        let ps = podman(&program, &run, &["ps", "--all", "--quiet"]);
        ok(&ps);
        assert_eq!(
            String::from_utf8_lossy(&ps.stdout),
            "",
            "a container is left in the run's store"
        );
        // the run's runtime state was in the run folder
        assert!(run.join("engineroot").is_dir() && run.join("enginetmp").is_dir());
        let after = verb(&top, &run, "after").output().unwrap();
        ok(&after);
        let ev: serde_json::Value = serde_json::from_slice(&after.stdout).unwrap();
        assert_eq!(ev["ev"], "after");
        ok(&verb(&top, &run, "cleanup").output().unwrap());
        let mut left = set(LEFT);
        left.insert("status".into());
        assert_eq!(names(&run), left);
        ok(&verb(&top, &run, "cleanup").output().unwrap());
        assert_eq!(names(&run), left);
    }
}

#[test]
fn podman_stops_the_container_on_sigterm() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let sleep = coreutils("sleep");
    let (top, run) = world("podman-term", "podman", &program, &[&sleep, "60"]);
    prepare(&top, &run);
    let mut child = verb(&top, &run, "run").stdin(Stdio::null()).spawn().unwrap();
    wait_for("the container", || {
        !String::from_utf8_lossy(&podman(&program, &run, &["ps", "--quiet"]).stdout)
            .trim()
            .is_empty()
    });
    let started = Instant::now();
    ok(&Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .output()
        .unwrap());
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(143));
    assert!(
        started.elapsed() < Duration::from_secs(9),
        "SIGTERM did not reach the command"
    );
    assert_eq!(engine_processes(&run), Vec::<String>::new());
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// N6: a run killed with its container running: cleanup removes the container and the store through the engine
/// (files of the user namespace's other ids included), leaves no engine process, and can run again.
#[test]
fn podman_cleanup_removes_a_killed_runs_container_and_store() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let sleep = coreutils("sleep");
    let (top, run) = world("podman-killed", "podman", &program, &[&sleep, "60"]);
    prepare(&top, &run);
    let mut child = verb(&top, &run, "run").stdin(Stdio::null()).spawn().unwrap();
    wait_for("the container", || {
        !String::from_utf8_lossy(&podman(&program, &run, &["ps", "--quiet"]).stdout)
            .trim()
            .is_empty()
    });
    let cid = fs::read_to_string(run.join("container.id")).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        !engine_processes(&run).is_empty(),
        "the container should outlive its killed run"
    );
    // a folder owned by another id of the user namespace, as a container's files are: the user can't remove it
    let scratch = top.join("unshare");
    fs::create_dir_all(scratch.join("home")).unwrap();
    let theirs = run.join("enginestore/theirs");
    let made = Command::new(&program)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("XDG_RUNTIME_DIR", std::env::var_os("XDG_RUNTIME_DIR").unwrap())
        .env("HOME", scratch.join("home"))
        .env("CONTAINERS_CONF", "/dev/null")
        .args([
            "--root",
            &scratch.join("s").display().to_string(),
            "--runroot",
            &scratch.join("r").display().to_string(),
        ])
        .args([
            "--tmpdir",
            &scratch.join("t").display().to_string(),
            "unshare",
            "sh",
            "-c",
        ])
        .arg(r#"mkdir -p "$1/x" && touch "$1/x/f" && chown -R 1:1 "$1""#)
        .arg("sh")
        .arg(&theirs)
        .output()
        .unwrap();
    ok(&made);
    assert!(fs::remove_dir_all(&theirs).is_err());
    let out = verb(&top, &run, "cleanup").output().unwrap();
    ok(&out);
    assert_eq!(names(&run), set(LEFT));
    assert_eq!(engine_processes(&run), Vec::<String>::new());
    let xdg = std::env::var("XDG_RUNTIME_DIR").unwrap();
    assert!(
        !Path::new(&xdg).join("crun").join(cid.trim()).exists(),
        "crun still has the container"
    );
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert_eq!(names(&run), set(LEFT));
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn docker_runs_the_command_in_an_empty_image_and_removes_the_container() {
    let Ok(program) = std::env::var("POT_TEST_DOCKER") else {
        eprintln!("POT_TEST_DOCKER is not set: no container started");
        return;
    };
    for (name, argv, code) in [
        ("true", vec!["/bin/sh", "-c", "true"], 0),
        ("three", vec!["/bin/sh", "-c", "exit 3"], 3),
    ] {
        let (top, run) = world(&format!("docker-{name}"), "docker", &program, &argv);
        prepare(&top, &run);
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(code),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let cid = fs::read_to_string(run.join("container.id")).unwrap();
        let gone = Command::new(&program)
            .args(["container", "inspect", cid.trim()])
            .output()
            .unwrap();
        assert!(!gone.status.success(), "the container is still there");
        let layers = Command::new(&program)
            .args([
                "image",
                "inspect",
                "--format",
                "{{json .RootFS.Layers}}",
                "localhost/pot-empty:1",
            ])
            .output()
            .unwrap();
        ok(&layers);
        assert_eq!(
            String::from_utf8_lossy(&layers.stdout).trim(),
            r#"["sha256:5f70bf18a086007016e948b04aed3b82103a36bea41755b6cddfaf10ace3c6ef"]"#
        );
        ok(&verb(&top, &run, "cleanup").output().unwrap());
        let mut left = set(LEFT);
        left.insert("status".into());
        assert_eq!(names(&run), left);
    }
}

/// cleanup never follows a link planted in the run folder: the link goes, what it points to stays.
#[test]
fn cleanup_never_follows_a_link() {
    let top = Path::new(env!("CARGO_TARGET_TMPDIR")).join("run/links");
    let _ = fs::remove_dir_all(&top);
    let (run, victim) = (top.join("run"), top.join("victim"));
    fs::create_dir_all(victim.join("sub")).unwrap();
    fs::write(victim.join("sub/keep"), "keep").unwrap();
    fs::create_dir_all(&run).unwrap();
    for n in [
        "enginestore",
        "engineroot",
        "enginetmp",
        "enginehome",
        "rootfs",
        "dockercfg",
        "potcfg",
    ] {
        symlink(&victim, run.join(n)).unwrap();
    }
    for n in ["container.id", "storage.conf", "passwd"] {
        symlink(victim.join("sub/keep"), run.join(n)).unwrap();
    }
    let out = Command::new(BIN).arg("cleanup").arg(&run).output().unwrap();
    ok(&out);
    assert_eq!(names(&run), BTreeSet::new());
    assert_eq!(fs::read_to_string(victim.join("sub/keep")).unwrap(), "keep");
}

/// N6: cleanup reports every removal it could not make, naming the path, and exits non-zero; never in silence.
#[test]
fn cleanup_reports_what_it_could_not_remove() {
    let top = Path::new(env!("CARGO_TARGET_TMPDIR")).join("run/unremovable");
    let _ = fs::remove_dir_all(&top);
    let run = top.join("run");
    fs::create_dir_all(run.join("rootfs")).unwrap();
    fs::create_dir_all(run.join("passwd")).unwrap();
    fs::write(run.join("rootfs/written"), "").unwrap();
    fs::write(run.join("policy.json"), r#"{"context":"work"}"#).unwrap();
    let out = Command::new(BIN).arg("cleanup").arg(&run).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    let r = fs::canonicalize(&run).unwrap().display().to_string();
    assert!(err.starts_with("pot (work): cleanup could not remove: "), "{err}");
    assert!(
        err.contains(&format!("{r}/rootfs")) && err.contains(&format!("{r}/passwd")),
        "{err}"
    );
    assert_eq!(err.lines().count(), 1, "{err}");
    // what it could not remove is left as it was
    assert!(run.join("rootfs/written").exists());
}
