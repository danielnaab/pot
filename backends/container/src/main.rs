//! pot-container: pot's container backend (policy.json/1, "For backend authors").
//!
//! `pot-container [--config CONFIG] probe|prepare|run|after|cleanup RUNDIR [--print]`
//!
//! This slice: `probe`, `prepare` (and `prepare --print`, which writes nothing), `run`, `after` and `cleanup`, for
//! runs in place with read access or none. See docs/reference/container.md.

mod engine;
mod host;
mod launch;
mod lifecycle;
mod policy;
mod probe;
mod render;

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use launch::{Config, Launch};
use policy::{Policy, Refusal};
use render::{Engine, Inputs, Plan, RunFile};

#[derive(Parser)]
#[command(
    name = "pot-container",
    version,
    about = "pot's container backend: a run's policy.json carried out with podman or docker"
)]
struct Cli {
    /// The backend's configuration (JSON): the engine, and the store paths this build binds into every run
    #[arg(long, global = true, value_name = "CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    /// Print the guarantees this host's engine proves: { "guarantees": [...] }, or an error beside none
    Probe { rundir: PathBuf },
    /// Check the run's policy.json and launch.json and place the run's files; --print shows the command instead
    Prepare {
        rundir: PathBuf,
        /// Print the engine's command for this run, and write nothing (pot explain --backend-command)
        #[arg(long)]
        print: bool,
    },
    /// Run the command in a container; exit with its status, once the container is gone
    Run { rundir: PathBuf },
    /// Print the run's after event
    After { rundir: PathBuf },
    /// Remove the run's container and what prepare placed in the run directory; name what could not be removed
    Cleanup { rundir: PathBuf },
}

/// Run-chosen text, safe for a terminal: controls as \xNN, the bidi and zero-width format characters as \uNNNN
/// (pot.py's and bwrap.py's `visible`, for the characters a message can hold).
fn visible(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let o = u32::from(c);
        let format_char = matches!(o, 0xad | 0x61c | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 | 0x2066..=0x206f | 0xfeff);
        if c.is_control() {
            let _ = write!(out, "\\x{o:02x}");
        } else if format_char {
            let _ = write!(out, "\\u{o:04x}");
        } else {
            out.push(c);
        }
    }
    out
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// The engine and its program: the config's, else podman when it is on PATH, else docker.
fn engine(cfg: &Config) -> Result<(Engine, String), String> {
    let e = match cfg.engine.as_deref() {
        Some("podman") => Engine::Podman,
        Some("docker") => Engine::Docker,
        Some(other) => {
            return Err(format!(
                "the backend's --config names engine {other:?}: podman or docker"
            ));
        }
        None if cfg.podman.is_some() || on_path("podman") => Engine::Podman,
        None => Engine::Docker,
    };
    let program = match e {
        Engine::Podman => cfg.podman.clone().unwrap_or_else(|| "podman".into()),
        Engine::Docker => cfg.docker.clone().unwrap_or_else(|| "docker".into()),
    };
    Ok((e, program))
}

