//! policy.json/1 as a `podman run` / `docker run` command: a pure function of the policy, the launcher's facts,
//! the backend's config, the engine and the host facts it reads through [`Host`].
//!
//! It carries out what this slice can, and refuses the rest (policy-1, rules 2, 3 and 5): every gap is named in
//! one refusal, never dropped. The flag set is the phase 2 spike's (executed on platform-vm, 2026-11-21).

use std::collections::{BTreeMap, BTreeSet};

use crate::host::{Entry, Host};
use crate::launch::{Kind, Launch, LookTools, NetTools, Paths, Placeholder, WsPins};
use crate::policy::{Policy, known_kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Podman,
    Docker,
}

impl Engine {
    pub fn name(self) -> &'static str {
        match self {
            Engine::Podman => "podman",
            Engine::Docker => "docker",
        }
    }
}

/// The guarantees this slice upholds when a policy's `backend.proved` names them (rule 2). resource-limits only
/// under podman: `--cgroups=split` keeps the container in the cgroup of the pot that started it, the user systemd
/// scope pot proves resource-limits by (pot.py `scope_for`). private-proxy only when the config names the network's
/// programs (`NET`): the proxy's socket is in the run folder, served in the run's own network namespace.
/// protected-as-mounts when it names the look's (`LOOK`: write access in place, its pins as read-only mounts).
pub fn upholds(engine: Engine, net: bool, look: bool, g: &str) -> bool {
    matches!(
        g,
        "closure-store"
            | "own-netns"
            | "own-pidns"
            | "no-tiocsti"
            | "local-listen"
            | "in-place"
            | "root-files"
    ) || (g == "resource-limits" && engine == Engine::Podman)
        || (g == "private-proxy" && net)
        || (g == "protected-as-mounts" && look)
}

/// Entries a sealed folder takes (bwrap's `SEAL_MAX`): one bind each; a folder with more stays plainly read-only.
pub const SEAL_MAX: usize = 256;

pub const DOCKER_IMAGE: &str = "localhost/pot-empty:1";

/// Where the run reads its variables: the named pipe `run` makes in the run folder (`secrets.fifo`), bound read-only.
pub const FIFO: &str = "/run/pot/secrets.fifo";

/// Where the run reaches pot's proxy: the run folder's `proxy.sock`, bound read-only (bwrap's path).
pub const PROXY_SOCK: &str = "/run/pot/proxy.sock";

/// The proxy's address in the run, as bwrap's `run_env` gives it.
pub const PROXY_URL: &str = "http://127.0.0.1:3128";

/// The run's bash, before the command (R3, S1, S3, T1), with a secret, a binding or the network. It starts in an environment
/// `env -i` cleared. It opens the named pipe (its first argument) read-write, so the open never waits for a writer,
/// and reads records, each ended by a NUL and within 30 s: exactly one for each planned name (its third argument is
/// their count, the names follow: they are not secret), in the planned order, `NAME=value`, or the bare `NAME` for one
/// the host has unset (it stays unset), then the sentinel record `.`. Any other stream (a name not planned, one twice,
/// one missing, an early sentinel: V1, records a process of the same user wrote first) refuses, and nothing runs. With the network (its second argument, socat's path, not empty), it
/// starts socat on 127.0.0.1:3128 towards the proxy's socket, its streams /dev/null and its environment bash's own
/// (no record is declared yet), and waits until it listens (`0100007F:0C38 0A` in /proc/net/tcp, bwrap's netwrap).
/// Each record is then declared in bash (`unset -v NAME`, which drops a special meaning such as RANDOM's, then
/// `declare -x`), never passed as an argument, and bash execs the last hop: `env -u` what bash adds,
/// `-C CWD -- POT'S VALUES… ARGV`.
pub const ENTRY_SCRIPT: &str = concat!(
    r#"__pot_fifo=$1 __pot_socat=$2 __pot_n=$3; shift 3; __pot_names=("${@:1:__pot_n}"); shift "$__pot_n"; "#,
    r#"__pot_recs=(); __pot_whole=; __pot_i=0; "#,
    r#"exec {__pot_fd}<>"$__pot_fifo" || exit 125; "#,
    r#"while IFS= read -r -d '' -t 30 -u "$__pot_fd" __pot_r; do "#,
    r#"if [ "$__pot_r" = . ]; then [ "$__pot_i" = "$__pot_n" ] && __pot_whole=1; break; fi; "#,
    r#"[ "$__pot_i" -lt "$__pot_n" ] && [ "${__pot_r%%=*}" = "${__pot_names[__pot_i]}" ] || break; "#,
    r#"case $__pot_r in *=*) __pot_recs+=("$__pot_r") ;; esac; __pot_i=$((__pot_i + 1)); done; "#,
    r#"[ "$__pot_whole" = 1 ] || { echo "pot: the run's variables did not arrive as planned: nothing runs" >&2; exit 125; }; "#,
    r#"if [ -n "$__pot_socat" ]; then "#,
    r#""$__pot_socat" -lf /dev/null TCP-LISTEN:3128,bind=127.0.0.1,fork,reuseaddr UNIX-CONNECT:/run/pot/proxy.sock </dev/null >/dev/null 2>&1 {__pot_fd}<&- & "#,
    r#"__pot_up=; for __pot_x in {1..300}; do "#,
    r#"while read -r __pot_x __pot_l __pot_x __pot_s __pot_x; do [ "$__pot_l $__pot_s" != "0100007F:0C38 0A" ] || { __pot_up=1; break; }; done </proc/net/tcp; "#,
    r#"[ -z "$__pot_up" ] || break; read -r -t 0.01 -u "$__pot_fd" __pot_x; done; "#,
    r#"[ -n "$__pot_up" ] || { echo "pot: the network relay (socat) did not start" >&2; exit 125; }; fi; "#,
    r#"exec {__pot_fd}<&-; "#,
    r#"for __pot_r in "${__pot_recs[@]}"; do unset -v "${__pot_r%%=*}" && declare -x -- "$__pot_r" || exit 125; done; "#,
    r#"unset -v __pot_r __pot_recs __pot_whole __pot_fd __pot_fifo __pot_socat __pot_n __pot_names __pot_i __pot_up __pot_l __pot_s __pot_x; exec "$@""#
);

/// What bash adds to the environment of what it execs: the last hop's `env -u` removes it (T1).
pub const BASH_ADDS: &[&str] = &["_", "SHLVL", "PWD", "OLDPWD"];

/// What the engines add to the command's environment that pot does not plan: `env -u` removes it (S3).
pub const ENGINE_ADDS: &[&str] = &["HOSTNAME"];

/// S5: with the entry script (a secret, a binding or the network), the config's shell must be a real file (not a link) under a
/// path the plan binds. Otherwise the gap.
pub fn program_gaps(plan: &Plan, shell: &str, host: &dyn Host) -> Vec<String> {
    let bound = plan
        .sources
        .iter()
        .any(|s| shell == s || shell.starts_with(&format!("{}/", s.trim_end_matches('/'))));
    if !plan.fifo || (real_file(shell, host) && bound) {
        return vec![];
    }
    vec![format!(
        "the config's shell {shell} (the run's entry runs it: it must be a real file under a path the run binds)"
    )]
}

/// A real file: it exists, is not a folder, and is its own real path (no link, no `..`).
fn real_file(p: &str, host: &dyn Host) -> bool {
    host.realpath(p).is_some_and(|r| r == p) && !host.is_real_dir(p) && host.lexists(p)
}

/// V2: the one test of the network's programs, which probe (for private-proxy) and prepare (for the network and
/// private-proxy; not `--print`, which reads no host file) share:
/// python and proxy.py files (they run on the host), and socat a real file, not a link, under a path of `netClosure`,
/// which the run binds with the network (required: without it the run could not see socat's closure).
pub fn net_gaps(t: &NetTools, host: &dyn Host) -> Vec<String> {
    let mut out = vec![];
    for (what, f) in [("python", &t.python), ("proxy", &t.proxy)] {
        let file = host
            .realpath(f)
            .is_some_and(|r| host.lexists(&r) && !host.is_real_dir(&r));
        if !file {
            out.push(format!(
                "the config's {what} {f} (the network's proxy runs it: it must be a file)"
            ));
        }
    }
    let under = t
        .closure
        .iter()
        .any(|c| t.socat.starts_with(&format!("{}/", c.trim_end_matches('/'))));
    if !(real_file(&t.socat, host) && under) {
        out.push(format!(
            "the config's socat {} (the run's entry runs it: it must be a real file under a path of netClosure)",
            t.socat
        ));
    }
    out
}

/// podman's global options for a run: its store, its runtime state (runroot: the conmon sockets, the mount
/// table) and its temporary files (exit codes), all in the run folder. `cleanup` names the same.
pub fn podman_globals(rundir: &str) -> Vec<String> {
    let rd = |n: &str| format!("{}/{n}", rundir.trim_end_matches('/'));
    vec![
        "--root".into(),
        rd("enginestore"),
        "--runroot".into(),
        rd("engineroot"),
        "--tmpdir".into(),
        rd("enginetmp"),
    ]
}

/// Everything `render` needs.
pub struct Inputs<'a> {
    pub policy: &'a Policy,
    pub launch: &'a Launch,
    pub paths: &'a Paths,
    pub engine: Engine,
    /// The engine's program, as the config names it.
    pub program: &'a str,
    /// docker's empty image.
    pub image: &'a str,
    /// The run directory, absolute.
    pub rundir: &'a str,
    /// The config's base closure (its file's lines).
    pub base_closure: &'a [String],
    /// The config's programs for the network, when it names them.
    pub net: Option<&'a NetTools>,
    /// The config's programs for the host's look at a write run, when it names them.
    pub look: Option<&'a LookTools>,
    pub host: &'a dyn Host,
}

/// A sealed folder (bwrap's `seal`): BASE, a folder of the run folder's, bound writable at DEST, with each of ENTRIES
/// (the host folder's) bound read-only into it, a link as the same link. `after` lists what else BASE holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seal {
    pub dest: String,
    /// Relative to the run folder.
    pub base: String,
    pub entries: Vec<String>,
    /// DEST relative to the workspace's top.
    pub rel: String,
}

/// One option and its values; `closure` marks a closure bind (`--print` folds those into a count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opt {
    pub args: Vec<String>,
    pub closure: bool,
}

/// What `prepare` places in the run directory for the command to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFile {
    /// A folder of the run's, emptied: removed with what is in it, then made again.
    FreshDir {
        name: String,
    },
    File {
        name: String,
        text: String,
        mode: u32,
    },
    Dir {
        name: String,
    },
    /// A folder of the run's alone (mode 0700), emptied: the via-file bindings' `secrets/`, which run fills.
    PrivateDir {
        name: String,
    },
    Link {
        name: String,
        target: String,
    },
}

#[derive(Debug)]
pub struct Plan {
    pub program: String,
    pub opts: Vec<Opt>,
    /// The image (docker) or `--rootfs DIR:O` (podman): the last option, then the entry.
    pub root: Vec<String>,
    pub entry: Vec<String>,
    pub files: Vec<RunFile>,
    /// Host folders `prepare` creates (the state mounts' directories, in pot's state).
    pub host_dirs: Vec<String>,
    /// Every bind's source, the closure's included: `prepare` checks that each exists ([`missing_sources`]).
    pub sources: Vec<String>,
    /// The engine's environment (audit F2): only these of the caller's variables, then `engine_set`.
    pub engine_keep: Vec<String>,
    pub engine_set: Vec<(String, String)>,
    /// U2: pot's values for variables passed by name (an env item named like one): run's value when the host has
    /// the variable unset.
    pub defaults: BTreeMap<String, String>,
    /// The variables `run` hands over the named pipe, by name: with a secret, a binding or the network, every one passed by
    /// name.
    pub piped: Vec<String>,
    /// Whether the entry reads the named pipe (a secret, a binding or the network): `run` makes `secrets.fifo` and writes it.
    pub fifo: bool,
    /// With the network: the proxy's command, which `run` starts on the host in the run folder.
    pub proxy: Option<Vec<String>>,
    /// The credential and mcp bindings (references only), whose values `run` reads on the host and writes to the pipe.
    pub creds: Vec<crate::bindings::Cred>,
    /// The bindings' variables that take no value from the host's variable of the same name when the binding has
    /// none (only their default, if any): those that are not also an env item, `TERM` or `COLORTERM`.
    pub hostless: Vec<String>,
    /// Where the run sees a via-file binding's file: `/run/pot/secrets` (policy `mounts.run`'s `secrets`).
    pub secrets_at: String,
    /// Write access: the placeholders `prepare` creates before anything else binds them (launch.json's pins).
    pub placeholders: Vec<Placeholder>,
    /// Write access: the sealed folders.
    pub seals: Vec<Seal>,
    /// Write access: every host path of the workspace the plan binds (a pin, the top, a sealed entry) and its kind,
    /// which `prepare` records and `run` checks again just before the engine starts.
    pub pins: Vec<(String, Kind)>,
    /// Start notes (launch.json `notes`): a sealed folder over the cap.
    pub notes: Vec<String>,
}

/// The sources that do not exist on the host, but for the run folder's (prepare places them) and the state
/// directories (prepare creates them). docker would create a missing source, as root.
pub fn missing_sources(plan: &Plan, rundir: &str, host: &dyn Host) -> Vec<String> {
    plan.sources
        .iter()
        .filter(|s| {
            !under(s, rundir)
                && !plan.host_dirs.contains(s)
                && !plan.placeholders.iter().any(|p| &p.path == *s)
                && !host.lexists(s)
        })
        .cloned()
        .collect()
}

impl Plan {
    pub fn argv(&self) -> Vec<String> {
        let mut v = vec![self.program.clone()];
        v.extend(self.opts.iter().flat_map(|o| o.args.iter().cloned()));
        v.extend(self.root.iter().cloned());
        v.extend(self.entry.iter().cloned());
        v
    }
}

#[derive(Debug, Clone)]
enum Mount {
    Bind { src: String, ro: bool },
    Tmpfs { opts: String },
}

/// bwrap's priorities: at one destination, a hidden mask beats a read-only bind beats a writable one.
const WRITE: u8 = 0;
const READ: u8 = 1;
const HIDE: u8 = 2;

