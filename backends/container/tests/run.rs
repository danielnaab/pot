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
    // the closure the run sees; none in the flake's sandbox (no nix-store), where no container starts
    let closure = Command::new("nix-store")
        .arg("-qR")
        .arg(&sh)
        .arg(&env)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
        .unwrap_or_default();
    // without nix-store (the flake's sandbox): the store paths of sh and env, so their programs are bound
    let closure = if closure.is_empty() {
        let top_of = |p: &Path| p.components().take(4).collect::<PathBuf>().display().to_string();
        format!("{}\n{}\n", top_of(&sh), top_of(&env)).into_bytes()
    } else {
        closure
    };
    fs::write(store.join("base.txt"), &closure).unwrap();
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
    // docker can't be handed a secret (R3): its runs pass the variable by name, as a plain one
    if engine == "docker" {
        for it in p["items"].as_array_mut().unwrap() {
            if it["kind"] == "env" {
                it["secret"] = false.into();
            }
        }
    }
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
    // S2: a store that is not a real folder is named, and kept with the engine's state; the rest goes
    let out = Command::new(BIN).arg("cleanup").arg(&run).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    let r = fs::canonicalize(&run).unwrap().display().to_string();
    assert!(
        err.contains(&format!("{r}/enginestore (not a folder of the run's)")),
        "{err}"
    );
    assert_eq!(names(&run), set(&["enginestore", "engineroot", "enginetmp"]));
    assert_eq!(fs::read_to_string(victim.join("sub/keep")).unwrap(), "keep");
    fs::remove_file(run.join("enginestore")).unwrap();
    ok(&Command::new(BIN).arg("cleanup").arg(&run).output().unwrap());
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

/// The argument after OPT in the run's `container.argv`.
fn argv_value(run: &Path, opt: &str) -> Option<String> {
    let argv = fs::read(run.join("container.argv")).unwrap();
    let words: Vec<String> = argv
        .split(|b| *b == 0)
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .collect();
    words.iter().position(|w| w == opt).map(|i| words[i + 1].clone())
}

/// A program standing in for the engine: it records that it ran, and fails.
fn fake_engine(top: &Path) -> (PathBuf, PathBuf) {
    let (fake, ran) = (top.join("fake-engine"), top.join("fake-engine-ran"));
    fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$@\" >> {}\nexit 1\n", ran.display()),
    )
    .unwrap();
    Command::new("chmod").arg("755").arg(&fake).status().unwrap();
    (fake, ran)
}

fn set_engine_program(top: &Path, engine: &str, program: &Path) {
    let path = top.join("config.json");
    let mut c: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    c[engine] = program.display().to_string().into();
    fs::write(&path, c.to_string()).unwrap();
}

/// R1: a SIGTERM before the engine wrote the cidfile leaves no container: run removes it by the run's own name
/// (docker) or store (podman), never only by the cidfile.
#[test]
fn an_early_sigterm_leaves_no_container() {
    for (engine, var) in [("podman", "POT_TEST_PODMAN"), ("docker", "POT_TEST_DOCKER")] {
        let Ok(program) = std::env::var(var) else {
            eprintln!("{var} is not set: no container started");
            continue;
        };
        let sleep = coreutils("sleep");
        for delay in [10, 20, 40] {
            let (top, run) = world(&format!("{engine}-early"), engine, &program, &[&sleep, "300"]);
            prepare(&top, &run);
            let mut child = verb(&top, &run, "run").stdin(Stdio::null()).spawn().unwrap();
            std::thread::sleep(Duration::from_millis(delay));
            ok(&Command::new("kill")
                .arg("-TERM")
                .arg(child.id().to_string())
                .output()
                .unwrap());
            let started = Instant::now();
            child.wait().unwrap();
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "{engine}: the run did not stop"
            );
            if engine == "docker" {
                let name = argv_value(&run, "--name").expect("docker's container has the run's name");
                let left = Command::new(&program)
                    .args(["ps", "--all", "--quiet", "--filter", &format!("name=^{name}$")])
                    .output()
                    .unwrap();
                ok(&left);
                assert_eq!(
                    String::from_utf8_lossy(&left.stdout),
                    "",
                    "{engine} at {delay} ms: a container is left"
                );
            } else {
                let ps = podman(&program, &run, &["ps", "--all", "--quiet"]);
                assert_eq!(
                    String::from_utf8_lossy(&ps.stdout),
                    "",
                    "{engine} at {delay} ms: a container is left"
                );
            }
            assert_eq!(engine_processes(&run), Vec::<String>::new());
            ok(&verb(&top, &run, "cleanup").output().unwrap());
        }
    }
}