fn read_config(path: Option<&Path>) -> Result<Config, String> {
    let Some(path) = path else {
        return Ok(Config::default());
    };
    let text = fs::read_to_string(path)
        .map_err(|e| format!("can't read the backend's --config {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| e.to_string())
        .and_then(policy::decode)
        .map_err(|e| format!("the backend's --config {}: {e}", path.display()))
}

fn load(rundir: &Path) -> Result<(Policy, Launch), Refusal> {
    let read = |n: &str| {
        fs::read_to_string(rundir.join(n))
            .map_err(|e| Refusal::new("run", format!("can't read {}: {e}", rundir.join(n).display())))
    };
    let p = policy::parse(&read("policy.json")?)?;
    policy::check_backend(&p)?;
    let l: Launch = serde_json::from_str(&read("launch.json")?)
        .map_err(|e| e.to_string())
        .and_then(policy::decode)
        .map_err(|e| Refusal::new(&p.context, format!("launch.json: {e}")))?;
    Ok((p, l))
}

/// The context's name, for a refusal before or without a valid policy.
fn context_of(rundir: &Path) -> String {
    fs::read_to_string(rundir.join("policy.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("context").and_then(|c| c.as_str()).map(String::from))
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "run".into())
}

fn plan(cfg: &Config, rundir: &Path) -> Result<(Policy, Launch, Plan), Refusal> {
    let (p, l) = load(rundir)?;
    let refuse = |m: String| Refusal::new(&p.context, m);
    let paths = cfg.paths().map_err(refuse)?;
    let (engine, program) = engine(cfg).map_err(refuse)?;
    let base = match &cfg.base_closure {
        Some(f) => fs::read_to_string(f)
            .map_err(|e| refuse(format!("can't read the base closure {f}: {e}")))?
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        None => vec![],
    };
    let abs = fs::canonicalize(rundir)
        .map_err(|e| refuse(format!("the run directory {}: {e}", rundir.display())))?;
    let image = cfg.image.clone().unwrap_or_else(|| render::DOCKER_IMAGE.into());
    let inputs = Inputs {
        policy: &p,
        launch: &l,
        paths: &paths,
        engine,
        program: &program,
        image: &image,
        rundir: &abs.to_string_lossy(),
        base_closure: &base,
        host: &host::RealHost,
    };
    let plan = render::render(&inputs).map_err(|gaps| {
        refuse(format!(
            "the container backend does not carry out yet: {}",
            gaps.join("; ")
        ))
    })?;
    Ok((p, l, plan))
}

/// A word as a POSIX shell reads it back.
fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn print_plan(engine: &str, p: &Plan) -> String {
    let mut out =
        format!("# the {engine} command for this run, an option a line (pot explain --backend-command)\n");
    let _ = writeln!(
        out,
        "# the engine runs with only these of the caller's variables: {}; and {}",
        p.engine_keep.join(" "),
        p.engine_set
            .iter()
            .map(|(k, v)| format!("{k}={}", quote(v)))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let words = |a: &[String]| a.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ");
    let _ = writeln!(out, "{}", quote(&p.program));
    let mut closure = 0;
    for o in &p.opts {
        if o.closure {
            closure += 1;
        } else {
            let _ = writeln!(out, "  {}", words(&o.args));
        }
    }
    let _ = writeln!(out, "  {}", words(&p.root));
    let _ = writeln!(out, "  {}", words(&p.entry));
    if closure > 0 {
        let _ = writeln!(
            out,
            "# and before the root: --volume P:P:ro for each of {closure} store paths (the closure)"
        );
    }
    out
}

/// An error that names its path.
fn at(path: &Path) -> impl Fn(std::io::Error) -> std::io::Error + '_ {
    move |e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

/// Write PATH afresh: whatever is there is removed first (a file pot left read-only, or a link someone planted),
/// then a new file is created exclusively, which never follows a link (audit F9).
fn write_new(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "a folder is in the way",
            ))
            .map_err(at(path));
        }
        Ok(_) => fs::remove_file(path).map_err(at(path))?,
        Err(_) => {}
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .map_err(at(path))?;
    f.write_all(bytes).map_err(at(path))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(at(path))
}

/// A folder of the run's: created, or kept when it is a real folder; a link there is refused.
fn dir_new(path: &Path) -> std::io::Result<()> {
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "not a folder",
        ))
        .map_err(at(path)),
        Err(_) => fs::create_dir(path).map_err(at(path)),
    }
}

