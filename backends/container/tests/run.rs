//! `run`, `after` and `cleanup` on real run folders.
//!
//! The engine tests start real containers, so they run only when asked: `POT_TEST_PODMAN` names a podman program
//! (rootless), `POT_TEST_DOCKER` a docker one (a daemon the caller may use). Elsewhere (the flake's sandbox) they
//! pass without running. `cleanup`'s tests that need no engine always run.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
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
    find(name).unwrap_or_else(|| panic!("{name} is not on PATH"))
}

/// NAME on PATH, as its real path; None when it is not there (the flake's sandbox has no socat or python).
fn find(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let found = std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())?;
    fs::canonicalize(found).ok()
}

/// The store paths P needs (`nix-store -qR`), when nix-store is there.
fn closure_of(paths: &[&Path]) -> Vec<u8> {
    Command::new("nix-store")
        .arg("-qR")
        .args(paths)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
        .unwrap_or_default()
}

/// pot's proxy, the repository's.
fn proxy_py() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/proxy.py")
}

/// The coreutils program NAME, as a store path the run sees (its closure is bound).
fn coreutils(name: &str) -> String {
    which("env").with_file_name(name).display().to_string()
}

/// A run folder for ARGV: the read-only fixture with a real workspace and state directory, and the config's store
/// paths real ones (sh, env and their closure), so the command runs. ENGINE is `podman` or `docker`; PROGRAM its
/// program, where a bare name (a test that starts no container) is one that is not there: the backend never looks one
/// up on PATH.
fn world(name: &str, engine: &str, program: &str, argv: &[&str]) -> (PathBuf, PathBuf) {
    let absent = format!("/nonexistent/{program}");
    let program = if program.contains('/') {
        program
    } else {
        absent.as_str()
    };
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
    let mut config = serde_json::json!({ "engine": engine, engine: program, "sh": sh, "shell": sh, "env": env,
        "cacert": s("ca.crt"), "terminfo": s("terminfo"), "tzdir": s("zoneinfo"), "baseClosure": s("base.txt") });
    // the network's programs, where they are (not in the flake's sandbox): the host's python runs pot's proxy, socat
    // serves it in the run
    if let (Some(python), Some(socat)) = (find("python3"), find("socat")) {
        fs::write(store.join("net.txt"), closure_of(&[&socat])).unwrap();
        config["python"] = python.display().to_string().into();
        config["proxy"] = fs::canonicalize(proxy_py()).unwrap().display().to_string().into();
        config["socat"] = socat.display().to_string().into();
        config["netClosure"] = s("net.txt").into();
    }
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
    let f = top.join("stream");
    // the stream is the file the entry is given in place of the run's named pipe
    let entry: Vec<String> = words[at..]
        .iter()
        .map(|w| {
            if w == "/run/pot/secrets.fifo" {
                f.display().to_string()
            } else {
                w.clone()
            }
        })
        .collect();
    for (name, stream, ran) in [
        ("complete", &b"TERM\0COLORTERM\0NOTES_TOKEN=v\0.\0"[..], true),
        ("no sentinel", &b"TERM\0COLORTERM\0NOTES_TOKEN=v\0"[..], false),
        ("short", &b"TERM\0COLORTERM\0NOTES_TOKEN=v"[..], false),
        ("empty", &b""[..], false),
        ("a name short", &b"TERM\0NOTES_TOKEN=v\0.\0"[..], false),
        // V1: a name that is not planned, a planned one twice, and a sentinel before the planned names
        (
            "not planned",
            &b"TERM\0COLORTERM\0NOTES_TOKEN=v\0LD_PRELOAD=/x.so\0.\0"[..],
            false,
        ),
        (
            "not planned first",
            &b"LD_PRELOAD=/x.so\0TERM\0COLORTERM\0NOTES_TOKEN=v\0.\0"[..],
            false,
        ),
        (
            "a repeat",
            &b"TERM\0COLORTERM\0NOTES_TOKEN=v\0NOTES_TOKEN=w\0.\0"[..],
            false,
        ),
        (
            "an early sentinel",
            &b".\0TERM\0COLORTERM\0NOTES_TOKEN=v\0.\0"[..],
            false,
        ),
        (
            "an early sentinel after one",
            &b"TERM\0.\0COLORTERM\0NOTES_TOKEN=v\0.\0"[..],
            false,
        ),
    ] {
        fs::write(&f, stream).unwrap();
        let out = Command::new(&entry[0])
            .args(&entry[1..])
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
        "PIPESTATUS",
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
                if words.get(1) == Some(&&b"--noprofile"[..]) && words.contains(&&b"pot-entry"[..]) {
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
                            || (!c.windows(9).any(|w| w == b"pot-entry") && !c.starts_with(b"/usr/bin/env"))
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
    let stream = top.join("stream");
    let entry: Vec<String> = words[at..]
        .iter()
        .map(|w| match w.as_str() {
            "/usr/bin/env" => rec.display().to_string(),
            "/run/pot/secrets.fifo" => stream.display().to_string(),
            _ => w.clone(),
        })
        .collect();
    fs::write(&stream, b"TERM\0COLORTERM\0NOTES_TOKEN=t1-value\0.\0").unwrap();
    let out = Command::new(&entry[0])
        .args(&entry[1..])
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
        "EPOCHREALTIME",
        "BASHPID",
        "DIRSTACK",
        "FUNCNAME",
        "OPTIND",
        "REPLY",
        "COLUMNS",
        "LINES",
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

/// A prepared run folder whose env items are ITEMS (name, secret), in place of the fixture's, running `env -0`.
fn item_world(name: &str, engine: &str, program: &str, items: &[(&str, bool)]) -> (PathBuf, PathBuf) {
    let envp = coreutils("env");
    let (top, run) = world(name, engine, program, &[&envp, "-0"]);
    let path = run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let list = p["items"].as_array_mut().unwrap();
    let at = list.iter().position(|i| i["kind"] == "env").unwrap();
    let template = list.remove(at);
    for (n, secret) in items {
        let mut it = template.clone();
        it["value"] = (*n).into();
        it["secret"] = (*secret).into();
        list.push(it);
    }
    fs::write(&path, p.to_string()).unwrap();
    prepare(&top, &run);
    (top, run)
}

fn env_lines(out: &Output, names: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .filter(|w| names.iter().any(|n| w.starts_with(&format!("{n}="))))
        .collect();
    v.sort();
    v
}

/// U2: an env item named like one of pot's values (`TZ` from env.set, `LANG` fixed) takes the host's value when the
/// host has it set, and leaves pot's value when it has not, as bwrap's `run_env` (its items come last); on the
/// secret path and the plain one.
#[test]
fn podman_an_item_set_on_the_host_wins_over_pots_value() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    for secret in [true, false] {
        for host_set in [true, false] {
            let (top, run) = item_world(
                "podman-item-wins",
                "podman",
                &program,
                &[("TZ", secret), ("LANG", false)],
            );
            let mut c = verb(&top, &run, "run");
            if host_set {
                c.env("TZ", "Europe/Paris").env("LANG", "de_DE.UTF-8");
            } else {
                c.env_remove("TZ").env_remove("LANG");
            }
            let out = c.stdin(Stdio::null()).output().unwrap();
            ok(&out);
            let want = if host_set {
                ["LANG=de_DE.UTF-8", "TZ=Europe/Paris"]
            } else {
                ["LANG=C.UTF-8", "TZ=America/Chicago"]
            };
            assert_eq!(
                env_lines(&out, &["TZ", "LANG"]),
                want,
                "secret {secret}, set on the host {host_set}"
            );
            ok(&verb(&top, &run, "cleanup").output().unwrap());
        }
    }
}

/// U2 under docker (the plain path): the same rule.
#[test]
fn docker_an_item_set_on_the_host_wins_over_pots_value() {
    let Ok(program) = std::env::var("POT_TEST_DOCKER") else {
        eprintln!("POT_TEST_DOCKER is not set: no container started");
        return;
    };
    for host_set in [true, false] {
        let (top, run) = item_world(
            "docker-item-wins",
            "docker",
            &program,
            &[("TZ", false), ("LANG", false)],
        );
        let mut c = verb(&top, &run, "run");
        if host_set {
            c.env("TZ", "Europe/Paris").env("LANG", "de_DE.UTF-8");
        } else {
            c.env_remove("TZ").env_remove("LANG");
        }
        let out = c.stdin(Stdio::null()).output().unwrap();
        ok(&out);
        let want = if host_set {
            ["LANG=de_DE.UTF-8", "TZ=Europe/Paris"]
        } else {
            ["LANG=C.UTF-8", "TZ=America/Chicago"]
        };
        assert_eq!(
            env_lines(&out, &["TZ", "LANG"]),
            want,
            "set on the host {host_set}"
        );
        ok(&verb(&top, &run, "cleanup").output().unwrap());
    }
}

// ---------------------------------------------------------------- the named pipe under both engines, and the network

/// The engine program a test runs under ENGINE, or None (the variable unset: no container started).
fn engine_program(engine: &str) -> Option<String> {
    let var = if engine == "podman" {
        "POT_TEST_PODMAN"
    } else {
        "POT_TEST_DOCKER"
    };
    let p = std::env::var(var).ok();
    if p.is_none() {
        eprintln!("{var} is not set: no {engine} container started");
    }
    p
}

/// The words of `container.argv`.
fn argv_words(run: &Path) -> Vec<String> {
    let argv = fs::read(run.join("container.argv")).unwrap();
    argv.strip_suffix(&[0])
        .unwrap()
        .split(|b| *b == 0)
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .collect()
}

/// Whether any file under DIR (never through a link) holds NEEDLE; the files that do.
fn files_holding(dir: &Path, needle: &str) -> Vec<String> {
    let mut found = vec![];
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(m) = e.path().symlink_metadata() else {
                continue;
            };
            if m.is_dir() {
                todo.push(e.path());
            } else if m.is_file()
                && fs::read(e.path()).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes()))
            {
                found.push(e.path().display().to_string());
            }
        }
    }
    found
}

/// A run folder with the network: the policy's network reach items ALLOW (and private-proxy proved), curl's closure
/// in the run's, and the host's CA bundle as the run's. None where the config has no network programs.
fn net_world(
    name: &str,
    engine: &str,
    program: &str,
    argv: &[&str],
    allow: &[&str],
) -> Option<(PathBuf, PathBuf)> {
    let (top, run) = world(name, engine, program, argv);
    let cfg_path = top.join("config.json");
    let mut cfg: serde_json::Value = serde_json::from_str(&fs::read_to_string(&cfg_path).unwrap()).unwrap();
    if cfg.get("socat").is_none() {
        eprintln!("no python3 or socat on PATH: no network run");
        return None;
    }
    cfg["cacert"] = fs::canonicalize("/etc/ssl/certs/ca-certificates.crt")
        .unwrap()
        .display()
        .to_string()
        .into();
    fs::write(&cfg_path, cfg.to_string()).unwrap();
    let path = run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    for a in allow {
        p["items"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "kind": "network", "value": a,
            "class": "reach", "sources": [ { "flag": "--net" } ], "effective": true,
            "request": format!("network {a}"), "requestClass": "ordinary" }));
    }
    p["backend"]["proved"]
        .as_array_mut()
        .unwrap()
        .push("private-proxy".into());
    fs::write(&path, p.to_string()).unwrap();
    let curl = which("curl");
    let lpath = run.join("launch.json");
    let mut l: serde_json::Value = serde_json::from_str(&fs::read_to_string(&lpath).unwrap()).unwrap();
    let closure: Vec<String> = String::from_utf8(closure_of(&[&curl]))
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    l["closure"] = closure.into();
    fs::write(&lpath, l.to_string()).unwrap();
    Some((top, run))
}

/// The pids of pot's proxy for RUN (its socket proxy.sock and the run's denials).
fn proxies(run: &Path) -> Vec<String> {
    let denials = format!("{}/denials", fs::canonicalize(run).unwrap().display());
    let mut out = vec![];
    for e in fs::read_dir("/proc").unwrap().flatten() {
        if let Ok(c) = fs::read(e.path().join("cmdline")) {
            let words: Vec<&[u8]> = c.split(|b| *b == 0).collect();
            if words.contains(&&b"proxy.sock"[..]) && words.contains(&denials.as_bytes()) {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out
}

/// The network under ENGINE: an allowed host is reached over HTTPS through pot's proxy; a host not allowed is refused
/// (403) and recorded in the run's denials and the after event; a connection that bypasses the proxy has no route
/// (only `lo`); the host's loopback is never reached, through the proxy (LOOPBACK, in the after event) or directly
/// (the run's own loopback); the proxy and the run's socket and named pipe are gone when run returns.
fn network_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let host_lo = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    host_lo.set_nonblocking(true).unwrap();
    let port = host_lo.local_addr().unwrap().port();
    let curl = which("curl").display().to_string();
    let script = format!(
        r#"c={curl}
$c -sS -o /dev/null --max-time 20 -w 'allowed=%{{http_code}}\n' https://example.com/
$c -sS -o /dev/null --max-time 20 -w 'denied=%{{http_connect}}\n' https://example.org/ 2>/dev/null
$c --noproxy '*' -sS -o /dev/null --max-time 5 https://example.com/ 2>/dev/null; echo "direct=$?"
$c --noproxy '' -sS -o /dev/null --max-time 5 -w 'loopback=%{{http_code}}\n' http://localhost:{port}/
$c --noproxy '*' -sS -o /dev/null --max-time 5 http://127.0.0.1:{port}/ 2>/dev/null; echo "hostlo=$?"
while read -r l; do case $l in *:*) echo "if=${{l%%:*}}" ;; esac; done </proc/net/dev
echo "proxy=$HTTPS_PROXY"
"#
    );
    let Some((top, run)) = net_world(
        &format!("{engine}-net"),
        engine,
        &program,
        &["/bin/sh", "-c", &script],
        &["example.com:443", &format!("localhost:{port}")],
    ) else {
        return;
    };
    prepare(&top, &run);
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}{err}");
    let lines: Vec<&str> = said.lines().collect();
    assert!(lines.contains(&"allowed=200"), "{said}{err}");
    assert!(lines.contains(&"denied=403"), "{said}");
    assert!(
        !lines.contains(&"direct=0") && lines.iter().any(|l| l.starts_with("direct=")),
        "{said}"
    );
    assert!(lines.contains(&"loopback=403"), "{said}");
    assert!(
        !lines.contains(&"hostlo=0") && lines.iter().any(|l| l.starts_with("hostlo=")),
        "{said}"
    );
    let ifs: Vec<&&str> = lines.iter().filter(|l| l.starts_with("if=")).collect();
    assert_eq!(ifs, [&"if=lo"], "only lo: {said}");
    assert!(lines.contains(&"proxy=http://127.0.0.1:3128"), "{said}");
    // the host's listener was never reached
    assert!(
        host_lo
            .accept()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
    );
    // the run's own record of the refusal, and the after event's
    let denials = fs::read_to_string(run.join("denials")).unwrap();
    assert!(
        denials.contains("the run tried example.org:443 (not in this run)"),
        "{denials}"
    );
    assert!(
        denials.contains(&format!("localhost:{port}, the host's loopback")),
        "{denials}"
    );
    let after = verb(&top, &run, "after").output().unwrap();
    ok(&after);
    let ev: serde_json::Value = serde_json::from_slice(&after.stdout).unwrap();
    assert_eq!(
        ev["denials"],
        serde_json::json!([{ "host": "example.org:443", "count": 1 }])
    );
    let lo = ev["loopback"].as_array().unwrap();
    assert_eq!(lo.len(), 1, "{ev}");
    assert_eq!(lo[0]["host"], format!("localhost:{port}"));
    assert!(
        ["127.0.0.1", "::1"].contains(&lo[0]["address"].as_str().unwrap()),
        "{ev}"
    );
    // run stopped the proxy and removed its socket and the named pipe
    assert_eq!(proxies(&run), Vec::<String>::new());
    assert!(run.join("proxy.sock").symlink_metadata().is_err());
    assert!(run.join("secrets.fifo").symlink_metadata().is_err());
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    let mut left = set(LEFT);
    left.extend(["status".to_string(), "proxy.log".to_string()]);
    assert_eq!(names(&run), left);
}

/// L1 (decided 2026-11-23): no run creates a user namespace, under either engine (podman's own default profile allows
/// one): `unshare -U` is refused, and so is `clone` with `CLONE_NEWUSER`; `clone3` is ENOSYS. A fork and a thread (libc
/// tries `clone3` first, then `clone`) still work.
fn userns_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let (Some(unshare), Some(python)) = (find("unshare"), find("python3")) else {
        eprintln!("no unshare or python3 on PATH: no container started");
        return;
    };
    let script = format!(
        "{u} -U {t} 2>/dev/null && echo unshare-userns || echo unshare-refused; \
         {t} & wait $! && echo forked; \
         {py} -I -S -c '{code}'",
        u = unshare.display(),
        t = coreutils("true"),
        py = python.display(),
        code = "import ctypes, errno, os, platform, threading\n\
                libc = ctypes.CDLL(None, use_errno=True)\n\
                nr = {\"x86_64\": (56, 435), \"aarch64\": (220, 435)}[platform.machine()]\n\
                def call(n, *a):\n\
                \x20   r = libc.syscall(n, *a)\n\
                \x20   if r == 0: os._exit(0)\n\
                \x20   if r > 0: os.waitpid(r, 0); return \"ok\"\n\
                \x20   return errno.errorcode[ctypes.get_errno()]\n\
                print(\"clone-newuser\", call(nr[0], 0x10000000 | 17, 0, 0, 0, 0))\n\
                print(\"clone3\", call(nr[1], 0, 0))\n\
                t = threading.Thread(target=lambda: print(\"thread\")); t.start(); t.join()"
    );
    let (top, run) = world(
        &format!("{engine}-userns"),
        engine,
        &program,
        &["/bin/sh", "-c", &script],
    );
    // unshare's and python's store paths in the run's closure
    let base = top.join("store/base.txt");
    let mut closure = fs::read(&base).unwrap();
    closure.extend(closure_of(&[&unshare, &python]));
    fs::write(&base, closure).unwrap();
    prepare(&top, &run);
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    ok(&out);
    let got = String::from_utf8_lossy(&out.stdout);
    for want in [
        "unshare-refused",
        "forked",
        "clone-newuser EPERM",
        "clone3 ENOSYS",
        "thread",
    ] {
        assert!(got.lines().any(|l| l == want), "{engine}: no {want:?} in\n{got}");
    }
    assert!(
        argv_words(&run).iter().any(|w| w.starts_with("seccomp=")),
        "{:?}",
        argv_words(&run)
    );
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