/// R2: run refuses an engine path that is not what prepare left (a link, or a cidfile), before the engine starts.
#[test]
fn run_refuses_an_engine_path_planted_after_prepare() {
    for (name, plant) in [
        ("engineroot", "link"),
        ("enginetmp", "link"),
        ("enginestore", "link"),
        ("enginehome", "link"),
        ("rootfs", "link"),
        ("container.id", "file"),
        // S4: every file the plan binds from the run folder
        ("denials", "filelink"),
        ("policy.json", "filelink"),
        ("passwd", "filelink"),
        ("hidden.txt", "filelink"),
        ("emptydir", "link"),
        // the status run records: a folder with something in it
        ("status", "fulldir"),
    ] {
        let (top, run) = world(
            &format!("planted-{name}"),
            "podman",
            "podman",
            &["/bin/sh", "-c", "true"],
        );
        prepare(&top, &run);
        let (fake, log) = fake_engine(&top);
        set_engine_program(&top, "podman", &fake);
        // run checks container.argv's program against the config's: prepare again with the fake
        prepare(&top, &run);
        let elsewhere = top.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        let p = run.join(name);
        if p.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            fs::remove_dir_all(&p).unwrap();
        }
        match plant {
            "link" => symlink(&elsewhere, &p).unwrap(),
            "filelink" => {
                let target = top.join("elsewhere-file");
                if p.symlink_metadata().is_ok() {
                    fs::copy(&p, &target).unwrap();
                    fs::remove_file(&p).unwrap();
                } else {
                    fs::write(&target, "").unwrap();
                }
                symlink(&target, &p).unwrap();
            }
            "fulldir" => {
                fs::create_dir_all(p.join("x")).unwrap();
            }
            _ => fs::write(&p, "0000").unwrap(),
        }
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{name}");
        let err = String::from_utf8_lossy(&out.stderr);
        let r = fs::canonicalize(&run).unwrap().display().to_string();
        assert!(err.contains(&format!("{r}/{name}")), "{name}: {err}");
        assert!(!log.exists(), "{name}: the engine ran");
    }
}