fn place(rundir: &Path, p: &Plan) -> std::io::Result<()> {
    // the engines refuse a cidfile that exists: one left from before goes (a link: the link)
    let cid = rundir.join("container.id");
    if cid.symlink_metadata().is_ok() {
        fs::remove_file(&cid).map_err(at(&cid))?;
    }
    for f in &p.files {
        match f {
            RunFile::Dir { name } => dir_new(&rundir.join(name))?,
            RunFile::FreshDir { name } => {
                let path = rundir.join(name);
                match path.symlink_metadata() {
                    Ok(m) if m.is_dir() => fs::remove_dir_all(&path).map_err(at(&path))?,
                    Ok(_) => fs::remove_file(&path).map_err(at(&path))?,
                    Err(_) => {}
                }
                fs::create_dir(&path).map_err(at(&path))?;
            }
            RunFile::File { name, text, mode } => write_new(&rundir.join(name), text.as_bytes(), *mode)?,
            RunFile::Link { name, target } => {
                let path = rundir.join(name);
                if path.symlink_metadata().is_ok() {
                    fs::remove_file(&path).map_err(at(&path))?;
                }
                std::os::unix::fs::symlink(target, &path).map_err(at(&path))?;
            }
        }
    }
    for d in &p.host_dirs {
        fs::create_dir_all(d).map_err(at(Path::new(d)))?;
    }
    let mut argv = Vec::new();
    for a in p.argv() {
        argv.extend_from_slice(a.as_bytes());
        argv.push(0);
    }
    write_new(&rundir.join("container.argv"), &argv, 0o600)?;
    // run starts the engine with only these variables of its own, then these set (no value of a secret is here)
    let set: std::collections::BTreeMap<_, _> = p.engine_set.iter().cloned().collect();
    let env = serde_json::json!({ "keep": p.engine_keep, "set": set });
    write_new(
        &rundir.join("container.env"),
        format!("{env}\n").as_bytes(),
        0o600,
    )
}

fn prepare(cfg: &Config, rundir: &Path, print: bool) -> Result<(), Refusal> {
    let (p, l, plan) = plan(cfg, rundir)?;
    if print {
        let (engine, _) = engine(cfg).map_err(|m| Refusal::new(&p.context, m))?;
        print!("{}", print_plan(engine.name(), &plan));
        return Ok(());
    }
    // podman's keep-id keeps the caller's id; --user must name the same one, or the run writes as another user
    // every bind's source exists (docker would create a missing one, as root)
    let missing = render::missing_sources(
        &plan,
        &fs::canonicalize(rundir).unwrap_or_default().to_string_lossy(),
        &host::RealHost,
    );
    if !missing.is_empty() {
        return Err(Refusal::new(
            &p.context,
            format!("a bind's source does not exist: {}", missing.join(", ")),
        ));
    }
    if host::euid() != Some(l.uid) {
        return Err(Refusal::new(
            &p.context,
            format!("launch.json's uid {} is not this process's", l.uid),
        ));
    }
    place(rundir, &plan).map_err(|e| Refusal::new(&p.context, format!("can't place the run's files: {e}")))
}

/// A file of the run's, read only when it is a regular file: never through a link planted in the run folder.
fn read_own(path: &Path) -> std::io::Result<Vec<u8>> {
    let m = path.symlink_metadata().map_err(at(path))?;
    if !m.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a regular file",
        ))
        .map_err(at(path));
    }
    fs::read(path).map_err(at(path))
}

/// The id the engine wrote to `container.id`, when it did.
fn container_id(rundir: &Path) -> Option<String> {
    let text = read_own(&rundir.join("container.id")).ok()?;
    let id = String::from_utf8(text).ok()?.trim().to_string();
    lifecycle::is_cid(&id).then_some(id)
}

fn is_real_dir(p: &Path) -> bool {
    p.symlink_metadata().is_ok_and(|m| m.is_dir())
}

/// How long `run` and `cleanup` wait for the engine's processes for a run to end.
const ENGINE_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The host's processes, but this one, whose arguments name the run folder's engine state.
fn engine_processes(rundir: &str) -> Vec<String> {
    let me = std::process::id().to_string();
    let Ok(entries) = fs::read_dir("/proc") else {
        return vec![];
    };
    entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|pid| pid.bytes().all(|b| b.is_ascii_digit()) && *pid != me)
        .filter(|pid| {
            fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| lifecycle::names_engine_state(&c, rundir))
        })
        .collect()
}