#[test]
fn podman_no_run_creates_a_user_namespace() {
    userns_case("podman");
}

#[test]
fn docker_no_run_creates_a_user_namespace() {
    userns_case("docker");
}

#[test]
fn podman_network_reaches_only_the_allowlist_through_pots_proxy() {
    network_case("podman");
}

#[test]
fn docker_network_reaches_only_the_allowlist_through_pots_proxy() {
    network_case("docker");
}

/// The proxy stops when run is killed with SIGKILL (its parent is gone: `proxy.py`'s `orphaned`), and cleanup removes the
/// container, the socket and the named pipe.
fn killed_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let sleep = coreutils("sleep");
    let Some((top, run)) = net_world(
        &format!("{engine}-net-killed"),
        engine,
        &program,
        &[&sleep, "60"],
        &["example.com:443"],
    ) else {
        return;
    };
    prepare(&top, &run);
    let mut child = verb(&top, &run, "run").stdin(Stdio::null()).spawn().unwrap();
    wait_for("the container", || {
        run.join("container.id").exists() && !proxies(&run).is_empty()
    });
    std::thread::sleep(Duration::from_millis(500));
    let pids = proxies(&run);
    assert_eq!(pids.len(), 1, "{pids:?}");
    child.kill().unwrap();
    child.wait().unwrap();
    let killed = Instant::now();
    wait_for("the proxy to stop itself", || proxies(&run).is_empty());
    assert!(killed.elapsed() < Duration::from_secs(5));
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    let mut left = set(LEFT);
    left.insert("proxy.log".into());
    assert_eq!(names(&run), left);
    if engine == "podman" {
        assert_eq!(engine_processes(&run), Vec::<String>::new());
    }
}

#[test]
fn podman_proxy_stops_when_run_is_killed() {
    killed_case("podman");
}

#[test]
fn docker_proxy_stops_when_run_is_killed() {
    killed_case("docker");
}

/// cleanup stops a live proxy of the run's (here one this test started, so it is not orphaned) and removes its
/// socket. No engine is needed.
#[test]
fn cleanup_stops_a_live_proxy_and_removes_its_socket() {
    let Some((top, run)) = net_world(
        "net-cleanup",
        "podman",
        "podman",
        &["/bin/sh", "-c", "true"],
        &["example.com:443"],
    ) else {
        return;
    };
    let curl = find("curl");
    if curl.is_none() || find("nix-store").is_none() {
        return;
    }
    prepare(&top, &run);
    let cenv: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("container.env")).unwrap()).unwrap();
    let argv: Vec<String> = cenv["proxy"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    let mut proxy = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&run)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the proxy's socket", || run.join("proxy.sock").exists());
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    let start = Instant::now();
    let status = loop {
        if let Some(s) = proxy.try_wait().unwrap() {
            break s;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "the proxy still runs");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success() || status.code().is_some());
    assert!(run.join("proxy.sock").symlink_metadata().is_err());
    assert_eq!(names(&run), set(LEFT));
}

/// The decision of 2026-11-22: a secret under docker too, over the named pipe. It arrives exactly; the marker is in no
/// file of the run folder, no engine argument and not in docker's configuration of the container; the container's
/// user (the run's uid, no user namespace) can open the pipe; and the pipe is gone when run returns. A link planted at
/// the pipe's path is removed, never written through.
#[test]
fn docker_secrets_arrive_exactly_and_nowhere_else() {
    secret_case("docker");
}

#[test]
fn podman_secrets_arrive_exactly_and_nowhere_else() {
    secret_case("podman");
}

fn secret_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let marker = "pot-fifo-marker-5d2c8e";
    let sleep = coreutils("sleep");
    let script = format!("printf '[%s]\\n' \"$NOTES_TOKEN\"; {sleep} 2");
    let (top, run) = world(
        &format!("{engine}-fifo"),
        engine,
        &program,
        &["/bin/sh", "-c", &script],
    );
    prepare(&top, &run);
    // a link at the pipe's path, to a file outside the run folder
    let outside = top.join("outside");
    fs::write(&outside, "").unwrap();
    symlink(&outside, run.join("secrets.fifo")).unwrap();
    let child = verb(&top, &run, "run")
        .env("NOTES_TOKEN", marker)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("the container", || {
        run.join("container.id").exists()
            && !fs::read_to_string(run.join("container.id"))
                .unwrap_or_default()
                .is_empty()
    });
    std::thread::sleep(Duration::from_millis(700));
    let cid = fs::read_to_string(run.join("container.id")).unwrap();
    // the engine's own record of the container (docker inspect; podman's in the run's store, checked below)
    let inspect = if engine == "docker" {
        Command::new(&program)
            .args(["container", "inspect", cid.trim()])
            .output()
            .unwrap()
    } else {
        podman(&program, &run, &["container", "inspect", cid.trim()])
    };
    ok(&inspect);
    assert!(
        !String::from_utf8_lossy(&inspect.stdout).contains(marker),
        "the marker is in the engine's config"
    );
    let mut found = files_holding(&run, marker);
    let xdg = PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap()).join("crun");
    found.extend(files_holding(&xdg, marker));
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("[{marker}]\n"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(found, Vec::<String>::new(), "the secret's value is there");
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "",
        "written through the planted link"
    );
    assert!(
        run.join("secrets.fifo").symlink_metadata().is_err(),
        "the named pipe is left"
    );
    assert!(!argv_words(&run).iter().any(|w| w.contains(marker)));
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// No exec on the host is ever handed a value (strace follows every exec of run and its children: the engine, conmon,
/// the OCI runtime and, under podman, the run's own processes), while the command has the value.
fn strace_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some(strace) = find("strace") else {
        eprintln!("no strace on PATH");
        return;
    };
    let marker = "pot-strace-marker-9a41f0";
    let (top, run) = world(
        &format!("{engine}-strace"),
        engine,
        &program,
        &["/bin/sh", "-c", "echo \"[$NOTES_TOKEN]\""],
    );
    prepare(&top, &run);
    let log = top.join("strace.log");
    let out = Command::new(&strace)
        .args(["-f", "-qq", "-e", "trace=execve,execveat", "-s", "1000000", "-o"])
        .arg(&log)
        .arg(BIN)
        .arg("--config")
        .arg(top.join("config.json"))
        .arg("run")
        .arg(&run)
        .env("NOTES_TOKEN", marker)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("[{marker}]\n"));
    let execs = fs::read_to_string(&log).unwrap();
    // the trace saw the engine (and, under podman, the run's bash and its last hop)
    assert!(execs.contains("execve("), "{execs}");
    if engine == "podman" {
        assert!(
            execs.contains("pot-entry"),
            "strace did not follow into the container"
        );
    }
    let bad: Vec<&str> = execs.lines().filter(|l| l.contains(marker)).collect();
    assert!(bad.is_empty(), "a value in an exec's arguments: {bad:?}");
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

#[test]
fn podman_no_exec_is_handed_a_value() {
    strace_case("podman");
}

#[test]
fn docker_no_exec_is_handed_a_value() {
    strace_case("docker");
}

/// A container that ends before it opens the named pipe: run does not hang, and returns its status.
#[test]
fn podman_run_returns_when_the_container_never_reads_the_pipe() {
    let Some(program) = engine_program("podman") else {
        return;
    };
    let (top, run) = world("podman-noread", "podman", &program, &["/bin/sh", "-c", "true"]);
    prepare(&top, &run);
    // the entry's script replaced by one that exits at once (container.argv is the run's own file)
    let words: Vec<String> = argv_words(&run)
        .into_iter()
        .map(|w| {
            if w.starts_with("__pot_fifo=") {
                "exit 7".to_string()
            } else {
                w
            }
        })
        .collect();
    let mut bytes = vec![];
    for w in &words {
        bytes.extend_from_slice(w.as_bytes());
        bytes.push(0);
    }
    fs::write(run.join("container.argv"), bytes).unwrap();
    let start = Instant::now();
    let out = verb(&top, &run, "run")
        .env("NOTES_TOKEN", "x")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(start.elapsed() < Duration::from_secs(20), "run hung");
    assert!(run.join("secrets.fifo").symlink_metadata().is_err());
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// The command's environment with the network is exactly bwrap's `run_env`: pot's values, the proxy's variables, the
/// items; nothing the engine, bash or socat adds. Under both engines.
fn net_env_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let envp = coreutils("env");
    let Some((top, run)) = net_world(
        &format!("{engine}-net-env"),
        engine,
        &program,
        &[&envp, "-0"],
        &["example.com:443"],
    ) else {
        return;
    };
    prepare(&top, &run);
    let out = verb(&top, &run, "run")
        .env("NOTES_TOKEN", "net-value")
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
    let words = argv_words(&run);
    let mut want: BTreeSet<String> = words
        .windows(2)
        .filter(|w| w[0] == "--env" && w[1].contains('='))
        .map(|w| w[1].clone())
        .collect();
    want.insert("NOTES_TOKEN=net-value".into());
    want.insert("TERM=xterm".into());
    for v in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        assert!(want.contains(&format!("{v}=http://127.0.0.1:3128")), "{v}");
    }
    assert!(
        want.contains("NO_PROXY=localhost,127.0.0.1,::1")
            && want.contains("no_proxy=localhost,127.0.0.1,::1")
    );
    assert_eq!(got, want);
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

#[test]
fn podman_the_commands_environment_with_the_network_is_the_plan() {
    net_env_case("podman");
}

#[test]
fn docker_the_commands_environment_with_the_network_is_the_plan() {
    net_env_case("docker");
}

// ---------------------------------------------------------------- the QA round's findings V1-V3

/// V1: a host process of the same user that opens the named pipe before run, and writes records of its own (a name
/// that is not planned, then the sentinel), never widens the run: the entry refuses (125, nothing runs), or, when
/// run's records came first, the injected ones after the sentinel are never read. Under both engines.
fn injected_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let envp = coreutils("env");
    let (top, run) = world(&format!("{engine}-inject"), engine, &program, &[&envp, "-0"]);
    prepare(&top, &run);
    let fifo = run.join("secrets.fifo");
    let injector = std::thread::spawn(move || {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            // O_NONBLOCK: ENXIO until a reader has it open
            if let Ok(mut f) = fs::OpenOptions::new()
                .write(true)
                .custom_flags(0o4000)
                .open(&fifo)
            {
                let _ = f.write_all(b"LD_PRELOAD=/nonexistent-qa.so\0QA_INJECTED=yes\0.\0");
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    });
    let out = verb(&top, &run, "run")
        .env("NOTES_TOKEN", "inject-real")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    // the injector may lose the race: run delivers, then unlinks the pipe (V3) before it opens; the refusal itself is
    // tested deterministically by the host-run script test
    let _ = injector.join().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        !said.contains("QA_INJECTED") && !said.contains("LD_PRELOAD"),
        "an injected record reached the command: {said}"
    );
    eprintln!("{engine}: the injected stream gave exit {:?}", out.status.code());
    match out.status.code() {
        Some(125) => assert_eq!(said, "", "something ran"),
        Some(0) => assert!(said.contains("NOTES_TOKEN=inject-real"), "{said}"),
        other => panic!("exit {other:?}: {}", String::from_utf8_lossy(&out.stderr)),
    }
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

#[test]
fn podman_an_injected_record_never_widens_the_run() {
    injected_case("podman");
}

#[test]
fn docker_an_injected_record_never_widens_the_run() {
    injected_case("docker");
}

/// V2: probe proves private-proxy only where prepare carries the network out: the same test (socat a real file, not a
/// link, under a path of `netClosure`; python and proxy files). A socat that is a link, one outside `netClosure`, or
/// no `netClosure`: neither probe nor prepare.
#[test]
fn podman_probe_and_prepare_agree_on_private_proxy() {
    let Some(program) = engine_program("podman") else {
        return;
    };
    let Some((top, run)) = net_world(
        "podman-agree",
        "podman",
        &program,
        &["/bin/sh", "-c", "true"],
        &["example.com:443"],
    ) else {
        return;
    };
    let cfg_path = top.join("config.json");
    let good: serde_json::Value = serde_json::from_str(&fs::read_to_string(&cfg_path).unwrap()).unwrap();
    let link = top.join("socat-link");
    symlink(good["socat"].as_str().unwrap(), &link).unwrap();
    let outside = top.join("store").join("socat");
    fs::copy(good["socat"].as_str().unwrap(), &outside).unwrap();
    let mut no_closure = good.clone();
    no_closure.as_object_mut().unwrap().remove("netClosure");
    let mut linked = good.clone();
    linked["socat"] = link.display().to_string().into();
    let mut unbound = good.clone();
    unbound["socat"] = outside.display().to_string().into();
    for (name, cfg, proves) in [
        ("good", &good, true),
        ("a link", &linked, false),
        ("outside netClosure", &unbound, false),
        ("no netClosure", &no_closure, false),
    ] {
        fs::write(&cfg_path, cfg.to_string()).unwrap();
        let probe = verb(&top, &run, "probe").output().unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let has = doc["guarantees"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g == "private-proxy");
        assert_eq!(has, proves, "{name}: probe {doc}");
        let prep = verb(&top, &run, "prepare").output().unwrap();
        assert_eq!(
            prep.status.success(),
            proves,
            "{name}: prepare {}",
            String::from_utf8_lossy(&prep.stderr)
        );
    }
    fs::write(&cfg_path, good.to_string()).unwrap();
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// V3: the named pipe leaves the run folder as soon as run has delivered the records (the container's bind keeps its
/// own inode): while the command still runs, `secrets.fifo` is gone. Under both engines.
fn unlinked_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let sleep = coreutils("sleep");
    let script = format!("echo \"[$NOTES_TOKEN]\"; {sleep} 4");
    let (top, run) = world(
        &format!("{engine}-unlink"),
        engine,
        &program,
        &["/bin/sh", "-c", &script],
    );
    prepare(&top, &run);
    let mut child = verb(&top, &run, "run")
        .env("NOTES_TOKEN", "v3-value")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("the named pipe", || {
        run.join("secrets.fifo").symlink_metadata().is_ok()
    });
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.as_mut().unwrap()),
        &mut line,
    )
    .unwrap();
    assert_eq!(line, "[v3-value]\n");
    let start = Instant::now();
    while run.join("secrets.fifo").symlink_metadata().is_ok() {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "the named pipe is still in the run folder"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        child.try_wait().unwrap().is_none(),
        "the command had already ended"
    );
    assert_eq!(child.wait().unwrap().code(), Some(0));
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

#[test]
fn podman_the_named_pipe_is_unlinked_once_delivered() {
    unlinked_case("podman");
}

#[test]
fn docker_the_named_pipe_is_unlinked_once_delivered() {
    unlinked_case("docker");
}

// ---------------------------------------------------------------- the credential and mcp bindings

/// A fake key, a marker no other file holds: each `from` kind, in each `via` mode.
const KEYS: &[(&str, &str, &str)] = &[
    // (variable, via, from kind)
    ("A_TOKFILE", "env", "file"),
    ("B_ENVFILE", "env", "envFile"),
    ("C_FROMENV", "env", "env"),
    ("D_SAME", "env", "none"),
    ("E_TOKFILE", "file", "file"),
    ("F_ENVFILE", "file", "envFile"),
    ("G_FROMENV", "file", "env"),
    ("H_SAME", "file", "none"),
];

fn marker(name: &str) -> String {
    format!("pot-bind-marker-{}-7c1e", name.to_lowercase())
}

/// The host's variables a bindings run is given: the `env` and `none` kinds' values, and the mcp keys' none.
fn bind_env() -> Vec<(String, String)> {
    let mut v = vec![];
    for (n, _, kind) in KEYS {
        match *kind {
            "env" => v.push((format!("POT_TEST_HOST_{n}"), marker(n))),
            "none" => v.push(((*n).to_string(), marker(n))),
            _ => {}
        }
    }
    v
}

/// A run folder whose policy has a credential binding for each of KEYS, and two mcp bindings (MCPKEY via env, MCPFILE
/// via file), their fake key files in the test's own `keys/`; ARGV the command.
fn bind_world(name: &str, engine: &str, program: &str, argv: &[&str]) -> (PathBuf, PathBuf) {
    let (top, run) = world(name, engine, program, argv);
    let keys = top.join("keys");
    fs::create_dir_all(&keys).unwrap();
    let mut bindings = vec![];
    for (n, via, kind) in KEYS {
        let from = match *kind {
            "file" => {
                // a token file: the whole file is the value
                fs::write(keys.join(n), format!("{}\n", marker(n))).unwrap();
                serde_json::json!({ "file": keys.join(n) })
            }
            "envFile" => {
                fs::write(
                    keys.join(n),
                    format!("# fake\nexport {n}=\"{}\"\nOTHER=x\n", marker(n)),
                )
                .unwrap();
                serde_json::json!({ "envFile": keys.join(n) })
            }
            "env" => serde_json::json!({ "env": format!("POT_TEST_HOST_{n}") }),
            _ => serde_json::json!({}),
        };
        bindings.push(serde_json::json!({ "capability": "credential", "selector": format!("sel-{n}"),
            "kit": "root:test", "request": format!("capability credential sel-{n} env={n}"), "via": via, "env": [n],
            "from": from, "holds": "the run holds it" }));
    }
    for (n, via, server) in [("MCPKEY", "env", "s"), ("MCPFILE", "file", "t")] {
        fs::write(keys.join(n), format!("{n}={}\n", marker(n))).unwrap();
        bindings.push(serde_json::json!({ "capability": "mcp", "selector": server, "kit": "root:test",
            "request": format!("capability mcp {server}"), "via": via, "env": [n], "from": { "file": keys.join(n) },
            "holds": "the run holds it", "server": server, "url": format!("https://{server}.example/mcp"),
            "header": "Authorization", "scheme": "Bearer", "config": "/run/pot/mcp.json" }));
    }
    let path = run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    p["bindings"] = bindings.into();
    fs::write(&path, p.to_string()).unwrap();
    (top, run)
}

/// Every variable of a bindings run, with the markers: the via-file ones hold their file's path in the run.
fn bind_names() -> Vec<&'static str> {
    KEYS.iter().map(|k| k.0).chain(["MCPKEY", "MCPFILE"]).collect()
}

