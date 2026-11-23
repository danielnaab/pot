//! pot-container: pot's container backend (policy.json/1, "For backend authors").
//!
//! `pot-container [--config CONFIG] probe|prepare|run|after|cleanup RUNDIR [--print]`
//!
//! This slice: `probe`, `prepare` (and `prepare --print`, which writes nothing), `run`, `after` and `cleanup`, for
//! runs in place with read access or none. See docs/reference/container.md.

mod bindings;
mod engine;
mod host;
mod launch;
mod lifecycle;
mod policy;
mod probe;
mod render;

use std::fmt::Write as _;
use std::fs;
use std::os::unix::ffi::OsStrExt as _;
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
    // 0.3.0: a pin pot's core could not make (a link, a hard link) stops the run, with its message (bwrap's)
    if p.access == "write" && p.mode == "in-place" {
        if let Some(r) = l.pins.iter().flatten().find_map(|w| w.refused.as_ref()) {
            return Err(refuse(r.message.clone()));
        }
    }
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
    let net = cfg.net_tools().map_err(refuse)?;
    let look = cfg.look_tools();
    let broker = cfg.broker_tools().map_err(refuse)?;
    let inputs = Inputs {
        policy: &p,
        launch: &l,
        paths: &paths,
        engine,
        program: &program,
        image: &image,
        rundir: &abs.to_string_lossy(),
        base_closure: &base,
        net: net.as_ref(),
        look: look.as_ref(),
        broker: broker.as_ref(),
        nix_socat: cfg.socat.as_deref(),
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
    if let Some(proxy) = &p.proxy {
        let _ = writeln!(
            out,
            "# before the engine, run starts pot's proxy in the run folder: {}",
            words(proxy)
        );
    }
    if let Some(broker) = &p.broker {
        let _ = writeln!(
            out,
            "# before the engine, run starts pot's broker in the run folder: {}",
            words(broker)
        );
    }
    if let (Some(relay), Some(script)) = (&p.nix_relay, &p.nix_script) {
        let _ = writeln!(
            out,
            "# before the engine, run starts the nix relay in the run folder: {}; each connection runs ./{}: {}",
            words(relay),
            render::NIX_SCRIPT,
            script.lines().last().unwrap_or_default()
        );
    }
    if !p.creds.is_empty() {
        // bwrap's print_command: a via-file variable's value is its file's path; another's, "(the run holds it)"
        let files = bindings::via_file(&p.creds);
        let shown: Vec<String> = bindings::names(&p.creds)
            .iter()
            .map(|n| {
                if files.contains_key(n) {
                    format!("{n}={}/{n}", p.secrets_at)
                } else {
                    format!("{n}=(the run holds it)")
                }
            })
            .collect();
        let _ = writeln!(
            out,
            "# the bindings' values, read on the host when run starts, go over the named pipe: {}",
            shown.join(" ")
        );
        if !files.is_empty() {
            let _ = writeln!(
                out,
                "# a via-file value is written to the run folder's secrets/NAME (0400), removed when the command ends"
            );
        }
    }
    if p.fifo {
        let _ = writeln!(
            out,
            "# and writes the variables passed by name to the run folder's named pipe, secrets.fifo: {}",
            if p.piped.is_empty() {
                "none".to_string()
            } else {
                p.piped.join(" ")
            }
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

/// What `run` makes in the run folder just before the engine starts, and removes when the command ends: the named pipe
/// for the variables, the proxy's socket, the bindings' sockets and the nix relay's script. `prepare` records no kind
/// for them.
const RUNTIME: &[&str] = &[
    "secrets.fifo",
    "proxy.sock",
    "relay.sock",
    "git.sock",
    "nix.sock",
    "nix-relay",
];

/// A workspace path the plan binds, as `prepare` found it: `run` checks it again just before the engine starts.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PinRecord {
    path: String,
    kind: launch::Kind,
    dev: u64,
    ino: u64,
    /// X3: a file's size and status-change time too, so a file replaced by one that reuses its inode (or rewritten in
    /// place) is not taken for it.
    #[serde(default)]
    size: u64,
    #[serde(default)]
    ctime: (i64, i64),
    /// A protected file: it must have no second hard link.
    single: bool,
}

/// What identifies a pin: its device and inode, and for a file its size and status-change time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    size: u64,
    ctime: (i64, i64),
}

/// What is at PATH now, never through a link: (kind, identity, links), or why it is no pin. A folder's identity is its
/// device and inode (its size and times change with its entries).
fn pin_now(path: &str) -> Result<(launch::Kind, Identity, u64), &'static str> {
    use std::os::unix::fs::MetadataExt as _;
    let m = Path::new(path).symlink_metadata().map_err(|_| "missing")?;
    if m.file_type().is_symlink() {
        return Err("a link");
    }
    let (kind, size, ctime) = if m.is_dir() {
        (launch::Kind::Folder, 0, (0, 0))
    } else {
        (launch::Kind::File, m.size(), (m.ctime(), m.ctime_nsec()))
    };
    Ok((
        kind,
        Identity {
            dev: m.dev(),
            ino: m.ino(),
            size,
            ctime,
        },
        m.nlink(),
    ))
}

/// Each pin as it should be: KIND, no link, and (SINGLE) no second hard link; with RECORDED, the same identity.
fn pin_gap(path: &str, kind: launch::Kind, single: bool, recorded: Option<Identity>) -> Option<String> {
    let why = match pin_now(path) {
        Err(why) => why.to_string(),
        Ok((k, _, _)) if k != kind => format!("not the {kind:?} it was").to_lowercase(),
        Ok((_, id, _)) if recorded.is_some_and(|r| r != id) => "another file, or changed".into(),
        Ok((launch::Kind::File, _, n)) if single && n > 1 => "a second hard link".into(),
        Ok(_) => return None,
    };
    Some(format!("{path} ({why})"))
}

/// The placeholders, created before anything binds them: exclusively, never through a link; a file's path and text
/// recorded in `placeholders.json` (bwrap's), for cleanup, even when one fails.
fn make_placeholders(rundir: &Path, p: &Plan) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut placed: Vec<(String, String)> = vec![];
    let mut result = Ok(());
    for ph in &p.placeholders {
        let path = Path::new(&ph.path);
        let made = match ph.kind {
            launch::Kind::Folder => fs::create_dir(path),
            launch::Kind::File => fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .and_then(|mut f| f.write_all(ph.text.as_deref().unwrap_or_default().as_bytes())),
        };
        match made {
            Ok(()) if ph.kind == launch::Kind::File => {
                placed.push((ph.path.clone(), ph.text.clone().unwrap_or_default()));
            }
            Ok(()) => {}
            Err(e) => {
                result = Err(std::io::Error::new(
                    e.kind(),
                    format!(
                        "{} (it appeared after pot resolved the workspace's pins: run again): {e}",
                        ph.path
                    ),
                ));
                break;
            }
        }
    }
    if !placed.is_empty() {
        write_new(
            &rundir.join("placeholders.json"),
            serde_json::to_string(&placed).unwrap_or_default().as_bytes(),
            0o600,
        )?;
    }
    result
}