/// Remove the run's container through the engine, and wait until no process of the engine's for this run is left
/// (conmon and its exit command outlive `podman run`, N7). docker's daemon removes the container before `rm` returns.
fn remove_container(e: Engine, program: &str, rundir: &str, set: &[(String, String)]) -> Result<(), String> {
    let cid = container_id(Path::new(rundir));
    if let Some(argv) = lifecycle::remove_argv(e, program, rundir, cid.as_deref()) {
        let out = engine::command(&argv[0], &[], set)
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|err| format!("can't run {program} to remove the run's container: {err}"))?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "{} rm for {rundir} exited {}: {}",
                e.name(),
                lifecycle::exit_code(out.status),
                why.lines().next().unwrap_or("")
            ));
        }
    }
    if e == Engine::Docker {
        return Ok(());
    }
    let start = std::time::Instant::now();
    loop {
        let left = engine_processes(rundir);
        if left.is_empty() {
            return Ok(());
        }
        if start.elapsed() > ENGINE_WAIT {
            return Err(format!(
                "the engine's processes for {rundir} still run after {} s (pids {})",
                ENGINE_WAIT.as_secs(),
                left.join(" ")
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// docker's root: the config's image, which must be pot's empty one (one empty layer); imported from an empty tar
/// when it is missing. Its name is content-stable, so runs share it and `cleanup` leaves it.
fn docker_image(program: &str, set: &[(String, String)], image: &str) -> Result<(), String> {
    use std::io::Write as _;
    let layers = || {
        let out = engine::command(program, &[], set)
            .args(["image", "inspect", "--format", "{{json .RootFS.Layers}}", image])
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| serde_json::from_slice::<Vec<String>>(&out.stdout).ok())
            .flatten()
    };
    let have = if let Some(l) = layers() {
        l
    } else {
        let mut child = engine::command(program, &[], set)
            .args(["import", "-", image])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| format!("can't run {program} to import the image {image}: {err}"))?;
        // an empty tar: two zero blocks
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&[0u8; 1024]);
        }
        let out = child
            .wait_with_output()
            .map_err(|err| format!("docker import {image}: {err}"))?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "docker import {image} failed: {}",
                why.lines().next().unwrap_or("")
            ));
        }
        layers().unwrap_or_default()
    };
    if have != [lifecycle::EMPTY_LAYER] {
        return Err(format!(
            "the image {image} is not pot's empty image (one empty layer): remove it, or name another in the backend's --config"
        ));
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct EngineEnv {
    keep: Vec<String>,
    set: std::collections::BTreeMap<String, String>,
}

/// Run the command `prepare` wrote, with the engine environment it recorded; return the command's status once the
/// container is removed and the engine's processes for the run have ended. SIGTERM and SIGHUP stop the container
/// (the engine's stop: SIGTERM, then SIGKILL after 10 s); SIGINT is the command's (the terminal sends it to the
/// engine too, which passes it on), as bwrap's run ignores it.
fn run(cfg: &Config, rundir: &Path) -> Result<u8, Refusal> {
    use std::io::IsTerminal as _;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let ctx = context_of(rundir);
    let refuse = |m: String| Refusal::new(&ctx, m);
    let abs = fs::canonicalize(rundir)
        .map_err(|e| refuse(format!("the run directory {}: {e}", rundir.display())))?;
    let r = abs.to_string_lossy().into_owned();
    let (e, program) = engine(cfg).map_err(refuse)?;
    let argv: Vec<String> = read_own(&abs.join("container.argv"))
        .map_err(|err| refuse(format!("{err} (run prepare first)")))?
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let env: EngineEnv = read_own(&abs.join("container.env"))
        .map_err(|err| err.to_string())
        .and_then(|t| serde_json::from_slice(&t).map_err(|err| format!("container.env: {err}")))
        .map_err(refuse)?;
    if argv.first() != Some(&program) {
        return Err(refuse(format!(
            "container.argv was not prepared for {program}: run prepare again"
        )));
    }
    let set: Vec<(String, String)> = env.set.into_iter().collect();
    if e == Engine::Docker {
        let image = cfg.image.clone().unwrap_or_else(|| render::DOCKER_IMAGE.into());
        if !argv.contains(&image) {
            return Err(refuse(format!(
                "container.argv does not run the image {image}: run prepare again"
            )));
        }
        docker_image(&program, &set, &image).map_err(refuse)?;
    }
    let argv = lifecycle::with_tty(&argv, std::io::stdin().is_terminal());

    let stop = Arc::new(AtomicBool::new(false));
    let int = Arc::new(AtomicBool::new(false));
    for (sig, flag) in [
        (signal_hook::consts::SIGTERM, &stop),
        (signal_hook::consts::SIGHUP, &stop),
        (signal_hook::consts::SIGINT, &int),
    ] {
        signal_hook::flag::register(sig, Arc::clone(flag))
            .map_err(|err| refuse(format!("can't handle signal {sig}: {err}")))?;
    }
    let mut child = engine::command(&argv[0], &env.keep, &set)
        .args(&argv[1..])
        .spawn()
        .map_err(|err| refuse(format!("can't run {program}: {err}")))?;
    let mut stopping = false;
    let mut stopper = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(err) => return Err(refuse(format!("waiting for {program}: {err}"))),
        }
        if stop.load(Ordering::Relaxed) && !stopping {
            stopping = true;
            match container_id(&abs) {
                Some(cid) => {
                    let a = lifecycle::stop_argv(e, &program, &r, &cid);
                    stopper = engine::command(&a[0], &[], &set)
                        .args(&a[1..])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .spawn()
                        .ok();
                }
                // no container yet: the engine is still creating it; whatever it made, remove_container removes
                None => {
                    let _ = child.kill();
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    if let Some(mut s) = stopper {
        let _ = s.wait();
    }
    let code = lifecycle::exit_code(status);
    let gone = remove_container(e, &program, &r, &set);
    write_new(&abs.join("status"), code.to_string().as_bytes(), 0o644)
        .map_err(|err| refuse(format!("can't record the status: {err}")))?;
    if let Err(m) = gone {
        eprintln!("{}", visible(&refuse(m).to_string()));
    }
    Ok(code)
}

fn after(rundir: &Path) -> Result<(), Refusal> {
    let (p, l) = load(rundir)?;
    let ev = lifecycle::after_event(&p, &l)
        .map_err(|m| Refusal::new(&p.context, format!("the container backend's after: {m}")))?;
    let _ = std::io::Write::write_all(&mut std::io::stdout(), format!("{ev}\n").as_bytes());
    Ok(())
}

/// Remove PATH: a folder with what is in it, or a file or link (never followed). Missing is done.
fn remove_tree(path: &Path) -> std::io::Result<()> {
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    }
}

/// A new private folder (0700) in the temporary directory, which did not exist: never one another user placed.
fn private_dir(what: &str) -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("pot-container-{what}-{}-{nanos}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|err| format!("{}: {err}", dir.display()))?;
    Ok(dir)
}

/// Remove PATHS inside podman's user namespace, where the ids a container wrote with are the user's to remove. The
/// podman that does it keeps its own store in a private folder, so it never opens (or mounts) the run's.
fn unshare_rm(program: &str, paths: &[PathBuf]) -> Result<(), String> {
    let dir = private_dir("unshare")?;
    let d = dir.to_string_lossy().into_owned();
    let made = fs::create_dir(dir.join("home")).and_then(|()| {
        write_new(
            &dir.join("storage.conf"),
            engine::storage_conf(&format!("{d}/enginestore")).as_bytes(),
            0o444,
        )
    });
    let result = made.map_err(|err| err.to_string()).and_then(|()| {
        let out = engine::command(program, &[], &engine::probe_set(Engine::Podman, &d))
            .args(render::podman_globals(&d))
            .args(["unshare", "rm", "-rf", "--"])
            .args(paths)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|err| format!("can't run {program} unshare: {err}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{program} unshare rm exited {}",
                lifecycle::exit_code(out.status)
            ))
        }
    });
    let removed = remove_tree(&dir).map_err(|err| format!("{}: {err}", dir.display()));
    result.and(removed)
}