fn via_file(n: &str) -> bool {
    n == "MCPFILE" || KEYS.iter().any(|k| k.0 == n && k.1 == "file")
}

/// Under ENGINE: each `from` kind's value arrives, as the variable (via env) or as a 0400 file of the run's user at
/// /run/pot/secrets/NAME, which the run can read and not write, remove or replace (via file); mcp.json is bwrap's,
/// with no value; no marker is in a file of the run folder (but the via-file one's own, in secrets/) or of
/// `$XDG_RUNTIME_DIR/crun`, in the engine's `inspect`, or in `container.argv`; and the secrets are gone when the
/// command ends.
fn bindings_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let (cat, stat, chmod, rm, sleep) = (
        coreutils("cat"),
        coreutils("stat"),
        coreutils("chmod"),
        coreutils("rm"),
        coreutils("sleep"),
    );
    let mut script = String::new();
    for n in bind_names() {
        let _ = write!(script, "printf 'V %s=%s\\n' {n} \"${n}\"; ");
    }
    let _ = write!(
        script,
        "for f in /run/pot/secrets/*; do printf 'F %s=%s\\n' \"$f\" \"$({cat} \"$f\")\"; {stat} -c 'S %n %a %u' \"$f\"; done; \
         (printf x > /run/pot/secrets/E_TOKFILE) 2>/dev/null && echo WRITABLE; \
         (: > /run/pot/secrets/NEW) 2>/dev/null && echo CREATED; \
         {chmod} 600 /run/pot/secrets/E_TOKFILE 2>/dev/null && echo CHMODDED; \
         {rm} -f /run/pot/secrets/E_TOKFILE 2>/dev/null; [ -e /run/pot/secrets/E_TOKFILE ] || echo REMOVED; \
         printf 'M '; {cat} /run/pot/mcp.json; echo; {sleep} 3"
    );
    let (top, run) = bind_world(
        &format!("{engine}-bindings"),
        engine,
        &program,
        &["/bin/sh", "-c", &script],
    );
    prepare(&top, &run);
    let child = verb(&top, &run, "run")
        .envs(bind_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for("the container", || {
        !fs::read_to_string(run.join("container.id"))
            .unwrap_or_default()
            .is_empty()
    });
    std::thread::sleep(Duration::from_millis(1500));
    let cid = fs::read_to_string(run.join("container.id")).unwrap();
    // on the host, while the command runs: each via-file value in its own file, 0400, the caller's; nowhere else
    let me = id("Uid:");
    for n in bind_names().into_iter().filter(|n| via_file(n)) {
        use std::os::unix::fs::MetadataExt as _;
        let m = run
            .join("secrets")
            .join(n)
            .symlink_metadata()
            .unwrap_or_else(|e| panic!("{n}: {e}"));
        assert!(
            m.file_type().is_file() && m.mode() & 0o7777 == 0o400 && u64::from(m.uid()) == me,
            "{n}: {m:?}"
        );
    }
    let inspect = if engine == "docker" {
        Command::new(&program)
            .args(["container", "inspect", cid.trim()])
            .output()
            .unwrap()
    } else {
        podman(&program, &run, &["container", "inspect", cid.trim()])
    };
    ok(&inspect);
    let xdg = PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap()).join("crun");
    let words = argv_words(&run);
    for n in bind_names() {
        let mk = marker(n);
        let mut found = files_holding(&run, &mk);
        found.extend(files_holding(&xdg, &mk));
        let own = run.join("secrets").join(n).display().to_string();
        let want: Vec<String> = if via_file(n) { vec![own] } else { vec![] };
        // the keys/ folder holds the fake key files; it is outside the run folder
        assert_eq!(found, want, "{n}'s value is in a file");
        assert!(
            !String::from_utf8_lossy(&inspect.stdout).contains(&mk),
            "{n} in inspect"
        );
        assert!(!words.iter().any(|w| w.contains(&mk)), "{n} in container.argv");
    }
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for n in bind_names() {
        let want = if via_file(n) {
            format!("V {n}=/run/pot/secrets/{n}")
        } else {
            format!("V {n}={}", marker(n))
        };
        assert!(stdout.lines().any(|l| l == want), "{want}\n{stdout}");
        if via_file(n) {
            assert!(
                stdout
                    .lines()
                    .any(|l| l == format!("F /run/pot/secrets/{n}={}", marker(n))),
                "{n}\n{stdout}"
            );
            assert!(
                stdout
                    .lines()
                    .any(|l| l == format!("S /run/pot/secrets/{n} 400 {me}")),
                "{n}\n{stdout}"
            );
        }
    }
    for bad in ["WRITABLE", "CREATED", "CHMODDED", "REMOVED"] {
        assert!(!stdout.lines().any(|l| l == bad), "the run could: {bad}");
    }
    // mcp.json: bwrap's shape, a header naming its variable, a helper reading the file; no value
    let mcp = stdout
        .split("\nM ")
        .nth(1)
        .unwrap()
        .split("\n\n")
        .next()
        .unwrap_or_default();
    let mcp: serde_json::Value = serde_json::from_str(mcp.trim()).unwrap_or_else(|e| panic!("{e}: {stdout}"));
    assert_eq!(
        mcp,
        serde_json::json!({ "mcpServers": {
            "s": { "type": "http", "url": "https://s.example/mcp", "headers": { "Authorization": "Bearer ${MCPKEY}" } },
            "t": { "type": "http", "url": "https://t.example/mcp",
                   "headersHelper": "read -r v < \"$MCPFILE\"; printf '%s%s%s' '{\"Authorization\": \"Bearer ' \"$v\" '\"}'" } } })
    );
    // gone when the command ends
    assert!(
        run.join("secrets").symlink_metadata().is_err(),
        "secrets/ is left"
    );
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    let mut left = set(LEFT);
    left.insert("status".into());
    assert_eq!(names(&run), left);
}

#[test]
fn podman_bindings_arrive_from_every_source_and_nowhere_else() {
    bindings_case("podman");
}

#[test]
fn docker_bindings_arrive_from_every_source_and_nowhere_else() {
    bindings_case("docker");
}

/// No exec (strace follows run, the engine and, under podman, the container's processes) is handed a binding's value.
fn bind_strace_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some(strace) = find("strace") else {
        eprintln!("no strace on PATH");
        return;
    };
    let cat = coreutils("cat");
    let script = format!("echo \"[$A_TOKFILE][$MCPKEY]\"; {cat} \"$E_TOKFILE\" \"$MCPFILE\"");
    let (top, run) = bind_world(
        &format!("{engine}-bind-strace"),
        engine,
        &program,
        &["/bin/sh", "-c", &script],
    );
    prepare(&top, &run);
    let log = top.join("strace.log");
    let out = Command::new(&strace)
        .args(["-f", "-qq", "-e", "trace=execve,execveat", "-s", "1000000", "-o"])
        .arg(&log)
        .arg(BIN)
        .arg("--config")
        .arg(top.join("config.json"))
        .arg("run")
        .arg(&run)
        .envs(bind_env())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    ok(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    for n in ["A_TOKFILE", "MCPKEY", "E_TOKFILE", "MCPFILE"] {
        assert!(stdout.contains(&marker(n)), "{n} did not arrive: {stdout}");
    }
    let execs = fs::read_to_string(&log).unwrap();
    if engine == "podman" {
        assert!(
            execs.contains("pot-entry"),
            "strace did not follow into the container"
        );
    }
    for n in bind_names() {
        let bad: Vec<&str> = execs.lines().filter(|l| l.contains(&marker(n))).collect();
        assert!(bad.is_empty(), "{n}'s value in an exec's arguments: {bad:?}");
    }
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

#[test]
fn podman_no_exec_is_handed_a_bindings_value() {
    bind_strace_case("podman");
}

#[test]
fn docker_no_exec_is_handed_a_bindings_value() {
    bind_strace_case("docker");
}

/// A `run` killed with SIGKILL leaves the via-file secrets (and the container) to `cleanup`, which removes them.
fn bind_killed_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let sleep = coreutils("sleep");
    let (top, run) = bind_world(
        &format!("{engine}-bind-killed"),
        engine,
        &program,
        &[&sleep, "60"],
    );
    prepare(&top, &run);
    let mut child = verb(&top, &run, "run")
        .envs(bind_env())
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the container", || {
        !fs::read_to_string(run.join("container.id"))
            .unwrap_or_default()
            .is_empty()
    });
    std::thread::sleep(Duration::from_millis(500));
    assert!(run.join("secrets/E_TOKFILE").exists());
    let docker_name = argv_value(&run, "--name").unwrap_or_default();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        run.join("secrets/E_TOKFILE").exists(),
        "a killed run can't remove it: cleanup does"
    );
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert!(
        run.join("secrets").symlink_metadata().is_err(),
        "secrets/ is left"
    );
    assert_eq!(names(&run), set(LEFT));
    if engine == "podman" {
        assert_eq!(engine_processes(&run), Vec::<String>::new());
    } else {
        let ps = Command::new(&program)
            .args(["ps", "-a", "--quiet", "--filter", &format!("name={docker_name}")])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&ps.stdout).trim().is_empty(),
            "docker's container is left"
        );
    }
}

#[test]
fn podman_a_killed_runs_secrets_go_at_cleanup() {
    bind_killed_case("podman");
}

#[test]
fn docker_a_killed_runs_secrets_go_at_cleanup() {
    bind_killed_case("docker");
}

/// What a killed run left in secrets/ goes when `prepare` runs again, even when it stops (and `cleanup` after it).
#[test]
fn prepare_removes_secrets_even_when_it_stops() {
    let (top, run) = bind_world("bind-prepare-stops", "podman", "podman", &["true"]);
    prepare(&top, &run);
    // as a killed run leaves it
    fs::write(run.join("secrets/E_TOKFILE"), marker("E_TOKFILE")).unwrap();
    // prepare stops: an item this slice does not carry out
    let path = run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    p["items"].as_array_mut().unwrap().push(
        serde_json::json!({ "kind": "dir", "value": "~/.links", "class": "see", "effective": true,
        "mode": "fixed", "sources": [ { "kit": "root:test", "file": "t", "as": "see" } ], "links": [] }),
    );
    fs::write(&path, p.to_string()).unwrap();
    let out = verb(&top, &run, "prepare").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("dir ~/.links (a folder of links)"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        run.join("secrets").symlink_metadata().is_err(),
        "secrets/ is left after prepare stopped"
    );
    // a link planted at secrets/ is removed, never followed
    let outside = top.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep"), "").unwrap();
    symlink(&outside, run.join("secrets")).unwrap();
    let _ = verb(&top, &run, "prepare").output().unwrap();
    assert!(run.join("secrets").symlink_metadata().is_err() && outside.join("keep").exists());
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// run refuses, before the engine starts, a value it can't deliver: an mcp via-file key with a quote (its helper
/// could not carry it), and a key file that is not there. The refusal names the binding and the file, never the
/// value; nothing is left in secrets/.
fn bind_refusal_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let (top, run) = bind_world(&format!("{engine}-bind-refused"), engine, &program, &["true"]);
    prepare(&top, &run);
    let quoted = "pot-quoted\"marker";
    fs::write(top.join("keys/MCPFILE"), format!("MCPFILE='{quoted}'\n")).unwrap();
    let out = verb(&top, &run, "run")
        .envs(bind_env())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert_eq!(
        err.trim(),
        "pot (review): the mcp binding t: its key holds a quote, a backslash or a control character, which its header \
         helper cannot carry: use via = \"env\""
    );
    assert!(
        run.join("container.id").symlink_metadata().is_err(),
        "the engine started"
    );
    assert!(
        fs::read_dir(run.join("secrets")).map_or(true, |mut d| d.next().is_none()),
        "a secret is left"
    );
    // a missing key file: named, with the binding
    fs::write(top.join("keys/MCPFILE"), "MCPFILE=fine\n").unwrap();
    fs::remove_file(top.join("keys/B_ENVFILE")).unwrap();
    let out = verb(&top, &run, "run")
        .envs(bind_env())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert_eq!(
        err.trim(),
        format!(
            "pot (review): the credential binding sel-B_ENVFILE reads {}: No such file or directory",
            top.join("keys/B_ENVFILE").display()
        )
    );
    assert!(run.join("container.id").symlink_metadata().is_err());
    for n in bind_names() {
        assert!(!err.contains(&marker(n)));
    }
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert_eq!(names(&run), set(LEFT));
}

#[test]
fn podman_a_value_run_cannot_deliver_is_refused_before_the_engine() {
    bind_refusal_case("podman");
}

#[test]
fn docker_a_value_run_cannot_deliver_is_refused_before_the_engine() {
    bind_refusal_case("docker");
}

/// The command's environment equals bwrap's for the same policy: `run_env` updated with `deliver(credentials())`, as
/// bwrap.py's `run` computes it (run here with the repository's core/bwrap.py), but for what this backend does not
/// place yet: `BASH_ENV` and the exported not-found handler (`bash_env`), and `/run/pot/bin` and `/run/pot/notfound` on
/// `PATH` (the shims, the not-found advice).
fn bwrap_env_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some(python) = find("python3") else {
        eprintln!("no python3 on PATH");
        return;
    };
    let envp = coreutils("env");
    let (top, run) = bind_world(&format!("{engine}-bwrap-env"), engine, &program, &[&envp, "-0"]);
    // the packages real store paths, as bwrap's run_env keeps only those that exist
    let path = run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let pkg = |b: &str| which(b).parent().unwrap().parent().unwrap().display().to_string();
    let mut n = 0;
    for it in p["items"].as_array_mut().unwrap() {
        if it["kind"] == "package" {
            it["value"] = if n == 0 { pkg("env") } else { pkg("sh") }.into();
            n += 1;
        }
    }
    // a binding named like an env.set value (TZ): bwrap's binding wins
    p["bindings"].as_array_mut().unwrap().push(
        serde_json::json!({ "capability": "credential", "selector": "tz",
        "kit": "root:test", "request": "capability credential tz env=TZ", "via": "env", "env": ["TZ"],
        "from": { "env": "POT_TEST_TZ" }, "holds": "the run holds it" }),
    );
    fs::write(&path, p.to_string()).unwrap();
    prepare(&top, &run);
    let mut host = bind_env();
    host.extend([
        ("NOTES_TOKEN".to_string(), "notes-value".to_string()),
        ("TERM".to_string(), "xterm".to_string()),
        ("POT_TEST_TZ".to_string(), "Europe/Oslo".to_string()),
    ]);
    let out = verb(&top, &run, "run")
        .envs(host.clone())
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
    let scratch = top.join("bwrap-secrets");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(scratch.join("secrets")).unwrap();
    let core = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core")).unwrap();
    let bwrap_py = "import json, sys\nsys.path.insert(0, sys.argv[1])\nimport bwrap\n\
        cfg = json.load(open(sys.argv[2]))\npolicy, launch = bwrap.load(sys.argv[3])\n\
        env = bwrap.run_env(cfg, policy, launch)\n\
        env.update(bwrap.deliver(policy, sys.argv[4], policy['mounts']['run'] + '/secrets', bwrap.credentials(policy)))\n\
        sys.stdout.write(json.dumps(env))\n";
    let py = Command::new(python)
        .args(["-I", "-S", "-c", bwrap_py])
        .arg(&core)
        .arg(top.join("config.json"))
        .arg(&run)
        .arg(&scratch)
        .envs(host)
        .env_remove("COLORTERM")
        .output()
        .unwrap();
    ok(&py);
    let _ = fs::remove_dir_all(&scratch);
    let bwrap: BTreeMap<String, String> = serde_json::from_slice(&py.stdout).unwrap();
    let want: BTreeSet<String> = bwrap
        .into_iter()
        .filter(|(k, _)| k != "BASH_ENV" && !k.starts_with("BASH_FUNC_"))
        .map(|(k, v)| {
            let v = if k == "PATH" {
                v.split(':')
                    .filter(|d| !matches!(*d, "/run/pot/bin" | "/run/pot/notfound"))
                    .collect::<Vec<_>>()
                    .join(":")
            } else {
                v
            };
            format!("{k}={v}")
        })
        .collect();
    assert!(want.contains("TZ=Europe/Oslo") && want.contains(&format!("A_TOKFILE={}", marker("A_TOKFILE"))));
    assert!(want.contains("E_TOKFILE=/run/pot/secrets/E_TOKFILE"));
    assert_eq!(got, want);
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

#[test]
fn podman_the_commands_environment_is_bwraps_with_bindings() {
    bwrap_env_case("podman");
}

#[test]
fn docker_the_commands_environment_is_bwraps_with_bindings() {
    bwrap_env_case("docker");
}

/// A link planted after prepare, at secrets/ or at a file's name in it, is never written through: at secrets/ run
/// refuses before the engine (the folder prepare left, S4); in it, run removes the link and creates its own file.
#[test]
fn a_link_planted_in_secrets_is_never_followed() {
    let Some(program) = engine_program("podman") else {
        return;
    };
    let cat = coreutils("cat");
    let (top, run) = bind_world(
        "bind-link",
        "podman",
        &program,
        &[&cat, "/run/pot/secrets/E_TOKFILE"],
    );
    prepare(&top, &run);
    let outside = top.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("E_TOKFILE"), "").unwrap();
    // in secrets/: the link goes, run's own file takes its name
    symlink(outside.join("E_TOKFILE"), run.join("secrets/E_TOKFILE")).unwrap();
    let out = verb(&top, &run, "run")
        .envs(bind_env())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    ok(&out);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), marker("E_TOKFILE"));
    assert_eq!(
        fs::read_to_string(outside.join("E_TOKFILE")).unwrap(),
        "",
        "written through the link"
    );
    // at secrets/: refused, nothing written
    prepare(&top, &run);
    fs::remove_dir(run.join("secrets")).unwrap();
    symlink(&outside, run.join("secrets")).unwrap();
    let out = verb(&top, &run, "run")
        .envs(bind_env())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("secrets (not the dir prepare left)"), "{err}");
    assert_eq!(names(&outside), set(&["E_TOKFILE"]));
    assert_eq!(fs::read_to_string(outside.join("E_TOKFILE")).unwrap(), "");
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert!(outside.join("E_TOKFILE").exists(), "cleanup followed the link");
}

// ---------------------------------------------------------------- write access in place (0.3.0)

/// pot's core, the repository's (core/pins.py: the pins and the host's look).
fn core_dir() -> PathBuf {
    fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core")).unwrap()
}