#[allow(clippy::too_many_lines)] // what prepare places, in order, each with what run checks again
fn place(rundir: &Path, p: &Plan, single: &std::collections::BTreeSet<String>) -> std::io::Result<()> {
    // the engines refuse a cidfile that exists: one left from before goes (a link: the link); so does what an earlier
    // run left of its own (never a folder)
    for n in std::iter::once(&"container.id").chain(RUNTIME) {
        let f = rundir.join(n);
        if f.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
            fs::remove_file(&f).map_err(at(&f))?;
        }
    }
    for f in &p.files {
        match f {
            RunFile::Dir { name } => dir_new(&rundir.join(name))?,
            RunFile::FreshDir { name } => {
                let path = rundir.join(name);
                remove_tree_writable(&path).map_err(at(&path))?;
                fs::create_dir(&path).map_err(at(&path))?;
            }
            RunFile::File { name, text, mode } => write_new(&rundir.join(name), text.as_bytes(), *mode)?,
            RunFile::PrivateDir { name } => {
                use std::os::unix::fs::DirBuilderExt as _;
                let path = rundir.join(name);
                remove_tree(&path).map_err(at(&path))?;
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&path)
                    .map_err(at(&path))?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(at(&path))?;
            }
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
    // write access: the placeholders, then every pin's inode as it is now, for run to check again
    make_placeholders(rundir, p)?;
    let mut pins = vec![];
    let mut listed = String::new();
    for (path, kind) in &p.pins {
        let (_, id, _) = pin_now(path)
            .map_err(|why| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{path}: {why}")))?;
        pins.push(PinRecord {
            path: path.clone(),
            kind: *kind,
            dev: id.dev,
            ino: id.ino,
            size: id.size,
            ctime: id.ctime,
            single: single.contains(path),
        });
    }
    // X1, Y1: what the entry checks inside the container: each bind's destination, with its source's device and inode
    // as it is now (the placeholders and the run's own files made)
    for (dest, src) in &p.checks {
        let (_, id, _) = pin_now(src)
            .map_err(|why| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{src}: {why}")))?;
        let _ = writeln!(listed, "{}:{} {dest}", id.dev, id.ino);
    }
    // A1: the shims in bin/ (no mount points: their device, inode and one link) and, for run to check again before
    // the engine, bin/ and each shim as prepare made them
    let mut own_ids = vec![];
    for (dest, src) in &p.file_checks {
        let (_, id, _) = pin_now(src)
            .map_err(|why| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{src}: {why}")))?;
        let _ = writeln!(listed, "={}:{}:1 {dest}", id.dev, id.ino);
    }
    let abs_dir = fs::canonicalize(rundir).map_err(at(rundir))?;
    let checked_own = p
        .checks
        .iter()
        .chain(&p.file_checks)
        .filter(|(_, src)| Path::new(src).starts_with(&abs_dir));
    for (_, src) in checked_own {
        let (kind, id, _) = pin_now(src)
            .map_err(|why| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{src}: {why}")))?;
        own_ids.push(PinRecord {
            path: src.clone(),
            kind,
            dev: id.dev,
            ino: id.ino,
            size: id.size,
            ctime: id.ctime,
            single: kind == launch::Kind::File,
        });
    }
    if p.stat.is_some() {
        write_new(&rundir.join("binds.txt"), listed.as_bytes(), 0o444)?;
    }
    let mut argv = Vec::new();
    for a in p.argv() {
        argv.extend_from_slice(a.as_bytes());
        argv.push(0);
    }
    write_new(&rundir.join("container.argv"), &argv, 0o600)?;
    // S4: every file and folder of the run's that the plan binds, by kind, for run to check again
    let abs = fs::canonicalize(rundir).map_err(at(rundir))?;
    let prefix = format!("{}/", abs.display());
    let mut own = std::collections::BTreeMap::new();
    for src in p
        .sources
        .iter()
        .filter(|s| s.starts_with(&prefix) && !RUNTIME.contains(&&s[prefix.len()..]))
    {
        let path = Path::new(src);
        let kind = match path.symlink_metadata() {
            Ok(m) if m.is_dir() => "dir",
            Ok(m) if m.file_type().is_file() => "file",
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "a file the run binds is a link, or missing",
                ))
                .map_err(at(path));
            }
        };
        own.insert(src[prefix.len()..].to_string(), kind);
    }
    // run starts the engine with only these variables of its own, then these set (no value of a secret is here)
    let set: std::collections::BTreeMap<_, _> = p.engine_set.iter().cloned().collect();
    let env = serde_json::json!({ "keep": p.engine_keep, "set": set, "piped": p.piped, "own": own, "defaults": p.defaults,
        "fifo": p.fifo, "proxy": p.proxy, "credentials": p.creds, "hostless": p.hostless, "secretsAt": p.secrets_at,
        "pins": pins, "broker": p.broker, "nixRelay": p.nix_relay, "nixScript": p.nix_script, "clear": p.clear,
        "ownIds": own_ids, "socketChecks": p.socket_checks });
    write_new(
        &rundir.join("container.env"),
        format!("{env}\n").as_bytes(),
        0o600,
    )
}

fn prepare(cfg: &Config, rundir: &Path, print: bool) -> Result<(), Refusal> {
    // what a run killed with SIGKILL left of the via-file bindings' values goes first, whatever prepare decides (a link
    // there is removed, never followed)
    if !print {
        let s = rundir.join("secrets");
        remove_tree(&s)
            .map_err(|e| Refusal::new(&context_of(rundir), format!("can't remove {}: {e}", s.display())))?;
    }
    let (p, l, plan) = plan(cfg, rundir)?;
    // bwrap's run stops so when a binding's host program is missing (launch-1: before the command starts); --print
    // too (A5), rather than show a command prepare would not place
    if let Some(stop) = &plan.stop {
        return Err(Refusal::new(&p.context, stop.clone()));
    }
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
    let mut gaps = render::program_gaps(
        &plan,
        &cfg.paths().map_err(|m| Refusal::new(&p.context, m))?.shell,
        &host::RealHost,
    );
    // V2: the network and private-proxy on probe's own test of the network's programs
    if plan.proxy.is_some() || p.proves("private-proxy") {
        if let Some(t) = cfg.net_tools().map_err(|m| Refusal::new(&p.context, m))? {
            gaps.extend(render::net_gaps(&t, &host::RealHost));
        }
    }
    if plan.broker.is_some() {
        if let Some(t) = cfg.broker_tools().map_err(|m| Refusal::new(&p.context, m))? {
            gaps.extend(render::broker_gaps(&t, &plan, &host::RealHost));
        }
    }
    if let (Some(relay), Some(socat)) = (&plan.nix_relay, &cfg.socat) {
        if !fs::metadata(socat).is_ok_and(|m| m.is_file()) || relay.first() != Some(socat) {
            gaps.push(format!(
                "the config's socat {socat} (the nix relay runs it on the host: it must be a file)"
            ));
        }
    }
    if !gaps.is_empty() {
        return Err(Refusal::new(&p.context, gaps.join("; ")));
    }
    if host::euid() != Some(l.uid) {
        return Err(Refusal::new(
            &p.context,
            format!("launch.json's uid {} is not this process's", l.uid),
        ));
    }
    // write access: each pin is still as pot's core resolved it (no link, the same kind, a protected file with no
    // second hard link), and no placeholder is there yet
    let single: std::collections::BTreeSet<String> = l
        .pins
        .iter()
        .flatten()
        .flat_map(|w| w.ro.iter())
        .filter(|r| r.kind == launch::Kind::File)
        .map(|r| r.path.clone())
        .collect();
    let mut changed = vec![];
    for (path, kind) in &plan.pins {
        if plan.placeholders.iter().any(|ph| &ph.path == path) {
            if Path::new(path).symlink_metadata().is_ok() {
                changed.push(format!("{path} (there now, where pot planned a placeholder)"));
            }
        } else if let Some(g) = pin_gap(path, *kind, single.contains(path), None) {
            changed.push(g);
        }
    }
    if !changed.is_empty() {
        return Err(Refusal::new(
            &p.context,
            format!(
                "the workspace changed since pot resolved its pins: {}; run again",
                changed.join(", ")
            ),
        ));
    }
    place(rundir, &plan, &single)
        .map_err(|e| Refusal::new(&p.context, format!("can't place the run's files: {e}")))?;
    if p.access == "write" && p.mode == "in-place" {
        write_look(cfg, rundir, &plan).map_err(|m| Refusal::new(&p.context, m))?;
    }
    Ok(())
}

/// The host's look at a write run, by pot's own code (core's pins.py, which bwrap.py runs too): `python -I -S PINS
/// VERB GIT RUNDIR`, in pot's environment (as bwrap's prepare and after run), with no value in its arguments.
fn look(cfg: &Config, verb: &str, rundir: &Path) -> Result<serde_json::Value, String> {
    let t = cfg
        .look_tools()
        .ok_or("write access: the backend's --config names no python, pins and git")?;
    // X4: an environment of its own: the caller's PATH, home, identity and locale, and what git and its signature
    // check read (XDG_CONFIG_HOME, GNUPGHOME); never another variable (a secret's), nor a GIT_* one
    let mut cmd = std::process::Command::new(&t.python);
    cmd.env_clear();
    for k in LOOK_KEEP {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    let out = cmd
        .args(["-I", "-S", &t.pins, verb, &t.git])
        .arg(rundir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("can't run {}: {e}", t.python))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "the host's look ({verb}) failed: {}",
            why.lines().last().unwrap_or("no message")
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("the host's look ({verb}): {e}"))
}

/// The caller's variables the host's look keeps (X4).
const LOOK_KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "GNUPGHOME",
];