/// cleanup's first part: the run's container, removed through the engine, then the engine's state (store, runroot,
/// temporary files). Returns what it could not remove.
fn cleanup_engine(cfg: Result<&Config, &String>, abs: &Path) -> Vec<String> {
    let r = abs.to_string_lossy().into_owned();
    let rd = |n: &str| abs.join(n);
    let mut failed: Vec<String> = vec![];
    let program = |e: Engine| -> Result<String, String> {
        let c = cfg.map_err(Clone::clone)?;
        Ok(match e {
            Engine::Podman => c.podman.clone().unwrap_or_else(|| "podman".into()),
            Engine::Docker => c.docker.clone().unwrap_or_else(|| "docker".into()),
        })
    };
    let own_file = |n: &str| {
        rd(n)
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_file())
            .then(|| rd(n).to_string_lossy().into_owned())
    };

    // the container: podman's in the run's own store (which exists only once the engine has started)
    let store = rd("enginestore");
    if is_real_dir(&store) && fs::read_dir(&store).is_ok_and(|mut d| d.next().is_some()) {
        let home = rd("enginehome");
        if !is_real_dir(&home) {
            let _ = remove_tree(&home);
            let _ = fs::create_dir(&home);
        }
        let set = vec![
            (
                "CONTAINERS_CONF".to_string(),
                own_file("containers.conf").unwrap_or_else(|| "/dev/null".into()),
            ),
            (
                "CONTAINERS_STORAGE_CONF".to_string(),
                own_file("storage.conf").unwrap_or_else(|| "/dev/null".into()),
            ),
            ("HOME".to_string(), home.to_string_lossy().into_owned()),
        ];
        if let Err(m) = program(Engine::Podman).and_then(|p| remove_container(Engine::Podman, &p, &r, &set)) {
            failed.push(m);
        }
    }
    // docker's, which its cidfile names
    if is_real_dir(&rd("dockercfg")) && container_id(abs).is_some() {
        let set = vec![(
            "DOCKER_CONFIG".to_string(),
            rd("dockercfg").to_string_lossy().into_owned(),
        )];
        if let Err(m) = program(Engine::Docker).and_then(|p| remove_container(Engine::Docker, &p, &r, &set)) {
            failed.push(m);
        }
    }

    // the engine's state; what the user can't remove (another id of the user namespace) goes through podman unshare
    let state: Vec<PathBuf> = ["enginestore", "engineroot", "enginetmp"]
        .iter()
        .map(|n| rd(n))
        .collect();
    let mut theirs = vec![];
    for p in &state {
        match remove_tree(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => theirs.push(p.clone()),
            Err(e) => failed.push(format!("{}: {e}", p.display())),
        }
    }
    if !theirs.is_empty() {
        let unshared = program(Engine::Podman).and_then(|p| unshare_rm(&p, &theirs));
        for p in &theirs {
            if p.symlink_metadata().is_ok() {
                let why = unshared
                    .as_ref()
                    .err()
                    .map_or("permission denied", String::as_str);
                failed.push(format!("{}: {why}", p.display()));
            }
        }
        // the run's state is gone, but not unshare's own private folder
        if theirs.iter().all(|p| p.symlink_metadata().is_err()) {
            if let Err(m) = unshared {
                failed.push(m);
            }
        }
    }
    failed
}

