//! The engine's own environment and configuration (audit F2): podman and docker read configuration files and
//! variables that can add a volume, a variable or `seccomp_profile = "unconfined"` to every container, or send
//! the command to another engine (`CONTAINER_HOST`, `DOCKER_HOST`). The engine therefore runs with only the
//! variables below, and with pot's configuration in place of the user's: `CONTAINERS_CONF` (which replaces every
//! other containers.conf; `CONTAINERS_CONF_OVERRIDE` is not passed) and `DOCKER_CONFIG`.

use std::process::Command;

use crate::render::Engine;

/// The caller's variables the engine keeps (when set): what rootless podman needs to find its storage, runtime
/// folder and the user's systemd, and the terminal. The run's own variables passed by name are added to these.
pub const KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "DBUS_SESSION_BUS_ADDRESS",
    "CONTAINERS_STORAGE_CONF",
    "TMPDIR",
    "LANG",
    "TERM",
    "COLORTERM",
];

/// probe's configuration: podman reads no containers.conf at all, and no user file under HOME, an empty folder of
/// pot's; docker a config folder that does not exist.
pub fn probe_set(engine: Engine, home: &str) -> Vec<(String, String)> {
    let v = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    };
    match engine {
        Engine::Podman => v(&[("CONTAINERS_CONF", "/dev/null"), ("HOME", home)]),
        Engine::Docker => v(&[("DOCKER_CONFIG", "/nonexistent/pot-container")]),
    }
}

/// The run's containers.conf: nothing in it adds to a container. The default seccomp profile applies.
pub fn containers_conf(empty_file: &str) -> String {
    format!(
        "# pot-container: the only containers.conf podman reads for this run (CONTAINERS_CONF).\n\
         # Nothing here adds to the run: no default mounts, volumes, variables or devices.\n\
         [containers]\n\
         default_mounts_file = \"{empty_file}\"\n\
         volumes = []\n\
         env = []\n\
         devices = []\n"
    )
}

/// The engine's command: the caller's environment cleared, then KEEP (and EXTRA) from it, then SET. SET may move
/// `HOME` (podman reads `~/.config/containers/mounts.conf` from it whatever containers.conf says), so the caller's
/// storage is named first: `XDG_DATA_HOME` (rootless podman's storage is under it) and the user's storage.conf.
pub fn command(program: &str, extra: &[String], set: &[(String, String)]) -> Command {
    let mut c = Command::new(program);
    c.env_clear();
    for k in KEEP.iter().copied().chain(extra.iter().map(String::as_str)) {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        if std::env::var_os("XDG_DATA_HOME").is_none() {
            c.env("XDG_DATA_HOME", home.join(".local/share"));
        }
        let storage = home.join(".config/containers/storage.conf");
        if std::env::var_os("CONTAINERS_STORAGE_CONF").is_none() && storage.is_file() {
            c.env("CONTAINERS_STORAGE_CONF", storage);
        }
    }
    for (k, v) in set {
        c.env(k, v);
    }
    c
}