/// After `place`, for a write run: the seals' record (`sealed.json`, for after's `sealed_left`), the look before the run
/// (`before.json`), and the start notes in launch.json (a sealed folder over the cap, a hidden path git tracks).
fn write_look(cfg: &Config, rundir: &Path, plan: &Plan) -> Result<(), String> {
    let abs = fs::canonicalize(rundir).map_err(|e| format!("{}: {e}", rundir.display()))?;
    let seals: Vec<serde_json::Value> = plan
        .seals
        .iter()
        .map(|s| {
            serde_json::json!({ "dest": s.dest, "base": abs.join(&s.base), "entries": s.entries, "rel": s.rel })
        })
        .collect();
    write_new(
        &abs.join("sealed.json"),
        serde_json::Value::from(seals).to_string().as_bytes(),
        0o600,
    )
    .map_err(|e| e.to_string())?;
    let before = look(cfg, "before", &abs)?;
    let mut notes = plan.notes.clone();
    notes.extend(
        before["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n.as_str().map(String::from)),
    );
    if !notes.is_empty() {
        // a backend built apart adds only notes to launch.json (launch.json/1)
        let path = abs.join("launch.json");
        let mut l: serde_json::Value = read_own(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("launch.json: {e}")))?;
        let mut all: Vec<serde_json::Value> = l["notes"].as_array().cloned().unwrap_or_default();
        all.extend(notes.into_iter().map(serde_json::Value::from));
        l["notes"] = all.into();
        write_new(&path, format!("{l}\n").as_bytes(), 0o644).map_err(|e| e.to_string())?;
    }
    Ok(())
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
fn engine_processes(rundir: &str, names: &[String]) -> Vec<String> {
    let me = std::process::id().to_string();
    let Ok(entries) = fs::read_dir("/proc") else {
        return vec![];
    };
    entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|pid| pid.bytes().all(|b| b.is_ascii_digit()) && *pid != me)
        .filter(|pid| {
            fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|c| lifecycle::names_engine_state(&c, rundir, names))
        })
        .collect()
}

/// Remove the run's container through the engine, and wait until no process of the engine's for this run is left
/// (conmon and its exit command outlive `podman run`, N7). docker's daemon removes the container before `rm` returns.
fn remove_container(e: Engine, program: &str, rundir: &str, set: &[(String, String)]) -> Result<(), String> {
    let argv = lifecycle::remove_argv(e, program, rundir);
    let out = engine::command(&argv[0], &[], set)
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|err| format!("can't run {program} to remove the run's container: {err}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "{} rm exited {}: {}",
            e.name(),
            lifecycle::exit_code(out.status),
            why.lines().next().unwrap_or("")
        ));
    }
    if e == Engine::Docker {
        return Ok(());
    }
    let names = lifecycle::engine_names(program);
    let start = std::time::Instant::now();
    loop {
        let left = engine_processes(rundir, &names);
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

/// docker's root: the config's image, which must be pot's empty one (the one empty layer and no configuration);
/// imported from an empty tar when it is missing. Its name is content-stable, so runs share it and `cleanup` leaves
/// it.
fn docker_image(program: &str, set: &[(String, String)], image: &str) -> Result<(), String> {
    use std::io::Write as _;
    let inspect = || {
        let out = engine::command(program, &[], set)
            .args(["image", "inspect", "--format", "{{json .}}", image])
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
            .flatten()
    };
    let have = if let Some(v) = inspect() {
        v
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
        inspect().unwrap_or_default()
    };
    let gaps = lifecycle::image_gaps(&have);
    if !gaps.is_empty() {
        return Err(format!(
            "the image {image} is not pot's empty image (it has {}): remove it, or name another in the backend's --config",
            gaps.join(", ")
        ));
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct EngineEnv {
    keep: Vec<String>,
    set: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    piped: Vec<String>,
    #[serde(default)]
    own: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    defaults: std::collections::BTreeMap<String, String>,
    /// Whether the entry reads the named pipe: `run` makes it and writes the records.
    #[serde(default)]
    fifo: bool,
    /// With the network: pot's proxy, started in the run folder before the engine.
    #[serde(default)]
    proxy: Option<Vec<String>>,
    /// The credential and mcp bindings: references to the host's key files and variables, never a value.
    #[serde(default)]
    credentials: Vec<bindings::Cred>,
    /// The bindings' variables that take only their default when the binding has no value (never the host's).
    #[serde(default)]
    hostless: Vec<String>,
    /// Where the run sees a via-file binding's file.
    #[serde(default, rename = "secretsAt")]
    secrets_at: String,
    /// Write access: every workspace path the plan binds, as prepare found it.
    #[serde(default)]
    pins: Vec<PinRecord>,
    /// status.relay, git.fetch: pot's broker, started in the run folder before the engine.
    #[serde(default)]
    broker: Option<Vec<String>>,
    /// nix: the relay (socat) and the script it runs, started and written in the run folder before the engine.
    #[serde(default, rename = "nixRelay")]
    nix_relay: Option<Vec<String>>,
    #[serde(default, rename = "nixScript")]
    nix_script: Option<String>,
    /// status.relay: workmux's argv and the pane, run when the command ends.
    #[serde(default)]
    clear: Option<Vec<String>>,
    /// A1: the run folder's files the entry checks (bin/, the shims), as prepare made them, for run to check again.
    #[serde(default, rename = "ownIds")]
    own_ids: Vec<PinRecord>,
    /// A1: the bindings' sockets the entry checks (destination, name in the run folder), recorded by run.
    #[serde(default, rename = "socketChecks")]
    socket_checks: Vec<(String, String)>,
}

/// Write access, just before the engine starts: each pin is the inode prepare recorded, of its kind, no link, and a
/// protected file has no second hard link. The engine resolves a bind's source when it mounts it, so this narrows
/// the window between the check and the bind to the engine's own start; bwrap binds at its start with no such check.
fn pins_unchanged(pins: &[PinRecord], what: &str) -> Result<(), String> {
    let changed: Vec<String> = pins
        .iter()
        .filter_map(|r| {
            let id = Identity {
                dev: r.dev,
                ino: r.ino,
                size: r.size,
                ctime: r.ctime,
            };
            pin_gap(&r.path, r.kind, r.single, Some(id))
        })
        .collect();
    if changed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{what} changed since prepare: {}; run prepare again",
            changed.join(", ")
        ))
    }
}

/// R2: the run folder's engine paths as `prepare` left them, checked just before the engine starts: the engine
/// writes through a link (its runroot, temporary files, cidfile), and binds the run's files.
fn engine_paths(
    abs: &Path,
    e: Engine,
    own: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let (dirs, maybe_dirs, maybe_files): (&[&str], &[&str], &[&str]) = match e {
        Engine::Podman => (
            &["enginestore", "enginehome", "rootfs"],
            &["engineroot", "enginetmp", "emptydir", "potcfg"],
            &[
                "containers.conf",
                "storage.conf",
                "passwd",
                "group",
                "hosts",
                "empty",
                "hidden.txt",
            ],
        ),
        Engine::Docker => (
            &["dockercfg"],
            &["emptydir", "potcfg"],
            &["passwd", "group", "hosts", "empty", "hidden.txt"],
        ),
    };
    let mut bad = vec![];
    for (n, need) in dirs
        .iter()
        .map(|n| (n, true))
        .chain(maybe_dirs.iter().map(|n| (n, false)))
    {
        match abs.join(n).symlink_metadata() {
            Ok(m) if m.is_dir() => {}
            Err(_) if !need => {}
            _ => bad.push(format!("{} (not a folder of the run's)", abs.join(n).display())),
        }
    }
    for n in maybe_files {
        match abs.join(n).symlink_metadata() {
            Ok(m) if m.file_type().is_file() => {}
            Err(_) => {}
            _ => bad.push(format!("{} (not a file of the run's)", abs.join(n).display())),
        }
    }
    // S4: what the plan binds from the run folder, as prepare found it
    for (n, kind) in own {
        let ok = abs.join(n).symlink_metadata().is_ok_and(|m| match kind.as_str() {
            "dir" => m.is_dir(),
            _ => m.file_type().is_file(),
        });
        if !ok {
            bad.push(format!("{} (not the {kind} prepare left)", abs.join(n).display()));
        }
    }
    // the status run records: a file, or nothing; an empty folder in its place is removed
    let status = abs.join("status");
    match status.symlink_metadata() {
        Ok(m) if m.is_dir() => {
            if fs::remove_dir(&status).is_err() {
                bad.push(format!("{} (a folder, not empty)", status.display()));
            }
        }
        _ => {}
    }
    if abs.join("container.id").symlink_metadata().is_ok() {
        bad.push(format!(
            "{} (left from another run)",
            abs.join("container.id").display()
        ));
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the run folder is not as prepare left it: {}; run prepare again",
            bad.join(", ")
        ))
    }
}