/// Rule 7 (and N6): remove the run's container through the engine, then its store and what prepare placed, whatever
/// state the run stopped in; never following a link planted in the run folder. Every removal it could not make is
/// named in one refusal; what it removed stays removed, so it can run again.
fn cleanup(cfg: Result<&Config, &String>, rundir: &Path) -> Result<(), Refusal> {
    let ctx = context_of(rundir);
    let abs = match fs::canonicalize(rundir) {
        Ok(a) => a,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(Refusal::new(
                &ctx,
                format!("the run directory {}: {e}", rundir.display()),
            ));
        }
    };
    let rd = |n: &str| abs.join(n);
    let mut failed = cleanup_engine(cfg, &abs);
    for n in ["enginehome", "dockercfg"] {
        if let Err(e) = remove_tree(&rd(n)) {
            failed.push(format!("{}: {e}", rd(n).display()));
        }
    }
    // pot's launcher link, only inside a real potcfg folder (never through a link in its place)
    if is_real_dir(&rd("potcfg")) {
        let l = rd("potcfg").join("launcher");
        match fs::remove_file(&l) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                failed.push(format!("{}: {e}", l.display()));
            }
            _ => {}
        }
    }
    for n in [
        "container.argv",
        "container.env",
        "containers.conf",
        "storage.conf",
        "container.id",
        "passwd",
        "group",
        "hosts",
        "empty",
        "hidden.txt",
    ] {
        match fs::remove_file(rd(n)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                failed.push(format!("{}: {e}", rd(n).display()));
            }
            _ => {}
        }
    }
    // empty folders only: the rootfs is an overlay's lower layer and emptydir a mask, which the run never writes;
    // anything in one is reported, never removed. A link in a folder's place is removed, not followed.
    for n in ["potcfg", "rootfs", "emptydir"] {
        let p = rd(n);
        let removed = match p.symlink_metadata() {
            Err(_) => Ok(()),
            Ok(m) if m.is_dir() => fs::remove_dir(&p),
            Ok(_) => fs::remove_file(&p),
        };
        if let Err(e) = removed {
            failed.push(format!("{}: {e}", p.display()));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(Refusal::new(
            &ctx,
            format!("cleanup could not remove: {}", failed.join("; ")),
        ))
    }
}