/// R2: cleanup never hands the engine a linked path, and keeps the store (exit 1, named) when it could not remove
/// the container, rather than deleting it under a live one.
#[test]
fn cleanup_keeps_the_store_when_the_container_could_not_be_removed() {
    for linked in [Some("engineroot"), Some("enginetmp"), None] {
        let (top, run) = world("keeps-store", "podman", "podman", &["/bin/sh", "-c", "true"]);
        prepare(&top, &run);
        let (fake, log) = fake_engine(&top);
        set_engine_program(&top, "podman", &fake);
        fs::write(run.join("enginestore/db.sql"), "").unwrap();
        if let Some(n) = linked {
            let elsewhere = top.join("elsewhere");
            fs::create_dir_all(&elsewhere).unwrap();
            symlink(&elsewhere, run.join(n)).unwrap();
        }
        let out = verb(&top, &run, "cleanup").output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{linked:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        let r = fs::canonicalize(&run).unwrap().display().to_string();
        assert!(err.contains(&format!("{r}/enginestore")), "{linked:?}: {err}");
        assert!(
            run.join("enginestore/db.sql").exists(),
            "{linked:?}: the store was removed"
        );
        match linked {
            Some(n) => {
                assert!(!log.exists(), "{n}: the engine was given a linked path");
                assert!(err.contains(&format!("{r}/{n}")), "{err}");
            }
            None => assert!(log.exists()),
        }
        assert_eq!(err.lines().count(), 1, "{err}");
    }
}

/// R3: a secret variable's value is in no file of the run folder or the OCI runtime's state, and in no engine
/// process's arguments or environment, while the command has it.
#[test]
fn podman_keeps_a_secret_out_of_every_file() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let marker = "pot-qa-marker-7f3a1c";
    let sleep = coreutils("sleep");
    // the value only from the environment: the marker is in no argument of the command
    let script = format!("echo \"$NOTES_TOKEN\"; {sleep} 3");
    let (top, run) = world("podman-secret", "podman", &program, &["/bin/sh", "-c", &script]);
    prepare(&top, &run);
    let child = verb(&top, &run, "run")
        .env("NOTES_TOKEN", marker)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("the container", || {
        !String::from_utf8_lossy(&podman(&program, &run, &["ps", "--quiet"]).stdout)
            .trim()
            .is_empty()
    });
    std::thread::sleep(Duration::from_millis(500));
    let mut found = vec![];
    let xdg = PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap()).join("crun");
    let mut todo = vec![run.clone(), xdg];
    while let Some(d) = todo.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(m) = e.path().symlink_metadata() else {
                continue;
            };
            if m.is_dir() {
                todo.push(e.path());
            } else if m.is_file()
                && fs::read(e.path()).is_ok_and(|b| b.windows(marker.len()).any(|w| w == marker.as_bytes()))
            {
                found.push(e.path().display().to_string());
            }
        }
    }
    let needle = format!("{}/engine", fs::canonicalize(&run).unwrap().display());
    for e in fs::read_dir("/proc").unwrap().flatten() {
        let Ok(c) = fs::read(e.path().join("cmdline")) else {
            continue;
        };
        if !String::from_utf8_lossy(&c).contains(&needle) {
            continue;
        }
        for f in ["cmdline", "environ"] {
            if fs::read(e.path().join(f))
                .is_ok_and(|b| b.windows(marker.len()).any(|w| w == marker.as_bytes()))
            {
                found.push(format!("{}/{f}", e.path().display()));
            }
        }
    }
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        marker,
        "the command has the secret"
    );
    assert_eq!(found, Vec::<String>::new(), "the secret's value is there");
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// R4: docker refuses an image with the empty layer but a configuration (here an entrypoint), and removes the
/// container with its anonymous volumes.
#[test]
fn docker_refuses_an_image_with_a_configuration() {
    let Ok(program) = std::env::var("POT_TEST_DOCKER") else {
        eprintln!("POT_TEST_DOCKER is not set: no container started");
        return;
    };
    let image = "localhost/pot-qa-hostile:1";
    let mut import = Command::new(&program)
        .args(["import", "--change", r#"ENTRYPOINT ["/bin/false"]"#, "-", image])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut import.stdin.take().unwrap(), &[0u8; 1024]).unwrap();
    assert!(import.wait().unwrap().success());
    let (top, run) = world("docker-hostile", "docker", &program, &["/bin/sh", "-c", "true"]);
    let path = top.join("config.json");
    let mut c: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    c["image"] = image.into();
    fs::write(&path, c.to_string()).unwrap();
    prepare(&top, &run);
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    let _ = Command::new(&program).args(["rmi", image]).output();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("is not pot's empty image") && err.contains("Entrypoint"),
        "{err}"
    );
    assert_eq!(argv_value(&run, "--entrypoint").as_deref(), Some(""));
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// R5: a process that only names the run folder's engine state (here a sleep given a path in the store) is not
/// the engine's: run does not wait for it.
#[test]
fn podman_waits_only_for_the_engines_processes() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let (top, run) = world("podman-bystander", "podman", &program, &["/bin/sh", "-c", "true"]);
    prepare(&top, &run);
    let store = fs::canonicalize(run.join("enginestore")).unwrap();
    let mut bystander = Command::new("sh")
        .args(["-c", "sleep 20; :", "sh"])
        .arg(store.join("x"))
        .arg("--root")
        .arg(&store)
        .spawn()
        .unwrap();
    let started = Instant::now();
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    let took = started.elapsed();
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    let _ = bystander.kill();
    let _ = bystander.wait();
    ok(&out);
    assert!(
        took < Duration::from_secs(10),
        "run waited {took:?} for a bystander"
    );
}

/// R6: what only podman's user namespace can remove (a mode-000 folder of another id, as an overlay's work/work
/// is) goes through podman unshare; when that fails too, cleanup names the folder and exits 1.
#[test]
fn podman_cleanup_removes_a_mode_000_folder_of_another_id_or_names_it() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let plant = |top: &Path, dir: &Path| {
        let scratch = top.join("unshare");
        fs::create_dir_all(scratch.join("home")).unwrap();
        let made = Command::new(&program)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("XDG_RUNTIME_DIR", std::env::var_os("XDG_RUNTIME_DIR").unwrap())
            .env("HOME", scratch.join("home"))
            .env("CONTAINERS_CONF", "/dev/null")
            .args(["--root", &scratch.join("s").display().to_string()])
            .args(["--runroot", &scratch.join("r").display().to_string()])
            .args([
                "--tmpdir",
                &scratch.join("t").display().to_string(),
                "unshare",
                "sh",
                "-c",
            ])
            .arg(r#"mkdir -p "$1/work/work/x" && chown -R 1:1 "$1" && chmod 000 "$1/work/work""#)
            .arg("sh")
            .arg(dir)
            .output()
            .unwrap();
        ok(&made);
        let _ = fs::remove_dir_all(&scratch);
    };
    // removed through podman unshare
    let (top, run) = world("podman-mode000", "podman", &program, &["/bin/sh", "-c", "true"]);
    prepare(&top, &run);
    plant(&top, &run.join("engineroot/overlay"));
    assert!(fs::remove_dir_all(run.join("engineroot")).is_err());
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert_eq!(names(&run), set(LEFT));
    // podman unshare fails (a podman that can't run): named, exit 1
    let (top, run) = world(
        "podman-mode000-kept",
        "podman",
        &program,
        &["/bin/sh", "-c", "true"],
    );
    prepare(&top, &run);
    plant(&top, &run.join("engineroot/overlay"));
    let (fake, _) = fake_engine(&top);
    set_engine_program(&top, "podman", &fake);
    let out = verb(&top, &run, "cleanup").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    let r = fs::canonicalize(&run).unwrap().display().to_string();
    assert!(err.contains(&format!("{r}/engineroot")), "{err}");
    set_engine_program(&top, "podman", Path::new(&program));
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert_eq!(names(&run), set(LEFT));
}

/// The run's `podman` as the tests run it, for the engine-free tests.
fn secret_off(run: &Path) {
    let path = run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    for it in p["items"].as_array_mut().unwrap() {
        if it["kind"] == "env" {
            it["secret"] = false.into();
        }
    }
    fs::write(&path, p.to_string()).unwrap();
}

/// S1: the secrets' pipe is the fd the script reads, whatever fds the caller holds; it is closed before the command,
/// and no fd but 0, 1 and 2 is open in the command.
#[test]
fn podman_secrets_arrive_whatever_fds_the_caller_holds() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let script = r#"echo "token=$NOTES_TOKEN"; for i in 3 4 5 6 7 8 9 10 11 12; do [ -e /proc/$$/fd/$i ] && echo "open=$i"; done; true"#;
    let (top, run) = world("podman-fds", "podman", &program, &["/bin/sh", "-c", script]);
    prepare(&top, &run);
    // the caller holds fds 3 and 4 (a file each)
    let out = Command::new("sh")
        .arg("-c")
        .arg(r#"exec 3</etc/hostname 4</etc/hostname; exec "$@""#)
        .arg("sh")
        .arg(BIN)
        .arg("--config")
        .arg(top.join("config.json"))
        .arg("run")
        .arg(&run)
        .env("NOTES_TOKEN", "s1-value")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "token=s1-value\n");
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// S1: the script execs nothing unless fd 3 gives the complete record stream, ended by its sentinel.
#[test]
fn the_secrets_script_refuses_an_incomplete_stream() {
    if !Path::new("/usr/bin/env").exists() {
        eprintln!("no /usr/bin/env here: the script is not run");
        return;
    }
    let (top, run) = world(
        "secrets-script",
        "podman",
        "podman",
        &["/bin/sh", "-c", "echo RAN"],
    );
    prepare(&top, &run);
    let argv = fs::read(run.join("container.argv")).unwrap();
    let words: Vec<String> = argv
        .strip_suffix(&[0])
        .unwrap()
        .split(|b| *b == 0)
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .collect();
    let at = words.iter().position(|w| w == "--noprofile").unwrap() - 1;
    let entry = &words[at..];
    for (name, stream, ran) in [
        ("complete", &b"NOTES_TOKEN=v\0.\0"[..], true),
        ("no sentinel", &b"NOTES_TOKEN=v\0"[..], false),
        ("short", &b"NOTES_TOKEN=v"[..], false),
        ("empty", &b""[..], false),
        ("not a record", &b"NOTES_TOKEN\0.\0"[..], false),
    ] {
        let f = top.join("stream");
        fs::write(&f, stream).unwrap();
        let out = Command::new("sh")
            .arg("-c")
            .arg(r#"exec 3<"$0"; exec "$@""#)
            .arg(&f)
            .args(entry)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            said.contains("RAN"),
            ran,
            "{name}: {said} {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.status.success(), ran, "{name}");
    }
}

/// S3: the command's environment is exactly the planned one: pot's variables, env.set, the items passed by name and
/// the secrets, and nothing the engine or the run's bash adds; with secrets and without.
#[test]
fn podman_the_commands_environment_is_the_plan() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let envp = coreutils("env");
    for secret in [true, false] {
        let (top, run) = world(
            &format!("podman-env-{secret}"),
            "podman",
            &program,
            &[&envp, "-0"],
        );
        if !secret {
            secret_off(&run);
        }
        prepare(&top, &run);
        let out = verb(&top, &run, "run")
            .env("NOTES_TOKEN", "s3-value")
            .env("TERM", "xterm")
            .env_remove("COLORTERM")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        ok(&out);
        let got: BTreeSet<String> = out
            .stdout
            .split(|b| *b == 0)
            .filter(|w| !w.is_empty())
            .map(|w| String::from_utf8_lossy(w).into_owned())
            .collect();
        // the plan: --env NAME=VALUE as is, --env NAME with the caller's value when set, the secrets
        let argv = fs::read(run.join("container.argv")).unwrap();
        let words: Vec<String> = argv
            .strip_suffix(&[0])
            .unwrap()
            .split(|b| *b == 0)
            .map(|w| String::from_utf8_lossy(w).into_owned())
            .collect();
        let caller = |n: &str| match n {
            "NOTES_TOKEN" => Some("s3-value".to_string()),
            "TERM" => Some("xterm".to_string()),
            _ => None,
        };
        let mut want = BTreeSet::new();
        for w in words.windows(2).filter(|w| w[0] == "--env") {
            if w[1].contains('=') {
                want.insert(w[1].clone());
            } else if let Some(v) = caller(&w[1]) {
                want.insert(format!("{}={v}", w[1]));
            }
        }
        // and, with a secret, what goes over the pipe: the secret and every variable passed by name
        let cenv: serde_json::Value =
            serde_json::from_slice(&fs::read(run.join("container.env")).unwrap()).unwrap();
        for n in cenv["piped"].as_array().unwrap() {
            if let Some(v) = caller(n.as_str().unwrap()) {
                want.insert(format!("{}={v}", n.as_str().unwrap()));
            }
        }
        assert_eq!(cenv["piped"].as_array().unwrap().is_empty(), !secret);
        assert_eq!(got, want, "secret {secret}");
        ok(&verb(&top, &run, "cleanup").output().unwrap());
    }
}

/// T1: a variable bash can't hand over faithfully (readonly or set by bash itself) is refused by prepare, whatever
/// the engine.
#[test]
fn prepare_refuses_a_variable_bash_cannot_hand_over() {
    for name in [
        "UID",
        "EUID",
        "PPID",
        "SHELLOPTS",
        "BASHOPTS",
        "SHLVL",
        "PWD",
        "OLDPWD",
        "_",
        "PS2",
        "BASH_ARGV0",
        "BASH_ENV",
    ] {
        let (top, run) = world("bash-refused", "podman", "podman", &["/bin/sh", "-c", "true"]);
        let path = run.join("policy.json");
        let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for it in p["items"].as_array_mut().unwrap() {
            if it["kind"] == "env" {
                it["value"] = name.into();
            }
        }
        fs::write(&path, p.to_string()).unwrap();
        let out = verb(&top, &run, "prepare").output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{name}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains(&format!(
                "the variable {name}, which bash sets or keeps read-only"
            )),
            "{err}"
        );
    }
}