/// The named pipe that carries the run's variables (a secret's among them) to the container: made afresh in the run
/// folder, mode 0600, never through a link (whatever is at its path is removed first; mkfifo follows no link), and
/// checked: a pipe of this user's, mode 0600. No value is in a file, an argument or the engine's environment.
fn make_fifo(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() => return Err(format!("{}: a folder is in the way", path.display())),
        Ok(_) => fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(_) => {}
    }
    rustix::fs::mkfifoat(rustix::fs::CWD, path, rustix::fs::Mode::from_raw_mode(0o600))
        .map_err(|e| format!("can't make the named pipe {}: {e}", path.display()))?;
    let ok = path.symlink_metadata().is_ok_and(|m| {
        m.file_type().is_fifo() && Some(m.uid()) == host::euid() && m.mode() & 0o7777 == 0o600
    });
    if ok {
        Ok(())
    } else {
        let _ = fs::remove_file(path);
        Err(format!(
            "{}: not the named pipe run made (mode 0600, this user's)",
            path.display()
        ))
    }
}

/// How long `run` waits for the container to open the named pipe, at most, while the engine runs.
const FIFO_WAIT: std::time::Duration = std::time::Duration::from_secs(300);

/// Write the records to the named pipe once the container has opened it, in a thread: the open is non-blocking
/// (ENXIO until a reader has it open: the entry opens it read-write, so it never waits for this writer) and retried,
/// and so are the writes, until DONE (the engine has exited) or `FIFO_WAIT`: `run` never hangs on a container that
/// died before it read. The pipe is opened without following a link, and must be one.
fn feed_fifo(
    path: PathBuf,
    data: Vec<u8>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    use rustix::fs::{FileType, Mode, OFlags};
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let give_up = || done.load(Ordering::Relaxed) || start.elapsed() > FIFO_WAIT;
        let fd = loop {
            match rustix::fs::open(
                &path,
                OFlags::WRONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(fd) => break fd,
                Err(rustix::io::Errno::NXIO | rustix::io::Errno::INTR) if !give_up() => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => return,
            }
        };
        if !rustix::fs::fstat(&fd).is_ok_and(|st| FileType::from_raw_mode(st.st_mode) == FileType::Fifo) {
            return;
        }
        let mut off = 0;
        while off < data.len() {
            match rustix::io::write(&fd, &data[off..]) {
                Ok(n) => off += n,
                Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) if !give_up() => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => return,
            }
        }
        // V3: delivered, the sentinel included: the pipe leaves the run folder (the container's bind keeps its own
        // inode, and the entry has it open), so no host process can open it any more
        let still = path.symlink_metadata().is_ok_and(|m| {
            use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
            m.file_type().is_fifo()
                && rustix::fs::fstat(&fd).is_ok_and(|st| st.st_ino == m.ino() && st.st_dev == m.dev())
        });
        if still {
            let _ = fs::remove_file(&path);
        }
    })
}

/// The caller's variables pot's proxy keeps (when set): what it needs to show a denial on the caller's tmux pane. The
/// rest of the caller's environment (its secrets among them) never reaches the proxy.
const PROXY_KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "TMUX",
    "TMUX_TMPDIR",
    "XDG_RUNTIME_DIR",
];

/// Start pot's proxy (ARGV, which `prepare` wrote) in the run folder, as bwrap's `run` does: its log `proxy.log`
/// (written afresh, never through a link), its socket `proxy.sock` (whatever is there removed first, never a folder).
/// It is this process's child: when `run` is gone (even killed with SIGKILL), the proxy stops itself (`proxy.py`'s `orphaned`).
fn start_proxy(abs: &Path, argv: &[String]) -> Result<std::process::Child, String> {
    use std::os::unix::fs::FileTypeExt as _;
    let sock = abs.join("proxy.sock");
    make_room(&sock)?;
    let log_path = abs.join("proxy.log");
    let log = fresh_log(&log_path)?;
    let err = log
        .try_clone()
        .map_err(|e| format!("{}: {e}", log_path.display()))?;
    let mut c = std::process::Command::new(&argv[0]);
    c.args(&argv[1..]).current_dir(abs).env_clear();
    for k in PROXY_KEEP {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    let mut child = c
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("can't start the network proxy ({}): {e}", argv[0]))?;
    let start = std::time::Instant::now();
    let listening = || sock.symlink_metadata().is_ok_and(|m| m.file_type().is_socket());
    while !listening() && start.elapsed() < std::time::Duration::from_secs(5) {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if listening() {
        return Ok(child);
    }
    stop_process(&mut child);
    let said = read_own(&log_path).unwrap_or_default();
    let said = String::from_utf8_lossy(&said);
    Err(format!(
        "the network proxy did not start: {}",
        said.lines().last().unwrap_or("(nothing in its log)")
    ))
}

/// Whatever is at PATH removed (a link: the link, never followed); a folder there refused.
fn make_room(path: &Path) -> Result<(), String> {
    match path.symlink_metadata() {
        Ok(m) if m.is_dir() => Err(format!("{}: a folder is in the way", path.display())),
        Ok(_) => fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display())),
        Err(_) => Ok(()),
    }
}

/// A helper's log, made afresh: whatever is at its path removed first, then created exclusively (never through a link).
fn fresh_log(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    make_room(path)?;
    fs::OpenOptions::new()
        .append(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The caller's variables pot's broker keeps (when set): the proxy's, and the ssh agent's socket (git.fetch's
/// `key.agent`). Host workmux gets the pane from the broker (`TMUX_PANE`), never from the run.
const BROKER_KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "TMUX",
    "TMUX_TMPDIR",
    "XDG_RUNTIME_DIR",
    "SSH_AUTH_SOCK",
    // A4: GSSAPI ssh's credential cache
    "KRB5CCNAME",
];

/// The caller's variables the nix relay keeps (when set): what host nix reads to reach the caller's store and its
/// configuration. The run is an untrusted client whatever they say (`--force-untrusted`).
const RELAY_KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "TMPDIR",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_CONFIG_DIRS",
    "XDG_CACHE_HOME",
    "NIX_REMOTE",
    "NIX_CONF_DIR",
    "NIX_USER_CONF_FILES",
    "NIX_CONFIG",
    // A4: where host nix finds the caller's daemon and store
    "NIX_DAEMON_SOCKET_PATH",
    "NIX_STATE_DIR",
    "NIX_STORE_DIR",
];

/// A1 (QA 2026-11-22): each binding's socket as the entry must find it, appended to `binds.txt` (prepare's lines
/// kept, the file written afresh): its device and inode as `lstat` sees it now, just before the engine starts, never
/// through a link. A link or another kind there is refused. The entry then stops the command (125) when the engine bound
/// another file at a socket's path: one swapped in after this check.
fn record_sockets(abs: &Path, socks: &[(String, String)]) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
    if socks.is_empty() {
        return Ok(());
    }
    let list = abs.join("binds.txt");
    let mut text = String::from_utf8(read_own(&list).map_err(|e| e.to_string())?)
        .map_err(|_| format!("{}: not text", list.display()))?;
    for (dest, name) in socks {
        let path = abs.join(name);
        match path.symlink_metadata() {
            Ok(m) if m.file_type().is_socket() => {
                let _ = writeln!(text, "{}:{} {dest}", m.dev(), m.ino());
            }
            _ => return Err(format!("not what run made just now: {}", path.display())),
        }
    }
    write_new(&list, text.as_bytes(), 0o444).map_err(|e| e.to_string())
}