/// probe's engine, in a temporary folder of its own (podman's home, its storage.conf and store), removed after:
/// probe leaves nothing in the folder pot gives it, often pot's state (audit N3).
fn probe_now(e: Engine, program: &str) -> Result<Vec<&'static str>, String> {
    use std::os::unix::fs::DirBuilderExt;
    let found = if program.contains('/') {
        Path::new(program).is_file()
    } else {
        on_path(program)
    };
    if !found {
        return Err(format!("{}: can't run {program}: not found", e.name()));
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("pot-container-probe-{}-{nanos}", std::process::id()));
    // a new folder, 0700, that did not exist: never one another user placed
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|err| format!("{}: {err}", dir.display()))?;
    let d = dir.to_string_lossy().into_owned();
    let made = fs::create_dir(dir.join("home")).and_then(|()| {
        write_new(
            &dir.join("storage.conf"),
            engine::storage_conf(&format!("{d}/store")).as_bytes(),
            0o444,
        )
    });
    let result = made
        .map_err(|err| err.to_string())
        .and_then(|()| probe::assess(e, &probe::facts(e, program, &engine::probe_set(e, &d))));
    let _ = fs::remove_dir_all(&dir);
    result
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    let cfg = read_config(cli.config.as_deref());
    let mut exit = 0;
    let result = match &cli.verb {
        Verb::Probe { .. } => {
            let doc = match cfg
                .and_then(|c| engine(&c))
                .and_then(|(e, program)| probe_now(e, &program))
            {
                Ok(g) => serde_json::json!({ "guarantees": g }),
                Err(err) => serde_json::json!({ "guarantees": [], "error": err }),
            };
            let _ = std::io::Write::write_all(&mut std::io::stdout(), format!("{doc}\n").as_bytes());
            Ok(())
        }
        Verb::Prepare { rundir, print } => match cfg {
            Ok(c) => prepare(&c, rundir, *print),
            Err(m) => Err(Refusal::new(&context_of(rundir), m)),
        },
        Verb::Run { rundir } => match &cfg {
            Ok(c) => run(c, rundir).map(|code| exit = code),
            Err(m) => Err(Refusal::new(&context_of(rundir), m.clone())),
        },
        Verb::After { rundir } => after(rundir),
        Verb::Cleanup { rundir } => cleanup(cfg.as_ref(), rundir),
    };
    match result {
        Ok(()) => ExitCode::from(exit),
        Err(r) => {
            eprintln!("{}", visible(&r.to_string()));
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_escapes_controls_and_bidi() {
        assert_eq!(visible("a\nb\u{1b}[31m\u{202e}c"), "a\\x0ab\\x1b[31m\\u202ec");
    }

    #[test]
    fn quote_round_trips_through_a_shell() {
        assert_eq!(quote("/nix/store/x-y:z,ro"), "/nix/store/x-y:z,ro");
        assert_eq!(quote("PS1=(pot:work) \\w \\$ "), "'PS1=(pot:work) \\w \\$ '");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }
}