struct Mounts {
    by_dest: BTreeMap<String, (u8, Mount)>,
}

impl Mounts {
    fn add(&mut self, dest: &str, prio: u8, m: Mount) {
        let dest = dest.trim_end_matches('/').to_string();
        match self.by_dest.get(&dest) {
            Some((p, _)) if *p > prio => {}
            _ => {
                self.by_dest.insert(dest, (prio, m));
            }
        }
    }
    fn bind(&mut self, src: &str, dest: &str, prio: u8, ro: bool) {
        self.add(
            dest,
            prio,
            Mount::Bind {
                src: src.to_string(),
                ro,
            },
        );
    }
}

fn under(p: &str, dir: &str) -> bool {
    p.strip_prefix(dir.trim_end_matches('/'))
        .is_some_and(|r| r.starts_with('/'))
}

fn at_or_under(p: &str, dir: &str) -> bool {
    p == dir || under(p, dir)
}

fn in_home(home: &str, v: &str) -> String {
    match v.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{home}{rest}"),
        _ => v.to_string(),
    }
}

/// launch.json's pins for the workspace TOP (with COMMON), when pot's core resolved them.
pub fn pins_of<'a>(l: &'a Launch, top: &str, common: &str) -> Option<&'a WsPins> {
    l.pins
        .as_ref()?
        .iter()
        .find(|w| w.top == top && w.common == common)
}

/// P as the run's home shows it (bwrap's `tilde`).
fn tilde(p: &str, home: &str) -> String {
    match p.strip_prefix(home) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => p.to_string(),
    }
}

/// The gaps this slice refuses, in the policy's order: each a short phrase.
#[allow(clippy::too_many_lines)] // one check per policy feature, listed in the reference page's order
fn gaps(i: &Inputs) -> Vec<String> {
    let p = i.policy;
    let mut out = vec![];
    for g in &p.backend.proved {
        if !upholds(i.engine, i.net.is_some(), i.look.is_some(), g) {
            out.push(match g.as_str() {
                "resource-limits" => {
                    "resource-limits under docker (a container is not in pot's scope)".to_string()
                }
                "private-proxy" => {
                    "the private-proxy guarantee (the backend's --config names no python, proxy, socat and netClosure)".to_string()
                }
                _ => format!("the {g} guarantee"),
            });
        }
    }
    if p.mode == "clone" {
        out.push("clone mode (the clone at its original path, its pins, the staging fetch)".into());
    } else if p.access == "write" {
        // 0.3.0: carried out from the pins pot's core resolved, with the host's look before and after (pins.py)
        if i.look.is_none() {
            out.push("write access: the backend's --config names no python, pins and git (the host's look before and after the run)".into());
        }
        for w in &p.workspace {
            if pins_of(i.launch, &w.path, &w.common).is_none() {
                out.push(format!(
                    "write access without launch.json's pins for {} (pot 0.3.0 or later resolves them)",
                    w.path
                ));
            }
        }
    }
    if p.effective("view").next().is_some() {
        out.push("inspect's view".into());
    }
    let net: Vec<&str> = p
        .effective("network")
        .filter(|n| n.class == "reach")
        .map(|n| n.value.as_str())
        .collect();
    match i.net {
        _ if net.is_empty() => {}
        None => out.push(format!(
            "the network through pot's proxy ({}): the backend's --config names no python, proxy, socat and netClosure",
            net.join(", ")
        )),
        Some(_) => {}
    }
    // credential and mcp are carried out (their values over the named pipe, a via-file one's file in secrets/)
    let caps: BTreeSet<&str> = p
        .bindings
        .iter()
        .map(|b| b.capability.as_str())
        .filter(|c| !matches!(*c, "credential" | "mcp"))
        .collect();
    for c in caps {
        out.push(format!("the {c} binding"));
    }
    if p.children.values().any(|c| c.how.as_deref() == Some("sibling")) {
        out.push("sibling children (the run's children broker)".into());
    }
    for f in p.effective("file") {
        if f.mode.as_deref() != Some("fixed") {
            out.push(format!(
                "file {} (mode {})",
                f.value,
                f.mode.as_deref().unwrap_or("?")
            ));
        }
    }
    for d in p.effective("dir") {
        out.push(format!("dir {} (a folder of links)", d.value));
    }
    for e in p
        .effective("env")
        .filter(|e| crate::engine::configures_engine(&e.value))
    {
        out.push(format!(
            "the variable {}, which configures the container engine, as a run's variable",
            e.value
        ));
    }
    for e in p.effective("env").filter(|e| crate::engine::bash_sets(&e.value)) {
        out.push(format!(
            "the variable {}, which bash sets or keeps read-only, as a run's variable",
            e.value
        ));
    }
    // a binding's variable never reaches the engine (it goes over the named pipe), but bash declares it (T1)
    for n in crate::bindings::names(&crate::bindings::creds(p)) {
        if !crate::bindings::is_name(&n) {
            out.push(format!("a binding's variable {n:?}, which is not a NAME"));
        } else if crate::engine::bash_sets(&n) {
            out.push(format!(
                "the variable {n}, which bash sets or keeps read-only, as a binding's variable"
            ));
        }
    }
    for it in p.items.iter().filter(|it| it.effective && !known_kind(&it.kind)) {
        out.push(format!("an item of kind {} ({})", it.kind, it.class));
    }
    if let Some(a) = i.launch.argv.first() {
        if a.contains('=') {
            out.push(format!("a command whose name holds '=' ({a})"));
        }
    } else {
        out.push("an empty command".into());
    }
    out
}