/// A2 (QA 2026-11-22): git.fetch's key files in the broker's command, each `~` the caller's `HOME` (bwrap's
/// `expanduser`, as the credential bindings take it), never launch.json's `home`.
fn expand_key_files(argv: &[String], getenv: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(argv.len());
    for a in argv {
        let key = out.last().is_some_and(|p| p == "--key-file");
        out.push(if key {
            bindings::expand_home(a, getenv).to_string_lossy().into_owned()
        } else {
            a.clone()
        });
    }
    out
}

/// The bindings' sockets the run folder's helpers make: status.relay's and git.fetch's (the broker), nix's (the relay).
fn binding_socks(env: &EngineEnv) -> Vec<&'static str> {
    let mut socks = vec![];
    if let Some(b) = &env.broker {
        for s in ["relay.sock", "git.sock"] {
            if b.iter().any(|a| a == s) {
                socks.push(s);
            }
        }
    }
    if env.nix_relay.is_some() {
        socks.push("nix.sock");
    }
    socks
}

/// The host side of the bindings, as bwrap's `start_bindings`: pot's broker (status.relay, git.fetch) and the nix relay
/// (socat, running the script `nix-relay` for each connection), each in the run folder with an environment of its own,
/// their stderr in `bindings.log` (made afresh). Whatever is at a socket's or the script's name is removed first (a link
/// is removed, never followed); `run` waits up to 5 s for every socket, else refuses with the log's last line. The
/// broker stops itself when `run` is gone (`orphaned`); the relay does not, and `cleanup` stops it.
fn start_bindings(abs: &Path, env: &EngineEnv, helpers: &mut Helpers) -> Result<(), String> {
    use std::os::unix::fs::FileTypeExt as _;
    let socks = binding_socks(env);
    if socks.is_empty() {
        return Ok(());
    }
    helpers.rundir = Some(abs.to_path_buf());
    for s in &socks {
        helpers.files.push(abs.join(s));
        make_room(&abs.join(s))?;
    }
    let log_path = abs.join("bindings.log");
    let log = fresh_log(&log_path)?;
    let spawn = |argv: &[String], keep: &[&str]| -> Result<std::process::Child, String> {
        let mut c = std::process::Command::new(&argv[0]);
        c.args(&argv[1..]).current_dir(abs).env_clear();
        for k in keep {
            if let Some(v) = std::env::var_os(k) {
                c.env(k, v);
            }
        }
        let err = log
            .try_clone()
            .map_err(|e| format!("{}: {e}", log_path.display()))?;
        c.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(err)
            .spawn()
            .map_err(|e| format!("can't start the run's bindings ({}): {e}", argv[0]))
    };
    if let Some(b) = &env.broker {
        helpers.procs.push(spawn(
            &expand_key_files(b, &|n| std::env::var_os(n)),
            BROKER_KEEP,
        )?);
    }
    if let (Some(relay), Some(script)) = (&env.nix_relay, &env.nix_script) {
        let path = abs.join(render::NIX_SCRIPT);
        helpers.files.push(path.clone());
        write_new(&path, script.as_bytes(), 0o500).map_err(|e| e.to_string())?;
        helpers.procs.push(spawn(relay, RELAY_KEEP)?);
    }
    let start = std::time::Instant::now();
    let up = || {
        socks.iter().all(|s| {
            abs.join(s)
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_socket())
        })
    };
    while !up() && start.elapsed() < std::time::Duration::from_secs(5) {
        if helpers
            .procs
            .iter_mut()
            .any(|c| matches!(c.try_wait(), Ok(Some(_))))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if up() {
        return Ok(());
    }
    let said = read_own(&log_path).unwrap_or_default();
    let said = String::from_utf8_lossy(&said);
    Err(format!(
        "the run's bindings ({}) did not start: {}",
        socks.join(", "),
        said.lines().last().unwrap_or("(nothing in bindings.log)")
    ))
}

/// SIGTERM, then SIGKILL after 2 s (bwrap's `stop_helpers`); the child is reaped.
fn stop_process(child: &mut std::process::Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    if let Some(pid) = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    }
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(2) {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// What `run` started beside the engine, stopped and removed when the command ends (and on every early return): the
/// proxy, its socket and the named pipe. A `run` killed with SIGKILL leaves them to the proxy itself (it stops when its parent
/// is gone) and to `cleanup`.
#[derive(Default)]
struct Helpers {
    /// The proxy, the broker and the nix relay.
    procs: Vec<std::process::Child>,
    files: Vec<PathBuf>,
    /// The via-file bindings' folder, `secrets/`: removed with the values in it (a link there removed, not followed).
    secrets: Option<PathBuf>,
    /// status.relay: workmux's argv and the caller's pane, run once the command has ended (bwrap's: clear the pane).
    clear: Option<Vec<String>>,
    /// With the bindings' helpers: the run folder, where a relay daemon still serving a connection is stopped too.
    rundir: Option<PathBuf>,
    /// The programs the helpers run (A3).
    exes: HelperExes,
}

impl Drop for Helpers {
    fn drop(&mut self) {
        if let Some(c) = self.clear.as_ref().filter(|c| c.len() == 4) {
            let mut cmd = std::process::Command::new(&c[0]);
            cmd.args(&c[1..3]).env_clear();
            for k in PROXY_KEEP {
                if let Some(v) = std::env::var_os(k) {
                    cmd.env(k, v);
                }
            }
            let _ = cmd
                .env("TMUX_PANE", &c[3])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        for c in &mut self.procs {
            if let Some(pid) = i32::try_from(c.id())
                .ok()
                .and_then(rustix::process::Pid::from_raw)
            {
                let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
            }
        }
        for c in &mut self.procs {
            stop_process(c);
        }
        if let Some(r) = &self.rundir {
            for f in stop_helpers(r, &self.exes) {
                eprintln!("{}", visible(&format!("pot: {f}")));
            }
        }
        for f in &self.files {
            if f.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
                let _ = fs::remove_file(f);
            }
        }
        if let Some(s) = &self.secrets {
            if let Err(e) = remove_tree(s) {
                eprintln!("{}", visible(&format!("pot: can't remove {}: {e}", s.display())));
            }
        }
    }
}

/// The via-file bindings' values: each variable's name, the file's bytes.
type SecretFiles = Vec<(String, Vec<u8>)>;

/// bwrap's `deliver`, for the via-file values: each written to DIR/NAME (the run folder's `secrets/`, which prepare
/// made 0700), mode 0400, this user's. DIR is opened without following a link and must be this user's 0700 folder;
/// each file is created exclusively in it (`openat`, `O_EXCL|O_NOFOLLOW`), whatever was at its name removed first.
fn write_secrets(dir: &Path, files: &SecretFiles) -> Result<(), String> {
    use rustix::fs::{AtFlags, Mode, OFlags};
    let at = |e: rustix::io::Errno, what: &str| format!("{}: {e}", dir.join(what).display());
    let d = rustix::fs::open(
        dir,
        OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| at(e, ""))?;
    let st = rustix::fs::fstat(&d).map_err(|e| at(e, ""))?;
    if Some(st.st_uid) != host::euid() || st.st_mode & 0o7777 != 0o700 {
        return Err(format!(
            "{}: not the folder prepare made (mode 0700, this user's)",
            dir.display()
        ));
    }
    for (n, v) in files {
        match rustix::fs::unlinkat(&d, n.as_str(), AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => {}
            Err(e) => return Err(at(e, n)),
        }
        let fd = rustix::fs::openat(
            &d,
            n.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o400),
        )
        .map_err(|e| at(e, n))?;
        let mut off = 0;
        while off < v.len() {
            match rustix::io::write(&fd, &v[off..]) {
                Ok(k) => off += k,
                Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(at(e, n)),
            }
        }
        rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o400)).map_err(|e| at(e, n))?;
    }
    Ok(())
}