/// T1: no record's value is in any process's arguments at any moment of the run (every process on the host is
/// watched while it runs; a 100 000-byte variable passed by name slows the script's read loop, as the audit did).
#[test]
fn podman_no_value_is_ever_in_an_argument() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let marker = "pot-t1-marker-4b9e2d";
    let (top, run) = world(
        "podman-argv",
        "podman",
        &program,
        &["/bin/sh", "-c", "echo \"$NOTES_TOKEN\""],
    );
    prepare(&top, &run);
    for attempt in 0..6 {
        let mut child = verb(&top, &run, "run")
            .env("NOTES_TOKEN", marker)
            .env("COLORTERM", "x".repeat(100_000))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut caught = vec![];
        let mut scans = 0;
        let has = |c: &[u8]| c.windows(marker.len()).any(|w| w == marker.as_bytes());
        let show = |c: &[u8]| -> Vec<String> {
            c.split(|b| *b == 0)
                .take(4)
                .map(|a| String::from_utf8_lossy(a).chars().take(40).collect())
                .collect()
        };
        let mut seen = BTreeSet::new();
        while child.try_wait().unwrap().is_none() {
            for e in fs::read_dir("/proc").unwrap().flatten() {
                let pid = e.file_name().to_string_lossy().into_owned();
                if !pid.bytes().all(|b| b.is_ascii_digit()) || !seen.insert(pid) {
                    continue;
                }
                let Ok(c) = fs::read(e.path().join("cmdline")) else {
                    continue;
                };
                if has(&c) {
                    caught.push(show(&c));
                }
                // the run's bash: watched closely until it has exec'd the command (the audit's catch.py)
                let words: Vec<&[u8]> = c.split(|b| *b == 0).collect();
                if words.get(1) == Some(&&b"--noprofile"[..]) && words.contains(&&b"pot-secrets"[..]) {
                    loop {
                        scans += 1;
                        let Ok(c) = fs::read(e.path().join("cmdline")) else {
                            break;
                        };
                        if has(&c) {
                            caught.push(show(&c));
                            break;
                        }
                        // gone (a zombie's is empty), or the command itself
                        if c.is_empty()
                            || (!c.windows(11).any(|w| w == b"pot-secrets")
                                && !c.starts_with(b"/usr/bin/env"))
                        {
                            break;
                        }
                    }
                }
            }
        }
        let out = child.wait_with_output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            marker,
            "attempt {attempt}: the command has the secret"
        );
        assert!(
            caught.is_empty(),
            "attempt {attempt} ({scans} scans): a value in argv: {caught:?}"
        );
        ok(&verb(&top, &run, "cleanup").output().unwrap());
        prepare(&top, &run);
    }
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// T1, deterministic: the entry run on the host, every `/usr/bin/env` in it replaced by a program that records its
/// arguments, then runs env: no exec's arguments hold a record's value, and the command has it.
#[test]
fn no_exec_in_the_entry_is_given_a_value() {
    let (top, run) = world(
        "entry-argv",
        "podman",
        "podman",
        &["/bin/sh", "-c", "echo \"[$NOTES_TOKEN]\""],
    );
    prepare(&top, &run);
    let argv = fs::read(run.join("container.argv")).unwrap();
    let words: Vec<String> = argv
        .strip_suffix(&[0])
        .unwrap()
        .split(|b| *b == 0)
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .collect();
    let at = words.iter().position(|w| w == "--rootfs").unwrap() + 2;
    let (rec, log) = (top.join("record-env"), top.join("record-env.log"));
    fs::write(
        &rec,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexec {} \"$@\"\n",
            log.display(),
            coreutils("env")
        ),
    )
    .unwrap();
    Command::new("chmod").arg("755").arg(&rec).status().unwrap();
    let entry: Vec<String> = words[at..]
        .iter()
        .map(|w| {
            if w == "/usr/bin/env" {
                rec.display().to_string()
            } else {
                w.clone()
            }
        })
        .collect();
    let stream = top.join("stream");
    fs::write(&stream, b"NOTES_TOKEN=t1-value\0.\0").unwrap();
    let out = Command::new("sh")
        .arg("-c")
        .arg(r#"exec 3<"$0"; exec "$@""#)
        .arg(&stream)
        .args(&entry)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[t1-value]\n");
    let calls = fs::read_to_string(&log).unwrap();
    assert!(!calls.is_empty());
    assert!(
        !calls.contains("t1-value"),
        "a value in an exec's arguments: {calls}"
    );
}