/// Host git for the tests' own repositories: no user or system configuration (no signing, no hooks).
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@e")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@e")
        .output()
        .unwrap();
    ok(&out);
}

fn put(dir: &Path, files: &[(&str, &str)]) {
    for (rel, text) in files {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}

/// A write world: `world`'s run folder with a real repository in its workspace (or, with WORKTREE, a worktree of a
/// repository beside it, its common directory outside the workspace), `access: write`, protected paths and a sealed
/// `.pi`, the backend's config naming the look's programs (python, core's pins.py, git), git's closure in the run, and
/// launch.json's pins as pot's core resolves them (`pins.resolve_workspace`, run here as pot runs it). BEFORE runs on
/// the workspace before the pins are resolved. Returns (top, run, workspace, common).
fn write_world(
    name: &str,
    engine: &str,
    program: &str,
    worktree: bool,
    before: impl FnOnce(&Path),
) -> Option<(PathBuf, PathBuf, PathBuf, PathBuf)> {
    let python = find("python3")?;
    let gitbin = find("git")?;
    let (top, run) = world(name, engine, program, &["true"]);
    let files: &[(&str, &str)] = &[
        ("flake.nix", "{ }\n"),
        ("sub/flake.nix", "{ }\n"),
        ("AGENTS.md", "# agents\n"),
        ("notes.md", "notes\n"),
        (".pi/settings.json", "{}\n"),
    ];
    let mut ws = top.join("ws");
    fs::remove_dir_all(&ws).unwrap();
    let common = if worktree {
        let main = top.join("main");
        fs::create_dir_all(&main).unwrap();
        git(&main, &["init", "-q", "-b", "main"]);
        put(&main, files);
        git(&main, &["add", "-A"]);
        git(&main, &["commit", "-qm", "init"]);
        git(
            &main,
            &["worktree", "add", "-q", &ws.display().to_string(), "-b", "w"],
        );
        main.join(".git")
    } else {
        fs::create_dir_all(&ws).unwrap();
        git(&ws, &["init", "-q", "-b", "main"]);
        put(&ws, files);
        git(&ws, &["add", "-A"]);
        git(&ws, &["commit", "-qm", "init"]);
        ws.join(".git")
    };
    ws = fs::canonicalize(&ws).unwrap();
    let common = fs::canonicalize(common).unwrap();
    before(&ws);
    let (w, c) = (ws.display().to_string(), common.display().to_string());
    let mut p: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run.join("policy.json")).unwrap()).unwrap();
    p["access"] = "write".into();
    for it in p["items"].as_array_mut().unwrap() {
        if it["kind"] == "access" {
            it["value"] = "write".into();
        }
    }
    p["workspace"][0]["path"] = w.clone().into();
    p["workspace"][0]["common"] = c.clone().into();
    p["workspace"][0]["protected"] =
        serde_json::json!(["**/flake.nix", "**/.pi", "AGENTS.md", "missing.md", ".git/hooks"]);
    p["workspace"][0]["sealed"] = serde_json::json!(["**/.pi"]);
    p["backend"]["proved"]
        .as_array_mut()
        .unwrap()
        .push("protected-as-mounts".into());
    fs::write(run.join("policy.json"), p.to_string()).unwrap();
    // pot's core: the pins, resolved once (core/pot.py workspace_pins)
    let resolved = Command::new(&python)
        .args([
            "-I",
            "-c",
            "import json, sys; sys.path.insert(0, sys.argv[1]); import pins; \
             print(json.dumps([pins.resolve_workspace(sys.argv[2], json.loads(sys.argv[3]))]))",
        ])
        .arg(core_dir())
        .arg(&gitbin)
        .arg(p["workspace"][0].to_string())
        .output()
        .unwrap();
    ok(&resolved);
    let mut l: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run.join("launch.json")).unwrap()).unwrap();
    l["pins"] = serde_json::from_slice(&resolved.stdout).unwrap();
    l["repoKitPaths"] = serde_json::json!({ w.clone(): ["flake.nix", "flake.lock"] });
    l["cwd"] = w.into();
    // git in the run (its closure, bound read-only with the rest under closure-store)
    l["closure"] = String::from_utf8(closure_of(&[&gitbin]))
        .unwrap()
        .lines()
        .collect::<Vec<_>>()
        .into();
    fs::write(run.join("launch.json"), l.to_string()).unwrap();
    let path = top.join("config.json");
    let mut cfg: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    cfg["python"] = python.display().to_string().into();
    cfg["pins"] = core_dir().join("pins.py").display().to_string().into();
    cfg["git"] = gitbin.display().to_string().into();
    fs::write(&path, cfg.to_string()).unwrap();
    Some((top, run, ws, common))
}

/// Set the run's command: bash's sh with SCRIPT, then ARGS ($1...).
fn set_script(run: &Path, script: &str, args: &[&str]) {
    let mut l: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run.join("launch.json")).unwrap()).unwrap();
    let mut words = vec![
        which("sh").display().to_string(),
        "-c".into(),
        script.to_string(),
        "_".into(),
    ];
    words.extend(args.iter().map(|a| (*a).to_string()));
    l["argv"] = words.into();
    fs::write(run.join("launch.json"), l.to_string()).unwrap();
}

/// What the run prints for each try: `NAME: ok` or `NAME: refused`.
const TRY: &str = r#"t() { n=$1; shift; if "$@" >/dev/null 2>&1; then echo "$n: ok"; else echo "$n: refused"; fi; }
w() { if (echo x >> "$2") 2>/dev/null; then echo "$1: ok"; else echo "$1: refused"; fi; }
c=$1 g=$2
"#;

fn after_event(top: &Path, run: &Path) -> serde_json::Value {
    let out = verb(top, run, "after").output().unwrap();
    ok(&out);
    serde_json::from_slice(&out.stdout).unwrap()
}

fn gitbin() -> String {
    find("git").unwrap().display().to_string()
}

/// Write access in place, under each engine: the workspace is writable and a commit works; every pin pot's core
/// resolved holds against a write, a rename, a removal and a replacement (`**/flake.nix`, a protected file, `.git/config`,
/// `.git/hooks`, the git directory itself, the placeholders, a sealed folder's entry), the folder above a pin can't be
/// moved, `git config` can't write, and nothing outside the workspace is reached. The engines mount by destination depth,
/// so a read-only pin lands over the writable workspace: this run is the proof. `after` reports what bwrap's does.
#[test]
#[allow(clippy::too_many_lines)] // one script of tries, its expected lines, and the host's view after
fn write_access_in_place_holds_every_pin() {
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let Some((top, run, ws, _)) =
            write_world(&format!("{engine}-write"), engine, &program, false, |_| {})
        else {
            eprintln!("python3 or git is not on PATH: no write run");
            continue;
        };
        let script = format!(
            "{TRY}{}",
            r#"w "new file" new.txt
w "flake.nix" flake.nix
t "rm flake.nix" $c/rm -f flake.nix
t "mv flake.nix" $c/mv flake.nix f2
echo y > other; t "rename over flake.nix" $c/mv -f other flake.nix
w "sub/flake.nix" sub/flake.nix
t "mv sub" $c/mv sub sub2
t "rm -r sub" $c/rm -rf sub
w "AGENTS.md" AGENTS.md
t "replace AGENTS.md" $c/ln -sf /etc/passwd AGENTS.md
w ".git/config" .git/config
t "mv .git/config" $c/mv .git/config .git/c2
t "rm .git/config" $c/rm -f .git/config
w ".git/hooks/pre-commit" .git/hooks/pre-commit
t "mv .git/hooks" $c/mv .git/hooks .git/h2
t "mv .git" $c/mv .git .git2
t "git config" $g config user.name intruder
t "git config --global" $g config --file .git/config core.hooksPath /tmp
w ".git/commondir" .git/commondir
t "rm .git/commondir" $c/rm -f .git/commondir
w ".git/config.worktree" .git/config.worktree
w ".git/gitdir" .git/gitdir
w ".pi/settings.json" .pi/settings.json
t "rm .pi/settings.json" $c/rm -f .pi/settings.json
w ".pi/new.json" .pi/new.json
t "mv .pi" $c/mv .pi pi2
w "outside" ../outside
t "mkdir outside" $c/mkdir ../outside-dir
w "missing.md" missing.md
echo more >> notes.md
t "git commit" $g -c user.name=t -c user.email=t@e commit -qam run
echo "commondir: $($c/cat .git/commondir)"
"#
        );
        let cbin = Path::new(&coreutils("cat"))
            .parent()
            .unwrap()
            .display()
            .to_string();
        set_script(&run, &script, &[&cbin, &gitbin()]);
        let config_before = fs::read(ws.join(".git/config")).unwrap();
        prepare(&top, &run);
        // the placeholders are there for the run, read-only in it
        assert_eq!(fs::read_to_string(ws.join(".git/commondir")).unwrap(), ".\n");
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        ok(&out);
        let got: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(String::from)
            .collect();
        let want = [
            "new file: ok",
            "flake.nix: refused",
            "rm flake.nix: refused",
            "mv flake.nix: refused",
            "rename over flake.nix: refused",
            "sub/flake.nix: refused",
            "mv sub: refused",
            "rm -r sub: refused",
            "AGENTS.md: refused",
            "replace AGENTS.md: refused",
            ".git/config: refused",
            "mv .git/config: refused",
            "rm .git/config: refused",
            ".git/hooks/pre-commit: refused",
            "mv .git/hooks: refused",
            "mv .git: refused",
            "git config: refused",
            "git config --global: refused",
            ".git/commondir: refused",
            "rm .git/commondir: refused",
            ".git/config.worktree: refused",
            ".git/gitdir: refused",
            ".pi/settings.json: refused",
            "rm .pi/settings.json: refused",
            ".pi/new.json: ok",
            "mv .pi: refused",
            "outside: refused",
            "mkdir outside: refused",
            "missing.md: ok",
            "git commit: ok",
            "commondir: .",
        ];
        assert_eq!(got, want, "{engine}: {}", String::from_utf8_lossy(&out.stderr));
        // on the host: what the run wrote in the workspace is there; every pin is as it was; nothing outside
        assert_eq!(fs::read_to_string(ws.join("new.txt")).unwrap(), "x\n");
        assert_eq!(fs::read_to_string(ws.join("flake.nix")).unwrap(), "{ }\n");
        assert_eq!(fs::read_to_string(ws.join("sub/flake.nix")).unwrap(), "{ }\n");
        assert_eq!(fs::read_to_string(ws.join("AGENTS.md")).unwrap(), "# agents\n");
        assert_eq!(fs::read(ws.join(".git/config")).unwrap(), config_before);
        assert!(!ws.join(".git/hooks/pre-commit").exists());
        assert!(
            !ws.join(".pi/new.json").exists(),
            "{engine}: a sealed folder's new entry reached the host"
        );
        assert!(!top.join("outside").exists() && !top.join("outside-dir").exists());
        // the after event: bwrap's, from the host's look (pins.py)
        let ev = after_event(&top, &run);
        assert_eq!(ev["commits"].as_array().unwrap().len(), 1, "{engine}: {ev}");
        assert_eq!(
            ev["touches_host_run"],
            serde_json::json!(["missing.md"]),
            "{engine}: {ev}"
        );
        assert_eq!(ev["new_git_metadata"], serde_json::json!([]), "{engine}: {ev}");
        assert_eq!(ev["repo_kit_change"], serde_json::json!([]), "{engine}: {ev}");
        assert_eq!(
            ev["sealed_left"],
            serde_json::json!([{ "path": ".pi/new.json", "type": "file", "size": 2 }]),
            "{engine}: {ev}"
        );
        assert_eq!(ev["unsigned"], 1);
        ok(&verb(&top, &run, "cleanup").output().unwrap());
        // cleanup: the placeholders that still hold what prepare wrote, and the sealed folders' bases, are gone
        for n in ["commondir", "config.worktree", "gitdir"] {
            assert!(!ws.join(".git").join(n).exists(), "{engine}: .git/{n} is left");
        }
        assert!(!run.join("sealed").exists());
        assert_eq!(engine_processes(&run), Vec::<String>::new());
    }
}

/// A worktree: its common directory (outside the workspace) is bound writable with its pins over it, so the run commits
/// but can't write the common config or hooks, or move the worktree's git directory.
#[test]
fn write_access_in_a_worktree_pins_its_common_directory() {
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let Some((top, run, ws, common)) =
            write_world(&format!("{engine}-worktree"), engine, &program, true, |_| {})
        else {
            continue;
        };
        let k = common.display().to_string();
        let script = format!(
            "{TRY}k=$3\n{}",
            r#"w "common config" $k/config
w "common hooks" $k/hooks/pre-commit
t "mv common hooks" $c/mv $k/hooks $k/h2
t "mv the worktree's git directory" $c/mv $k/worktrees/ws $k/worktrees/x
w "the worktree's commondir" $k/worktrees/ws/commondir
w ".git file" .git
w "flake.nix" flake.nix
echo more >> notes.md
t "git commit" $g -c user.name=t -c user.email=t@e commit -qam run
"#
        );
        let cbin = Path::new(&coreutils("cat"))
            .parent()
            .unwrap()
            .display()
            .to_string();
        set_script(&run, &script, &[&cbin, &gitbin(), &k]);
        prepare(&top, &run);
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        ok(&out);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).lines().collect::<Vec<_>>(),
            [
                "common config: refused",
                "common hooks: refused",
                "mv common hooks: refused",
                "mv the worktree's git directory: refused",
                "the worktree's commondir: refused",
                ".git file: refused",
                "flake.nix: refused",
                "git commit: ok",
            ],
            "{engine}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!common.join("hooks/pre-commit").exists());
        assert_eq!(after_event(&top, &run)["commits"].as_array().unwrap().len(), 1);
        ok(&verb(&top, &run, "cleanup").output().unwrap());
        assert!(ws.join("notes.md").exists());
    }
}

/// A link on a protected path, a link above one, and a protected file with a second hard link stop the run in
/// prepare, with the message pot's core gave (bwrap's), and no container starts.
#[test]
fn a_link_or_a_hard_link_on_a_pin_stops_the_run() {
    type Plant = fn(&Path);
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let cases: [(&str, Plant, &str); 3] = [
            (
                "link",
                |ws| {
                    fs::remove_file(ws.join("sub/flake.nix")).unwrap();
                    symlink("../flake.nix", ws.join("sub/flake.nix")).unwrap();
                },
                "sub/flake.nix is a symbolic link, so the sandbox can't make it read-only",
            ),
            (
                "above",
                |ws| {
                    fs::rename(ws.join(".pi"), ws.join("pi-real")).unwrap();
                    symlink("pi-real", ws.join(".pi")).unwrap();
                },
                ".pi is a symbolic link, so the sandbox can't make it read-only",
            ),
            (
                "hard",
                |ws| fs::hard_link(ws.join("AGENTS.md"), ws.join("agents-link")).unwrap(),
                "AGENTS.md has another hard link, so a run could write it through that link",
            ),
        ];
        for (case, plant, said) in cases {
            let Some((top, run, ws, _)) = write_world(
                &format!("{engine}-refused-{case}"),
                engine,
                &program,
                false,
                plant,
            ) else {
                continue;
            };
            let out = verb(&top, &run, "prepare").output().unwrap();
            assert_eq!(out.status.code(), Some(1), "{engine} {case}");
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(
                err.starts_with(&format!("pot (review): {said}")),
                "{engine} {case}: {err}"
            );
            assert!(!run.join("container.argv").exists());
            assert!(
                !ws.join(".git/commondir").exists(),
                "{engine} {case}: a placeholder was made"
            );
            let refused_run = verb(&top, &run, "run").output().unwrap();
            assert_eq!(refused_run.status.code(), Some(1));
            ok(&verb(&top, &run, "cleanup").output().unwrap());
        }
    }
}

/// A pin that changed after prepare (replaced, so another inode; or made a link) stops `run` before the engine
/// starts: run checks each pin again, by inode, just before it.
#[test]
fn run_refuses_a_pin_changed_after_prepare() {
    type Change = fn(&Path);
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let cases: [(&str, Change, &str); 4] = [
            (
                // X3: the same inode, rewritten in place (as inode reuse would look): its size and ctime differ
                "rewritten",
                |ws| fs::write(ws.join("flake.nix"), "{ changed = true; }\n").unwrap(),
                "flake.nix",
            ),
            (
                "replaced",
                |ws| {
                    fs::copy(ws.join("flake.nix"), ws.join("f2")).unwrap();
                    fs::rename(ws.join("f2"), ws.join("flake.nix")).unwrap();
                },
                "flake.nix",
            ),
            (
                "linked",
                |ws| {
                    fs::remove_file(ws.join("AGENTS.md")).unwrap();
                    symlink("/etc/passwd", ws.join("AGENTS.md")).unwrap();
                },
                "AGENTS.md",
            ),
            (
                "hard-linked",
                |ws| fs::hard_link(ws.join(".git/config"), ws.join("config-link")).unwrap(),
                ".git/config",
            ),
        ];
        for (case, change, path) in cases {
            let Some((top, run, ws, _)) = write_world(
                &format!("{engine}-changed-{case}"),
                engine,
                &program,
                false,
                |_| {},
            ) else {
                continue;
            };
            set_script(&run, "echo ran", &[]);
            prepare(&top, &run);
            change(&ws);
            let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(1), "{engine} {case}: {err}");
            assert!(
                err.contains("changed since prepare") && err.contains(&format!("{}/{path}", ws.display())),
                "{engine} {case}: {err}"
            );
            assert!(!String::from_utf8_lossy(&out.stdout).contains("ran"));
            assert!(
                !run.join("container.id").exists(),
                "{engine} {case}: the engine started"
            );
            ok(&verb(&top, &run, "cleanup").output().unwrap());
        }
    }
}