/// Just before the engine starts: the named pipe and the proxy's socket are what `run` made (a link or another kind of
/// file in their place is refused).
fn runtime_paths(
    abs: &Path,
    fifo: bool,
    proxy: bool,
    socks: &[&str],
    secrets: Option<&[String]>,
) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
    let mut bad = vec![];
    // the via-file bindings' folder: this user's, 0700, holding exactly the files run wrote, each 0400 and this user's
    if let Some(names) = secrets {
        let dir = abs.join("secrets");
        let mine = |m: &fs::Metadata, mode: u32| Some(m.uid()) == host::euid() && m.mode() & 0o7777 == mode;
        let held: Option<std::collections::BTreeSet<String>> = fs::read_dir(&dir).ok().map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        });
        let want: std::collections::BTreeSet<String> = names.iter().cloned().collect();
        let ok = dir
            .symlink_metadata()
            .is_ok_and(|m| m.is_dir() && mine(&m, 0o700))
            && held.as_ref() == Some(&want)
            && names.iter().all(|n| {
                dir.join(n)
                    .symlink_metadata()
                    .is_ok_and(|m| m.file_type().is_file() && mine(&m, 0o400))
            });
        if !ok {
            bad.push(dir.display().to_string());
        }
    }
    let check = |n: &str, ok: &dyn Fn(&fs::FileType) -> bool, bad: &mut Vec<String>| {
        if !abs.join(n).symlink_metadata().is_ok_and(|m| ok(&m.file_type())) {
            bad.push(abs.join(n).display().to_string());
        }
    };
    if fifo {
        check("secrets.fifo", &|t| t.is_fifo(), &mut bad);
    }
    if proxy {
        check("proxy.sock", &|t| t.is_socket(), &mut bad);
    }
    for s in socks {
        check(s, &|t| t.is_socket(), &mut bad);
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("not what run made just now: {}", bad.join(", ")))
    }
}

/// Wait for the engine; on SIGTERM or SIGHUP, stop the container through the engine. R1: a stop before the container
/// exists is retried until it does (each second, once the last stop has ended), so the engine's create is never cut
/// off half done; podman with no container after 10 s is killed (its store is the run's alone).
fn wait_engine(
    child: &mut std::process::Child,
    stop: &std::sync::atomic::AtomicBool,
    e: Engine,
    program: &str,
    abs: &Path,
    set: &[(String, String)],
) -> Result<std::process::ExitStatus, String> {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let r = abs.to_string_lossy().into_owned();
    let mut asked: Option<Instant> = None;
    let mut last_stop: Option<Instant> = None;
    let mut stopper: Option<std::process::Child> = None;
    let mut killed = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(err) => return Err(format!("waiting for {program}: {err}")),
        }
        if stop.load(Ordering::Relaxed) && asked.is_none() {
            asked = Some(Instant::now());
        }
        // R1: a stop before the container exists is retried until it does (each second, once the last stop has
        // ended): the engine's create is never cut off half done
        if let Some(since) = asked {
            let busy = stopper.as_mut().is_some_and(|s| matches!(s.try_wait(), Ok(None)));
            if !busy && last_stop.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
                match lifecycle::stop_argv(e, program, &r, container_id(abs).as_deref()) {
                    Some(a) => {
                        stopper = engine::command(&a[0], &[], set)
                            .args(&a[1..])
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()
                            .ok();
                        last_stop = Some(Instant::now());
                    }
                    // podman with no container after 10 s: the engine itself goes; its store is the run's alone
                    None if since.elapsed() > Duration::from_secs(10) && !killed => {
                        killed = true;
                        let _ = child.kill();
                    }
                    None => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if let Some(mut s) = stopper {
        let _ = s.wait();
    }
    Ok(status)
}

/// The bindings' values, read on the host now (bwrap's `credentials`), and refused before anything starts when one
/// can't be delivered (bwrap's `deliver`); with the via-file ones' files. They live in `run`'s memory, on the pipe and
/// (via file) in `secrets/` only.
fn bound_values(env: &EngineEnv) -> Result<(Vec<bindings::Val>, SecretFiles), String> {
    let vals = bindings::read(&env.credentials, &|n: &str| std::env::var_os(n))?;
    bindings::check(&env.credentials, &vals)?;
    let files = bindings::via_file(&env.credentials);
    let secret_files = vals
        .iter()
        .filter(|v| files.contains_key(&v.name))
        .map(|v| (v.name.clone(), v.value.as_bytes().to_vec()))
        .collect();
    Ok((vals, secret_files))
}

/// The value `run` writes to the pipe for N: a binding's (a via-file one's: its file in the run), as bwrap's `run`
/// updates `run_env` with `deliver`; else the host's, unless N is a binding's variable only (`hostless`); else the
/// plan's default for it.
fn piped_value(env: &EngineEnv, vals: &[bindings::Val], n: &str) -> Option<std::ffi::OsString> {
    if let Some(v) = vals.iter().find(|v| v.name == n) {
        return Some(if bindings::via_file(&env.credentials).contains_key(n) {
            format!("{}/{n}", env.secrets_at).into()
        } else {
            v.value.clone()
        });
    }
    let host = if env.hostless.iter().any(|h| h == n) {
        None
    } else {
        std::env::var_os(n)
    };
    host.or_else(|| env.defaults.get(n).map(Into::into))
}

/// Run the command `prepare` wrote, with the engine environment it recorded; return the command's status once the
/// container is removed and the engine's processes for the run have ended. SIGTERM and SIGHUP stop the container
/// (the engine's stop: SIGTERM, then SIGKILL after 10 s), retried until the container is gone; SIGINT is the
/// command's (the terminal sends it to the engine too, which passes it on), as bwrap's run ignores it.
#[allow(clippy::too_many_lines)] // the run's steps in order, each naming the audit finding it answers
fn run(cfg: &Config, rundir: &Path) -> Result<u8, Refusal> {
    use std::io::IsTerminal as _;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    let ctx = context_of(rundir);
    let refuse = |m: String| Refusal::new(&ctx, m);
    // S6: the handlers first, so a SIGTERM before the engine starts ends the run as a stopped one, with its status
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
    let abs = fs::canonicalize(rundir)
        .map_err(|e| refuse(format!("the run directory {}: {e}", rundir.display())))?;
    let r = abs.to_string_lossy().into_owned();
    let (e, program) = engine(cfg).map_err(refuse)?;
    let argv: Vec<String> = read_own(&abs.join("container.argv"))
        .map_err(|err| refuse(format!("{err} (run prepare first)")))?
        .strip_suffix(&[0])
        .unwrap_or_default()
        // each argument ends with a NUL; an empty one (docker's `--entrypoint ""`) is kept
        .split(|b| *b == 0)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    let mut env: EngineEnv = read_own(&abs.join("container.env"))
        .map_err(|err| err.to_string())
        .and_then(|t| serde_json::from_slice(&t).map_err(|err| format!("container.env: {err}")))
        .map_err(refuse)?;
    if argv.first() != Some(&program) {
        return Err(refuse(format!(
            "container.argv was not prepared for {program}: run prepare again"
        )));
    }
    let set: Vec<(String, String)> = std::mem::take(&mut env.set).into_iter().collect();
    if e == Engine::Docker {
        let image = cfg.image.clone().unwrap_or_else(|| render::DOCKER_IMAGE.into());
        if !argv.contains(&image) {
            return Err(refuse(format!(
                "container.argv does not run the image {image}: run prepare again"
            )));
        }
        docker_image(&program, &set, &image).map_err(refuse)?;
    }
    engine_paths(&abs, e, &env.own).map_err(refuse)?;
    if stop.load(std::sync::atomic::Ordering::Relaxed) {
        write_new(&abs.join("status"), b"143", 0o644)
            .map_err(|err| refuse(format!("can't record the status: {err}")))?;
        return Ok(143);
    }
    let (vals, secret_files) = bound_values(&env).map_err(refuse)?;
    // the named pipe and the proxy, just before the engine; stopped and removed when the command ends (Helpers' drop)
    let mut helpers = Helpers::default();
    helpers.exes = HelperExes::new(Some(cfg), &abs);
    if env.fifo {
        helpers.files.push(abs.join("secrets.fifo"));
        make_fifo(&abs.join("secrets.fifo")).map_err(refuse)?;
    }
    if let Some(proxy) = &env.proxy {
        helpers.files.push(abs.join("proxy.sock"));
        helpers.procs.push(start_proxy(&abs, proxy).map_err(refuse)?);
    }
    start_bindings(&abs, &env, &mut helpers).map_err(refuse)?;
    let secret_names = (!bindings::via_file(&env.credentials).is_empty()).then(|| {
        helpers.secrets = Some(abs.join("secrets"));
        secret_files.iter().map(|f| f.0.clone()).collect::<Vec<_>>()
    });
    if secret_names.is_some() {
        write_secrets(&abs.join("secrets"), &secret_files).map_err(refuse)?;
    }
    // R2 again, now that the proxy runs: every path the engine is handed is as prepare left it, or as run made it
    engine_paths(&abs, e, &env.own).map_err(refuse)?;
    runtime_paths(
        &abs,
        env.fifo,
        env.proxy.is_some(),
        &binding_socks(&env),
        secret_names.as_deref(),
    )
    .map_err(refuse)?;
    pins_unchanged(&env.pins, "the workspace").map_err(refuse)?;
    pins_unchanged(&env.own_ids, "the run folder").map_err(refuse)?;
    record_sockets(&abs, &env.socket_checks).map_err(refuse)?;
    let argv = lifecycle::run_argv(&argv, std::io::stdin().is_terminal());

    // U2: a variable passed by name that the host has unset takes pot's value: in the engine's environment (the
    // engine reads it for --env NAME), or as its record on the pipe
    let mut engine_set = set.clone();
    if !env.fifo {
        engine_set.extend(
            env.defaults
                .iter()
                .filter(|(n, _)| std::env::var_os(n).is_none())
                .map(|(n, v)| (n.clone(), v.clone())),
        );
    }
    let mut child = engine::command(&argv[0], &env.keep, &engine_set)
        .args(&argv[1..])
        .spawn()
        .map_err(|err| refuse(format!("can't run {program}: {err}")))?;
    helpers.clear.clone_from(&env.clear);
    let done = Arc::new(AtomicBool::new(false));
    let feeder = env.fifo.then(|| {
        let data = lifecycle::secret_records(&env.piped, |n| piped_value(&env, &vals, n));
        feed_fifo(abs.join("secrets.fifo"), data, Arc::clone(&done))
    });
    let status = wait_engine(&mut child, &stop, e, &program, &abs, &set);
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(f) = feeder {
        let _ = f.join();
    }
    drop(helpers);
    let code = lifecycle::exit_code(status.map_err(refuse)?);
    let gone = remove_container(e, &program, &r, &set);
    write_new(&abs.join("status"), code.to_string().as_bytes(), 0o644)
        .map_err(|err| refuse(format!("can't record the status: {err}")))?;
    if let Err(m) = gone {
        eprintln!("{}", visible(&refuse(m).to_string()));
    }
    Ok(code)
}

fn after(cfg: Result<&Config, &String>, rundir: &Path) -> Result<(), Refusal> {
    let (p, l) = load(rundir)?;
    // the proxy's log, when the run had the network: a regular file only, never through a link
    let log = rundir.join("proxy.log");
    let log = match read_own(&log) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(_) if log.symlink_metadata().is_err() => String::new(),
        Err(e) => {
            return Err(Refusal::new(
                &p.context,
                format!("the container backend's after: {e}"),
            ));
        }
    };
    // write access: the host's look after the run (pins.py: bwrap's code), against before.json
    let looked = if p.access == "write" && p.mode == "in-place" {
        let c = cfg.map_err(|m| Refusal::new(&p.context, m.clone()))?;
        Some(
            look(c, "after", rundir)
                .map_err(|m| Refusal::new(&p.context, format!("the container backend's after: {m}")))?,
        )
    } else {
        None
    };
    let ev = lifecycle::after_event(&p, &l, &log, looked)
        .map_err(|m| Refusal::new(&p.context, format!("the container backend's after: {m}")))?;
    let _ = std::io::Write::write_all(&mut std::io::stdout(), format!("{ev}\n").as_bytes());
    Ok(())
}