/// S3: a secret named like one of bash's own variables arrives with its value, not bash's.
#[test]
fn podman_a_secret_named_like_a_bash_variable_keeps_its_value() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let envp = coreutils("env");
    for name in [
        "RANDOM",
        "SECONDS",
        "LINENO",
        "GROUPS",
        "IFS",
        "HISTCMD",
        "EPOCHSECONDS",
        "SRANDOM",
    ] {
        let (top, run) = world("podman-bashvar", "podman", &program, &[&envp, "-0"]);
        let path = run.join("policy.json");
        let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for it in p["items"].as_array_mut().unwrap() {
            if it["kind"] == "env" {
                it["value"] = name.into();
            }
        }
        fs::write(&path, p.to_string()).unwrap();
        prepare(&top, &run);
        let out = verb(&top, &run, "run")
            .env(name, "s3-mine")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        ok(&out);
        let want = format!("{name}=s3-mine");
        assert!(
            out.stdout.split(|b| *b == 0).any(|w| w == want.as_bytes()),
            "{name}: {}",
            String::from_utf8_lossy(&out.stdout).replace('\0', " ")
        );
        ok(&verb(&top, &run, "cleanup").output().unwrap());
    }
}

/// S2: a store that is not a real folder is reported, and engineroot and enginetmp are kept while a process of the
/// run lives; a cleanup once the store is back removes everything.
#[test]
fn podman_cleanup_keeps_the_state_of_a_live_run_with_a_linked_store() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let sleep = coreutils("sleep");
    let (top, run) = world("podman-linked-store", "podman", &program, &[&sleep, "60"]);
    prepare(&top, &run);
    let mut child = verb(&top, &run, "run").stdin(Stdio::null()).spawn().unwrap();
    wait_for("the container", || {
        !String::from_utf8_lossy(&podman(&program, &run, &["ps", "--quiet"]).stdout)
            .trim()
            .is_empty()
    });
    child.kill().unwrap();
    child.wait().unwrap();
    let real = top.join("store-moved");
    fs::rename(run.join("enginestore"), &real).unwrap();
    symlink(&real, run.join("enginestore")).unwrap();
    // T2: whatever happens below, the store goes back and cleanup runs, so the container and crun's state go
    let _restore = Restore {
        top: top.clone(),
        run: run.clone(),
        real: real.clone(),
    };
    let out = verb(&top, &run, "cleanup").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    let r = fs::canonicalize(&run).unwrap().display().to_string();
    assert!(err.contains(&format!("{r}/enginestore")), "{err}");
    assert!(
        run.join("engineroot").is_dir() && run.join("enginetmp").is_dir(),
        "{err}"
    );
    assert!(
        !engine_processes(&run).is_empty(),
        "the container should still run"
    );
    restore_store(&run, &real);
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert_eq!(names(&run), set(LEFT));
    assert_eq!(engine_processes(&run), Vec::<String>::new());
}