/// X1 (QA, 2026-11-22): a process exchanges a `dirs` pin (`newdir`, holding the pin `newdir/flake.nix`) with a link to
/// a folder outside the workspace, over and over (`renameat2` `RENAME_EXCHANGE`), while `run` starts the engine. The
/// engine resolves a bind's source by path, so it may bind the outside folder; the entry's pin check (every pin a
/// mount point of the inode prepare recorded, from inside the container, before the command) must then stop the run.
/// Over many tries under each engine the command never reads or writes the outside folder; each try either runs on
/// the real pins or is stopped by the check, which says so.
#[test]
fn a_pin_swapped_for_a_link_while_run_starts_is_never_used() {
    let swap = r#"
import ctypes, os, sys, time
d, victim, secs = sys.argv[1], sys.argv[2], float(sys.argv[3])
libc = ctypes.CDLL(None, use_errno=True); AT = -100; EXCH = 2
link = d + ".lnk"; os.symlink(victim, link)
def x():
    if libc.renameat2(AT, d.encode(), AT, link.encode(), EXCH) != 0: raise OSError(ctypes.get_errno(), "renameat2")
end = time.time() + secs; real = True
try:
    while time.time() < end:
        x(); real = not real; time.sleep(0.001)
finally:
    if not real: x()
    os.unlink(link)
"#;
    let tries: usize = std::env::var("POT_TEST_RACE_TRIES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(12);
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let Some((top, run, ws, _)) = write_world(&format!("{engine}-race"), engine, &program, false, |ws| {
            put(ws, &[("newdir/flake.nix", "{ }\n")]);
        }) else {
            continue;
        };
        let victim = top.join("victim");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("mark"), "VICTIM-MARK\n").unwrap();
        let cat = coreutils("cat");
        set_script(
            &run,
            r"$1 newdir/mark 2>/dev/null; (echo x > newdir/written) 2>/dev/null && echo WROTE; echo done",
            &[&cat],
        );
        let python = find("python3").unwrap();
        let (mut ran_real, mut checked, mut stopped) = (0, 0, 0);
        for i in 0..tries {
            prepare(&top, &run);
            let mut swapper = Command::new(&python)
                .args(["-I", "-c", swap])
                .arg(ws.join("newdir"))
                .arg(&victim)
                .arg("1.5")
                .spawn()
                .unwrap();
            std::thread::sleep(Duration::from_millis(100 + 40 * (i as u64 % 8)));
            let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
            swapper.wait().unwrap();
            let (stdout, stderr) = (
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
            assert!(
                !stdout.contains("VICTIM-MARK"),
                "{engine} try {i}: the run read the outside folder: {stdout}"
            );
            assert!(
                !victim.join("written").exists(),
                "{engine} try {i}: the run wrote the outside folder"
            );
            if stdout.contains("done") {
                ran_real += 1;
                assert_eq!(out.status.code(), Some(0), "{engine} try {i}: {stderr}");
            } else {
                // stopped: by run's check, by the entry's pin check, or by the engine itself (podman stats each
                // source before it starts, and may find the swap's moment); never with the command run
                assert_ne!(out.status.code(), Some(0), "{engine} try {i}: {stderr}");
                if stderr.contains("not the binds prepare recorded") {
                    checked += 1;
                } else {
                    stopped += 1;
                }
            }
            // the engine may have made a mount point in the outside folder when it bound it: only a stopped run may
            // leave one, and the check named the pin
            let extra = remove_engine_made(engine, &program, &top, &victim, &["mark"]);
            if !extra.is_empty() {
                assert!(
                    stderr.contains("newdir"),
                    "{engine} try {i}: {extra:?} made with no report: {stderr}"
                );
            }
            ok(&verb(&top, &run, "cleanup").output().unwrap());
            assert_eq!(engine_processes(&run), Vec::<String>::new());
        }
        eprintln!(
            "{engine}: {tries} tries: {ran_real} ran on the real pins, {checked} stopped by the entry's pin check, {stopped} stopped before (run's check or the engine)"
        );
    }
}

/// X1: the entry's pin check itself, deterministically: a pin whose recorded inode is not what the container sees
/// stops the run (125) before the command, naming the pin, under each engine.
#[test]
fn the_entrys_pin_check_stops_a_run_whose_pins_are_not_recorded() {
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let Some((top, run, ws, _)) =
            write_world(&format!("{engine}-pincheck"), engine, &program, false, |_| {})
        else {
            continue;
        };
        set_script(&run, "echo ran", &[]);
        prepare(&top, &run);
        let list = run.join("binds.txt");
        let flake = format!(" {}/flake.nix", ws.display());
        let text = fs::read_to_string(&list).unwrap();
        assert!(text.lines().any(|l| l.ends_with(&flake)), "{text}");
        let wrong: String = text
            .lines()
            .map(|l| {
                if l.ends_with(&flake) {
                    format!("0:0{flake}\n")
                } else {
                    format!("{l}\n")
                }
            })
            .collect();
        fs::set_permissions(&list, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
        fs::write(&list, wrong).unwrap();
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(125), "{engine}: {err}");
        assert!(!String::from_utf8_lossy(&out.stdout).contains("ran"));
        assert!(
            err.contains("not the binds prepare recorded")
                && err.contains(&format!(
                    "{}/flake.nix (another file: mounted from ",
                    ws.display()
                )),
            "{engine}: {err}"
        );
        ok(&verb(&top, &run, "cleanup").output().unwrap());
        // with the pins as recorded the command runs, and the check's bash adds nothing to its environment
        let mut l: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(run.join("launch.json")).unwrap()).unwrap();
        l["argv"] = serde_json::json!([coreutils("env")]);
        fs::write(run.join("launch.json"), l.to_string()).unwrap();
        prepare(&top, &run);
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        ok(&out);
        let env = String::from_utf8_lossy(&out.stdout);
        for n in ["SHLVL=", "_=", "OLDPWD=", "PWD=", "__pot"] {
            assert!(!env.lines().any(|l| l.starts_with(n)), "{engine}: {n} in {env}");
        }
        assert!(env.lines().any(|l| l.starts_with("POT_RUN=")), "{engine}: {env}");
        ok(&verb(&top, &run, "cleanup").output().unwrap());
    }
}

/// X4 (QA): the host's look (`pins.py`) runs in an environment of its own: `PATH`, `HOME`, the locale, and what git and
/// its signature check read, never the caller's other variables (a secret's among them) nor its `GIT_*` ones. Needs no
/// engine: `prepare` and `after` run the look, the config's `python` a stand-in that records its environment.
#[test]
fn the_hosts_look_runs_in_its_own_environment() {
    let Some((top, run, _, _)) = write_world("look-env", "podman", "podman", false, |_| {}) else {
        eprintln!("python3 or git is not on PATH: no look");
        return;
    };
    let python = find("python3").unwrap();
    let dump = top.join("look-env.txt");
    let stand_in = top.join("python3");
    fs::write(
        &stand_in,
        format!(
            "#!{}\n{} >> {}\nexec {} \"$@\"\n",
            which("sh").display(),
            coreutils("env"),
            dump.display(),
            python.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&stand_in, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = top.join("config.json");
    let mut cfg: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    cfg["python"] = stand_in.display().to_string().into();
    fs::write(&path, cfg.to_string()).unwrap();
    for v in ["prepare", "after"] {
        let out = verb(&top, &run, v)
            .env("POT_TEST_X4_MARKER", "look-environment-marker")
            .env("GIT_DIR", "/nonexistent")
            .env("GIT_CONFIG_PARAMETERS", "'core.fsmonitor'='/bin/false'")
            .output()
            .unwrap();
        ok(&out);
    }
    let seen = fs::read_to_string(&dump).unwrap();
    assert!(!seen.contains("look-environment-marker"), "{seen}");
    assert!(!seen.lines().any(|l| l.starts_with("GIT_")), "{seen}");
    assert!(
        !seen
            .lines()
            .any(|l| l.starts_with("CARGO") || l.starts_with("RUST")),
        "{seen}"
    );
    for k in ["PATH=", "HOME="] {
        assert!(seen.lines().any(|l| l.starts_with(k)), "{k} missing: {seen}");
    }
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// Add a hidden item for PATHS to the run's policy.
fn hide(run: &Path, paths: &[&Path]) {
    let mut p: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run.join("policy.json")).unwrap()).unwrap();
    p["items"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "kind": "hidden", "value": "sub/private",
        "class": "limits", "sources": [{ "root": "test" }], "effective": true,
        "paths": paths.iter().map(|q| q.display().to_string()).collect::<Vec<_>>() }));
    fs::write(run.join("policy.json"), p.to_string()).unwrap();
}

/// A world with a hidden folder `sub/private` holding a secret, and `decoy/private` beside it; WRITE gives write access
/// (the pins), else read access. Returns (top, run, workspace).
fn hidden_world(name: &str, engine: &str, program: &str, write: bool) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let files: &[(&str, &str)] = &[
        ("sub/private/s.txt", "HIDDEN-SECRET\n"),
        ("decoy/private/d.txt", "decoy\n"),
    ];
    let (top, run, ws) = if write {
        let (top, run, ws, _) = write_world(name, engine, program, false, |ws| put(ws, files))?;
        (top, run, ws)
    } else {
        let (top, run) = world(name, engine, program, &["true"]);
        let ws = fs::canonicalize(top.join("ws")).unwrap();
        put(&ws, files);
        (top, run, ws)
    };
    hide(&run, &[&ws.join("sub/private")]);
    Some((top, run, ws))
}

/// Y1 (QA re-audit, 2026-11-22): the same race against a mask. A process exchanges the hidden folder's parent `sub`
/// with a link to `decoy` (in the workspace) while `run` starts, so the engine may land the mask on `decoy/private`;
/// after the swap back the command would read `sub/private`. The entry's check covers every bind under a bound host
/// folder, masks included (a mask must be a mount point of the run folder's empty file or folder), under read and
/// write access. Over many tries under each engine the command never reads the hidden secret.
#[test]
fn a_mask_swapped_away_while_run_starts_never_shows_the_hidden_path() {
    let swap = r#"
import ctypes, os, sys, time
d, victim, secs = sys.argv[1], sys.argv[2], float(sys.argv[3])
libc = ctypes.CDLL(None, use_errno=True); AT = -100; EXCH = 2
link = d + ".lnk"; os.symlink(victim, link)
def x():
    if libc.renameat2(AT, d.encode(), AT, link.encode(), EXCH) != 0: raise OSError(ctypes.get_errno(), "renameat2")
end = time.time() + secs; real = True
try:
    while time.time() < end:
        x(); real = not real; time.sleep(0.001)
finally:
    if not real: x()
    os.unlink(link)
"#;
    let tries: usize = std::env::var("POT_TEST_RACE_TRIES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(10);
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        for write in [false, true] {
            let mode = if write { "write" } else { "read" };
            let Some((top, run, ws)) =
                hidden_world(&format!("{engine}-hidrace-{mode}"), engine, &program, write)
            else {
                continue;
            };
            let cat = coreutils("cat");
            let sleep = coreutils("sleep");
            set_script(
                &run,
                "$2 2; $1 sub/private/s.txt 2>/dev/null; echo DONE",
                &[&cat, &sleep],
            );
            let python = find("python3").unwrap();
            let (mut masked, mut checked, mut stopped) = (0, 0, 0);
            for i in 0..tries {
                prepare(&top, &run);
                let mut swapper = Command::new(&python)
                    .args(["-I", "-c", swap])
                    .arg(ws.join("sub"))
                    .arg(ws.join("decoy"))
                    .arg("1.5")
                    .spawn()
                    .unwrap();
                std::thread::sleep(Duration::from_millis(100 + 40 * (i as u64 % 8)));
                let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
                swapper.wait().unwrap();
                let (stdout, stderr) = (
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr),
                );
                assert!(
                    !stdout.contains("HIDDEN-SECRET"),
                    "{engine} {mode} try {i}: the command read the hidden path"
                );
                if stdout.contains("DONE") {
                    masked += 1;
                } else if stderr.contains("not the binds prepare recorded") {
                    checked += 1;
                } else {
                    assert_ne!(out.status.code(), Some(0), "{engine} {mode} try {i}: {stderr}");
                    stopped += 1;
                }
                ok(&verb(&top, &run, "cleanup").output().unwrap());
                // Y2: what the engine made in the swapped-in folder (a mount point named like a pin under `sub`),
                // only when the run was stopped
                let extra = remove_engine_made(engine, &program, &top, &ws.join("decoy"), &["private"]);
                assert!(
                    extra.is_empty() || !stdout.contains("DONE"),
                    "{engine} {mode} try {i}: {extra:?} made in a run that went ahead"
                );
            }
            eprintln!(
                "{engine} {mode}: {tries} tries: {masked} masked, {checked} stopped by the entry's check, {stopped} stopped before"
            );
        }
    }
}

/// Y1, Y2: the check itself, deterministically, for a mask: a mask whose recorded file is not what the container sees
/// stops the run (125) before the command, naming it with its mount root and source (`/proc/self/mountinfo`), under
/// read access too.
#[test]
fn the_entrys_check_stops_a_run_whose_mask_is_not_recorded() {
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let Some((top, run, ws)) = hidden_world(&format!("{engine}-maskcheck"), engine, &program, false)
        else {
            continue;
        };
        set_script(&run, "echo ran", &[]);
        prepare(&top, &run);
        let list = run.join("binds.txt");
        let mask = format!(" {}/sub/private", ws.display());
        let text = fs::read_to_string(&list).unwrap();
        assert!(text.lines().any(|l| l.ends_with(&mask)), "{text}");
        let wrong: String = text
            .lines()
            .map(|l| {
                if l.ends_with(&mask) {
                    format!("0:0{mask}\n")
                } else {
                    format!("{l}\n")
                }
            })
            .collect();
        fs::set_permissions(&list, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
        fs::write(&list, wrong).unwrap();
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(125), "{engine}: {err}");
        assert!(!String::from_utf8_lossy(&out.stdout).contains("ran"));
        // Y2: the mount there, named by its root and source
        assert!(
            err.contains(&format!(
                "{}/sub/private (another file: mounted from ",
                ws.display()
            )) && err.contains("emptydir"),
            "{engine}: {err}"
        );
        ok(&verb(&top, &run, "cleanup").output().unwrap());
    }
}

/// Y3: a pin whose path ends with a space is checked as it is (the check reads its list without stripping).
#[test]
fn a_pin_with_a_trailing_space_is_checked_as_it_is() {
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let Some((top, run, ws, _)) =
            write_world(&format!("{engine}-space"), engine, &program, false, |ws| {
                put(ws, &[("spaced /flake.nix", "{ }\n")]);
            })
        else {
            continue;
        };
        set_script(&run, "echo ran", &[]);
        prepare(&top, &run);
        assert!(
            fs::read_to_string(run.join("binds.txt"))
                .unwrap()
                .contains(&format!("{}/spaced \n", ws.display()))
        );
        let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{engine}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("ran"));
        ok(&verb(&top, &run, "cleanup").output().unwrap());
    }
}

/// Y2: remove what the engine made in a test's own swapped-in folder DIR (an empty mount point named like a pin),
/// keeping KEEP; returns the names it found. podman's belong to a subordinate id: removed in podman's user namespace,
/// with a store of the test's own under TOP (never the caller's). docker's would be root's: the test stops, naming it.
fn remove_engine_made(engine: &str, program: &str, top: &Path, dir: &Path, keep: &[&str]) -> Vec<String> {
    let extra: Vec<String> = names(dir)
        .into_iter()
        .filter(|n| !keep.contains(&n.as_str()))
        .collect();
    for n in &extra {
        let p = dir.join(n);
        if fs::remove_file(&p).is_ok() || fs::remove_dir(&p).is_ok() {
            continue;
        }
        assert_eq!(
            engine,
            "podman",
            "{engine} left {} (root's): remove it as root",
            p.display()
        );
        let store = top.join("unshare");
        let out = Command::new(program)
            .arg("--root")
            .arg(store.join("s"))
            .arg("--runroot")
            .arg(store.join("r"))
            .args(["unshare", "rm", "-rf", "--"])
            .arg(&p)
            .output()
            .unwrap();
        ok(&out);
        fs::remove_dir_all(&store).unwrap();
    }
    extra
}

// ---------------------------------------------------------------- the status.relay, git.fetch and nix bindings

