//! pot-container: pot's container backend (policy.json/1, "For backend authors").
//!
//! `pot-container [--config CONFIG] probe|prepare|run|after|cleanup RUNDIR [--print]`
//!
//! This slice: `probe`, `prepare` (and `prepare --print`, which writes nothing) and `cleanup`. `run` and `after`
//! refuse. See docs/reference/container.md.

mod engine;
mod host;
mod launch;
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
    /// Run the command in a container (not in this slice: refuses)
    Run { rundir: PathBuf },
    /// Print the run's after event (not in this slice: refuses)
    After { rundir: PathBuf },
    /// Remove what prepare placed in the run directory
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
    for f in &p.files {
        match f {
            RunFile::Dir { name } => dir_new(&rundir.join(name))?,
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

/// Rule 7: remove what prepare placed, whatever state it stopped in. Never fails.
fn cleanup(rundir: &Path) {
    for n in [
        "container.argv",
        "container.env",
        "containers.conf",
        "container.id",
        "passwd",
        "group",
        "hosts",
        "empty",
        "hidden.txt",
        "potcfg/launcher",
    ] {
        let _ = fs::remove_file(rundir.join(n));
    }
    // empty folders only: the rootfs is an overlay's lower layer and emptydir a mask, which the run never writes
    for d in ["potcfg", "rootfs", "emptydir", "dockercfg"] {
        let _ = fs::remove_dir(rundir.join(d));
    }
    // the engine's home: what podman wrote there (its .config), never the run
    let _ = fs::remove_dir_all(rundir.join("enginehome"));
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
    let result = match &cli.verb {
        Verb::Probe { rundir } => {
            // podman's home is an empty folder of pot's, so it reads none of the user's files (audit F2)
            let home = rundir.join("enginehome");
            let ready = cfg.and_then(|c| engine(&c)).and_then(|(e, program)| {
                let found = if program.contains('/') {
                    Path::new(&program).is_file()
                } else {
                    on_path(&program)
                };
                if !found {
                    return Err(format!("{}: can't run {program}: not found", e.name()));
                }
                if e == Engine::Podman {
                    dir_new(&home).map_err(|err| err.to_string())?;
                }
                Ok((e, program))
            });
            let doc = match ready {
                Ok((e, program)) => match probe::assess(
                    e,
                    &probe::facts(e, &program, &engine::probe_set(e, &home.to_string_lossy())),
                ) {
                    Ok(g) => serde_json::json!({ "guarantees": g }),
                    Err(err) => serde_json::json!({ "guarantees": [], "error": err }),
                },
                Err(err) => serde_json::json!({ "guarantees": [], "error": err }),
            };
            let _ = std::io::Write::write_all(&mut std::io::stdout(), format!("{doc}\n").as_bytes());
            Ok(())
        }
        Verb::Prepare { rundir, print } => match cfg {
            Ok(c) => prepare(&c, rundir, *print),
            Err(m) => Err(Refusal::new(&context_of(rundir), m)),
        },
        Verb::Run { rundir } => Err(Refusal::new(
            &context_of(rundir),
            "the container backend's run is not in this slice",
        )),
        Verb::After { rundir } => Err(Refusal::new(
            &context_of(rundir),
            "the container backend's after is not in this slice",
        )),
        Verb::Cleanup { rundir } => {
            cleanup(rundir);
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
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
