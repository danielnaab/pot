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
}

/// launch.json's `hostTools`, of which the backend reads `tmux` (the proxy's notices).
#[derive(Debug, Default, Deserialize)]
pub struct HostTools {
    #[serde(default)]
    pub tmux: Option<String>,
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
        let closure = match &self.net_closure {
            Some(f) => std::fs::read_to_string(f)
                .map_err(|e| format!("can't read the net closure {f}: {e}"))?
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            None => vec![],
        };
        Ok(Some(NetTools {
            python: python.clone(),
            proxy: proxy.clone(),
            socat: socat.clone(),
            closure,
        }))
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