/// pot's shim, the repository's, added to the store (as a root's pot source is there), so the run sees it under a path
/// of the config's `shimClosure`; None without nix-store (the flake's sandbox).
fn shim_in_store() -> Option<PathBuf> {
    // core/ is not in the flake's sandbox source: no relays run there
    let shim = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/shim.py")).ok()?;
    let out = Command::new("nix-store").arg("--add").arg(&shim).output().ok()?;
    out.status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// The processes whose working folder is the run folder: pot's helpers for it (the proxy, the broker, the nix relay
/// and its daemons) are started there. None may be left once run returns, or after cleanup.
fn helpers_in(run: &Path) -> Vec<String> {
    let r = fs::canonicalize(run).unwrap();
    let mut out = vec![];
    for e in fs::read_dir("/proc").unwrap().flatten() {
        if fs::read_link(e.path().join("cwd")).is_ok_and(|c| c == r) {
            let c = fs::read(e.path().join("cmdline")).unwrap_or_default();
            // a zombie (empty cmdline) is no longer a helper
            if !c.is_empty() {
                out.push(format!(
                    "{}: {}",
                    e.file_name().to_string_lossy(),
                    String::from_utf8_lossy(&c).replace('\0', " ")
                ));
            }
        }
    }
    out
}

/// What the relays test world holds beside the run folder.
struct Relays {
    top: PathBuf,
    run: PathBuf,
    /// The bare repository the run fetches, at `ssh://git@fake.example/PATH`.
    remote: PathBuf,
    /// What the fake host ssh and workmux were run with, a line each.
    ssh_log: PathBuf,
    workmux_log: PathBuf,
    /// git.fetch's key file (a fake: the fake ssh never reads it).
    key: PathBuf,
}

const RELAY_CAPS: &[&str] = &["status.relay", "git.fetch", "nix"];

/// A run folder whose policy has the bindings CAPS (of `RELAY_CAPS`): the config names pot's broker and shim, with
/// python, and the shim's closure; launch.json's hostTools a fake ssh (it runs git-upload-pack on this host for the
/// path asked, and logs its arguments), a fake workmux (it logs its arguments and `TMUX_PANE`) and the host's real nix; its
/// pane `%7`; git's closure in the run's. With nix, closure-store is not proved (pot drops it). None where python,
/// socat, nix-store or nix are missing.
#[allow(clippy::too_many_lines)] // the world, its remote, its fake host programs and its policy, in order
fn relay_world(name: &str, engine: &str, program: &str, argv: &[&str], caps: &[&str]) -> Option<Relays> {
    let (top, run) = world(name, engine, program, argv);
    let cfg_path = top.join("config.json");
    let mut cfg: serde_json::Value = serde_json::from_str(&fs::read_to_string(&cfg_path).unwrap()).unwrap();
    let (Some(python), Some(_), Some(shim), Some(nix), Some(gitb)) = (
        find("python3"),
        cfg.get("socat"),
        shim_in_store(),
        find("nix"),
        find("git"),
    ) else {
        eprintln!("no python3, socat, nix-store, nix or git: no relays run");
        return None;
    };
    let store = top.join("store");
    fs::write(store.join("shim.txt"), closure_of(&[&python, &shim])).unwrap();
    cfg["broker"] = core_dir().join("broker.py").display().to_string().into();
    cfg["shim"] = shim.display().to_string().into();
    cfg["shimClosure"] = store.join("shim.txt").display().to_string().into();
    fs::write(&cfg_path, cfg.to_string()).unwrap();
    // the remote: one commit
    let (remote, work) = (top.join("remote.git"), top.join("remote-work"));
    fs::create_dir_all(&work).unwrap();
    git(&top, &["init", "-q", "--bare", "remote.git"]);
    git(&work, &["init", "-q", "-b", "main"]);
    put(&work, &[("a.txt", "a\n")]);
    git(&work, &["add", "a.txt"]);
    git(&work, &["commit", "-q", "-m", "relay-commit"]);
    git(&work, &["push", "-q", remote.to_str().unwrap(), "main"]);
    git(
        &top,
        &[
            "--git-dir",
            remote.to_str().unwrap(),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );
    let (ssh, ssh_log, workmux, workmux_log, key) = (
        top.join("fake-ssh"),
        top.join("ssh.log"),
        top.join("fake-workmux"),
        top.join("workmux.log"),
        top.join("id_test"),
    );
    fs::write(&key, "not a key\n").unwrap();
    fs::write(
        &ssh,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nfor a; do last=$a; done\neval \"exec $last\"\n",
            ssh_log.display()
        ),
    )
    .unwrap();
    fs::write(
        &workmux,
        format!(
            "#!/bin/sh\nprintf '%s pane=%s\\n' \"$*\" \"$TMUX_PANE\" >> {}\n",
            workmux_log.display()
        ),
    )
    .unwrap();
    Command::new("chmod")
        .arg("755")
        .arg(&ssh)
        .arg(&workmux)
        .status()
        .unwrap();
    let path = run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let mut bindings = vec![];
    for c in caps {
        bindings.push(match *c {
            "status.relay" => serde_json::json!({ "capability": "status.relay", "selector": "*", "kit": "root:test",
                "request": "capability status.relay", "socket": "/run/pot/relay.sock", "see": ["workmux shim"],
                "allow": ["set-window-status", "register-agent"] }),
            "git.fetch" => serde_json::json!({ "capability": "git.fetch", "selector": "ssh://git@fake.example/*",
                "kit": "root:test", "request": "capability git.fetch ssh://git@fake.example/*",
                "socket": "/run/pot/git.sock", "see": ["ssh shim (GIT_SSH_COMMAND)"],
                "remotes": ["ssh://git@fake.example/*"], "key": { "file": key } }),
            _ => serde_json::json!({ "capability": "nix", "selector": "*", "kit": "root:test",
                "request": "capability nix", "socket": "/run/pot/nix.sock", "see": ["nix", "NIX_REMOTE"],
                "via": "daemon-untrusted", "notes": ["the whole store is visible (closure-store dropped)"] }),
        });
    }
    p["bindings"] = bindings.into();
    if caps.contains(&"nix") {
        let proved: Vec<serde_json::Value> = p["backend"]["proved"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|g| *g != "closure-store")
            .cloned()
            .collect();
        p["backend"]["proved"] = proved.into();
    }
    fs::write(&path, p.to_string()).unwrap();
    let lpath = run.join("launch.json");
    let mut l: serde_json::Value = serde_json::from_str(&fs::read_to_string(&lpath).unwrap()).unwrap();
    let closure: Vec<String> = String::from_utf8(closure_of(&[&gitb]))
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    l["closure"] = closure.into();
    l["pane"] = "%7".into();
    l["hostTools"] = serde_json::json!({ "ssh": ssh, "workmux": workmux, "nix": nix });
    fs::write(&lpath, l.to_string()).unwrap();
    Some(Relays {
        top,
        run,
        remote,
        ssh_log,
        workmux_log,
        key,
    })
}

/// The relays under ENGINE, end to end: the run's `workmux` shim reaches the broker, which relays an allowed verb to
/// host workmux on the caller's pane (and refuses another), and clears the pane when the command ends; git clones over
/// ssh through the broker (`GIT_SSH_COMMAND`, the `pot-ssh` shim), with the binding's key file and `BatchMode`, and a
/// remote not bound is refused and recorded in denials; nix reaches the host's daemon through the relay as an untrusted
/// client, writes to the store and reads the new path at once (the whole store is bound). When run returns, no helper
/// is left and their sockets and the relay's script are gone.
fn relays_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let (git, cat) = (gitbin(), coreutils("cat"));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let script = format!(
        r#"workmux set-window-status working
workmux kill-server
{git} clone -q "ssh://git@fake.example$REMOTE" /tmp/c && {git} -C /tmp/c log --format=fetched=%s
{git} ls-remote ssh://git@other.example/x >/dev/null 2>&1; echo "other=$?"
nix store info 2>&1
p=$(nix eval --raw --expr 'builtins.toFile "pot-relay-{nanos}" "MARK-{nanos}"') && echo "added=$({cat} "$p")"
o=$(nix build --no-link --print-out-paths --impure --expr 'derivation {{ name = "pot-relay-{nanos}"; system = builtins.currentSystem; builder = "/bin/sh"; args = [ "-c" "echo BUILT-{nanos} > $out" ]; }}') && echo "built=$({cat} "$o")"
"#
    );
    let Some(w) = relay_world(
        &format!("{engine}-relays"),
        engine,
        &program,
        &["/bin/sh", "-c", &script],
        RELAY_CAPS,
    ) else {
        return;
    };
    // the remote's path, as an env item the run is given
    let path = w.run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    p["items"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "kind": "env", "value": "REMOTE",
        "class": "reach", "sources": [ { "flag": "--env" } ], "effective": true, "request": "env REMOTE",
        "requestClass": "ordinary" }));
    fs::write(&path, p.to_string()).unwrap();
    prepare(&w.top, &w.run);
    let out = verb(&w.top, &w.run, "run")
        .env("REMOTE", &w.remote)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}{err}");
    let lines: Vec<&str> = said.lines().collect();
    assert!(lines.contains(&"fetched=relay-commit"), "{said}{err}");
    assert!(
        !lines.contains(&"other=0") && lines.iter().any(|l| l.starts_with("other=")),
        "{said}"
    );
    assert!(lines.contains(&"Trusted: 0"), "an untrusted client: {said}{err}");
    assert!(
        lines.contains(&format!("added=MARK-{nanos}").as_str()),
        "{said}{err}"
    );
    assert!(
        lines.contains(&format!("built=BUILT-{nanos}").as_str()),
        "{said}{err}"
    );
    let relayed = fs::read_to_string(&w.workmux_log).unwrap_or_default();
    assert_eq!(
        relayed.lines().collect::<Vec<_>>(),
        [
            "set-window-status working pane=%7",
            "set-window-status clear pane=%7"
        ],
        "only the allowed verb, on the caller's pane, then the clear"
    );
    let ssh = fs::read_to_string(&w.ssh_log).unwrap();
    let first = ssh.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("-o BatchMode=yes -o SendEnv=GIT_PROTOCOL"),
        "{ssh}"
    );
    assert!(
        first.contains(&format!(
            "-i {} -o IdentitiesOnly=yes git@fake.example",
            w.key.display()
        )),
        "{ssh}"
    );
    assert!(!ssh.contains("other.example"), "{ssh}");
    let denials = fs::read_to_string(w.run.join("denials")).unwrap();
    assert!(
        denials.contains("ssh://git@other.example/x (not a bound git.fetch remote)"),
        "{denials}"
    );
    assert_eq!(helpers_in(&w.run), Vec::<String>::new());
    for n in ["relay.sock", "git.sock", "nix.sock", "nix-relay"] {
        assert!(w.run.join(n).symlink_metadata().is_err(), "{n} is left");
    }
    ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
    let mut left = set(LEFT);
    left.extend(["status".to_string(), "bindings.log".to_string()]);
    assert_eq!(names(&w.run), left);
}

#[test]
fn podman_relays_reach_the_hosts_helpers() {
    relays_case("podman");
}

#[test]
fn docker_relays_reach_the_hosts_helpers() {
    relays_case("docker");
}

/// The helpers end with the run under ENGINE: a SIGTERM to run stops the container and the helpers (143); a SIGKILL of
/// run leaves them to cleanup, which stops every one (the broker, the nix relay and a daemon it started for a live
/// connection) and removes their sockets.
fn relays_stopped_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let sleep = coreutils("sleep");
    for (sig, killed) in [("TERM", false), ("KILL", true)] {
        // a nix build holds a connection open (a relay daemon runs for it) while the run sleeps
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let script = format!(
            r#"nix build --no-link --impure --expr 'derivation {{ name = "pot-held-{nanos}"; system = builtins.currentSystem; builder = "/bin/sh"; args = [ "-c" "read -t 60 x < /dev/zero; echo > $out" ]; }}' & {sleep} 60"#
        );
        let Some(w) = relay_world(
            &format!("{engine}-relays-{sig}"),
            engine,
            &program,
            &["/bin/sh", "-c", &script],
            RELAY_CAPS,
        ) else {
            return;
        };
        prepare(&w.top, &w.run);
        let said = w.top.join("said.txt");
        let mut child = verb(&w.top, &w.run, "run")
            .stdin(Stdio::null())
            .stdout(fs::File::create(&said).unwrap())
            .stderr(Stdio::from(fs::File::create(w.top.join("err.txt")).unwrap()))
            .spawn()
            .unwrap();
        wait_for("the container and the helpers", || {
            !fs::read_to_string(w.run.join("container.id"))
                .unwrap_or_default()
                .is_empty()
                && helpers_in(&w.run).iter().any(|h| h.contains("daemon --stdio"))
        });
        std::thread::sleep(Duration::from_millis(1500));
        let before = helpers_in(&w.run);
        assert!(before.iter().any(|h| h.contains("broker.py")), "{before:?}");
        assert!(
            before.iter().any(|h| h.contains("EXEC:./nix-relay")),
            "{before:?}"
        );
        assert!(
            before
                .iter()
                .any(|h| h.contains("daemon --stdio --force-untrusted")),
            "{before:?}: {}{}",
            fs::read_to_string(&said).unwrap_or_default(),
            fs::read_to_string(w.top.join("err.txt")).unwrap_or_default()
        );
        Command::new("kill")
            .arg(format!("-{sig}"))
            .arg(child.id().to_string())
            .status()
            .unwrap();
        let status = child.wait().unwrap();
        if killed {
            ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
        } else {
            assert_eq!(status.code(), Some(143));
            assert_eq!(helpers_in(&w.run), Vec::<String>::new(), "after SIGTERM");
            ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
        }
        assert_eq!(
            helpers_in(&w.run),
            Vec::<String>::new(),
            "after {sig} and cleanup"
        );
        for n in ["relay.sock", "git.sock", "nix.sock", "nix-relay"] {
            assert!(
                w.run.join(n).symlink_metadata().is_err(),
                "{n} is left after {sig}"
            );
        }
        let mut left = set(LEFT);
        left.insert("bindings.log".into());
        if !killed {
            left.insert("status".into());
        }
        assert_eq!(names(&w.run), left, "after {sig}");
        if engine == "podman" {
            assert_eq!(engine_processes(&w.run), Vec::<String>::new());
        }
    }
}

#[test]
fn podman_relays_end_with_the_run() {
    relays_stopped_case("podman");
}

#[test]
fn docker_relays_end_with_the_run() {
    relays_stopped_case("docker");
}

/// Links planted in the run folder after prepare are never followed: at a socket's name (towards a listener of the
/// test's), at the nix relay's script (towards a script that would leave a mark) and at bindings.log (towards a file
/// of the test's). run removes each and makes its own: the run reaches its own helpers, the listener is never
/// connected to, the script never runs and the file is never written.
fn relay_links_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let script = "workmux set-window-status linked; nix store info >/dev/null && echo nix=ok";
    let Some(w) = relay_world(
        &format!("{engine}-relay-links"),
        engine,
        &program,
        &["/bin/sh", "-c", script],
        RELAY_CAPS,
    ) else {
        return;
    };
    prepare(&w.top, &w.run);
    let decoy = w.top.join("decoy");
    fs::create_dir_all(&decoy).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(decoy.join("listen.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mark = decoy.join("ran");
    fs::write(
        decoy.join("relay"),
        format!("#!/bin/sh\ntouch {}\n", mark.display()),
    )
    .unwrap();
    Command::new("chmod")
        .arg("755")
        .arg(decoy.join("relay"))
        .status()
        .unwrap();
    fs::write(decoy.join("log"), "").unwrap();
    for n in ["relay.sock", "nix.sock"] {
        symlink(decoy.join("listen.sock"), w.run.join(n)).unwrap();
    }
    symlink(decoy.join("relay"), w.run.join("nix-relay")).unwrap();
    symlink(decoy.join("log"), w.run.join("bindings.log")).unwrap();
    let out = verb(&w.top, &w.run, "run").stdin(Stdio::null()).output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{said}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(said.lines().any(|l| l == "nix=ok"), "{said}");
    assert!(
        fs::read_to_string(&w.workmux_log)
            .unwrap_or_default()
            .contains("set-window-status linked pane=%7"),
        "the run's own broker relayed it"
    );
    assert!(
        listener
            .accept()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock),
        "the decoy listener was reached"
    );
    assert!(!mark.exists(), "the decoy relay ran");
    assert_eq!(
        fs::read_to_string(decoy.join("log")).unwrap(),
        "",
        "written through the link"
    );
    assert!(decoy.join("relay").exists() && decoy.join("listen.sock").exists());
    ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
    assert!(
        decoy.join("relay").exists() && decoy.join("listen.sock").exists(),
        "cleanup followed a link"
    );
}

#[test]
fn podman_a_link_planted_at_a_helpers_path_is_never_followed() {
    relay_links_case("podman");
}

#[test]
fn docker_a_link_planted_at_a_helpers_path_is_never_followed() {
    relay_links_case("docker");
}

/// The command's environment with the three bindings equals bwrap's for the same policy (`run_env`, run here with the
/// repository's core/bwrap.py, and `deliver`, which adds nothing without a credential): `/run/pot/bin` first on PATH
/// (the shims), nix's bin/ on it, `GIT_SSH_COMMAND`, `GIT_SSH_VARIANT`, `NIX_REMOTE` and `NIX_CONFIG`; but for what this
/// backend does not place yet (`BASH_ENV`, the exported not-found handler, `/run/pot/notfound` on PATH).
fn relay_env_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some(python) = find("python3") else {
        return;
    };
    let envp = coreutils("env");
    let Some(w) = relay_world(
        &format!("{engine}-relay-env"),
        engine,
        &program,
        &[&envp, "-0"],
        RELAY_CAPS,
    ) else {
        return;
    };
    let path = w.run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let pkg = |b: &str| which(b).parent().unwrap().parent().unwrap().display().to_string();
    let mut n = 0;
    for it in p["items"].as_array_mut().unwrap() {
        if it["kind"] == "package" {
            it["value"] = if n == 0 { pkg("env") } else { pkg("sh") }.into();
            n += 1;
        }
    }
    fs::write(&path, p.to_string()).unwrap();
    prepare(&w.top, &w.run);
    let host = [("TERM", "xterm"), ("NOTES_TOKEN", "notes-value")];
    let out = verb(&w.top, &w.run, "run")
        .envs(host)
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
    let bwrap_py = "import json, sys\nsys.path.insert(0, sys.argv[1])\nimport bwrap\n\
        cfg = json.load(open(sys.argv[2]))\npolicy, launch = bwrap.load(sys.argv[3])\n\
        env = bwrap.run_env(cfg, policy, launch)\n\
        env.update(bwrap.deliver(policy, sys.argv[3], policy['mounts']['run'] + '/secrets', bwrap.credentials(policy)))\n\
        sys.stdout.write(json.dumps(env))\n";
    let py = Command::new(python)
        .args(["-I", "-S", "-c", bwrap_py])
        .arg(core_dir())
        .arg(w.top.join("config.json"))
        .arg(&w.run)
        .envs(host)
        .env_remove("COLORTERM")
        .output()
        .unwrap();
    ok(&py);
    let bwrap: BTreeMap<String, String> = serde_json::from_slice(&py.stdout).unwrap();
    let want: BTreeSet<String> = bwrap
        .into_iter()
        .filter(|(k, _)| k != "BASH_ENV" && !k.starts_with("BASH_FUNC_"))
        .map(|(k, v)| {
            let v = if k == "PATH" {
                v.split(':')
                    .filter(|d| *d != "/run/pot/notfound")
                    .collect::<Vec<_>>()
                    .join(":")
            } else {
                v
            };
            format!("{k}={v}")
        })
        .collect();
    assert!(
        want.contains("GIT_SSH_COMMAND=/run/pot/bin/pot-ssh")
            && want.contains("NIX_REMOTE=unix:///run/pot/nix.sock")
    );
    assert!(
        want.iter().any(|v| v.starts_with("PATH=/run/pot/bin:")),
        "{want:?}"
    );
    assert_eq!(got, want);
    ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
}

#[test]
fn podman_the_commands_environment_is_bwraps_with_the_relays() {
    relay_env_case("podman");
}

#[test]
fn docker_the_commands_environment_is_bwraps_with_the_relays() {
    relay_env_case("docker");
}