/// The run's store back in its place, when a test moved it and linked it.
fn restore_store(run: &Path, real: &Path) {
    if run
        .join("enginestore")
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        let _ = fs::remove_file(run.join("enginestore"));
    }
    if real.exists() && run.join("enginestore").symlink_metadata().is_err() {
        let _ = fs::rename(real, run.join("enginestore"));
    }
}

/// On drop (a test's end, or its panic): the store back, then cleanup, which removes the container through the
/// engine (and with it crun's state) and the store.
struct Restore {
    top: PathBuf,
    run: PathBuf,
    real: PathBuf,
}

impl Drop for Restore {
    fn drop(&mut self) {
        restore_store(&self.run, &self.real);
        let _ = verb(&self.top, &self.run, "cleanup").output();
    }
}

/// S5: with secrets, prepare refuses a shell that is not a real file under a path the plan binds.
#[test]
fn prepare_refuses_a_shell_the_run_cannot_see() {
    let (top, run) = world("shell-unbound", "podman", "podman", &["/bin/sh", "-c", "true"]);
    let outside = top.join("bash");
    fs::copy(which("bash"), &outside).unwrap();
    for shell in [outside.clone(), top.join("no-such-bash")] {
        let path = top.join("config.json");
        let mut c: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        c["shell"] = shell.display().to_string().into();
        fs::write(&path, c.to_string()).unwrap();
        let out = verb(&top, &run, "prepare").output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{}", shell.display());
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains(&format!("the config's shell {}", shell.display())),
            "{err}"
        );
    }
    // without secrets the shell is not needed in the run
    secret_off(&run);
    prepare(&top, &run);
}

