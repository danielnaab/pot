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

/// probe's configuration: podman reads no containers.conf at all; docker a config folder that does not exist.
pub fn probe_set(engine: Engine) -> (&'static str, &'static str) {
    match engine {
        Engine::Podman => ("CONTAINERS_CONF", "/dev/null"),
        Engine::Docker => ("DOCKER_CONFIG", "/nonexistent/pot-container"),
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

/// The engine's command: the caller's environment cleared, then KEEP (and EXTRA) from it, then SET.
pub fn command(program: &str, extra: &[String], set: &[(String, String)]) -> Command {
    let mut c = Command::new(program);
    c.env_clear();
    for k in KEEP.iter().copied().chain(extra.iter().map(String::as_str)) {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    for (k, v) in set {
        c.env(k, v);
    }
    c
}