/// The placeholders `placeholders.json` lists, each removed while it is a regular file that still holds what prepare
/// wrote (never a link, never one the host changed). Returns what could not be removed.
fn remove_placeholders(list: &Path) -> Vec<String> {
    let Ok(bytes) = read_own(list) else {
        return vec![];
    };
    let placed: Vec<(String, String)> = serde_json::from_slice(&bytes).unwrap_or_default();
    let mut failed = vec![];
    for (path, text) in placed {
        let p = Path::new(&path);
        let ours = p.symlink_metadata().is_ok_and(|m| m.file_type().is_file())
            && fs::read(p).is_ok_and(|b| b == text.as_bytes());
        if ours {
            if let Err(e) = fs::remove_file(p) {
                failed.push(format!("{path}: {e}"));
            }
        }
    }
    failed
}

/// Remove PATH as [`remove_tree`] does, a folder the run made unwritable made writable first (bwrap's
/// `remove_sealed`); never through a link.
fn remove_tree_writable(path: &Path) -> std::io::Result<()> {
    fn open_up(d: &Path) {
        if d.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            let _ = fs::set_permissions(d, fs::Permissions::from_mode(0o700));
            if let Ok(rd) = fs::read_dir(d) {
                for e in rd.flatten() {
                    open_up(&e.path());
                }
            }
        }
    }
    if remove_tree(path).is_ok() {
        return Ok(());
    }
    open_up(path);
    remove_tree(path)
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