/// S6: a SIGTERM before the engine starts (here during docker's image check) ends run as a stopped run (143), with its
/// status recorded and no engine command started.
#[test]
fn a_sigterm_before_the_engine_starts_records_the_status() {
    let (top, run) = world("term-early", "docker", "docker", &["/bin/sh", "-c", "true"]);
    let (fake, log) = (top.join("slow-docker"), top.join("slow-docker-ran"));
    let empty = r#"{"RootFS":{"Layers":["sha256:5f70bf18a086007016e948b04aed3b82103a36bea41755b6cddfaf10ace3c6ef"]},"Config":{}}"#;
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho \"$1\" >> {}\ncase \"$1\" in image) sleep 2; echo '{empty}';; esac\n",
            log.display()
        ),
    )
    .unwrap();
    Command::new("chmod").arg("755").arg(&fake).status().unwrap();
    set_engine_program(&top, "docker", &fake);
    prepare(&top, &run);
    let mut child = verb(&top, &run, "run").stdin(Stdio::null()).spawn().unwrap();
    wait_for("the image check", || log.exists());
    ok(&Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .output()
        .unwrap());
    assert_eq!(child.wait().unwrap().code(), Some(143));
    assert_eq!(fs::read_to_string(run.join("status")).unwrap(), "143");
    let calls = fs::read_to_string(&log).unwrap();
    assert!(!calls.lines().any(|l| l == "run"), "the engine started: {calls}");
}
