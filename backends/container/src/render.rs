//! policy.json/1 as a `podman run` / `docker run` command: a pure function of the policy, the launcher's facts,
//! the backend's config, the engine and the host facts it reads through [`Host`].
//!
//! It carries out what this slice can, and refuses the rest (policy-1, rules 2, 3 and 5): every gap is named in
//! one refusal, never dropped. The flag set is the phase 2 spike's (executed on platform-vm, 2026-11-21).

use std::collections::{BTreeMap, BTreeSet};

use crate::host::Host;
use crate::launch::{Launch, Paths};
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
/// scope pot proves resource-limits by (pot.py `scope_for`).
pub fn upholds(engine: Engine, g: &str) -> bool {
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
}

pub const DOCKER_IMAGE: &str = "localhost/pot-empty:1";

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
    pub host: &'a dyn Host,
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
}

/// The sources that do not exist on the host, but for the run folder's (prepare places them) and the state
/// directories (prepare creates them). docker would create a missing source, as root.
pub fn missing_sources(plan: &Plan, rundir: &str, host: &dyn Host) -> Vec<String> {
    plan.sources
        .iter()
        .filter(|s| !under(s, rundir) && !plan.host_dirs.contains(s) && !host.lexists(s))
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

/// The gaps this slice refuses, in the policy's order: each a short phrase.
#[allow(clippy::too_many_lines)] // one check per policy feature, listed in the reference page's order
fn gaps(i: &Inputs) -> Vec<String> {
    let p = i.policy;
    let mut out = vec![];
    for g in &p.backend.proved {
        if !upholds(i.engine, g) {
            out.push(if g == "resource-limits" {
                "resource-limits under docker (a container is not in pot's scope)".to_string()
            } else {
                format!("the {g} guarantee")
            });
        }
    }
    if p.mode == "clone" {
        out.push("clone mode (the clone at its original path, its pins, the staging fetch)".into());
    } else if p.access == "write" {
        out.push("write access (the workspace's pins: git directories, hooks, protected paths)".into());
    }
    if p.effective("view").next().is_some() {
        out.push("inspect's view".into());
    }
    let net: Vec<&str> = p
        .effective("network")
        .filter(|n| n.class == "reach")
        .map(|n| n.value.as_str())
        .collect();
    if !net.is_empty() {
        out.push(format!("the network through pot's proxy ({})", net.join(", ")));
    }
    let caps: BTreeSet<&str> = p.bindings.iter().map(|b| b.capability.as_str()).collect();
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
    // directory each with their sockets and hidden paths masked; write access and clones need the pins and are
    // refused above. access none: no workspace at all (pot 0.2.0), masked where a bound folder would show it
    // (bwrap's `unseen`), and the command starts in the home
    if p.access == "read" && p.mode == "in-place" {
        for w in &p.workspace {
            let mut tops = vec![w.path.as_str()];
            if !under(&w.common, &w.path) {
                tops.push(w.common.as_str());
            }
            for top in tops {
                if !real(top, &mut unreal) {
                    continue;
                }
                m.bind(top, top, READ, true);
                for s in host.sockets_under(top) {
                    if !in_hidden(&s) {
                        m.bind(&rd("empty"), &s, HIDE, true);
                    }
                }
                // a hidden path there is masked, and nothing is bound inside a hidden folder there
                for h in hidden.iter().filter(|h| under(h, top) && host.lexists(h)) {
                    if host.is_real_dir(h) {
                        m.by_dest.retain(|d, _| !under(d, h));
                    }
                    mask(&mut m, h);
                }
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
        let set: BTreeSet<&String> = l.closure.iter().chain(i.base_closure).collect();
        set.into_iter().cloned().collect()
    } else {
        vec!["/nix/store".into()]
    };

    let mut mounts: Vec<(String, Mount)> = m.by_dest.into_iter().map(|(d, (_, mt))| (d, mt)).collect();
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
    let mut opts = vec![
        o(&["run", "--rm", "--interactive"]),
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
    let (set, pass) = environment(i);
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
    let mut entry = vec!["/usr/bin/env".to_string(), "-C".into(), cwd, "--".into()];
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
    })
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

/// The run's environment: what pot sets (bwrap's `run_env`, less what this slice does not place), then the
/// names passed from the host by name only (`--env NAME`: the engine reads the value, so no value is in argv).
fn environment(i: &Inputs) -> (BTreeMap<String, String>, Vec<String>) {
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
    let mut pass: Vec<String> = vec!["TERM".into(), "COLORTERM".into()];
    for e in p.effective("env") {
        if !pass.contains(&e.value) {
            pass.push(e.value.clone());
        }
    }
    pass.retain(|n| !env.contains_key(n));
    (env, pass)
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
            host,
        })
    }

    /// The argv as one line per option, for assertions.
    fn lines(plan: &Plan) -> Vec<String> {
        plan.opts.iter().map(|o| o.args.join(" ")).collect()
    }

    fn has(plan: &Plan, line: &str) -> bool {
        lines(plan).iter().any(|l| l == line)
    }

    #[test]
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
        // a secret variable is passed by name: its value never reaches argv
        assert!(has(&plan, "--env NOTES_TOKEN"));
        assert!(!plan.argv().iter().any(|a| a.starts_with("NOTES_TOKEN=")));
        assert!(has(&plan, "--env NOTES_CACHE=/home/u/.cache/notes"));
        assert_eq!(plan.root, ["--rootfs", "/r/rootfs:O"]);
        assert_eq!(
            plan.entry,
            [
                "/usr/bin/env",
                "-C",
                "/home/u/src/notes",
                "--",
                "notes",
                "check",
                "--all"
            ]
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
    fn write_access_and_unsayable_paths_are_refused() {
        let (p, l) = run_dir("write");
        let gaps = go(&p, &l, Engine::Podman, &nothing()).unwrap_err();
        assert_eq!(
            gaps,
            ["write access (the workspace's pins: git directories, hooks, protected paths)"]
        );
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
        p.backend.proved.push("protected-as-mounts".into());
        let gaps = go(&p, &l, Engine::Podman, &nothing()).unwrap_err();
        assert_eq!(gaps, ["the protected-as-mounts guarantee"]);
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
                    "the network through pot's proxy (nothing.invalid:1, nothing.invalid:2)",
                    "sibling children (the run's children broker)",
                ],
            ),
            (
                "git-fetch",
                &[
                    "write access (the workspace's pins: git directories, hooks, protected paths)",
                    "the network through pot's proxy (nothing.invalid:1)",
                    "the git.fetch binding",
                ],
            ),
            ("inspect", &["inspect's view"]),
            (
                "nix",
                &[
                    "write access (the workspace's pins: git directories, hooks, protected paths)",
                    "the network through pot's proxy (inline.example:443, nothing.invalid:1)",
                    "the nix binding",
                ],
            ),
            (
                "pi-agent",
                &[
                    "write access (the workspace's pins: git directories, hooks, protected paths)",
                    "the network through pot's proxy (api.pi-test.example:443, lan.pi-test.example:11434)",
                    "the credential binding",
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
                    "the network through pot's proxy (api.x:443)",
                    "sibling children (the run's children broker)",
                ],
            ),
            (
                "sibling",
                &[
                    "the network through pot's proxy (nothing.invalid:1)",
                    "sibling children (the run's children broker)",
                ],
            ),
            (
                "work-bindings",
                &[
                    "write access (the workspace's pins: git directories, hooks, protected paths)",
                    "the network through pot's proxy (nothing.invalid:1, mcp.example:443)",
                    "the credential binding",
                    "the mcp binding",
                    "the status.relay binding",
                    "file ~/.st/conf.json (mode reassert)",
                ],
            ),
            (
                "work-siblings",
                &[
                    "write access (the workspace's pins: git directories, hooks, protected paths)",
                    "the network through pot's proxy (nothing.invalid:1, nothing.invalid:2, nothing.invalid:3)",
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
            let got = go(&p, &l, Engine::Podman, &nothing()).unwrap_err();
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
        // the variables the run is passed by name reach the engine, which reads their values
        assert!(plan.engine_keep.iter().any(|k| k == "NOTES_TOKEN"));
        let (p, l) = run_dir("access-none");
        let plan = go(&p, &l, Engine::Docker, &nothing()).unwrap();
        assert_eq!(
            plan.engine_set,
            [("DOCKER_CONFIG".to_string(), "/r/dockercfg".to_string())]
        );
        assert!(plan.files.contains(&RunFile::Dir {
            name: "dockercfg".into()
        }));
    }
}
