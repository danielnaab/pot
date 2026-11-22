//! launch.json: the launcher's facts, pot's other input to a backend. Until launch.json/1 is written it is read
//! leniently (pot adds fields without a version; core/pot.py `launch_facts` and the start of `inner_run`), and
//! only the fields below are used. A missing one stops `prepare`.
//!
//! Also the backend's `--config`: the store paths and the engine this build of the backend uses (the keys bwrap's
//! config uses where they mean the same).

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Launch {
    /// The command, run in the container.
    pub argv: Vec<String>,
    /// Its working directory: where the command starts (a workspace path, usually).
    pub cwd: String,
    /// The caller's home: the run's `HOME`, an empty tmpfs but for what the policy places there.
    pub home: String,
    /// The caller's user and group names, for the run's /etc/passwd and /etc/group.
    pub user: String,
    pub group: String,
    /// The caller's ids: the run's user (`--user`), and with podman the id `--userns=keep-id` keeps.
    pub uid: u32,
    pub gid: u32,
    /// The run's closure: the store paths it sees when the backend proves closure-store.
    #[serde(default)]
    pub closure: Vec<String>,
    /// The root's pot (a store path's bin/): `/run/pot/launcher`, and on `PATH`.
    #[serde(default)]
    pub pot_bin: Option<String>,
    /// Whether the host signs commits: the `after` event's `signs`.
    #[serde(default)]
    pub host_signs: bool,
    /// The caller's tmux pane, where the proxy shows a denial (with `hostTools.tmux`).
    #[serde(default)]
    pub pane: Option<String>,
    /// The host programs the helpers run: `tmux`, for the proxy's notices.
    #[serde(default)]
    pub host_tools: HostTools,
    /// The permanent fix a denial names (Info): the proxy's `--fix`.
    #[serde(default)]
    pub net_fix: Option<String>,
    /// A sibling's caller's run id: its proxy shows no one-run fix (`--sibling`).
    #[serde(default)]
    pub sibling_of: Option<serde_json::Value>,
    /// 0.3.0, with write access in place: each workspace's pins, as pot's core resolved them at the run's start.
    #[serde(default)]
    pub pins: Option<Vec<WsPins>>,
}

/// One workspace's pins (launch.json/1 `pins`): read strictly, since a key this backend does not know could narrow
/// the run.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WsPins {
    pub top: String,
    pub common: String,
    /// Folders bound writable onto themselves (mount points: not renamed or removed).
    pub dirs: Vec<String>,
    /// Paths bound read-only onto themselves.
    pub ro: Vec<Pin>,
    /// What `prepare` creates before it binds (each also in `ro`).
    pub placeholders: Vec<Placeholder>,
    /// The before look's start (read by core's pins.py, not here).
    #[allow(dead_code)]
    pub dotgits: Vec<String>,
    #[allow(dead_code)]
    pub named: Vec<String>,
    /// The outermost sealed folders.
    pub sealed: Vec<String>,
    /// A pin the sandbox can't make: the run stops with `message`.
    #[serde(default)]
    pub refused: Option<PinRefusal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Folder,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub path: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placeholder {
    pub path: String,
    pub kind: Kind,
    #[serde(default)]
    pub text: Option<String>,
}

// why and path are Info beside the message, which is what the run stops with
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinRefusal {
    pub why: String,
    pub path: String,
    pub message: String,
}

/// launch.json's `hostTools`: the host programs the helpers run, never the run: `tmux` (the proxy's notices), `ssh`
/// (git.fetch's broker), `workmux` (status.relay's broker) and `nix` (the nix relay, and its bin/ on the run's `PATH`).
#[derive(Debug, Default, Deserialize)]
pub struct HostTools {
    #[serde(default)]
    pub tmux: Option<String>,
    #[serde(default)]
    pub ssh: Option<String>,
    #[serde(default)]
    pub workmux: Option<String>,
    #[serde(default)]
    pub nix: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    /// `podman` or `docker`; by default podman when it is on `PATH`, else docker.
    pub engine: Option<String>,
    /// The engines' programs (default: the name, on `PATH`).
    pub podman: Option<String>,
    pub docker: Option<String>,
    /// docker only: the empty image the run starts from (default `localhost/pot-empty:1`).
    pub image: Option<String>,
    /// Store paths bound into every run, as bwrap's config names them.
    pub sh: Option<String>,
    pub shell: Option<String>,
    pub env: Option<String>,
    pub cacert: Option<String>,
    pub terminfo: Option<String>,
    pub tzdir: Option<String>,
    /// A file of store paths, one a line: the closure of the above, bound beside the run's own.
    pub base_closure: Option<String>,
    /// The network (bwrap's names): pot's proxy, run on the host by `python` (`proxy.py`), and `socat`, which serves
    /// it in the run as 127.0.0.1:3128; `netClosure` a file of store paths, socat's closure, bound into a run with
    /// the network.
    pub python: Option<String>,
    pub proxy: Option<String>,
    pub socat: Option<String>,
    pub net_closure: Option<String>,
    /// Write access (0.3.0): pot's `pins.py` (core/), which `python` runs for the host's look before and after an
    /// in-place run (bwrap's before.json and after event), with host `git`.
    pub pins: Option<String>,
    pub git: Option<String>,
    /// The status.relay and git.fetch bindings (bwrap's names): pot's `broker.py`, which `python` runs on the host, and
    /// `shim.py`, which the run's shims (`workmux`, `pot-ssh`) run with the same `python`; `shimClosure` a file of
    /// store paths, python's and the shim's closure, bound into a run with either binding. The nix binding's relay is
    /// `socat` on the host.
    pub broker: Option<String>,
    pub shim: Option<String>,
    pub shim_closure: Option<String>,
}