/// The engine environment cleanup gives podman: the run's configuration files when they are its own regular files,
/// else none, and the run's engine home.
fn podman_cleanup_set(abs: &Path, home: &Path) -> Vec<(String, String)> {
    let own_file = |n: &str| {
        let p = abs.join(n);
        p.symlink_metadata()
            .is_ok_and(|m| m.file_type().is_file())
            .then(|| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/dev/null".into())
    };
    vec![
        ("CONTAINERS_CONF".to_string(), own_file("containers.conf")),
        ("CONTAINERS_STORAGE_CONF".to_string(), own_file("storage.conf")),
        ("HOME".to_string(), home.to_string_lossy().into_owned()),
    ]
}

/// cleanup's first part: the run's container, removed through the engine, then the engine's state (store, runroot,
/// temporary files). The engine is never handed a linked path (R2); when the container could not be removed, or its
/// processes outlive the wait, the engine's state is kept and named, never deleted under a live container. Returns
/// what it could not remove.
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
    let store = rd("enginestore");
    let mut keep = false;

    // the container: podman's in the run's own store (which exists only once the engine has started)
    if is_real_dir(&store) && fs::read_dir(&store).is_ok_and(|mut d| d.next().is_some()) {
        let home = rd("enginehome");
        if !home.exists() && home.symlink_metadata().is_err() {
            let _ = fs::create_dir(&home);
        }
        let linked: Vec<String> = ["engineroot", "enginetmp", "enginehome"]
            .iter()
            .map(|n| rd(n))
            .filter(|p| p.symlink_metadata().is_ok() && !is_real_dir(p))
            .map(|p| p.display().to_string())
            .collect();
        let removed = if linked.is_empty() {
            let set = podman_cleanup_set(abs, &home);
            program(Engine::Podman).and_then(|p| remove_container(Engine::Podman, &p, &r, &set))
        } else {
            Err(format!(
                "not a folder of the run's, so not handed to the engine: {}",
                linked.join(", ")
            ))
        };
        if let Err(m) = removed {
            keep = true;
            failed.push(format!("{}, kept with its container: {m}", store.display()));
        }
    }
    // S2: a store that is not a real folder is named and kept, and so is the rest of the engine's state
    if store.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
        keep = true;
        failed.push(format!(
            "{} (not a folder of the run's), kept with the engine's state: its container is not removed",
            store.display()
        ));
    }
    // S2: nothing of the engine's state goes while a process of this run's engine lives, whatever the store is
    let names = lifecycle::engine_names(&program(Engine::Podman).unwrap_or_else(|_| "podman".into()));
    let left = engine_processes(&r, &names);
    if !keep && !left.is_empty() {
        keep = true;
        failed.push(format!(
            "{}, kept with the engine's state: the engine's processes for the run still run (pids {})",
            store.display(),
            left.join(" ")
        ));
    }
    // docker's, by the run's name
    if is_real_dir(&rd("dockercfg")) {
        let set = vec![(
            "DOCKER_CONFIG".to_string(),
            rd("dockercfg").to_string_lossy().into_owned(),
        )];
        if let Err(m) = program(Engine::Docker).and_then(|p| remove_container(Engine::Docker, &p, &r, &set)) {
            failed.push(format!("docker's container {}: {m}", lifecycle::docker_name(&r)));
        }
    }

    // the engine's state; what the user can't remove (another id of the user namespace, a mode-000 folder) goes
    // through podman unshare. Kept whole while a container may still use it.
    let state: Vec<PathBuf> = ["enginestore", "engineroot", "enginetmp"]
        .iter()
        .map(|n| rd(n))
        .filter(|_| !keep)
        .collect();
    let mut theirs = vec![];
    for p in &state {
        if let Err(e) = remove_tree(p) {
            theirs.push((p.clone(), e));
        }
    }
    if !theirs.is_empty() {
        let paths: Vec<PathBuf> = theirs.iter().map(|(p, _)| p.clone()).collect();
        let unshared = program(Engine::Podman).and_then(|p| unshare_rm(&p, &paths));
        for (p, e) in &theirs {
            if p.symlink_metadata().is_ok() {
                let why = unshared.as_ref().err().cloned().unwrap_or_else(|| e.to_string());
                failed.push(format!("{}: {e}; podman unshare: {why}", p.display()));
            }
        }
        // the run's state is gone, but not unshare's own private folder
        if paths.iter().all(|p| p.symlink_metadata().is_err()) {
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
    let mut failed = stop_helpers(&abs, &HelperExes::new(cfg.ok(), &abs));
    failed.extend(cleanup_engine(cfg, &abs));
    // write access: the placeholders that still hold what prepare wrote (bwrap's cleanup), and the seals' bases
    failed.extend(remove_placeholders(&rd("placeholders.json")));
    if let Err(e) = remove_tree_writable(&rd("sealed")) {
        failed.push(format!("{}: {e}", rd("sealed").display()));
    }
    for n in ["enginehome", "dockercfg", "secrets", "bin"] {
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
        "secrets.fifo",
        "proxy.sock",
        "relay.sock",
        "git.sock",
        "nix.sock",
        "nix-relay",
        "mcp.json",
        "placeholders.json",
        "sealed.json",
        "before.json",
        "binds.txt",
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

/// A3 (QA 2026-11-22): the programs pot's helpers run, as their real files: the config's python (the proxy, the
/// broker) and socat (the nix relay), and launch.json's `hostTools.nix` (a relay daemon). A helper is stopped only when
/// its program is one of these.
#[derive(Default)]
struct HelperExes {
    python: Option<PathBuf>,
    socat: Option<PathBuf>,
    nix: Option<PathBuf>,
}

impl HelperExes {
    fn new(cfg: Option<&Config>, abs: &Path) -> Self {
        let real = |p: Option<&str>| p.and_then(|p| fs::canonicalize(p).ok());
        let nix = read_own(&abs.join("launch.json"))
            .ok()
            .and_then(|t| serde_json::from_slice::<serde_json::Value>(&t).ok())
            .and_then(|l| {
                l.pointer("/hostTools/nix")
                    .and_then(|n| n.as_str())
                    .map(String::from)
            });
        HelperExes {
            python: real(cfg.and_then(|c| c.python.as_deref())),
            socat: real(cfg.and_then(|c| c.socat.as_deref())),
            nix: real(nix.as_deref()),
        }
    }
    fn of(&self, h: lifecycle::Helper) -> Option<&PathBuf> {
        match h {
            lifecycle::Helper::Python => self.python.as_ref(),
            lifecycle::Helper::Socat => self.socat.as_ref(),
            lifecycle::Helper::Nix => self.nix.as_ref(),
        }
    }
}

/// pot's helpers for this run still alive, each started in the run folder: the proxy and the broker (a `run` killed with
/// SIGKILL a moment ago leaves them for up to a second), the nix relay and a daemon it started for a connection (which
/// never stop by themselves): stopped, SIGTERM then SIGKILL after 2 s. Returns those that would not stop.
fn stop_helpers(abs: &Path, exes: &HelperExes) -> Vec<String> {
    let r = abs.to_string_lossy().into_owned();
    let me = std::process::id().to_string();
    let find = || -> Vec<rustix::process::Pid> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return vec![];
        };
        entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|pid| pid.bytes().all(|b| b.is_ascii_digit()) && *pid != me)
            .filter(|pid| {
                fs::read_link(format!("/proc/{pid}/cwd")).is_ok_and(|c| c == abs)
                    && fs::read(format!("/proc/{pid}/cmdline"))
                        .ok()
                        .and_then(|c| lifecycle::helper_program(&c, &r))
                        .and_then(|h| exes.of(h))
                        .is_some_and(|want| {
                            fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|e| &e == want)
                        })
            })
            .filter_map(|pid| pid.parse().ok().and_then(rustix::process::Pid::from_raw))
            .collect()
    };
    // a zombie's cmdline is empty: a stopped proxy is no longer found
    let gone_within = |d: std::time::Duration| {
        let start = std::time::Instant::now();
        while start.elapsed() < d {
            if find().is_empty() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        find().is_empty()
    };
    for sig in [rustix::process::Signal::TERM, rustix::process::Signal::KILL] {
        let left = find();
        if left.is_empty() {
            return vec![];
        }
        for pid in &left {
            let _ = rustix::process::kill_process(*pid, sig);
        }
        if gone_within(std::time::Duration::from_secs(2)) {
            return vec![];
        }
    }
    vec![format!("the helpers of {} would not stop", abs.display())]
}

/// probe's engine, in a temporary folder of its own (podman's home, its storage.conf and store), removed after:
/// probe leaves nothing in the folder pot gives it, often pot's state (audit N3).
fn probe_now(e: Engine, program: &str, net: bool) -> Result<Vec<&'static str>, String> {
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
        .and_then(|()| probe::assess(e, &probe::facts(e, program, &engine::probe_set(e, &d)), net));
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
            let doc = match cfg.and_then(|c| {
                // V2: private-proxy exactly where prepare carries the network out (render::net_gaps)
                let net = c
                    .net_tools()
                    .ok()
                    .flatten()
                    .is_some_and(|t| render::net_gaps(&t, &host::RealHost).is_empty());
                // protected-as-mounts exactly where prepare carries write access out: the look's programs are named
                let look = c.look_tools().is_some();
                engine(&c)
                    .and_then(|(e, program)| probe_now(e, &program, net))
                    .map(|mut g| {
                        if look {
                            g.push("protected-as-mounts");
                        }
                        g
                    })
            }) {
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
        Verb::After { rundir } => after(cfg.as_ref(), rundir),
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

    /// A2 (QA 2026-11-22): git.fetch's key file's `~` is the caller's `HOME` at run, as bwrap's `expanduser` and the
    /// credential bindings take it, never launch.json's `home`; nothing else in the broker's command changes.
    #[test]
    fn a_key_files_tilde_is_the_callers_home() {
        let argv: Vec<String> = [
            "py",
            "broker.py",
            "--key-file",
            "~/.ssh/id_pot",
            "--remote",
            "~/x",
            "--key-file",
            "/k",
        ]
        .map(String::from)
        .to_vec();
        let home = |n: &str| (n == "HOME").then(|| std::ffi::OsString::from("/home/caller/"));
        assert_eq!(
            expand_key_files(&argv, &home),
            [
                "py",
                "broker.py",
                "--key-file",
                "/home/caller/.ssh/id_pot",
                "--remote",
                "~/x",
                "--key-file",
                "/k"
            ]
        );
    }

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