/// cleanup stops the run's live broker and nix relay (here started by this test from container.env's commands, so not
/// orphaned) and removes their sockets and the relay's script. No engine is needed.
#[test]
fn cleanup_stops_live_relays_and_removes_their_sockets() {
    let Some(w) = relay_world("relay-cleanup", "podman", "podman", &["true"], RELAY_CAPS) else {
        return;
    };
    prepare(&w.top, &w.run);
    let cenv: serde_json::Value =
        serde_json::from_slice(&fs::read(w.run.join("container.env")).unwrap()).unwrap();
    let words = |k: &str| -> Vec<String> {
        cenv[k]
            .as_array()
            .unwrap_or_else(|| panic!("container.env has no {k}: {cenv}"))
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect()
    };
    let (broker, relay) = (words("broker"), words("nixRelay"));
    fs::write(w.run.join("nix-relay"), "#!/bin/sh\nexit 0\n").unwrap();
    let mut started = vec![];
    for argv in [&broker, &relay] {
        started.push(
            Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(&w.run)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    wait_for("the sockets", || {
        ["relay.sock", "git.sock", "nix.sock"]
            .iter()
            .all(|n| w.run.join(n).exists())
    });
    assert_eq!(helpers_in(&w.run).len(), 2);
    ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
    for mut c in started {
        let start = Instant::now();
        while c.try_wait().unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(5), "a helper still runs");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert_eq!(helpers_in(&w.run), Vec::<String>::new());
    for n in ["relay.sock", "git.sock", "nix.sock", "nix-relay"] {
        assert!(w.run.join(n).symlink_metadata().is_err(), "{n} is left");
    }
}

/// prepare (and `prepare --print`) stops, as bwrap's run does, when a binding's host program is missing from launch.json's hostTools (ssh for
/// git.fetch, nix for the nix relay); status.relay without workmux runs (bwrap gives its broker /bin/false).
#[test]
fn prepare_stops_when_a_bindings_host_program_is_missing() {
    let Some(w) = relay_world("relay-no-tools", "podman", "podman", &["true"], RELAY_CAPS) else {
        return;
    };
    let lpath = w.run.join("launch.json");
    let l: serde_json::Value = serde_json::from_str(&fs::read_to_string(&lpath).unwrap()).unwrap();
    for (drop, said) in [
        ("ssh", "git.fetch's broker needs ssh on the host's PATH"),
        ("nix", "the nix capability's relay needs nix on the host's PATH"),
    ] {
        let mut l2 = l.clone();
        l2["hostTools"].as_object_mut().unwrap().remove(drop);
        fs::write(&lpath, l2.to_string()).unwrap();
        let out = verb(&w.top, &w.run, "prepare").output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{err}");
        assert!(err.contains(said), "{err}");
        // A5 (QA 2026-11-22): --print stops as prepare does, never showing `--ssh ''`
        let out = verb(&w.top, &w.run, "prepare").arg("--print").output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(1),
            "--print: {err}{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(err.contains(said), "--print: {err}");
    }
    let mut l2 = l.clone();
    l2["hostTools"].as_object_mut().unwrap().remove("workmux");
    fs::write(&lpath, l2.to_string()).unwrap();
    prepare(&w.top, &w.run);
    let cenv: serde_json::Value =
        serde_json::from_slice(&fs::read(w.run.join("container.env")).unwrap()).unwrap();
    let broker: Vec<&str> = cenv["broker"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert!(
        broker.windows(2).any(|p| p == ["--workmux", "/bin/false"]),
        "{broker:?}"
    );
    ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
}

/// No value reaches an exec it should not (`strace -f -e execve -v`, which prints each exec's arguments and
/// environment, following run's helpers and, under podman, into the container): git.fetch's key file's contents are in
/// no exec at all (the broker hands ssh its path); the caller's `SSH_AUTH_SOCK` (git.fetch's `key.agent`) reaches the
/// broker and the host ssh and workmux it runs (beside run itself, whose environment it is), and never the engine, the
/// nix relay or the run.
#[allow(clippy::too_many_lines)] // one traced run, then each exec looked at
fn relay_strace_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some(strace) = find("strace") else {
        eprintln!("no strace on PATH");
        return;
    };
    let git = gitbin();
    let script = format!(
        "workmux set-window-status traced; {git} ls-remote \"ssh://git@fake.example$REMOTE\" >/dev/null && echo fetched; nix store info >/dev/null && echo nix=ok"
    );
    let Some(w) = relay_world(
        &format!("{engine}-relay-strace"),
        engine,
        &program,
        &["/bin/sh", "-c", &script],
        RELAY_CAPS,
    ) else {
        return;
    };
    let held = "pot-relay-marker-in-the-file";
    let sock_marker = "/tmp/pot-relay-agent-marker-77e2";
    fs::write(&w.key, format!("{held}\n")).unwrap();
    let path = w.run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    p["items"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "kind": "env", "value": "REMOTE",
        "class": "reach", "sources": [ { "flag": "--env" } ], "effective": true, "request": "env REMOTE",
        "requestClass": "ordinary" }));
    fs::write(&path, p.to_string()).unwrap();
    prepare(&w.top, &w.run);
    let log = w.top.join("strace.log");
    let out = Command::new(&strace)
        .args([
            "-f",
            "-qq",
            "-v",
            "-e",
            "trace=execve,execveat",
            "-s",
            "1000000",
            "-o",
        ])
        .arg(&log)
        .arg(BIN)
        .arg("--config")
        .arg(w.top.join("config.json"))
        .arg("run")
        .arg(&w.run)
        .env("REMOTE", &w.remote)
        .env("SSH_AUTH_SOCK", sock_marker)
        .env("KRB5CCNAME", "FILE:/tmp/pot-relay-krb5-marker")
        .env("NIX_DAEMON_SOCKET_PATH", "/nix/var/nix/daemon-socket/socket")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    ok(&out);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("fetched") && said.contains("nix=ok"), "{said}");
    let execs = fs::read_to_string(&log).unwrap();
    // the trace saw the helpers (and, under podman, the run)
    for seen in ["broker.py", "EXEC:./nix-relay", "fake-ssh", "daemon\", \"--stdio"] {
        assert!(execs.contains(seen), "strace did not see {seen}");
    }
    if engine == "podman" {
        assert!(
            execs.contains("shim.py"),
            "strace did not follow into the container"
        );
    }
    let bad: Vec<&str> = execs.lines().filter(|l| l.contains(held)).collect();
    assert!(bad.is_empty(), "the key's contents in an exec: {bad:?}");
    let fake_ssh = format!("execve(\"{}\"", w.top.join("fake-ssh").display());
    let fake_workmux = format!("execve(\"{}\"", w.top.join("fake-workmux").display());
    let bad: Vec<&str> = execs
        .lines()
        .filter(|l| l.contains(sock_marker))
        .filter(|l| !l.contains(&format!("execve(\"{BIN}\"")))
        .filter(|l| {
            !(l.contains("broker.py")
                || l.contains(&fake_ssh)
                || l.contains(&fake_workmux)
                || l.contains("/git-upload-pack\""))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "SSH_AUTH_SOCK beyond the broker and its ssh: {bad:?}"
    );
    assert!(
        execs
            .lines()
            .any(|l| l.contains(sock_marker) && l.contains("broker.py")),
        "the broker lacks SSH_AUTH_SOCK"
    );
    // A4 (QA 2026-11-22): what bwrap's whole environment gave them, which the lists had dropped
    assert!(
        execs
            .lines()
            .any(|l| l.contains("broker.py") && l.contains("KRB5CCNAME=FILE:/tmp/pot-relay-krb5-marker")),
        "the broker lacks KRB5CCNAME"
    );
    assert!(
        execs
            .lines()
            .any(|l| l.contains("EXEC:./nix-relay") && l.contains("NIX_DAEMON_SOCKET_PATH=")),
        "the nix relay lacks NIX_DAEMON_SOCKET_PATH"
    );
    ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
}

#[test]
fn podman_no_exec_is_handed_a_relays_value() {
    relay_strace_case("podman");
}

#[test]
fn docker_no_exec_is_handed_a_relays_value() {
    relay_strace_case("docker");
}

/// A1 (QA 2026-11-22): a run-folder file the plan binds for the relays is checked by more than its kind. A shim
/// replaced by a hard link to another file, a shim given a second hard link, and `bin/` replaced by another folder are
/// each refused by run before the engine starts; the decoy is never run.
fn relay_swaps_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some(w) = relay_world(
        &format!("{engine}-relay-swaps"),
        engine,
        &program,
        &["/bin/sh", "-c", "workmux set-window-status swapped; echo ran"],
        &["status.relay"],
    ) else {
        return;
    };
    let decoy = w.top.join("decoy");
    fs::create_dir_all(&decoy).unwrap();
    let mark = decoy.join("ran");
    fs::write(
        decoy.join("workmux"),
        format!("#!/bin/sh\ntouch {}\n", mark.display()),
    )
    .unwrap();
    Command::new("chmod")
        .arg("755")
        .arg(decoy.join("workmux"))
        .status()
        .unwrap();
    let hard_link = |run: &Path| {
        fs::hard_link(decoy.join("workmux"), run.join("bin/.w")).unwrap();
        fs::rename(run.join("bin/.w"), run.join("bin/workmux")).unwrap();
    };
    let second_link = |run: &Path| {
        let _ = fs::remove_file(decoy.join("second"));
        fs::hard_link(run.join("bin/workmux"), decoy.join("second")).unwrap();
    };
    let other_bin = |run: &Path| {
        let other = run.join(".bin");
        fs::create_dir_all(&other).unwrap();
        fs::copy(decoy.join("workmux"), other.join("workmux")).unwrap();
        fs::remove_dir_all(run.join("bin")).unwrap();
        fs::rename(&other, run.join("bin")).unwrap();
    };
    #[allow(clippy::type_complexity)] // each swap, named
    let swaps: [(&str, &dyn Fn(&Path)); 3] = [
        ("a shim swapped for a hard link", &hard_link),
        ("a shim with a second hard link", &second_link),
        ("bin/ swapped for another folder", &other_bin),
    ];
    for (what, swap) in swaps {
        prepare(&w.top, &w.run);
        swap(&w.run);
        let out = verb(&w.top, &w.run, "run").stdin(Stdio::null()).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        let said = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(1), "{what}: {err}{said}");
        assert!(err.contains("run prepare again"), "{what}: {err}");
        assert!(!said.contains("ran"), "{what}: the command ran");
        assert!(!mark.exists(), "{what}: the decoy ran");
        ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
        let _ = fs::remove_file(decoy.join("second"));
    }
}

#[test]
fn podman_a_relays_run_folder_file_swapped_after_prepare_is_refused() {
    relay_swaps_case("podman");
}

#[test]
fn docker_a_relays_run_folder_file_swapped_after_prepare_is_refused() {
    relay_swaps_case("docker");
}

/// A1 (QA 2026-11-22), the reviewer's race: once run's broker has made `relay.sock`, a link to a decoy listener is
/// renamed over it, at a range of delays, while run checks the run folder and the engine binds it. The run never
/// reaches the decoy: run refuses (the link seen), or the entry's check stops the command (125: the socket in the
/// container is not the one run recorded), or the swap came too late to matter.
#[test]
fn a_socket_swapped_for_a_link_while_run_starts_is_never_used() {
    use std::os::unix::fs::FileTypeExt as _;
    let tries: usize = std::env::var("POT_TEST_RACE_TRIES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(10);
    for engine in ["podman", "docker"] {
        let Some(program) = engine_program(engine) else {
            continue;
        };
        let Some(w) = relay_world(
            &format!("{engine}-relay-race"),
            engine,
            &program,
            &["/bin/sh", "-c", "workmux set-window-status raced; echo wm=$?"],
            &["status.relay"],
        ) else {
            continue;
        };
        let lsock = w.top.join("decoy.sock");
        let (mut ran, mut refused, mut checked) = (0, 0, 0);
        for i in 0..tries {
            prepare(&w.top, &w.run);
            let _ = fs::remove_file(&lsock);
            let listener = std::os::unix::net::UnixListener::bind(&lsock).unwrap();
            listener.set_nonblocking(true).unwrap();
            let child = verb(&w.top, &w.run, "run")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let target = w.run.join("relay.sock");
            let start = Instant::now();
            while !target.symlink_metadata().is_ok_and(|m| m.file_type().is_socket())
                && start.elapsed() < Duration::from_secs(10)
            {
                std::thread::sleep(Duration::from_micros(200));
            }
            std::thread::sleep(Duration::from_micros(300 * (i as u64 % 10)));
            let tmp = w.run.join(".qa-link");
            let _ = fs::remove_file(&tmp);
            symlink(&lsock, &tmp).unwrap();
            let _ = fs::rename(&tmp, &target);
            // a run whose shim reached the decoy waits for an answer that never comes: stopped after 60 s
            let mut child = child;
            let start = Instant::now();
            while child.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(60) {
                std::thread::sleep(Duration::from_millis(50));
            }
            let hung = child.try_wait().unwrap().is_none();
            if hung {
                Command::new("kill").arg(child.id().to_string()).status().unwrap();
            }
            let out = child.wait_with_output().unwrap();
            let (said, err) = (
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr),
            );
            assert!(
                listener
                    .accept()
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
                    && !hung,
                "{engine} try {i}: the decoy listener was reached (hung: {hung}): {said}{err}"
            );
            match out.status.code() {
                Some(0) => ran += 1,
                Some(125) => {
                    assert!(
                        err.contains("not the binds prepare recorded"),
                        "{engine} try {i}: {err}"
                    );
                    assert!(!said.contains("wm="), "{engine} try {i}: the command ran");
                    checked += 1;
                }
                _ => {
                    assert!(!said.contains("wm="), "{engine} try {i}: the command ran: {err}");
                    refused += 1;
                }
            }
            ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
        }
        eprintln!(
            "{engine}: {tries} tries: {ran} ran, {refused} refused by run, {checked} stopped by the entry's check"
        );
    }
}

/// A3 (QA 2026-11-22): cleanup stops a helper only when its program is the config's (python for the broker and the
/// proxy, socat for the relay, hostTools.nix for a relay daemon), never a decoy in the run folder whose arguments hold
/// the same words. The control, the run's own broker, is stopped.
#[test]
fn cleanup_leaves_a_decoy_with_a_helpers_words() {
    let Some(w) = relay_world("relay-decoys", "podman", "podman", &["true"], RELAY_CAPS) else {
        return;
    };
    prepare(&w.top, &w.run);
    let cenv: serde_json::Value =
        serde_json::from_slice(&fs::read(w.run.join("container.env")).unwrap()).unwrap();
    let broker: Vec<String> = cenv["broker"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    let denials = format!("{}/denials", fs::canonicalize(&w.run).unwrap().display());
    let sh = which("sh");
    let script = format!("{} 30; :", coreutils("sleep"));
    let decoys: Vec<Vec<String>> = vec![
        vec!["daemon".into(), "--stdio".into(), "--force-untrusted".into()],
        vec!["relay.sock".into(), "--denials".into(), denials.clone()],
        vec![
            "UNIX-LISTEN:nix.sock,fork,unlink-early,mode=600".into(),
            "EXEC:./nix-relay".into(),
        ],
        vec!["proxy.sock".into(), "--denials".into(), denials],
    ];
    let mut started = vec![];
    for d in &decoys {
        started.push(
            Command::new(&sh)
                .arg("-c")
                .arg(&script)
                .args(d)
                .current_dir(&w.run)
                .stdin(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    let mut control = Command::new(&broker[0])
        .args(&broker[1..])
        .current_dir(&w.run)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the broker's socket", || w.run.join("relay.sock").exists());
    ok(&verb(&w.top, &w.run, "cleanup").output().unwrap());
    let start = Instant::now();
    while control.try_wait().unwrap().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the run's broker still runs"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    for (d, mut c) in decoys.iter().zip(started) {
        let alive = c.try_wait().unwrap().is_none();
        let _ = c.kill();
        let _ = c.wait();
        assert!(alive, "the decoy {d:?} was stopped");
    }
}

/// The siblings test world: `relay_world` with no binding (its config names python, pot's broker and shim), python's
/// closure in the run's, the work-siblings fixture's children table (a sibling row) and launch.json's `hostPot` a fake host
/// entry (`fake-entry.py`): it reads the broker's request from its fd, logs it (and any signal it gets) to `entry.log`,
/// then, by its first argument, prints and exits 7 (`ok`), prints what it reads on stdin (`cat`), or sleeps (`sleep`).
/// The run's client (`CLIENT`) asks the run's broker as pot's in-run client does: one JSON line, its fds 0-2 by
/// `SCM_RIGHTS`, then the answer line.
fn sibling_world(
    name: &str,
    engine: &str,
    program: &str,
    script: &str,
) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let python = find("python3")?;
    let w = relay_world(name, engine, program, &["/bin/sh", "-c", script], &[])?;
    let (top, run) = (w.top, w.run);
    let (entry, log) = (top.join("fake-entry.py"), top.join("entry.log"));
    fs::write(
        &entry,
        r#"import json, os, signal, sys, time
log, fd = sys.argv[1], int(sys.argv[4])
def note(s):
    with open(log, "a") as f:
        f.write(s + "\n")
def on(sig, _):
    note(f"SIG {sig}")
    sys.exit(128 + sig)
signal.signal(signal.SIGTERM, on)
signal.signal(signal.SIGINT, on)
data = b""
while True:
    b = os.read(fd, 65536)
    if not b:
        break
    data += b
req = json.loads(data)
note(json.dumps({"pid": os.getpid(), "sid": os.getsid(0), "req": req}))
mode = req["args"][0]
if mode == "container":
    # a sibling that is itself a container run: its backend's run, as pot's host entry starts it
    os.execv(req["args"][1], req["args"][1:])
if mode == "ok":
    print("sibling-ran", *req["args"], flush=True)
    sys.exit(7)
if mode == "cat":
    print("sibling-read", sys.stdin.readline().strip(), flush=True)
    sys.exit(0)
print("sibling-sleeping", flush=True)
time.sleep(60)
"#,
    )
    .unwrap();
    let path = run.join("policy.json");
    let mut p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let fx: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../contracts/fixtures/policy-1/work-siblings.json"),
        )
        .unwrap(),
    )
    .unwrap();
    p["children"] = fx["children"].clone();
    fs::write(&path, p.to_string()).unwrap();
    let lpath = run.join("launch.json");
    let mut l: serde_json::Value = serde_json::from_str(&fs::read_to_string(&lpath).unwrap()).unwrap();
    let closure: Vec<String> = String::from_utf8(closure_of(&[&python]))
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    l["closure"] = closure.into();
    l["hostPot"] = serde_json::json!([python, "-I", "-S", entry, log]);
    fs::write(&lpath, l.to_string()).unwrap();
    Some((top, run, log))
}