/// The config's programs for the status.relay and git.fetch bindings, all present.
#[derive(Debug, Clone)]
pub struct BrokerTools {
    pub python: String,
    pub broker: String,
    pub shim: String,
    /// The lines of `shimClosure`.
    pub closure: Vec<String>,
}

/// The config's programs for the host's look at a write run: `python -I -S PINS before|after GIT RUNDIR`.
#[derive(Debug, Clone)]
pub struct LookTools {
    pub python: String,
    pub pins: String,
    pub git: String,
}

/// The config's programs for the network, all present.
#[derive(Debug, Clone)]
pub struct NetTools {
    pub python: String,
    pub proxy: String,
    pub socat: String,
    /// The lines of `netClosure`.
    pub closure: Vec<String>,
}

/// The config's store paths `prepare` needs, all present.
#[derive(Debug, Clone)]
pub struct Paths {
    pub sh: String,
    pub shell: String,
    pub env: String,
    pub cacert: String,
    pub terminfo: String,
    pub tzdir: String,
}

impl Config {
    /// The network's programs, when the config names `python`, `proxy` and `socat` (`netClosure` may be absent: the
    /// whole store is bound without closure-store). `Err` when the closure file can't be read.
    pub fn net_tools(&self) -> Result<Option<NetTools>, String> {
        let (Some(python), Some(proxy), Some(socat)) = (&self.python, &self.proxy, &self.socat) else {
            return Ok(None);
        };
        let closure = closure_lines(self.net_closure.as_deref(), "net")?;
        Ok(Some(NetTools {
            python: python.clone(),
            proxy: proxy.clone(),
            socat: socat.clone(),
            closure,
        }))
    }

    /// The broker's and the shims' programs, when the config names `python`, `broker` and `shim` (`shimClosure` may be
    /// absent: the whole store is bound without closure-store, or with nix). `Err` when the closure file can't be read.
    pub fn broker_tools(&self) -> Result<Option<BrokerTools>, String> {
        let (Some(python), Some(broker), Some(shim)) = (&self.python, &self.broker, &self.shim) else {
            return Ok(None);
        };
        Ok(Some(BrokerTools {
            python: python.clone(),
            broker: broker.clone(),
            shim: shim.clone(),
            closure: closure_lines(self.shim_closure.as_deref(), "shim")?,
        }))
    }

    /// The look's programs, when the config names `python`, `pins` and `git`, each a file.
    pub fn look_tools(&self) -> Option<LookTools> {
        let (Some(python), Some(pins), Some(git)) = (&self.python, &self.pins, &self.git) else {
            return None;
        };
        let file = |f: &str| std::fs::metadata(f).is_ok_and(|m| m.is_file());
        (file(python) && file(pins) && file(git)).then(|| LookTools {
            python: python.clone(),
            pins: pins.clone(),
            git: git.clone(),
        })
    }

    pub fn paths(&self) -> Result<Paths, String> {
        let get = |v: &Option<String>, k: &str| {
            v.clone()
                .ok_or_else(|| format!("the backend's --config has no {k}"))
        };
        Ok(Paths {
            sh: get(&self.sh, "sh")?,
            shell: get(&self.shell, "shell")?,
            env: get(&self.env, "env")?,
            cacert: get(&self.cacert, "cacert")?,
            terminfo: get(&self.terminfo, "terminfo")?,
            tzdir: get(&self.tzdir, "tzdir")?,
        })
    }
}

/// A closure file's store paths, one a line; none without the file.
fn closure_lines(file: Option<&str>, what: &str) -> Result<Vec<String>, String> {
    match file {
        Some(f) => Ok(std::fs::read_to_string(f)
            .map_err(|e| format!("can't read the {what} closure {f}: {e}"))?
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()),
        None => Ok(vec![]),
    }
}
