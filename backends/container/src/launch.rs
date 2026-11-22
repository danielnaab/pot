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