/// The paths a `--volume` names: one with ':' or ',' can't be said there.
fn unsayable(plan: &[(String, Mount)], closure: &[String]) -> Vec<String> {
    let bad = |s: &str| s.contains(':') || s.contains(',') || s.contains('"');
    let mut out = vec![];
    for (dest, m) in plan {
        if bad(dest) {
            out.push(dest.clone());
        }
        if let Mount::Bind { src, .. } = m {
            if bad(src) {
                out.push(src.clone());
            }
        }
    }
    out.extend(closure.iter().filter(|c| bad(c)).cloned());
    let mut seen = BTreeSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// The command for this run, or every gap this slice can't carry out (one refusal).
#[allow(clippy::too_many_lines)] // the mapping table, in order; each step names its policy field
#[allow(clippy::many_single_char_names)] // p, l, m, o: the policy, launch, mounts, an option (bwrap.py's names)
pub fn render(i: &Inputs) -> Result<Plan, Vec<String>> {
    let mut refuse = gaps(i);
    let (p, l, host) = (i.policy, i.launch, i.host);
    let home = l.home.trim_end_matches('/');
    let run = p.mounts.run.as_str();
    let rd = |n: &str| format!("{}/{n}", i.rundir.trim_end_matches('/'));
    let mut m = Mounts {
        by_dest: BTreeMap::new(),
    };
    let mut files = vec![
        RunFile::File {
            name: "empty".into(),
            text: String::new(),
            mode: 0o444,
        },
        RunFile::Dir {
            name: "emptydir".into(),
        },
    ];
    let mut host_dirs = vec![];

    // the hidden paths, and (N4a) the real path of each that is a link, as Home Manager's files are: a link is
    // masked where a bound folder shows it, and so is what it points to
    let mut hidden: Vec<String> = vec![];
    for h in p.effective("hidden").flat_map(|h| h.paths.iter().flatten()) {
        for q in [Some(h.clone()), host.realpath(h)].into_iter().flatten() {
            if !hidden.contains(&q) {
                hidden.push(q);
            }
        }
    }
    let real_dir = |q: &str| host.is_real_dir(q) || host.realpath(q).is_some_and(|r| host.is_real_dir(&r));
    // K4: a hidden folder is hidden as one unit: nothing is bound inside it
    let in_hidden = |q: &str| hidden.iter().any(|h| under(q, h) && host.is_real_dir(h));
    let hidden_or_below = |q: &str| hidden.iter().any(|h| h == q) || in_hidden(q);
    // a mask is a read-only bind of an empty folder or file, by the kind of what it hides: never a tmpfs, which
    // podman fills with a copy of what is below it (tmpcopyup)
    let mask = |m: &mut Mounts, q: &str| {
        let empty = if real_dir(q) { "emptydir" } else { "empty" };
        m.bind(&rd(empty), q, HIDE, true);
    };

    // the base: an empty, read-only root with its own /tmp, /run and home
    m.add(
        "/tmp",
        WRITE,
        Mount::Tmpfs {
            opts: "rw,exec,nosuid,nodev,mode=1777".into(),
        },
    );
    m.add(
        "/run",
        WRITE,
        Mount::Tmpfs {
            opts: "rw,nosuid,nodev,mode=0755".into(),
        },
    );
    // the home is the run's user's: podman's U chowns a tmpfs to the container's user; docker takes uid=, gid=
    let owner = match i.engine {
        Engine::Podman => "U".to_string(),
        Engine::Docker => format!("uid={},gid={}", l.uid, l.gid),
    };
    m.add(
        home,
        WRITE,
        Mount::Tmpfs {
            opts: format!("rw,exec,nosuid,nodev,mode=0700,{owner}"),
        },
    );
    for (n, dest) in [
        ("passwd", "/etc/passwd"),
        ("group", "/etc/group"),
        ("hosts", "/etc/hosts"),
    ] {
        m.bind(&rd(n), dest, READ, true);
    }
    files.push(RunFile::File {
        name: "passwd".into(),
        text: format!("{}:x:{}:{}::{home}:{}\n", l.user, l.uid, l.gid, i.paths.shell),
        mode: 0o444,
    });
    files.push(RunFile::File {
        name: "group".into(),
        text: format!("{}:x:{}:\n", l.group, l.gid),
        mode: 0o444,
    });
    files.push(RunFile::File {
        name: "hosts".into(),
        text: "127.0.0.1 localhost pot\n::1 localhost pot\n".into(),
        mode: 0o444,
    });
    m.bind(&i.paths.sh, "/bin/sh", READ, true);
    m.bind(&i.paths.env, "/usr/bin/env", READ, true);
    let ca = "/etc/ssl/certs/ca-certificates.crt";
    if !p.effective("file").any(|f| f.value == ca) {
        m.bind(&i.paths.cacert, ca, READ, true);
    }

    // F3: masking compares paths as strings, so every path bound, masked or holding the workspace must be its
    // own real path (no link, no `..`): refused, saying which, otherwise
    let mut unreal = vec![];
    let real = |q: &str, unreal: &mut Vec<String>| match host.realpath(q) {
        Some(r) if r != q => {
            let say = format!("{q} is not its real path ({r})");
            if !unreal.contains(&say) {
                unreal.push(say);
            }
            false
        }
        _ => true,
    };

    // reach: read and write paths where they exist; hidden paths under them masked; their sockets masked
    let mut bound = vec![];
    for (kind, prio) in [("read", READ), ("write", WRITE)] {
        for it in p.effective(kind) {
            let q = it.path.as_deref().unwrap_or_default();
            if real(q, &mut unreal) && host.lexists(q) && !hidden_or_below(q) {
                m.bind(q, q, prio, prio == READ);
                bound.push(q.to_string());
            }
        }
    }
    for h in &hidden {
        if host.lexists(h) && bound.iter().any(|b| under(h, b)) && !in_hidden(h) {
            mask(&mut m, h);
        }
    }
    let named: BTreeSet<&str> = p.effective("socket").filter_map(|s| s.path.as_deref()).collect();
    for s in &named {
        if real(s, &mut unreal) && host.lexists(s) && !hidden_or_below(s) {
            m.bind(s, s, WRITE, false);
        }
    }
    for b in &bound {
        for s in host.sockets_under(b) {
            if !named.contains(s.as_str()) && !in_hidden(&s) {
                m.bind(&rd("empty"), &s, HIDE, true);
            }
        }
    }

    // the workspace (in place): read access binds it read-only at its path, its top and (a worktree's) common
    // directory each with their sockets and hidden paths masked. Write access (0.3.0) binds them writable, then the
    // pins pot's core resolved (launch.json's pins): DIRS writable and RO read-only, each onto itself, so a pin is a
    // mount point that can't be written, renamed, removed or replaced, nor moved away with its folder. The engines
    // mount by destination depth, so a pin lands over the workspace. Clones are refused above. access none: no
    // workspace at all (pot 0.2.0), masked where a bound folder would show it (bwrap's `unseen`), and the command
    // starts in the home
    let mut kinds: BTreeMap<String, Kind> = BTreeMap::new();
    let mut placeholders: Vec<Placeholder> = vec![];
    let write = p.access == "write";
    if (p.access == "read" || write) && p.mode == "in-place" {
        for w in &p.workspace {
            let mut tops = vec![w.path.as_str()];
            if !under(&w.common, &w.path) {
                tops.push(w.common.as_str());
            }
            for top in &tops {
                if real(top, &mut unreal) {
                    m.bind(top, top, if write { WRITE } else { READ }, !write);
                    kinds.insert((*top).to_string(), Kind::Folder);
                }
            }
            if let Some(wp) = pins_of(l, &w.path, &w.common).filter(|_| write) {
                for d in &wp.dirs {
                    if real(d, &mut unreal) {
                        m.bind(d, d, WRITE, false);
                        kinds.insert(d.clone(), Kind::Folder);
                    }
                }
                for r in &wp.ro {
                    if real(&r.path, &mut unreal) {
                        m.bind(&r.path, &r.path, READ, true);
                        kinds.insert(r.path.clone(), r.kind);
                    }
                }
                placeholders.extend(wp.placeholders.iter().cloned());
            }
            for top in tops {
                for s in host.sockets_under(top) {
                    if !in_hidden(&s) {
                        m.bind(&rd("empty"), &s, HIDE, true);
                    }
                }
                // a hidden path there is masked, and nothing is bound inside a hidden folder there (a pin neither)
                for h in hidden.iter().filter(|h| under(h, top) && host.lexists(h)) {
                    if host.is_real_dir(h) {
                        m.by_dest.retain(|d, _| !under(d, h));
                    }
                    mask(&mut m, h);
                }
            }
        }
    }
    // v2.19.1, as bwrap's `seal`: each sealed folder (launch.json's pins' `sealed`, sealed_dirs) a folder of the run's,
    // bound writable at its path, with each entry of the host's bound read-only into it (a link as the same link),
    // so no entry can be written, replaced, renamed or removed and a new one lands in the run folder (after's
    // sealed_left). A hidden one stays hidden; one with more than SEAL_MAX entries stays as it is, with a note.
    let mut seals: Vec<Seal> = vec![];
    let mut notes: Vec<String> = vec![];
    if write && p.mode == "in-place" {
        for w in &p.workspace {
            for d in pins_of(l, &w.path, &w.common)
                .map(|wp| wp.sealed.as_slice())
                .unwrap_or_default()
            {
                if hidden_or_below(d) || !real(d, &mut unreal) {
                    continue;
                }
                let entries = host.entries(d);
                if entries.len() > SEAL_MAX {
                    notes.push(format!(
                        "pot: {} has {} entries, more than a sealed folder takes ({SEAL_MAX}): it is read-only in this run, so pi cannot take its settings lock there",
                        tilde(d, home),
                        entries.len()
                    ));
                    continue;
                }
                if seals.is_empty() {
                    files.push(RunFile::FreshDir {
                        name: "sealed".into(),
                    });
                }
                let base = format!("sealed/{}", seals.len());
                files.push(RunFile::Dir { name: base.clone() });
                m.bind(&rd(&base), d, READ, false);
                for (name, kind) in &entries {
                    let (src, at) = (format!("{d}/{name}"), format!("{base}/{name}"));
                    match kind {
                        Entry::Link(target) => files.push(RunFile::Link {
                            name: at,
                            target: target.clone(),
                        }),
                        Entry::Folder => {
                            files.push(RunFile::Dir { name: at });
                            m.bind(&src, &src, READ, true);
                            kinds.insert(src, Kind::Folder);
                        }
                        Entry::File => {
                            files.push(RunFile::File {
                                name: at,
                                text: String::new(),
                                mode: 0o444,
                            });
                            m.bind(&src, &src, READ, true);
                            kinds.insert(src, Kind::File);
                        }
                    }
                }
                seals.push(Seal {
                    dest: d.clone(),
                    base,
                    entries: entries.into_iter().map(|e| e.0).collect(),
                    rel: d.strip_prefix(&format!("{}/", w.path)).unwrap_or(d).to_string(),
                });
            }
        }
    }
    let cwd = if p.access == "none" {
        home.to_string()
    } else {
        l.cwd.clone()
    };
    if p.access == "none" {
        let mut unseen: Vec<&str> = vec![];
        for w in &p.workspace {
            for q in [w.path.as_str(), w.common.as_str()] {
                if !unseen.iter().any(|u| at_or_under(q, u)) {
                    unseen.push(q);
                }
            }
        }
        for q in unseen {
            if host.lexists(q)
                && !in_hidden(q)
                && bound.iter().any(|b| at_or_under(q, b))
                && real(q, &mut unreal)
            {
                if host.is_real_dir(q) {
                    m.by_dest.retain(|d, _| !under(d, q));
                }
                mask(&mut m, q);
            }
        }
    }
    refuse.extend(unreal);

    // state, fixed files, pot's own
    for s in &p.mounts.state {
        m.bind(&s.dir, &in_home(home, &s.at), WRITE, false);
        host_dirs.push(s.dir.clone());
    }
    for f in p.effective("file").filter(|f| f.mode.as_deref() == Some("fixed")) {
        m.bind(
            f.source.as_deref().unwrap_or_default(),
            &in_home(home, &f.value),
            READ,
            true,
        );
    }
    files.push(RunFile::File {
        name: "denials".into(),
        text: String::new(),
        mode: 0o644,
    });
    for n in ["policy.json", "denials"] {
        m.bind(&rd(n), &format!("{run}/{n}"), READ, true);
    }
    // the network: pot's proxy listens on the run folder's proxy.sock (run starts it before the engine), bound where
    // the run's socat reaches it; the variables, with a secret, a binding or the network, over the run folder's named pipe
    let net: Vec<String> = p
        .effective("network")
        .filter(|n| n.class == "reach")
        .map(|n| n.value.clone())
        .collect();
    let net_tools = i.net.filter(|_| !net.is_empty());
    let creds = crate::bindings::creds(p);
    let fifo = net_tools.is_some() || p.effective("env").any(|e| e.secret == Some(true)) || !creds.is_empty();
    if net_tools.is_some() {
        m.bind(&rd("proxy.sock"), PROXY_SOCK, READ, true);
    }
    if fifo {
        m.bind(&rd("secrets.fifo"), FIFO, READ, true);
    }
    // the bindings (bwrap's write_own and prepare): mcp.json for every run, read-only; with a via-file binding, the
    // run folder's secrets/ (0700, empty: run writes each value there as NAME, 0400, and removes it), read-only
    files.push(RunFile::File {
        name: "mcp.json".into(),
        text: crate::bindings::mcp_json(p),
        mode: 0o444,
    });
    m.bind(&rd("mcp.json"), &format!("{run}/mcp.json"), READ, true);
    if !crate::bindings::via_file(&creds).is_empty() {
        files.push(RunFile::PrivateDir {
            name: "secrets".into(),
        });
        m.bind(&rd("secrets"), &format!("{run}/secrets"), READ, true);
    }
    let hidden_txt: String = p
        .effective("hidden")
        .flat_map(|h| [h.value.as_str(), "\n"])
        .collect();
    files.push(RunFile::File {
        name: "hidden.txt".into(),
        text: hidden_txt,
        mode: 0o444,
    });
    m.bind(&rd("hidden.txt"), &format!("{home}/.pot-hidden"), READ, true);
    if let Some(pb) = &l.pot_bin {
        // the launcher pointer (v2.18.0): this run's own pot, read-only, and ~/.config/pot/launcher naming it
        m.bind(
            &format!("{}/pot", pb.trim_end_matches('/')),
            &p.mounts.launcher,
            READ,
            true,
        );
        files.push(RunFile::Dir {
            name: "potcfg".into(),
        });
        files.push(RunFile::Link {
            name: "potcfg/launcher".into(),
            target: p.mounts.launcher.clone(),
        });
        m.bind(&rd("potcfg"), &format!("{home}/.config/pot"), READ, true);
    }

    // the closure: each store path read-only (closure-store), or the whole store read-only
    let closure: Vec<String> = if p.proves("closure-store") {
        let net_closure = net_tools.map(|t| t.closure.as_slice()).unwrap_or_default();
        let set: BTreeSet<&String> = l
            .closure
            .iter()
            .chain(i.base_closure)
            .chain(net_closure)
            .collect();
        set.into_iter().cloned().collect()
    } else {
        vec!["/nix/store".into()]
    };

    let mut mounts: Vec<(String, Mount)> = m.by_dest.into_iter().map(|(d, (_, mt))| (d, mt)).collect();
    // every workspace path still bound onto itself, by kind, for prepare to record and run to check again
    let pins: Vec<(String, Kind)> = mounts
        .iter()
        .filter_map(|(dest, mt)| match mt {
            Mount::Bind { src, .. } if src == dest => kinds.get(src).map(|k| (src.clone(), *k)),
            _ => None,
        })
        .collect();
    // podman and docker order mounts by destination themselves; this order is for people
    mounts.sort_by(|a, b| (a.0.matches('/').count(), &a.0).cmp(&(b.0.matches('/').count(), &b.0)));
    let bad = unsayable(&mounts, &closure);
    if !bad.is_empty() {
        refuse.push(format!(
            "paths a bind can't name (':', ',' or '\"'): {}",
            bad.join(", ")
        ));
    }
    if !refuse.is_empty() {
        return Err(refuse);
    }

    let o = |a: &[&str]| Opt {
        args: a.iter().map(|s| (*s).to_string()).collect(),
        closure: false,
    };
    // no --rm: run removes the container once the engine has exited, and waits for its processes (N7)
    let mut opts = vec![
        o(&["run", "--interactive"]),
        // the engine's init is PID 1, never the command (PID 1 ignores a signal it has no handler for)
        o(&["--init"]),
        o(&["--log-driver", "none"]),
        o(&["--cidfile", &rd("container.id")]),
        o(&["--hostname", "pot"]),
        o(&["--network", "none"]),
        o(&["--ipc", "private"]),
        o(&["--cgroupns", "private"]),
        o(&["--cap-drop", "all"]),
        o(&["--security-opt", "no-new-privileges"]),
        o(&["--read-only"]),
    ];
    if i.engine == Engine::Podman {
        opts.insert(
            0,
            Opt {
                args: podman_globals(i.rundir),
                closure: false,
            },
        );
    }
    let ids = format!("{}:{}", l.uid, l.gid);
    match i.engine {
        Engine::Podman => {
            opts.push(o(&["--userns", "keep-id"]));
            opts.push(o(&["--user", &ids]));
            opts.push(o(&["--passwd=false"]));
            if p.proves("resource-limits") {
                opts.push(o(&["--cgroups", "split"]));
            }
        }
        Engine::Docker => {
            opts.push(o(&["--user", &ids]));
            opts.push(o(&["--pull", "never"]));
            // R1: the run's name, so run and cleanup find the container before its id is in the cidfile
            opts.push(o(&["--name", &crate::lifecycle::docker_name(i.rundir)]));
            // R4: the image's entrypoint, if it had one, never runs
            opts.push(o(&["--entrypoint", ""]));
        }
    }
    for (dest, mt) in &mounts {
        opts.push(match mt {
            // podman copies what is under a tmpfs into it unless told not to (tmpcopyup is its default)
            Mount::Tmpfs { opts: t } if i.engine == Engine::Podman => {
                o(&["--tmpfs", &format!("{dest}:{t},notmpcopyup")])
            }
            Mount::Tmpfs { opts: t } => o(&["--tmpfs", &format!("{dest}:{t}")]),
            Mount::Bind { src, ro } => bind_opt(i.engine, src, dest, *ro),
        });
    }
    for c in &closure {
        let mut opt = bind_opt(i.engine, c, c, true);
        opt.closure = p.proves("closure-store");
        opts.push(opt);
    }
    let mut sources: Vec<String> = mounts
        .iter()
        .filter_map(|(_, mt)| match mt {
            Mount::Bind { src, .. } => Some(src.clone()),
            Mount::Tmpfs { .. } => None,
        })
        .collect();
    sources.extend(closure.iter().cloned());
    let (set, mut pass, defaults, hostless) =
        environment(i, net_tools.is_some(), &crate::bindings::names(&creds));
    // R3: a secret variable never goes to the engine (it writes `--env` values into the container's configuration).
    // S3: with a secret, a binding or the network, every variable passed by name goes over the named pipe, and the command
    // starts from an empty environment (env -i) with pot's values and the pipe's: the run's bash and the engine add
    // nothing to it
    let piped: Vec<String> = if fifo { std::mem::take(&mut pass) } else { vec![] };
    for (k, v) in &set {
        opts.push(o(&["--env", &format!("{k}={v}")]));
    }
    for k in &pass {
        opts.push(o(&["--env", k]));
    }

    // the engine reads pot's configuration, never the user's, and only allowlisted variables (audit F2)
    let mut engine_keep: Vec<String> = crate::engine::KEEP.iter().map(|k| (*k).to_string()).collect();
    for k in &pass {
        if !engine_keep.contains(k) {
            engine_keep.push(k.clone());
        }
    }
    let engine_set = match i.engine {
        Engine::Podman => {
            files.push(RunFile::File {
                name: "containers.conf".into(),
                text: crate::engine::containers_conf(&rd("empty")),
                mode: 0o444,
            });
            files.push(RunFile::File {
                name: "storage.conf".into(),
                text: crate::engine::storage_conf(&rd("enginestore")),
                mode: 0o444,
            });
            files.push(RunFile::FreshDir {
                name: "enginehome".into(),
            });
            files.push(RunFile::FreshDir {
                name: "enginestore".into(),
            });
            vec![
                ("CONTAINERS_CONF".to_string(), rd("containers.conf")),
                ("CONTAINERS_STORAGE_CONF".to_string(), rd("storage.conf")),
                ("HOME".to_string(), rd("enginehome")),
            ]
        }
        Engine::Docker => {
            files.push(RunFile::Dir {
                name: "dockercfg".into(),
            });
            vec![("DOCKER_CONFIG".to_string(), rd("dockercfg"))]
        }
    };
    let root = match i.engine {
        Engine::Podman => {
            files.push(RunFile::Dir {
                name: "rootfs".into(),
            });
            vec!["--rootfs".to_string(), format!("{}:O", rd("rootfs"))]
        }
        Engine::Docker => vec![i.image.to_string()],
    };
    // podman can't take -w with --rootfs (the spike): coreutils env changes directory, then execs the command
    let mut entry = vec![];
    if fifo {
        // T1: env -i clears what the engine set; bash reads the pipe, starts socat with the network, and declares the
        // records; the last hop drops what bash adds and adds pot's values. No record is ever an argument
        entry.extend(["/usr/bin/env", "-i", "--"].map(String::from));
        entry.extend(
            [
                i.paths.shell.as_str(),
                "--noprofile",
                "--norc",
                "-c",
                ENTRY_SCRIPT,
                "pot-entry",
                FIFO,
                net_tools.map_or("", |t| t.socat.as_str()),
            ]
            .map(String::from),
        );
        // V1: the planned names, which the script holds the records to
        entry.push(piped.len().to_string());
        entry.extend(piped.iter().cloned());
        entry.push("/usr/bin/env".into());
        for n in BASH_ADDS {
            entry.extend(["-u".to_string(), (*n).to_string()]);
        }
        entry.extend(["-C".to_string(), cwd, "--".into()]);
        entry.extend(set.iter().map(|(k, v)| format!("{k}={v}")));
    } else {
        // what the engine adds to the command's environment, unless the plan sets it (S3)
        entry.extend(["/usr/bin/env".to_string(), "-C".into(), cwd]);
        for n in ENGINE_ADDS
            .iter()
            .filter(|n| !set.contains_key(**n) && !pass.iter().any(|x| x == *n))
        {
            entry.extend(["-u".to_string(), (*n).to_string()]);
        }
        entry.push("--".into());
    }
    entry.extend(l.argv.iter().cloned());
    Ok(Plan {
        program: i.program.to_string(),
        opts,
        root,
        entry,
        files,
        host_dirs,
        sources,
        engine_keep,
        engine_set,
        piped,
        defaults,
        fifo,
        proxy: net_tools.map(|t| proxy_argv(t, &p.context, l, &rd("denials"), &net)),
        creds,
        hostless,
        secrets_at: format!("{run}/secrets"),
        placeholders,
        seals,
        pins,
        notes,
    })
}

/// pot's proxy as bwrap's `run` starts it, in the run folder (its socket `proxy.sock`, relative: a run folder's
/// path may be longer than `sun_path`'s 108 bytes), with the run's denials file, its tmux notices, the fix a denial
/// names, and the allowlist: the effective network reach items.
fn proxy_argv(t: &NetTools, context: &str, l: &Launch, denials: &str, allow: &[String]) -> Vec<String> {
    let mut v: Vec<String> = [
        t.python.as_str(),
        "-I",
        "-S",
        t.proxy.as_str(),
        "proxy.sock",
        "--denials",
        denials,
        "--context",
        context,
    ]
    .map(String::from)
    .to_vec();
    if let (Some(tmux), Some(pane)) = (&l.host_tools.tmux, &l.pane) {
        v.extend(["--tmux".to_string(), tmux.clone(), "--pane".into(), pane.clone()]);
    }
    v.extend([
        "--fix".to_string(),
        l.net_fix
            .clone()
            .unwrap_or_else(|| "the repo kit's requests.network".into()),
    ]);
    if l.sibling_of.as_ref().is_some_and(|s| !s.is_null()) {
        // no one-run line: pot --net is the host's
        v.extend(["--sibling".to_string(), "1".into()]);
    }
    v.extend(allow.iter().cloned());
    v
}

/// A bind: podman's `--volume`; docker's `--mount`, since docker's `--volume` creates a missing source as root.
fn bind_opt(engine: Engine, src: &str, dest: &str, ro: bool) -> Opt {
    let arg = match engine {
        Engine::Podman => (
            "--volume",
            format!("{src}:{dest}:{}", if ro { "ro" } else { "rw" }),
        ),
        Engine::Docker => (
            "--mount",
            format!(
                "type=bind,source={src},target={dest}{}",
                if ro { ",readonly" } else { "" }
            ),
        ),
    };
    Opt {
        args: vec![arg.0.to_string(), arg.1],
        closure: false,
    }
}

/// The variables bwrap's `run_env` sets to the proxy's URL in a run with the network.
const PROXY_VARS: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

fn is_proxy_var(n: &str) -> bool {
    PROXY_VARS.contains(&n) || n == "NO_PROXY" || n == "no_proxy"
}

/// The run's environment: what pot sets (bwrap's `run_env`, less what this slice does not place), then the
/// names passed from the host by name only (`--env NAME`: the engine reads the value, so no value is in argv).
/// BOUND, the bindings' variables, come last (bwrap's `run` updates `run_env`'s with `deliver`'s): each is passed by
/// name, and what the plan would have set is its default; those not passed from the host otherwise are returned as
/// `hostless`.
#[allow(clippy::type_complexity)] // the set values, the names passed, their defaults, the hostless ones
fn environment(
    i: &Inputs,
    net: bool,
    bound: &[String],
) -> (
    BTreeMap<String, String>,
    Vec<String>,
    BTreeMap<String, String>,
    Vec<String>,
) {
    let (p, l) = (i.policy, i.launch);
    let home = l.home.trim_end_matches('/');
    let mut path: Vec<String> = vec![];
    let mut tools = None;
    for pk in p.effective("package") {
        let b = format!("{}/bin", pk.value);
        if pk.tools == Some(true) {
            tools = Some(b.clone());
        }
        if !path.contains(&b) {
            path.push(b);
        }
    }
    if let Some(pb) = &l.pot_bin {
        path.push(pb.clone());
    }
    let ca = "/etc/ssl/certs/ca-certificates.crt";
    let mut env: BTreeMap<String, String> = [
        ("HOME", home.to_string()),
        ("USER", l.user.clone()),
        ("LOGNAME", l.user.clone()),
        ("SHELL", i.paths.shell.clone()),
        ("PATH", path.join(":")),
        ("LANG", "C.UTF-8".into()),
        ("TERMINFO_DIRS", i.paths.terminfo.clone()),
        ("SSL_CERT_FILE", ca.into()),
        ("NIX_SSL_CERT_FILE", ca.into()),
        ("TZDIR", i.paths.tzdir.clone()),
        ("POT", p.context.clone()),
        ("POT_POLICY", format!("{}/policy.json", p.mounts.run)),
        ("POT_RUN", p.run.clone()),
        ("PS1", format!("(pot:{}) \\w \\$ ", p.context)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    if let Some(t) = tools {
        env.insert("POT_TOOLS".into(), t);
    }
    if p.mode == "clone" {
        env.insert("POT_CLONE".into(), "1".into());
    }
    for e in p.effective("env.set") {
        if let Some((n, v)) = e.value.split_once('=') {
            env.insert(n.to_string(), in_home(home, v));
        }
    }
    // bwrap's order (run_env): pot's values, then the host's TERM and COLORTERM, then env.set, then the env items
    // the host has set. So env.set beats TERM and COLORTERM, and an item set on the host beats everything: an item
    // named like one of pot's values is passed by name, with pot's value as its default (U2)
    let items: Vec<String> = p.effective("env").map(|e| e.value.clone()).collect();
    let mut pass: Vec<String> = ["TERM", "COLORTERM"]
        .iter()
        .filter(|t| !env.contains_key(**t) || items.iter().any(|i| i == *t))
        .map(|t| (*t).to_string())
        .collect();
    for n in items {
        if !pass.contains(&n) {
            pass.push(n);
        }
    }
    // the network last, as bwrap's run_env: the proxy's variables beat env.set and the items (an item of the same
    // name is not passed)
    if net {
        for v in PROXY_VARS {
            env.insert((*v).to_string(), PROXY_URL.into());
        }
        for v in ["NO_PROXY", "no_proxy"] {
            env.insert(v.to_string(), "localhost,127.0.0.1,::1".into());
        }
        pass.retain(|n| !is_proxy_var(n));
    }
    let mut hostless = vec![];
    for n in bound {
        if !pass.contains(n) {
            pass.push(n.clone());
            hostless.push(n.clone());
        }
    }
    let defaults = pass
        .iter()
        .filter_map(|n| env.remove(n).map(|v| (n.clone(), v)))
        .collect();
    (env, pass, defaults, hostless)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::*;
    use crate::policy;

    /// A host where exactly these paths exist: DIRS are real folders; SOCKS are sockets (under some folder).
    struct Fake {
        dirs: Vec<&'static str>,
        files: Vec<&'static str>,
        socks: Vec<&'static str>,
        /// (path, its real path): a link, or a path with `..`
        links: Vec<(&'static str, &'static str)>,
    }

    impl Host for Fake {
        fn lexists(&self, p: &str) -> bool {
            self.dirs.contains(&p) || self.files.contains(&p) || self.socks.contains(&p)
        }
        fn is_real_dir(&self, p: &str) -> bool {
            self.dirs.contains(&p)
        }
        fn sockets_under(&self, d: &str) -> Vec<String> {
            self.socks
                .iter()
                .filter(|s| under(s, d))
                .map(|s| (*s).to_string())
                .collect()
        }
        fn realpath(&self, p: &str) -> Option<String> {
            for (from, to) in &self.links {
                if at_or_under(p, from) {
                    return Some(format!("{to}{}", &p[from.len()..]));
                }
            }
            self.lexists(p).then(|| p.to_string())
        }
        /// A folder's entries: a link (in LINKS, what it says), a folder (in DIRS) or a file.
        fn entries(&self, d: &str) -> Vec<(String, Entry)> {
            let mut out: Vec<(String, Entry)> = vec![];
            let child = |p: &str| {
                p.strip_prefix(&format!("{d}/"))
                    .filter(|r| !r.contains('/'))
                    .map(String::from)
            };
            for (from, to) in &self.links {
                if let Some(n) = child(from) {
                    out.push((n, Entry::Link((*to).to_string())));
                }
            }
            for p in &self.dirs {
                if let Some(n) = child(p) {
                    out.push((n, Entry::Folder));
                }
            }
            for p in &self.files {
                if let Some(n) = child(p) {
                    out.push((n, Entry::File));
                }
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            out
        }
    }

    fn nothing() -> Fake {
        Fake {
            dirs: vec![],
            files: vec![],
            socks: vec![],
            links: vec![],
        }
    }

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn read(rel: &str) -> String {
        std::fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    fn paths() -> Paths {
        Paths {
            sh: "/nix/store/b-bash/bin/sh".into(),
            shell: "/nix/store/b-bash/bin/bash".into(),
            env: "/nix/store/c-coreutils/bin/env".into(),
            cacert: "/nix/store/n-cacert/ca-bundle.crt".into(),
            terminfo: "/nix/store/t-ncurses/share/terminfo".into(),
            tzdir: "/nix/store/z-tzdata/share/zoneinfo".into(),
        }
    }

    const PREPARE: &str = "docs/reference/container/prepare.in";

    fn run_dir(name: &str) -> (Policy, Launch) {
        let p = policy::parse(&read(&format!("{PREPARE}/{name}/policy.json"))).unwrap();
        let l = serde_json::from_str(&read(&format!("{PREPARE}/{name}/launch.json"))).unwrap();
        (p, l)
    }

    fn go(p: &Policy, l: &Launch, engine: Engine, host: &dyn Host) -> Result<Plan, Vec<String>> {
        go_with(p, l, engine, host, None)
    }

    /// The config's network programs, as a test names them.
    fn net_tools() -> NetTools {
        NetTools {
            python: "/nix/store/p-python/bin/python3".into(),
            proxy: "/nix/store/q-pot/proxy.py".into(),
            socat: "/nix/store/s-socat/bin/socat".into(),
            closure: vec!["/nix/store/s-socat".into(), "/nix/store/o-openssl".into()],
        }
    }

    fn go_net(p: &Policy, l: &Launch, engine: Engine, host: &dyn Host) -> Result<Plan, Vec<String>> {
        go_with(p, l, engine, host, Some(&net_tools()))
    }

    fn go_with(
        p: &Policy,
        l: &Launch,
        engine: Engine,
        host: &dyn Host,
        net: Option<&NetTools>,
    ) -> Result<Plan, Vec<String>> {
        let paths = paths();
        let base = vec![
            "/nix/store/b-bash".to_string(),
            "/nix/store/c-coreutils".to_string(),
        ];
        render(&Inputs {
            policy: p,
            launch: l,
            paths: &paths,
            engine,
            program: engine.name(),
            image: DOCKER_IMAGE,
            rundir: "/r",
            base_closure: &base,
            net,
            look: Some(&look_tools()),
            host,
        })
    }

    /// The config's programs for the host's look at a write run, as a test names them.
    fn look_tools() -> LookTools {
        LookTools {
            python: "/nix/store/p-python/bin/python3".into(),
            pins: "/nix/store/q-pot/pins.py".into(),
            git: "/nix/store/g-git/bin/git".into(),
        }
    }

    /// The argv as one line per option, for assertions.
    fn lines(plan: &Plan) -> Vec<String> {
        plan.opts.iter().map(|o| o.args.join(" ")).collect()
    }

    fn has(plan: &Plan, line: &str) -> bool {
        lines(plan).iter().any(|l| l == line)
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one plan, every flag it must carry
    fn read_access_binds_the_workspace_read_only_and_masks_what_is_hidden() {
        let (p, l) = run_dir("read-only");
        let host = Fake {
            dirs: vec!["/home/u/docs", "/home/u/docs/.ssh", "/home/u/src/notes/secrets"],
            files: vec!["/home/u/docs/.netrc"],
            socks: vec![
                "/home/u/docs/agent.sock",
                "/home/u/src/notes/.direnv/s.sock",
                "/home/u/src/notes/secrets/x.sock",
            ],
            links: vec![],
        };
        let mut p = p;
        // hide a folder in the workspace, and paths under the read folder (one folder, one file)
        for it in p.items.iter_mut().filter(|i| i.kind == "hidden") {
            it.paths = Some(vec![
                "/home/u/docs/.ssh".into(),
                "/home/u/docs/.netrc".into(),
                "/home/u/src/notes/secrets".into(),
            ]);
        }
        let plan = go(&p, &l, Engine::Podman, &host).unwrap();
        for want in [
            "--volume /home/u/src/notes:/home/u/src/notes:ro",
            "--volume /home/u/docs:/home/u/docs:ro",
            "--volume /r/emptydir:/home/u/docs/.ssh:ro",
            "--volume /r/empty:/home/u/docs/.netrc:ro",
            "--volume /r/emptydir:/home/u/src/notes/secrets:ro",
            "--volume /r/empty:/home/u/docs/agent.sock:ro",
            "--volume /r/empty:/home/u/src/notes/.direnv/s.sock:ro",
            "--volume /home/u/.local/state/pot/state/9f2e/notes:/home/u/.cache/notes:rw",
            "--volume /nix/store/0aqd4v7xhp2c9z2lfy5h4v0rdf3q4k8n-config.toml:/home/u/.config/notes/config.toml:ro",
            "--tmpfs /home/u:rw,exec,nosuid,nodev,mode=0700,U,notmpcopyup",
            "--network none",
            "--cap-drop all",
            "--security-opt no-new-privileges",
            "--read-only",
            "--userns keep-id",
            "--user 1000:100",
            "--cgroups split",
        ] {
            assert!(has(&plan, want), "missing {want}\n{:#?}", lines(&plan));
        }
        // K4: nothing is bound inside a hidden folder (its socket is not masked: the whole folder is)
        assert!(!lines(&plan).iter().any(|x| x.contains("secrets/x.sock")));
        // R3: a secret variable never reaches the engine: not in argv, not as --env NAME (the engine would write
        // its value into the container's configuration), not in the engine's environment. run hands it over the run
        // folder's named pipe, bound read-only, and the run's bash sets it before the command starts
        assert!(!has(&plan, "--env NOTES_TOKEN"));
        assert!(!plan.argv().iter().any(|a| a.starts_with("NOTES_TOKEN=")));
        assert!(!plan.engine_keep.iter().any(|k| k == "NOTES_TOKEN"));
        // S3: with a secret, every variable passed by name goes over the pipe, and the command starts from env -i
        // with pot's values as operands: neither the run's bash nor the engine adds to its environment
        assert_eq!(plan.piped, ["TERM", "COLORTERM", "NOTES_TOKEN"]);
        assert!(!has(&plan, "--env TERM"));
        // T1: env -i, then bash, then the last hop with pot's values; the records are declared in bash
        assert!(plan.fifo && plan.entry[10].is_empty() && plan.proxy.is_none());
        assert!(has(&plan, "--volume /r/secrets.fifo:/run/pot/secrets.fifo:ro"));
        assert!(!lines(&plan).iter().any(|x| x.contains("--preserve-fd")));
        assert_eq!(
            plan.entry[..24],
            [
                "/usr/bin/env",
                "-i",
                "--",
                "/nix/store/b-bash/bin/bash",
                "--noprofile",
                "--norc",
                "-c",
                ENTRY_SCRIPT,
                "pot-entry",
                "/run/pot/secrets.fifo",
                "",
                "3",
                "TERM",
                "COLORTERM",
                "NOTES_TOKEN",
                "/usr/bin/env",
                "-u",
                "_",
                "-u",
                "SHLVL",
                "-u",
                "PWD",
                "-u",
                "OLDPWD"
            ]
        );
        // V1: the planned names (not secret) are the script's arguments: their count, then each, in the pipe's order
        assert_eq!(plan.entry[24..27], ["-C", "/home/u/src/notes", "--"]);
        let last = &plan.entry[27..];
        assert!(last.contains(&"NOTES_CACHE=/home/u/.cache/notes".to_string()));
        assert!(
            last.contains(&"PS1=(pot:review) \\w \\$ ".to_string()),
            "{last:?}"
        );
        assert_eq!(last[last.len() - 3..], ["notes", "check", "--all"]);
        assert!(has(&plan, "--env NOTES_CACHE=/home/u/.cache/notes"));
        assert_eq!(plan.root, ["--rootfs", "/r/rootfs:O"]);
        // without a secret: env -C, and what the engine adds (HOSTNAME) removed
        let (p2, l2) = run_dir("access-none");
        let plan2 = go(&p2, &l2, Engine::Podman, &nothing()).unwrap();
        assert!(plan2.piped.is_empty() && has(&plan2, "--env TERM"));
        assert_eq!(
            plan2.entry[..6],
            ["/usr/bin/env", "-C", "/home/u", "-u", "HOSTNAME", "--"]
        );
        assert_eq!(plan.host_dirs, ["/home/u/.local/state/pot/state/9f2e/notes"]);
        // closure-store: each store path of the closure and the base closure, read-only, and never the whole store
        let closure: BTreeSet<String> = plan
            .opts
            .iter()
            .filter(|o| o.closure)
            .map(|o| o.args[1].clone())
            .collect();
        assert_eq!(closure.len(), 5);
        assert!(closure.contains("/nix/store/b-bash:/nix/store/b-bash:ro"));
        assert!(!has(&plan, "--volume /nix/store:/nix/store:ro"));
    }

    #[test]
    fn a_read_path_that_does_not_exist_or_is_hidden_is_not_bound() {
        let (p, l) = run_dir("read-only");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(!lines(&plan).iter().any(|x| x.contains("/home/u/docs")));
        let mut p = p;
        for it in p.items.iter_mut().filter(|i| i.kind == "hidden") {
            it.paths = Some(vec!["/home/u".into()]);
        }
        let host = Fake {
            dirs: vec!["/home/u", "/home/u/docs"],
            files: vec![],
            socks: vec![],
            links: vec![],
        };
        let plan = go(&p, &l, Engine::Podman, &host).unwrap();
        assert!(
            !lines(&plan)
                .iter()
                .any(|x| x.starts_with("--volume /home/u/docs"))
        );
    }

    #[test]
    fn access_none_binds_no_workspace_and_starts_in_the_home() {
        let (p, l) = run_dir("access-none");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(
            !lines(&plan).iter().any(|x| x.contains("/home/u/src/notes")),
            "{:#?}",
            lines(&plan)
        );
        assert_eq!(plan.entry[..3], ["/usr/bin/env", "-C", "/home/u"]);
        assert!(!has(&plan, "--cgroups split"));
    }

    #[test]
    fn docker_gets_a_user_an_image_and_no_userns_and_refuses_resource_limits() {
        let (p, l) = run_dir("access-none");
        let plan = go(&p, &l, Engine::Docker, &nothing()).unwrap();
        assert!(has(&plan, "--user 1000:100") && has(&plan, "--pull never"));
        assert!(has(
            &plan,
            "--tmpfs /home/u:rw,exec,nosuid,nodev,mode=0700,uid=1000,gid=100"
        ));
        assert!(
            !lines(&plan)
                .iter()
                .any(|x| x.starts_with("--userns") || x.starts_with("--passwd"))
        );
        assert_eq!(plan.root, [DOCKER_IMAGE]);
        let (p, l) = run_dir("read-only");
        let gaps = go(&p, &l, Engine::Docker, &nothing()).unwrap_err();
        assert_eq!(
            gaps,
            ["resource-limits under docker (a container is not in pot's scope)"]
        );
        // a secret under docker, as under podman: the named pipe, never --env NAME
        let (mut p, l) = run_dir("read-only");
        p.backend.proved.retain(|g| g != "resource-limits");
        let plan = go(&p, &l, Engine::Docker, &nothing()).unwrap();
        assert!(plan.fifo && plan.piped.contains(&"NOTES_TOKEN".to_string()));
        assert!(has(
            &plan,
            "--mount type=bind,source=/r/secrets.fifo,target=/run/pot/secrets.fifo,readonly"
        ));
        assert!(!has(&plan, "--env NOTES_TOKEN") && !plan.engine_keep.iter().any(|k| k == "NOTES_TOKEN"));
        assert_eq!(plan.entry[..2], ["/usr/bin/env", "-i"]);
    }

    #[test]
    fn without_closure_store_the_whole_store_is_bound_read_only() {
        let (mut p, l) = run_dir("access-none");
        p.backend.proved.retain(|g| g != "closure-store");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(has(&plan, "--volume /nix/store:/nix/store:ro"));
        assert!(plan.opts.iter().all(|o| !o.closure));
    }

    #[test]
    fn unsayable_paths_are_refused() {
        let (mut p, l) = run_dir("read-only");
        p.workspace[0].path = "/home/u/src/a:b".into();
        p.workspace[0].common = "/home/u/src/a:b/.git".into();
        let gaps = go(&p, &l, Engine::Podman, &nothing()).unwrap_err();
        assert_eq!(
            gaps,
            ["paths a bind can't name (':', ',' or '\"'): /home/u/src/a:b"]
        );
    }

    #[test]
    fn a_guarantee_this_slice_does_not_uphold_is_refused() {
        let (mut p, l) = run_dir("access-none");
        p.backend.proved.push("same-path-clone".into());
        let gaps = go(&p, &l, Engine::Podman, &nothing()).unwrap_err();
        assert_eq!(gaps, ["the same-path-clone guarantee"]);
    }

    /// Every policy-1 fixture, as if pot had written it for this backend (its name, and the guarantees this
    /// backend's probe proves, less closure-store where pot dropped it): what this slice refuses in each.
    #[test]
    fn the_policy_1_fixtures_refuse_what_this_slice_lacks() {
        let l: Launch = serde_json::from_str(&read(&format!("{PREPARE}/read-only/launch.json"))).unwrap();
        let expect: &[(&str, &[&str])] = &[
            (
                "dispatch-clone",
                &[
                    "clone mode (the clone at its original path, its pins, the staging fetch)",
                    "sibling children (the run's children broker)",
                ],
            ),
            (
                "git-fetch",
                &[
                    "write access without launch.json's pins for /tmp/pot-test/sbc (pot 0.3.0 or later resolves them)",
                    "the git.fetch binding",
                ],
            ),
            ("inspect", &["inspect's view"]),
            (
                "nix",
                &[
                    "write access without launch.json's pins for /tmp/pot-test/sbc (pot 0.3.0 or later resolves them)",
                    "the nix binding",
                ],
            ),
            (
                "pi-agent",
                &[
                    "write access without launch.json's pins for /tmp/pot-test/piws (pot 0.3.0 or later resolves them)",
                    "the status.relay binding",
                    "dir ~/.pi/agent/extensions (a folder of links)",
                    "dir ~/.pi/agent/git (a folder of links)",
                    "dir ~/.pi/agent/npm (a folder of links)",
                    "dir ~/.pi/agent/prompts (a folder of links)",
                    "dir ~/.pi/agent/skills (a folder of links)",
                    "dir ~/.pi/agent/themes (a folder of links)",
                ],
            ),
            (
                "seatbelt",
                &[
                    "clone mode (the clone at its original path, its pins, the staging fetch)",
                    "sibling children (the run's children broker)",
                ],
            ),
            ("sibling", &["sibling children (the run's children broker)"]),
            (
                "work-bindings",
                &[
                    "write access without launch.json's pins for /tmp/pot-test/sbc (pot 0.3.0 or later resolves them)",
                    "the status.relay binding",
                    "file ~/.st/conf.json (mode reassert)",
                ],
            ),
            (
                "work-siblings",
                &[
                    "write access without launch.json's pins for /tmp/pot-test/nc (pot 0.3.0 or later resolves them)",
                    "sibling children (the run's children broker)",
                ],
            ),
        ];
        for (name, want) in expect {
            let text = read(&format!("contracts/fixtures/policy-1/{name}.json"));
            let mut p = policy::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            let had_store = p.proves("closure-store");
            p.backend.name = policy::BACKEND.into();
            p.backend.proved = crate::probe::GUARANTEES
                .iter()
                .filter(|g| had_store || **g != "closure-store")
                .map(|g| (*g).to_string())
                .collect();
            let got = go_net(&p, &l, Engine::Podman, &nothing()).unwrap_err();
            assert_eq!(&got, want, "{name}");
        }
    }

    #[test]
    fn the_invalid_policy_1_fixtures_are_refused_with_the_rule_they_break() {
        let expect = [
            (
                "capability-without-binding",
                "item capability \"status.relay\": no binding",
            ),
            ("clone-in-place", "a clone in mode in-place"),
            ("credential-value", "unknown field `value`"),
            ("effective-unsandboxed", "an unsandboxed item in effect"),
            ("hidden-not-effective", "a limits item that is not in effect"),
            (
                "network-without-port",
                "item network \"example.com\": not host:port",
            ),
            ("not-effective-without-why", "not in effect, and no why"),
            (
                "row-admitted-and-refused",
                "neither admitted nor refused, or both",
            ),
            ("unknown-field", "unknown field `owner`"),
            (
                "version-2",
                "policy.json's version is not 1: this backend reads version 1 only",
            ),
        ];
        let dir = repo().join("contracts/fixtures/policy-1/invalid");
        let mut seen = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let name = e
                .unwrap()
                .path()
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let (_, want) = expect
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("no expectation for {name}"));
            let err = policy::parse(&read(&format!("contracts/fixtures/policy-1/invalid/{name}.json")))
                .unwrap_err();
            assert!(err.to_string().contains(want), "{name}: {err}");
            seen += 1;
        }
        assert_eq!(seen, expect.len());
    }

    fn existing(dirs: Vec<&'static str>) -> Fake {
        Fake {
            dirs,
            files: vec![],
            socks: vec![],
            links: vec![],
        }
    }

    /// F1: with access none, a bound folder above the workspace shows neither the workspace nor its git
    /// directory (bwrap's `unseen`), and the command starts in the home wherever it was started.
    #[test]
    fn access_none_masks_the_workspace_under_a_bound_folder_and_starts_in_the_home() {
        let (mut p, mut l) = run_dir("access-none");
        p.items.push(read_item("/home/u/src"));
        p.workspace[0].common = "/home/u/wt/notes.git".into();
        p.items.push(read_item("/home/u/wt"));
        l.cwd = "/tmp".into();
        let host = existing(vec![
            "/home/u/src",
            "/home/u/src/notes",
            "/home/u/wt",
            "/home/u/wt/notes.git",
        ]);
        let plan = go(&p, &l, Engine::Podman, &host).unwrap();
        assert!(has(&plan, "--volume /home/u/src:/home/u/src:ro"));
        assert!(
            has(&plan, "--volume /r/emptydir:/home/u/src/notes:ro"),
            "{:#?}",
            lines(&plan)
        );
        assert!(has(&plan, "--volume /r/emptydir:/home/u/wt/notes.git:ro"));
        assert_eq!(plan.entry[..3], ["/usr/bin/env", "-C", "/home/u"]);
    }

    fn read_item(path: &str) -> crate::policy::Item {
        let v = serde_json::json!({ "kind": "read", "value": path, "class": "reach", "sources": [ { "flag": "--read" } ],
                                    "effective": true, "path": path });
        serde_json::from_value(v).unwrap()
    }

    /// F3: a bound path, a workspace or a hidden path that is not its own real path (a link, `..`) is refused:
    /// masking compares paths as strings.
    #[test]
    fn a_path_that_is_not_its_real_path_is_refused() {
        let (mut p, l) = run_dir("read-only");
        p.items.push(read_item("/home/u/docs/../src"));
        let host = Fake {
            dirs: vec!["/home/u/docs", "/home/u/src", "/home/u/src/notes", "/home/u/link"],
            files: vec![],
            socks: vec![],
            links: vec![
                ("/home/u/docs/../src", "/home/u/src"),
                ("/home/u/link", "/home/u/src/notes"),
            ],
        };
        let gaps = go(&p, &l, Engine::Podman, &host).unwrap_err();
        assert_eq!(gaps, ["/home/u/docs/../src is not its real path (/home/u/src)"]);
        let (mut p, l) = run_dir("read-only");
        p.workspace[0].path = "/home/u/link".into();
        p.workspace[0].common = "/home/u/link/.git".into();
        let gaps = go(&p, &l, Engine::Podman, &host).unwrap_err();
        assert_eq!(gaps, ["/home/u/link is not its real path (/home/u/src/notes)"]);
    }

    /// N4a: a hidden path that is a link (Home Manager's files are) is not refused: the link is masked where a bound
    /// folder shows it, and so is its real path, by the real path's kind.
    #[test]
    fn a_hidden_link_is_masked_with_its_real_path() {
        let (mut p, l) = run_dir("read-only");
        for it in p.items.iter_mut().filter(|i| i.kind == "hidden") {
            it.paths = Some(vec!["/home/u/.ssh".into(), "/home/u/docs/.gnupg".into()]);
        }
        let host = Fake {
            dirs: vec!["/home/u/docs", "/home/u/docs/ssh", "/home/u/docs/gnupg-real"],
            files: vec!["/home/u/.ssh", "/home/u/docs/.gnupg"],
            socks: vec![],
            links: vec![
                ("/home/u/.ssh", "/home/u/docs/ssh"),
                ("/home/u/docs/.gnupg", "/home/u/docs/gnupg-real"),
            ],
        };
        let plan = go(&p, &l, Engine::Podman, &host).unwrap();
        for want in [
            "--volume /r/emptydir:/home/u/docs/ssh:ro",
            "--volume /r/emptydir:/home/u/docs/.gnupg:ro",
            "--volume /r/emptydir:/home/u/docs/gnupg-real:ro",
        ] {
            assert!(has(&plan, want), "missing {want}\n{:#?}", lines(&plan));
        }
        assert!(!lines(&plan).iter().any(|x| x.ends_with(":/home/u/.ssh:ro")));
    }

    /// N4b: docker's --mount can't name a path with a double quote.
    #[test]
    fn docker_refuses_a_path_with_a_quote() {
        let (mut p, l) = run_dir("access-none");
        p.mounts.state[0].dir = "/home/u/state/\"x".into();
        let gaps = go(&p, &l, Engine::Docker, &nothing()).unwrap_err();
        assert_eq!(
            gaps,
            ["paths a bind can't name (':', ',' or '\"'): /home/u/state/\"x"]
        );
    }

    /// F10: a git common directory outside the workspace (a worktree's) gets the workspace's masking and socket search.
    #[test]
    fn a_common_outside_the_workspace_is_masked_and_searched_like_it() {
        let (mut p, l) = run_dir("read-only");
        p.workspace[0].common = "/home/u/wt/notes.git".into();
        for it in p.items.iter_mut().filter(|i| i.kind == "hidden") {
            it.paths = Some(vec!["/home/u/wt/notes.git/secret".into()]);
        }
        let host = Fake {
            dirs: vec!["/home/u/wt/notes.git"],
            files: vec!["/home/u/wt/notes.git/secret"],
            socks: vec!["/home/u/wt/notes.git/fsmonitor.sock"],
            links: vec![],
        };
        let plan = go(&p, &l, Engine::Podman, &host).unwrap();
        assert!(has(
            &plan,
            "--volume /home/u/wt/notes.git:/home/u/wt/notes.git:ro"
        ));
        assert!(
            has(&plan, "--volume /r/empty:/home/u/wt/notes.git/secret:ro"),
            "{:#?}",
            lines(&plan)
        );
        assert!(has(
            &plan,
            "--volume /r/empty:/home/u/wt/notes.git/fsmonitor.sock:ro"
        ));
    }

    /// F6: podman copies a tmpfs's underlying folder into it by default (tmpcopyup): every tmpfs says notmpcopyup.
    #[test]
    fn podman_tmpfs_never_copies_up() {
        let (p, l) = run_dir("read-only");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        let tmpfs: Vec<String> = lines(&plan)
            .into_iter()
            .filter(|x| x.starts_with("--tmpfs"))
            .collect();
        assert_eq!(tmpfs.len(), 3);
        assert!(tmpfs.iter().all(|t| t.ends_with(",notmpcopyup")), "{tmpfs:#?}");
    }

    /// F5: docker creates a missing bind source (as root): it gets --mount, and prepare checks every source exists.
    #[test]
    fn docker_binds_with_mount_and_missing_sources_are_found() {
        let (p, l) = run_dir("access-none");
        let plan = go(&p, &l, Engine::Docker, &nothing()).unwrap();
        assert!(
            has(
                &plan,
                "--mount type=bind,source=/r/passwd,target=/etc/passwd,readonly"
            ),
            "{:#?}",
            lines(&plan)
        );
        assert!(has(
            &plan,
            "--mount type=bind,source=/home/u/.local/state/pot/state/9f2e/notes,target=/home/u/.cache/notes"
        ));
        assert!(!lines(&plan).iter().any(|x| x.starts_with("--volume")));
        let host = existing(vec!["/nix/store/b-bash"]);
        let missing = missing_sources(&plan, "/r", &host);
        assert!(
            missing.contains(&"/nix/store/c-coreutils/bin/env".to_string()),
            "{missing:#?}"
        );
        assert!(
            !missing
                .iter()
                .any(|m| m.starts_with("/r/") || m.contains("state/9f2e")),
            "{missing:#?}"
        );
    }

    /// N7: no `--rm` (conmon's cleanup outlived `podman run` and recreated the store after cleanup): `run` removes
    /// the container itself. podman's store, runtime state (runroot) and temporary files are the run folder's,
    /// named by global options before `run` (podman ignores a runroot in the storage.conf `CONTAINERS_STORAGE_CONF`
    /// names, for a rootless user); docker has none.
    #[test]
    fn no_rm_and_podmans_state_is_the_run_folders() {
        let (p, l) = run_dir("access-none");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        let argv = plan.argv();
        assert!(!argv.iter().any(|a| a == "--rm"), "{argv:?}");
        assert_eq!(
            argv[..9],
            [
                "podman",
                "--root",
                "/r/enginestore",
                "--runroot",
                "/r/engineroot",
                "--tmpdir",
                "/r/enginetmp",
                "run",
                "--interactive"
            ]
        );
        assert_eq!(podman_globals("/r/"), argv[1..7]);
        assert!(has(&plan, "--cidfile /r/container.id"));
        // the command is never PID 1, which ignores SIGTERM and SIGINT it has no handler for: the engine's init is
        // (bwrap's own process is PID 1 in its namespace)
        assert!(has(&plan, "--init"));
        let plan = go(&p, &l, Engine::Docker, &nothing()).unwrap();
        assert!(has(&plan, "--init"));
        // R1, R4: docker's container has the run's name, and the image's entrypoint never runs
        assert!(has(
            &plan,
            &format!("--name {}", crate::lifecycle::docker_name("/r"))
        ));
        assert!(has(&plan, "--entrypoint "));
        let argv = plan.argv();
        assert!(!argv.iter().any(|a| a == "--rm"), "{argv:?}");
        assert_eq!(argv[..3], ["docker", "run", "--interactive"]);
    }

    /// T1: no record is ever an exec's argument: the entry holds no `NAME=` of a piped variable, and the script's only
    /// execs are the fd's close and the last hop's `exec "$@"` (the records are declared in bash, not passed).
    #[test]
    fn no_record_is_ever_an_execs_argument() {
        let (p, l) = run_dir("read-only");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(!plan.piped.is_empty());
        for w in plan.argv() {
            assert!(!plan.piped.iter().any(|n| w.starts_with(&format!("{n}="))), "{w}");
        }
        let execs: Vec<&str> = ENTRY_SCRIPT
            .match_indices("exec ")
            .map(|(i, _)| &ENTRY_SCRIPT[i..])
            .collect();
        assert_eq!(execs.len(), 3, "{ENTRY_SCRIPT}");
        assert!(
            execs[0].starts_with("exec {__pot_fd}<>\"$__pot_fifo\""),
            "{}",
            execs[0]
        );
        assert!(execs[1].starts_with("exec {__pot_fd}<&-;"), "{}", execs[1]);
        assert_eq!(execs[2], r#"exec "$@""#);
        assert!(!ENTRY_SCRIPT.contains("recs[@]}\" \"$@"));
        // socat starts before any record is declared, so its environment holds none
        assert!(
            ENTRY_SCRIPT.find("\"$__pot_socat\" -lf").unwrap() < ENTRY_SCRIPT.find("declare -x").unwrap()
        );
    }

    /// U2: as bwrap's `run_env`, an env item comes last: named like one of pot's values (fixed or env.set), it is
    /// passed by name and pot's value becomes its default, which run uses when the host has the item unset. env.set
    /// still beats TERM and COLORTERM, which come before it.
    #[test]
    fn an_item_named_like_pots_value_is_passed_with_that_value_as_default() {
        let (mut p, l) = run_dir("access-none");
        let mut lang = p.items.iter().find(|i| i.kind == "env").unwrap().clone();
        for it in p.items.iter_mut().filter(|i| i.kind == "env") {
            it.value = "TZ".into();
        }
        lang.value = "LANG".into();
        p.items.push(lang);
        for engine in [Engine::Podman, Engine::Docker] {
            let plan = go(&p, &l, engine, &nothing()).unwrap();
            assert!(has(&plan, "--env TZ") && has(&plan, "--env LANG"));
            assert!(
                !lines(&plan)
                    .iter()
                    .any(|x| x.starts_with("--env TZ=") || x.starts_with("--env LANG="))
            );
            assert_eq!(
                plan.defaults,
                BTreeMap::from([
                    ("LANG".to_string(), "C.UTF-8".to_string()),
                    ("TZ".to_string(), "America/Chicago".to_string())
                ])
            );
        }
        // env.set TERM: pot's value, never the host's (TERM comes before env.set in bwrap)
        let (mut p, l) = run_dir("access-none");
        for it in p.items.iter_mut().filter(|i| i.kind == "env.set") {
            it.value = "TERM=dumb".into();
        }
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(has(&plan, "--env TERM=dumb") && !has(&plan, "--env TERM") && plan.defaults.is_empty());
    }

    /// A run folder's policy with these bindings (the fixtures' shapes), its env items made plain (no secret) unless
    /// SECRET.
    fn bound(name: &str, bindings: &serde_json::Value, secret: bool) -> (Policy, Launch) {
        let mut v: serde_json::Value =
            serde_json::from_str(&read(&format!("{PREPARE}/{name}/policy.json"))).unwrap();
        v["bindings"] = bindings.clone();
        for it in v["items"].as_array_mut().unwrap() {
            if it["kind"] == "env" && !secret {
                it.as_object_mut().unwrap().remove("secret");
            }
        }
        let l = serde_json::from_str(&read(&format!("{PREPARE}/{name}/launch.json"))).unwrap();
        (policy::parse(&v.to_string()).unwrap(), l)
    }

    fn cred_binding(sel: &str, env: &str, via: &str, from: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "capability": "credential", "selector": sel, "kit": "root:test",
            "request": format!("capability credential {sel} env={env}"), "via": via, "env": [env], "from": from,
            "holds": "the run holds it" })
    }

    fn mcp_binding(via: &str) -> serde_json::Value {
        serde_json::json!({ "capability": "mcp", "selector": "s", "kit": "root:test", "request": "capability mcp s",
            "via": via, "env": ["MCPKEY"], "from": { "file": "/keys/mcp.env" }, "holds": "the run holds it",
            "server": "s", "url": "https://mcp.example/mcp", "header": "Authorization", "scheme": "Bearer",
            "config": "/run/pot/mcp.json" })
    }

    /// credential and mcp: the variables go over the named pipe (never to the engine), read by run on the host; a
    /// via-file binding's folder `secrets/` is bound read-only at /run/pot/secrets; mcp.json read-only at
    /// /run/pot/mcp.json, always (bwrap writes it for every run), holding no value.
    #[test]
    fn credential_and_mcp_bindings_go_over_the_pipe_with_their_files() {
        let b = serde_json::json!([
            cred_binding(
                "api.example",
                "TESTTOK",
                "file",
                &serde_json::json!({ "file": "/keys/tok" })
            ),
            cred_binding(
                "other",
                "ENVTOK",
                "env",
                &serde_json::json!({ "env": "HOST_TOK" })
            ),
            mcp_binding("env"),
        ]);
        let (p, l) = bound("access-none", &b, false);
        for engine in [Engine::Podman, Engine::Docker] {
            let plan = go(&p, &l, engine, &nothing()).unwrap_or_else(|g| panic!("{g:?}"));
            assert!(plan.fifo, "a binding takes the named pipe");
            let ro = |src: &str, dest: &str| match engine {
                Engine::Podman => format!("--volume {src}:{dest}:ro"),
                Engine::Docker => format!("--mount type=bind,source={src},target={dest},readonly"),
            };
            assert!(
                has(&plan, &ro("/r/secrets", "/run/pot/secrets")),
                "{:#?}",
                lines(&plan)
            );
            assert!(has(&plan, &ro("/r/mcp.json", "/run/pot/mcp.json")));
            assert!(has(&plan, &ro("/r/secrets.fifo", FIFO)));
            for n in ["TESTTOK", "ENVTOK", "MCPKEY"] {
                assert_eq!(
                    plan.piped.iter().filter(|x| *x == n).count(),
                    1,
                    "{n}: {:?}",
                    plan.piped
                );
                assert!(plan.hostless.iter().any(|x| x == n));
                // its name is in the entry's planned names (not secret), never a value or an --env
                let argv = plan.argv();
                assert!(
                    !argv.iter().any(|w| w.starts_with(&format!("{n}="))),
                    "{n}= in argv"
                );
                assert!(
                    !argv.windows(2).any(|w| w[0] == "--env" && w[1] == n),
                    "--env {n}"
                );
                assert!(!plan.engine_keep.iter().any(|x| x == n));
            }
            assert_eq!(plan.creds, crate::bindings::creds(&p));
            assert!(plan.files.contains(&RunFile::PrivateDir {
                name: "secrets".into()
            }));
            assert!(plan.files.contains(&RunFile::File {
                name: "mcp.json".into(),
                text: crate::bindings::mcp_json(&p),
                mode: 0o444
            }));
        }
        // no via-file binding: no secrets folder; no binding at all: mcp.json all the same, and no pipe for it
        let (p, l) = bound("access-none", &serde_json::json!([mcp_binding("env")]), false);
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(!lines(&plan).iter().any(|x| x.contains("/run/pot/secrets:")));
        let (p, l) = bound("access-none", &serde_json::json!([]), false);
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(!plan.fifo && plan.creds.is_empty());
        assert!(has(&plan, "--volume /r/mcp.json:/run/pot/mcp.json:ro"));
    }

    /// As bwrap's `run` (`env.update(deliver(…))` after `run_env`): a binding's variable beats pot's values, env.set,
    /// the items and the proxy's variables. Named like one of them, it leaves the plan's values and goes over the pipe;
    /// what the plan would have given is its default, for a run where the binding has no value.
    #[test]
    fn a_bindings_variable_beats_every_other_value() {
        let b = serde_json::json!([
            cred_binding("tz", "TZ", "env", &serde_json::json!({})),
            cred_binding("tok", "NOTES_TOKEN", "file", &serde_json::json!({})),
            cred_binding("px", "HTTPS_PROXY", "env", &serde_json::json!({})),
        ]);
        let (p, l) = bound("network", &b, false);
        let plan = go_net(&p, &l, Engine::Podman, &nothing()).unwrap_or_else(|g| panic!("{g:?}"));
        let last_hop: Vec<&String> = plan.entry.iter().skip_while(|w| *w != "--").collect();
        for n in ["TZ", "HTTPS_PROXY", "NOTES_TOKEN"] {
            assert!(
                !last_hop.iter().any(|w| w.starts_with(&format!("{n}="))),
                "{n} set by the last hop"
            );
            assert_eq!(plan.piped.iter().filter(|x| *x == n).count(), 1, "{n}");
        }
        assert_eq!(
            plan.defaults.get("TZ").map(String::as_str),
            Some("America/Chicago")
        );
        assert_eq!(
            plan.defaults.get("HTTPS_PROXY").map(String::as_str),
            Some(PROXY_URL)
        );
        // NOTES_TOKEN is an env item too: the host's value is its fallback, so it is not hostless
        assert!(plan.hostless.iter().any(|x| x == "TZ") && !plan.hostless.iter().any(|x| x == "NOTES_TOKEN"));
    }

    /// A binding's variable must be a name bash can hand over (T1), as an env item's.
    #[test]
    fn a_bindings_variable_bash_sets_is_refused() {
        for (name, refused) in [
            ("SHLVL", true),
            ("BASH_ENV", true),
            ("__pot_x", true),
            ("RANDOM", false),
        ] {
            let b = serde_json::json!([cred_binding("x", name, "env", &serde_json::json!({}))]);
            let (p, l) = bound("access-none", &b, false);
            let r = go(&p, &l, Engine::Podman, &nothing());
            let want =
                format!("the variable {name}, which bash sets or keeps read-only, as a binding's variable");
            assert_eq!(r.err().is_some_and(|g| g.contains(&want)), refused, "{name}");
        }
    }

    /// T1: a variable bash can't hand over faithfully is refused for every env item.
    #[test]
    fn a_variable_bash_sets_is_refused() {
        for (name, refused) in [
            ("UID", true),
            ("BASH_ENV", true),
            ("SHLVL", true),
            ("_", true),
            ("PIPESTATUS", true),
            ("RANDOM", false),
            ("IFS", false),
        ] {
            let (mut p, l) = run_dir("access-none");
            for it in p.items.iter_mut().filter(|i| i.kind == "env") {
                it.value = name.into();
            }
            for engine in [Engine::Podman, Engine::Docker] {
                let r = go(&p, &l, engine, &nothing());
                let want =
                    format!("the variable {name}, which bash sets or keeps read-only, as a run's variable");
                assert_eq!(r.err().is_some_and(|g| g.contains(&want)), refused, "{name}");
            }
        }
    }

    /// S5: the entry's programs (bash with a secret or the network, socat with the network) must be real files
    /// under a path the plan binds.
    #[test]
    fn the_entrys_programs_are_real_files_the_run_sees() {
        let (p, l) = run_dir("read-only");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        let bash = "/nix/store/b-bash/bin/bash";
        let mut host = nothing();
        host.files = vec![bash, "/opt/bash"];
        assert_eq!(program_gaps(&plan, bash, &host), Vec::<String>::new());
        let gap = |what: &str, s: &str| {
            vec![format!(
                "the config's {what} {s} (the run's entry runs it: it must be a real file under a path the run binds)"
            )]
        };
        assert_eq!(program_gaps(&plan, "/opt/bash", &host), gap("shell", "/opt/bash"));
        assert_eq!(program_gaps(&plan, bash, &nothing()), gap("shell", bash));
        host.links = vec![(bash, "/opt/bash")];
        assert_eq!(program_gaps(&plan, bash, &host), gap("shell", bash));
        // no secret and no network: no program needed
        let (p, l) = run_dir("access-none");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert_eq!(program_gaps(&plan, "/opt/bash", &nothing()), Vec::<String>::new());
    }

    /// V2: probe and prepare share one test of the network's programs: python and proxy files, socat a real file (not a
    /// link) under a path of `netClosure`.
    #[test]
    fn the_networks_programs_are_tested_once_for_probe_and_prepare() {
        let t = net_tools();
        let (py, px, so) = (
            "/nix/store/p-python/bin/python3",
            "/nix/store/q-pot/proxy.py",
            "/nix/store/s-socat/bin/socat",
        );
        let mut host = nothing();
        host.files = vec![py, px, so];
        assert_eq!(net_gaps(&t, &host), Vec::<String>::new());
        let gap = |s: &str| {
            format!(
                "the config's socat {s} (the run's entry runs it: it must be a real file under a path of netClosure)"
            )
        };
        host.links = vec![(so, "/opt/socat")];
        assert_eq!(net_gaps(&t, &host), [gap(so)]);
        host.links = vec![];
        let mut out = t.clone();
        out.socat = "/opt/socat".into();
        host.files.push("/opt/socat");
        assert_eq!(net_gaps(&out, &host), [gap("/opt/socat")]);
        let mut none = t.clone();
        none.closure = vec![];
        assert_eq!(net_gaps(&none, &host), [gap(so)]);
        host.files = vec![so];
        assert_eq!(
            net_gaps(&t, &host),
            [
                format!("the config's python {py} (the network's proxy runs it: it must be a file)"),
                format!("the config's proxy {px} (the network's proxy runs it: it must be a file)")
            ]
        );
    }

    fn net_item(v: &str) -> crate::policy::Item {
        let it = serde_json::json!({ "kind": "network", "value": v, "class": "reach",
            "sources": [ { "flag": "--net" } ], "effective": true, "request": format!("network {v}"),
            "requestClass": "ordinary" });
        serde_json::from_value(it).unwrap()
    }

    /// The network: carried out under either engine. The container keeps `--network none`; the run folder's
    /// proxy.sock is bound at /run/pot/proxy.sock, socat's closure is bound, the entry starts socat, the proxy's
    /// variables are bwrap's, and the proxy's command is bwrap's (relative socket, the run's denials, the allowlist).
    #[test]
    fn the_network_is_pots_proxy_over_the_run_folders_socket() {
        let (mut p, mut l) = run_dir("access-none");
        p.items.push(net_item("example.com:443"));
        p.items.push(net_item("*.example.net:8443"));
        p.backend.proved.push("private-proxy".into());
        l.net_fix = Some("the repo kit's requests.network".into());
        for engine in [Engine::Podman, Engine::Docker] {
            let plan = go_net(&p, &l, engine, &nothing()).unwrap();
            assert!(has(&plan, "--network none"));
            let bind = |src: &str, dest: &str| match engine {
                Engine::Podman => format!("--volume {src}:{dest}:ro"),
                Engine::Docker => format!("--mount type=bind,source={src},target={dest},readonly"),
            };
            assert!(
                has(&plan, &bind("/r/proxy.sock", "/run/pot/proxy.sock")),
                "{:#?}",
                lines(&plan)
            );
            assert!(has(&plan, &bind("/r/secrets.fifo", "/run/pot/secrets.fifo")));
            assert!(has(&plan, &bind("/nix/store/s-socat", "/nix/store/s-socat")));
            assert!(has(&plan, &bind("/nix/store/o-openssl", "/nix/store/o-openssl")));
            for v in [
                "HTTPS_PROXY",
                "https_proxy",
                "HTTP_PROXY",
                "http_proxy",
                "ALL_PROXY",
                "all_proxy",
            ] {
                assert!(has(&plan, &format!("--env {v}=http://127.0.0.1:3128")), "{v}");
            }
            assert!(has(&plan, "--env NO_PROXY=localhost,127.0.0.1,::1"));
            assert!(has(&plan, "--env no_proxy=localhost,127.0.0.1,::1"));
            // every variable passed by name goes over the named pipe; the entry starts socat
            assert!(plan.fifo && plan.piped.contains(&"NOTES_TOKEN".to_string()));
            assert_eq!(
                plan.entry[8..12],
                [
                    "pot-entry",
                    "/run/pot/secrets.fifo",
                    "/nix/store/s-socat/bin/socat",
                    "3"
                ]
            );
            assert_eq!(
                plan.proxy.as_deref().unwrap(),
                [
                    "/nix/store/p-python/bin/python3",
                    "-I",
                    "-S",
                    "/nix/store/q-pot/proxy.py",
                    "proxy.sock",
                    "--denials",
                    "/r/denials",
                    "--context",
                    "review",
                    "--fix",
                    "the repo kit's requests.network",
                    "example.com:443",
                    "*.example.net:8443"
                ]
            );
        }
        // tmux notices and a sibling's proxy, as bwrap starts them
        l.pane = Some("%3".into());
        l.host_tools.tmux = Some("/nix/store/t-tmux/bin/tmux".into());
        l.sibling_of = Some(serde_json::json!("7f3a"));
        let proxy = go_net(&p, &l, Engine::Podman, &nothing()).unwrap().proxy.unwrap();
        assert_eq!(
            proxy[9..17],
            [
                "--tmux",
                "/nix/store/t-tmux/bin/tmux",
                "--pane",
                "%3",
                "--fix",
                "the repo kit's requests.network",
                "--sibling",
                "1"
            ]
        );
        // no network item: no proxy, no socat, no socket
        let (p, l) = run_dir("access-none");
        let plan = go_net(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(plan.proxy.is_none() && !plan.fifo);
        assert!(
            !lines(&plan)
                .iter()
                .any(|x| x.contains("proxy.sock") || x.contains("s-socat"))
        );
    }

    /// bwrap's `run_env` sets the proxy's variables last: they beat `env.set` and an item of the same name.
    #[test]
    fn the_proxys_variables_beat_env_set_and_items() {
        let (mut p, l) = run_dir("access-none");
        p.items.push(net_item("example.com:443"));
        for it in p.items.iter_mut().filter(|i| i.kind == "env") {
            it.value = "HTTPS_PROXY".into();
        }
        for it in p.items.iter_mut().filter(|i| i.kind == "env.set") {
            it.value = "NO_PROXY=everything".into();
        }
        let plan = go_net(&p, &l, Engine::Podman, &nothing()).unwrap();
        assert!(!plan.piped.iter().any(|n| n == "HTTPS_PROXY") && !plan.defaults.contains_key("HTTPS_PROXY"));
        assert!(has(&plan, "--env HTTPS_PROXY=http://127.0.0.1:3128"));
        assert!(has(&plan, "--env NO_PROXY=localhost,127.0.0.1,::1"));
        assert!(!has(&plan, "--env NO_PROXY=everything"));
        // without the network, the item is passed as any other
        let (mut p, l) = run_dir("access-none");
        for it in p.items.iter_mut().filter(|i| i.kind == "env") {
            it.value = "HTTPS_PROXY".into();
        }
        assert!(has(
            &go_net(&p, &l, Engine::Podman, &nothing()).unwrap(),
            "--env HTTPS_PROXY"
        ));
    }

    /// Without the config's network programs, the network and private-proxy are refused, never dropped.
    #[test]
    fn the_network_without_the_configs_programs_is_refused() {
        let (mut p, l) = run_dir("access-none");
        p.items.push(net_item("example.com:443"));
        p.backend.proved.push("private-proxy".into());
        assert_eq!(
            go(&p, &l, Engine::Podman, &nothing()).unwrap_err(),
            [
                "the private-proxy guarantee (the backend's --config names no python, proxy, socat and netClosure)",
                "the network through pot's proxy (example.com:443): the backend's --config names no python, proxy, socat and netClosure"
            ]
        );
        assert!(go_net(&p, &l, Engine::Docker, &nothing()).is_ok());
    }

    /// F2: the engine reads only pot's configuration: podman a containers.conf of the run's own (no default mounts,
    /// volumes, env or devices) with `CONTAINERS_CONF_OVERRIDE` and the user's files out of reach; docker an empty
    /// config folder. Only allowlisted variables reach the engine (no `CONTAINER_HOST`, `DOCKER_HOST`, …).
    #[test]
    fn the_engine_reads_only_pots_configuration() {
        let (p, l) = run_dir("read-only");
        let plan = go(&p, &l, Engine::Podman, &nothing()).unwrap();
        // podman reads the user's mounts.conf from $HOME whatever containers.conf says: the engine's home is the run's,
        // emptied at each prepare (N3); its storage is pot's (N2): a store of the run's own, native overlay
        assert_eq!(
            plan.engine_set,
            [
                ("CONTAINERS_CONF".to_string(), "/r/containers.conf".to_string()),
                (
                    "CONTAINERS_STORAGE_CONF".to_string(),
                    "/r/storage.conf".to_string()
                ),
                ("HOME".to_string(), "/r/enginehome".to_string()),
            ]
        );
        assert!(plan.files.contains(&RunFile::FreshDir {
            name: "enginehome".into()
        }));
        assert!(plan.files.contains(&RunFile::FreshDir {
            name: "enginestore".into()
        }));
        let storage = plan.files.iter().find_map(|f| match f {
            RunFile::File { name, text, .. } if name == "storage.conf" => Some(text.clone()),
            _ => None,
        });
        let storage = storage.expect("storage.conf");
        assert!(storage.contains("driver = \"overlay\""), "{storage}");
        assert!(
            storage.contains("rootless_storage_path = \"/r/enginestore\""),
            "{storage}"
        );
        assert!(!storage.contains("mount_program"), "{storage}");
        let conf = plan.files.iter().find_map(|f| match f {
            RunFile::File { name, text, .. } if name == "containers.conf" => Some(text.clone()),
            _ => None,
        });
        let conf = conf.expect("containers.conf");
        assert!(conf.contains("default_mounts_file = \"/r/empty\""), "{conf}");
        assert!(conf.contains("volumes = []") && conf.contains("env = []") && conf.contains("devices = []"));
        assert!(!conf.contains("seccomp_profile"));
        for bad in [
            "CONTAINERS_CONF_OVERRIDE",
            "CONTAINERS_CONF",
            "CONTAINER_HOST",
            "CONTAINER_CONNECTION",
            "DOCKER_HOST",
            "DOCKER_CONTEXT",
            "DOCKER_CONFIG",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "CONTAINERS_STORAGE_CONF",
        ] {
            assert!(
                !plan.engine_keep.iter().any(|k| k == bad),
                "{bad} reaches the engine"
            );
        }
        // N1: a run variable reaches the container through the engine's environment (--env NAME), so a granted
        // variable that configures the engine is refused, never passed
        for bad in [
            "CONTAINERS_CONF_OVERRIDE",
            "CONTAINERS_CONF",
            "CONTAINER_HOST",
            "DOCKER_HOST",
            "DOCKER_CONFIG",
            "XDG_CONFIG_HOME",
            "STORAGE_OPTS",
            "_CONTAINERS_USERNS_CONFIGURED",
            "LD_PRELOAD",
            "TMPDIR",
            // N5: podman hands the caller's fds 3.. to the container under LISTEN_FDS (socket activation)
            "LISTEN_FDS",
            "LISTEN_PID",
            "LISTEN_FDNAMES",
        ] {
            let (mut p, l) = run_dir("read-only");
            for it in p.items.iter_mut().filter(|i| i.kind == "env") {
                it.value = bad.into();
                it.secret = Some(false);
            }
            for engine in [Engine::Podman, Engine::Docker] {
                p.backend.proved.retain(|g| g != "resource-limits");
                let gaps = go(&p, &l, engine, &nothing()).unwrap_err();
                assert_eq!(
                    gaps,
                    [format!(
                        "the variable {bad}, which configures the container engine, as a run's variable"
                    )]
                );
            }
        }
        // a variable the run is passed by name (not a secret) reaches the engine, which reads its value
        let (p, l) = run_dir("access-none");
        let plan = go(&p, &l, Engine::Docker, &nothing()).unwrap();
        assert!(plan.engine_keep.iter().any(|k| k == "NOTES_TOKEN"));
        assert!(has(&plan, "--env NOTES_TOKEN") && plan.piped.is_empty());
        assert_eq!(
            plan.engine_set,
            [("DOCKER_CONFIG".to_string(), "/r/dockercfg".to_string())]
        );
        assert!(plan.files.contains(&RunFile::Dir {
            name: "dockercfg".into()
        }));
    }

    const NOTES: &str = "/home/u/src/notes";

    /// The write fixture's workspace: the pins launch.json lists (the placeholders not there yet), a sealed `.pi`
    /// with a file, a folder and a link, and a socket.
    fn notes_host() -> Fake {
        Fake {
            dirs: vec![
                "/home/u/src/notes",
                "/home/u/src/notes/.git",
                "/home/u/src/notes/.git/hooks",
                "/home/u/src/notes/.git/info",
                "/home/u/src/notes/sub",
                "/home/u/src/notes/.pi",
                "/home/u/src/notes/.pi/skills",
                "/home/u/docs",
            ],
            files: vec![
                "/home/u/src/notes/.git/config",
                "/home/u/src/notes/.envrc",
                "/home/u/src/notes/flake.nix",
                "/home/u/src/notes/sub/flake.nix",
                "/home/u/src/notes/.pi/settings.json",
            ],
            socks: vec!["/home/u/src/notes/.direnv/s.sock"],
            links: vec![("/home/u/src/notes/.pi/current", "settings.json")],
        }
    }

    fn at(lines: &[String], line: &str) -> usize {
        lines
            .iter()
            .position(|l| l == line)
            .unwrap_or_else(|| panic!("no line {line}"))
    }

    /// Write access in place (0.3.0): the workspace writable, then every pin pot's core resolved bound onto itself
    /// over it, deeper after shallower (the engines mount by destination depth); the placeholders and the seal
    /// planned for prepare.
    #[test]
    fn write_access_binds_the_workspace_then_the_pins_over_it() {
        let (p, l) = run_dir("write");
        let plan = go(&p, &l, Engine::Podman, &notes_host()).unwrap();
        let v = |src: &str, how: &str| format!("--volume {src}:{src}:{how}");
        let n = |rel: &str| format!("{NOTES}/{rel}");
        let ls = lines(&plan);
        let top = at(&ls, &v(NOTES, "rw"));
        for d in [".git", "sub"] {
            assert!(at(&ls, &v(&n(d), "rw")) > top, "{d}");
        }
        for r in [
            ".git/config",
            ".git/config.worktree",
            ".git/hooks",
            ".git/info",
            ".git/commondir",
            ".git/gitdir",
            ".envrc",
            "flake.nix",
        ] {
            assert!(at(&ls, &v(&n(r), "ro")) > top, "{r}");
        }
        assert!(at(&ls, &v(&n("sub/flake.nix"), "ro")) > at(&ls, &v(&n("sub"), "rw")));
        assert!(at(&ls, &v(&n(".git/config"), "ro")) > at(&ls, &v(&n(".git"), "rw")));
        // a socket in the workspace is masked, as under read access
        assert!(has(
            &plan,
            "--volume /r/empty:/home/u/src/notes/.direnv/s.sock:ro"
        ));
        // the placeholders, for prepare to create first (they are sources that do not exist yet)
        assert_eq!(
            plan.placeholders
                .iter()
                .map(|p| (p.path.as_str(), p.kind, p.text.as_deref()))
                .collect::<Vec<_>>(),
            [
                (n(".git/commondir").as_str(), Kind::File, Some(".\n")),
                (n(".git/config.worktree").as_str(), Kind::File, Some("")),
                (n(".git/gitdir").as_str(), Kind::File, Some("")),
            ]
        );
        // what prepare and run check again before the engine binds it: every pin, by kind
        for (path, kind) in [
            (NOTES.to_string(), Kind::Folder),
            (n(".git"), Kind::Folder),
            (n(".git/commondir"), Kind::File),
            (n("sub/flake.nix"), Kind::File),
            (n(".git/hooks"), Kind::Folder),
            (n(".pi/settings.json"), Kind::File),
            (n(".pi/skills"), Kind::Folder),
        ] {
            assert!(plan.pins.contains(&(path.clone(), kind)), "{path}");
        }
        // the command starts where it was asked to, in the writable workspace
        assert!(plan.entry.windows(2).any(|w| w == ["-C", NOTES]));
    }

    /// A sealed folder, as bwrap's `seal`: a folder of the run's bound writable at its path, each entry of the
    /// host's bound read-only into it (a link as the same link), so a new entry lands in the run folder.
    #[test]
    fn write_access_seals_a_sealed_folder() {
        let (p, l) = run_dir("write");
        let plan = go(&p, &l, Engine::Podman, &notes_host()).unwrap();
        assert!(has(&plan, "--volume /r/sealed/0:/home/u/src/notes/.pi:rw"));
        assert!(has(
            &plan,
            "--volume /home/u/src/notes/.pi/settings.json:/home/u/src/notes/.pi/settings.json:ro"
        ));
        assert!(has(
            &plan,
            "--volume /home/u/src/notes/.pi/skills:/home/u/src/notes/.pi/skills:ro"
        ));
        assert!(!lines(&plan).iter().any(|x| x.contains(".pi/current")));
        for f in [
            RunFile::FreshDir {
                name: "sealed".into(),
            },
            RunFile::Dir {
                name: "sealed/0".into(),
            },
            RunFile::File {
                name: "sealed/0/settings.json".into(),
                text: String::new(),
                mode: 0o444,
            },
            RunFile::Dir {
                name: "sealed/0/skills".into(),
            },
            RunFile::Link {
                name: "sealed/0/current".into(),
                target: "settings.json".into(),
            },
        ] {
            assert!(plan.files.contains(&f), "{f:?}");
        }
        assert_eq!(plan.seals.len(), 1);
        assert_eq!(plan.seals[0].dest, "/home/u/src/notes/.pi");
        assert_eq!(plan.seals[0].base, "sealed/0");
        assert_eq!(plan.seals[0].rel, ".pi");
        assert_eq!(plan.seals[0].entries, ["current", "settings.json", "skills"]);
        assert!(plan.notes.is_empty());
    }

    #[test]
    fn write_access_under_docker_binds_with_mount() {
        let (mut p, l) = run_dir("write");
        p.backend.proved.retain(|g| g != "resource-limits");
        let plan = go(&p, &l, Engine::Docker, &notes_host()).unwrap();
        assert!(has(
            &plan,
            "--mount type=bind,source=/home/u/src/notes,target=/home/u/src/notes"
        ));
        assert!(has(
            &plan,
            "--mount type=bind,source=/home/u/src/notes/.git/config,target=/home/u/src/notes/.git/config,readonly"
        ));
        assert!(has(
            &plan,
            "--mount type=bind,source=/home/u/src/notes/.git,target=/home/u/src/notes/.git"
        ));
    }

    /// A worktree: its common directory is outside its top, bound writable too, with its pins over it.
    #[test]
    fn a_worktrees_common_directory_is_bound_with_its_pins() {
        let (mut p, mut l) = run_dir("write");
        p.workspace[0].common = "/home/u/src/main/.git".into();
        let w = &mut l.pins.as_mut().unwrap()[0];
        w.common = "/home/u/src/main/.git".into();
        w.dirs = vec![
            "/home/u/src/main/.git".into(),
            "/home/u/src/main/.git/worktrees/notes".into(),
        ];
        w.ro = vec![crate::launch::Pin {
            path: "/home/u/src/main/.git/config".into(),
            kind: Kind::File,
        }];
        w.placeholders = vec![];
        w.sealed = vec![];
        let mut host = notes_host();
        host.dirs
            .extend(["/home/u/src/main/.git", "/home/u/src/main/.git/worktrees/notes"]);
        host.files.push("/home/u/src/main/.git/config");
        let plan = go(&p, &l, Engine::Podman, &host).unwrap();
        let ls = lines(&plan);
        let common = at(&ls, "--volume /home/u/src/main/.git:/home/u/src/main/.git:rw");
        assert!(
            at(
                &ls,
                "--volume /home/u/src/main/.git/config:/home/u/src/main/.git/config:ro"
            ) > common
        );
        assert!(has(
            &plan,
            "--volume /home/u/src/main/.git/worktrees/notes:/home/u/src/main/.git/worktrees/notes:rw"
        ));
    }

    /// K4, as bwrap: nothing is bound inside a hidden folder in the workspace, not even a pin; the folder is masked.
    #[test]
    fn a_hidden_folder_in_the_workspace_takes_its_pins_with_it() {
        let (mut p, l) = run_dir("write");
        for it in p.items.iter_mut().filter(|i| i.kind == "hidden") {
            it.paths = Some(vec!["/home/u/src/notes/sub".into()]);
        }
        let plan = go(&p, &l, Engine::Podman, &notes_host()).unwrap();
        assert!(has(&plan, "--volume /r/emptydir:/home/u/src/notes/sub:ro"));
        assert!(!lines(&plan).iter().any(|x| x.contains("sub/flake.nix")));
    }

    #[test]
    fn write_access_needs_the_pins_and_the_look() {
        let (p, mut l) = run_dir("write");
        let paths = paths();
        let inputs = |l: &Launch, look: Option<&LookTools>| {
            render(&Inputs {
                policy: &p,
                launch: l,
                paths: &paths,
                engine: Engine::Podman,
                program: "podman",
                image: DOCKER_IMAGE,
                rundir: "/r",
                base_closure: &[],
                net: None,
                look,
                host: &notes_host(),
            })
        };
        assert_eq!(
            inputs(&l, None).unwrap_err(),
            [
                "write access: the backend's --config names no python, pins and git (the host's look before and after the run)"
            ]
        );
        let mut other = l.pins.clone().unwrap();
        other[0].top = "/home/u/src/other".into();
        l.pins = Some(other);
        assert_eq!(
            inputs(&l, Some(&look_tools())).unwrap_err(),
            [
                "write access without launch.json's pins for /home/u/src/notes (pot 0.3.0 or later resolves them)"
            ]
        );
        l.pins = None;
        assert_eq!(
            inputs(&l, Some(&look_tools())).unwrap_err(),
            [
                "write access without launch.json's pins for /home/u/src/notes (pot 0.3.0 or later resolves them)"
            ]
        );
    }

    /// More entries than a sealed folder takes (bwrap's `SEAL_MAX`): not sealed, a start note instead.
    #[test]
    fn a_sealed_folder_over_the_cap_gets_a_note() {
        let (p, l) = run_dir("write");
        let mut host = notes_host();
        let many: Vec<&'static str> = (0..=SEAL_MAX)
            .map(|k| &*Box::leak(format!("/home/u/src/notes/.pi/e{k}").into_boxed_str()))
            .collect();
        host.files.extend(many);
        let plan = go(&p, &l, Engine::Podman, &host).unwrap();
        assert!(plan.seals.is_empty());
        assert!(!lines(&plan).iter().any(|x| x.contains("/r/sealed")));
        assert_eq!(
            plan.notes,
            [
                "pot: ~/src/notes/.pi has 260 entries, more than a sealed folder takes (256): it is read-only in this run, so pi cannot take its settings lock there"
            ]
        );
    }

    #[test]
    fn protected_as_mounts_is_upheld_with_the_look() {
        assert!(upholds(Engine::Podman, false, true, "protected-as-mounts"));
        assert!(upholds(Engine::Docker, false, true, "protected-as-mounts"));
        assert!(!upholds(Engine::Podman, false, false, "protected-as-mounts"));
    }
}