/// The run's client for a sibling, as pot's in-run client asks (`python3 -I -c CLIENT ARGS`).
const CLIENT: &str = r#"import json, os, socket, sys
key = [k for k, r in json.load(open("/run/pot/policy.json"))["children"].items() if r.get("how") == "sibling"][0]
s = socket.socket(socket.AF_UNIX)
s.connect("/run/pot/children.sock")
head = {"v": 1, "key": key, "args": sys.argv[1:], "cwd": os.getcwd(), "hook": {}}
socket.send_fds(s, [json.dumps(head).encode() + b"\n"], [0, 1, 2])
print("answer", s.makefile().readline().strip(), flush=True)
"#;

fn entry_lines(log: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// v2.20.0 under ENGINE: a run's sibling is started by the host, never in the container. The run's client reaches the
/// run's children broker through /run/pot/children.sock; the broker starts the host entry (`hostPot`, `run --sibling FD`)
/// in a session of its own with the client's fds 0-2 and the request (the run folder, the row's key, the arguments, the
/// cwd) on FD; the sibling's output reaches the run's, its stdin is the run's, and its exit status comes back. A
/// request that goes away (the client killed) stops the sibling; so does the run's end; and the broker, its socket and
/// (cleanup) the tree's slots go.
#[allow(clippy::too_many_lines)] // three runs, each with what the host saw
fn siblings_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some(python) = find("python3") else {
        return;
    };
    let py = python.display().to_string();
    let script = format!(
        r#""{py}" -I -c '{CLIENT}' ok a b; echo "client-exit=$?"
echo piped-in | "{py}" -I -c '{CLIENT}' cat
"#
    );
    let Some((top, run, log)) = sibling_world(&format!("{engine}-siblings"), engine, &program, &script)
    else {
        return;
    };
    prepare(&top, &run);
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}{err}");
    assert!(
        said.lines().any(|l| l == "sibling-ran ok a b"),
        "the sibling's stdout is the run's: {said}{err}"
    );
    assert!(
        said.lines().any(|l| l == r#"answer {"exit": 7}"#),
        "its status comes back: {said}{err}"
    );
    assert!(
        said.lines().any(|l| l == "sibling-read piped-in"),
        "its stdin is the run's: {said}{err}"
    );
    let reqs = entry_lines(&log);
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    let r = &reqs[0]["req"];
    assert_eq!(r["parent"], fs::canonicalize(&run).unwrap().display().to_string());
    assert_eq!(r["key"], "git:file:///tmp/pot-test/nkp.git:nk-chk");
    assert_eq!(r["args"], serde_json::json!(["ok", "a", "b"]));
    assert_eq!(
        r["cwd"],
        fs::canonicalize(top.join("ws")).unwrap().display().to_string()
    );
    // started on the host, in a session of its own: a host pid (not the container's), the session's leader
    assert_eq!(reqs[0]["pid"], reqs[0]["sid"], "{reqs:?}");
    assert_eq!(
        helpers_in(&run),
        Vec::<String>::new(),
        "the broker is gone with the run"
    );
    assert!(
        run.join("children.sock").symlink_metadata().is_err(),
        "children.sock is left"
    );
    let trees = run.parent().unwrap().parent().unwrap().join("trees").join("run");
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert!(
        !trees.exists(),
        "cleanup leaves the top run's tree of slots: {}",
        trees.display()
    );

    // the client killed (the agent's tool timeout): the broker stops the sibling (SIGTERM to its session). The client's
    // stdout is a pipe (a file never crosses: pot's client puts a pipe of its own in front of one)
    fs::remove_file(&log).unwrap();
    let script = format!(
        r#"{{ "{py}" -I -c '{CLIENT}' sleep & echo $! > /tmp/pid; wait; }} | while read -r l; do echo "$l" >> /tmp/o; done &
until case "$(cat /tmp/o 2>/dev/null)" in *sibling-sleeping*) true ;; *) false ;; esac; do sleep 0.1; done; kill -9 "$(cat /tmp/pid)"; sleep 4; echo done"#
    );
    let Some((top, run, log)) = sibling_world(&format!("{engine}-siblings-kill"), engine, &program, &script)
    else {
        return;
    };
    prepare(&top, &run);
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{said}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.lines().any(|l| l == "SIG 15"),
        "the sibling got SIGTERM when its request went away: {text}"
    );
    ok(&verb(&top, &run, "cleanup").output().unwrap());

    // the run ends while its sibling runs: the broker stops it first, and run returns only then
    let script = format!(
        r#""{py}" -I -c '{CLIENT}' sleep | while read -r l; do echo "$l" >> /tmp/o; done &
until case "$(cat /tmp/o 2>/dev/null)" in *sibling-sleeping*) true ;; *) false ;; esac; do sleep 0.1; done; echo leaving"#
    );
    let Some((top, run, log)) = sibling_world(&format!("{engine}-siblings-end"), engine, &program, &script)
    else {
        return;
    };
    prepare(&top, &run);
    let started = Instant::now();
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(started.elapsed() < Duration::from_secs(30));
    let reqs = entry_lines(&log);
    let pid = reqs[0]["pid"].as_u64().unwrap();
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "the sibling outlived the run that asked: {}",
        fs::read_to_string(&log).unwrap_or_default()
    );
    assert_eq!(helpers_in(&run), Vec::<String>::new());
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

#[test]
fn podman_a_sibling_is_started_by_the_host() {
    siblings_case("podman");
}

#[test]
fn docker_a_sibling_is_started_by_the_host() {
    siblings_case("docker");
}

/// A clone world: `write_world`'s repository as the source, a clone of it in the world's state (as pot's core makes a
/// dispatch clone), the policy in clone mode with same-path-clone proved, a proposal pattern (`**/.envrc`), launch.json's
/// `clone` facts, and the config naming git's closure (`gitClosure`). Returns (top, run, source, clone).
fn clone_world(name: &str, engine: &str, program: &str) -> Option<(PathBuf, PathBuf, PathBuf, PathBuf)> {
    let gitb = find("git")?;
    let (top, run, ws, _) = write_world(name, engine, program, false, |_| {})?;
    let clones = top.join("state").join("clones");
    fs::create_dir_all(&clones).unwrap();
    git(&clones, &["clone", "-q", &ws.display().to_string(), "c1"]);
    let clone = fs::canonicalize(clones.join("c1")).unwrap();
    let base = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(&clone)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let mut p: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run.join("policy.json")).unwrap()).unwrap();
    p["mode"] = "clone".into();
    p["workspace"][0]["clone"] = clone.display().to_string().into();
    p["workspace"][0]["sealed"] = serde_json::json!([]);
    p["workspace"][0]["proposal"] = serde_json::json!(["**/.envrc"]);
    p["backend"]["proved"]
        .as_array_mut()
        .unwrap()
        .push("same-path-clone".into());
    fs::write(run.join("policy.json"), p.to_string()).unwrap();
    let mut l: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run.join("launch.json")).unwrap()).unwrap();
    l.as_object_mut().unwrap().remove("pins");
    l["clone"] = serde_json::json!({ "base": base, "branch": "main", "path": clone, "scions": [],
        "objects": ws.join(".git/objects") });
    fs::write(run.join("launch.json"), l.to_string()).unwrap();
    let path = top.join("config.json");
    let mut cfg: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    fs::write(top.join("store").join("git.txt"), closure_of(&[&gitb])).unwrap();
    cfg["gitClosure"] = top.join("store").join("git.txt").display().to_string().into();
    fs::write(&path, cfg.to_string()).unwrap();
    Some((top, run, ws, clone))
}

/// Clone mode under ENGINE, as bwrap's (same-path-clone): the run sees the clone at the source's own path, writes and
/// commits there, and can't write the clone's config, hooks or protected files; the source is untouched; the
/// placeholders are made in the clone and removed by cleanup. `after` stages the clone's commits through upload-pack
/// run in a container of its own (the `git` verb), and reports bwrap's clone fields: the commit on `main`, the agent
/// configuration the clone proposes (`.envrc`), what stays uncommitted. No git container, engine state or upload-pack
/// script is left, and before.json and staging/ stay (pot review reads them).
#[allow(clippy::too_many_lines)] // the run, the host's look, cleanup and a recovered look
/// The docker containers of pot's (`pot-…`) that bind a path under TOP: this test's alone, since the other docker tests'
/// containers, running meanwhile, are named `pot-…` too.
fn docker_left(program: &str, top: &Path) -> Vec<String> {
    let ps = Command::new(program)
        .args(["ps", "-aq", "--filter", "name=pot-"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&ps.stdout)
        .split_whitespace()
        .filter(|id| {
            Command::new(program)
                .args(["inspect", "--format", "{{range .Mounts}}{{.Source}} {{end}}", id])
                .output()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&*top.to_string_lossy()))
        })
        .map(String::from)
        .collect()
}

fn clone_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some((top, run, ws, clone)) = clone_world(&format!("{engine}-clone"), engine, &program) else {
        return;
    };
    let g = gitbin();
    set_script(
        &run,
        &format!(
            r#"{TRY}w agents AGENTS.md
w config .git/config
t hooks mkdir .git/hooks/x
echo here > .envrc; echo new > new.txt; echo left > left.txt
{g} add .envrc new.txt && {g} -c user.name=t -c user.email=t@e commit -qm in-clone && echo committed
echo "top=$({g} rev-parse --show-toplevel)"
"#
        ),
        &[],
    );
    prepare(&top, &run);
    let placeholder = clone.join(".git/commondir");
    assert!(placeholder.exists(), "prepare made the clone's placeholders");
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{said}{err}");
    for l in [
        "agents: refused",
        "config: refused",
        "hooks: refused",
        "committed",
    ] {
        assert!(said.lines().any(|x| x == l), "{l}: {said}{err}");
    }
    assert!(
        said.lines().any(|x| x == format!("top={}", ws.display())),
        "the clone at the source's path: {said}"
    );
    assert!(
        !ws.join("new.txt").exists() && !ws.join(".envrc").exists(),
        "the source is untouched"
    );
    assert!(clone.join("new.txt").exists(), "the run wrote the clone");
    let ev = after_event(&top, &run);
    assert_eq!(ev["commits"].as_array().map(Vec::len), Some(1), "{ev}");
    assert_eq!(
        ev["refs"]
            .as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>()),
        Some(vec!["main".to_string()]),
        "{ev}"
    );
    assert_eq!(ev["agent_config"], serde_json::json!([".envrc"]), "{ev}");
    assert_eq!(ev["uncommitted"], 1, "{ev}");
    assert!(ev.get("fetch_failed").is_none(), "{ev}");
    assert!(run.join("staging/top.git").is_dir());
    assert!(!run.join("gitlook").exists(), "the look's engine state is gone");
    if engine == "docker" {
        assert_eq!(
            docker_left(&program, &top),
            Vec::<String>::new(),
            "a git container is left"
        );
    }
    assert_eq!(helpers_in(&run), Vec::<String>::new());
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    assert!(!placeholder.exists(), "cleanup removes the clone's placeholders");
    assert!(
        run.join("before.json").is_file() && run.join("staging").is_dir(),
        "kept for pot review"
    );
    assert!(
        !names(&run)
            .iter()
            .any(|n| n.starts_with("upload-pack-") || n == "gitlook"),
        "{:?}",
        names(&run)
    );
    // a recovered after (pot review, once the host pot died before its after event): the same look, from the kept
    // before.json, with git in a container again
    let again = after_event(&top, &run);
    assert_eq!(
        (&again["commits"], &again["refs"], &again["agent_config"]),
        (&ev["commits"], &ev["refs"], &ev["agent_config"])
    );
    assert!(!run.join("gitlook").exists());
    // the git verb runs git only in this run's clone
    let out = verb(&top, &run, "git")
        .arg(&ws)
        .arg("/clone")
        .arg("--")
        .arg("status")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("is not this run's clone"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn podman_a_clone_runs_at_the_original_path_and_its_commits_are_staged_in_a_container() {
    clone_case("podman");
}

#[test]
fn docker_a_clone_runs_at_the_original_path_and_its_commits_are_staged_in_a_container() {
    clone_case("docker");
}

/// A sibling that is a container run of its own, under ENGINE, whose command ignores SIGTERM: when its request goes away
/// the broker sends SIGTERM to its session each second and SIGKILL after 10 s, so its backend's run must stop and
/// remove its container within that time; no container of the sibling's is left, and its run records its status.
fn sibling_stops_in_time_case(engine: &str) {
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some(python) = find("python3") else {
        return;
    };
    let sleep = coreutils("sleep");
    let (stop, srun) = world(
        &format!("{engine}-sibling-stubborn"),
        engine,
        &program,
        &[
            "/bin/sh",
            "-c",
            &format!("trap '' TERM; echo stubborn-up; {sleep} 120"),
        ],
    );
    let mut l: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(srun.join("launch.json")).unwrap()).unwrap();
    l["siblingOf"] = "0abc".into();
    fs::write(srun.join("launch.json"), l.to_string()).unwrap();
    prepare(&stop, &srun);
    let py = python.display().to_string();
    let (bin, cfg, sr) = (
        BIN,
        stop.join("config.json").display().to_string(),
        srun.display().to_string(),
    );
    let script = format!(
        r#"{{ "{py}" -I -c '{CLIENT}' container {bin} --config {cfg} run {sr} & echo $! > /tmp/pid; wait; }} | while read -r l; do echo "$l" >> /tmp/o; done &
until case "$(cat /tmp/o 2>/dev/null)" in *stubborn-up*) true ;; *) false ;; esac; do sleep 0.1; done; kill -9 "$(cat /tmp/pid)"; sleep 14; echo done"#
    );
    let Some((top, run, _)) =
        sibling_world(&format!("{engine}-siblings-stubborn"), engine, &program, &script)
    else {
        return;
    };
    prepare(&top, &run);
    let out = verb(&top, &run, "run").stdin(Stdio::null()).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // its run ended in time, recording its status (137 under podman: SIGKILL after the stop's 3 s)
    let st = fs::read_to_string(srun.join("status")).unwrap_or_default();
    assert!(!st.is_empty(), "the sibling's run was killed before it ended");
    if engine == "podman" {
        let ps = podman(&program, &srun, &["ps", "--all", "--quiet"]);
        assert_eq!(
            String::from_utf8_lossy(&ps.stdout).trim(),
            "",
            "the sibling's container is left"
        );
    } else {
        assert_eq!(
            docker_left(&program, &stop),
            Vec::<String>::new(),
            "the sibling's container is left"
        );
    }
    ok(&verb(&top, &run, "cleanup").output().unwrap());
    ok(&verb(&stop, &srun, "cleanup").output().unwrap());
}

#[test]
fn podman_a_container_sibling_stops_within_the_brokers_grace() {
    sibling_stops_in_time_case("podman");
}

#[test]
fn docker_a_container_sibling_stops_within_the_brokers_grace() {
    sibling_stops_in_time_case("docker");
}

/// I1: at a terminal (`--tty`), a run whose caller has a long TMPDIR, and whose folder's path is long, runs: conmon makes
/// its console socket in TMPDIR, and a unix socket's path (`sun_path`) holds 107 bytes.
#[test]
fn podman_runs_at_a_terminal_with_a_long_tmpdir_and_run_folder() {
    let Ok(program) = std::env::var("POT_TEST_PODMAN") else {
        eprintln!("POT_TEST_PODMAN is not set: no container started");
        return;
    };
    let Some(script) = find("script") else {
        eprintln!("no script(1) here: no terminal to run at");
        return;
    };
    let long = format!("podman-tty-{}", "l".repeat(120));
    let (top, run) = world(
        &long,
        "podman",
        &program,
        &["/bin/sh", "-c", "[ -t 0 ] && echo at-a-terminal"],
    );
    assert!(
        run.display().to_string().len() > 108,
        "the run folder's path is long"
    );
    prepare(&top, &run);
    let cmd = format!(
        "{BIN} --config {} run {}",
        top.join("config.json").display(),
        run.display()
    );
    // the caller's TMPDIR is long too (the engine keeps it)
    let long_tmp = top.join(format!("tmp-{}", "t".repeat(120)));
    fs::create_dir_all(&long_tmp).unwrap();
    let out = Command::new(script)
        .args(["-qec", &cmd, "/dev/null"])
        .env("TMPDIR", &long_tmp)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("at-a-terminal"),
        "exit {:?}: {text}{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_to_string(run.join("status")).unwrap(), "0");
    ok(&verb(&top, &run, "cleanup").output().unwrap());
}

/// L2: a git container's look killed with SIGKILL (the verb and its engine, one process group) at any moment of the
/// engine's create, then `cleanup`: the look's container, conmon and engine state go, and nothing is kept.
fn killed_create_case(engine: &str) {
    use std::os::unix::process::CommandExt as _;
    let Some(program) = engine_program(engine) else {
        return;
    };
    let Some((top, run, _, clone)) = clone_world(&format!("{engine}-killed-create"), engine, &program) else {
        return;
    };
    prepare(&top, &run);
    let (mut kept, mut orphans) = (vec![], 0);
    for delay_ms in (0..=600).step_by(25) {
        let mut child = verb(&top, &run, "git")
            .arg(&clone)
            .arg("/clone")
            .arg("--")
            .args(["upload-pack", "--strict", "/clone/.git"])
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(delay_ms));
        let pgid = format!("-{}", child.id());
        let _ = Command::new("kill").args(["-KILL", "--", &pgid]).status();
        let _ = child.wait();
        drop(child.stdin.take());
        let started = Instant::now();
        let out = verb(&top, &run, "cleanup").output().unwrap();
        let took = started.elapsed();
        let looks = run.join("gitlook").display().to_string();
        let left: Vec<String> = fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter_map(|e| fs::read(e.path().join("cmdline")).ok())
            .map(|c| String::from_utf8_lossy(&c).replace('\0', " "))
            .filter(|c| c.contains(&looks))
            .collect();
        if took > Duration::from_secs(3) {
            orphans += 1;
        }
        if !out.status.success() || run.join("gitlook").exists() || !left.is_empty() {
            kept.push(format!(
                "{delay_ms} ms: exit {:?} in {took:?}: {}; left running: {left:?}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ));
            break;
        }
        prepare(&top, &run);
    }
    assert_eq!(
        kept,
        Vec::<String>::new(),
        "a killed create left the look's container"
    );
    // how often the kill fell in the create's window (a conmon whose podman died before recording the container)
    eprintln!("{engine}: {orphans} orphaned creates recovered");
}

#[test]
fn podman_a_look_killed_during_create_is_cleaned_up() {
    killed_create_case("podman");
}

#[test]
fn docker_a_look_killed_during_create_is_cleaned_up() {
    killed_create_case("docker");
}
