//! The engine's own environment and configuration (audit F2): podman and docker read configuration files and
//! variables that can add a volume, a variable or `seccomp_profile = "unconfined"` to every container, or send
//! the command to another engine (`CONTAINER_HOST`, `DOCKER_HOST`). The engine therefore runs with only the
//! variables below, and with pot's configuration in place of the user's: `CONTAINERS_CONF` (which replaces every
//! other containers.conf; `CONTAINERS_CONF_OVERRIDE` is not passed) and `DOCKER_CONFIG`.

use std::process::Command;

use crate::render::Engine;

/// The caller's variables the engine keeps (when set): what rootless podman needs to find its runtime folder and
/// the user's systemd, and the terminal. Its storage is pot's (`storage_conf`). The run's own variables passed by
/// name are added to these, which is why [`configures_engine`] refuses some of them.
pub const KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "TMPDIR",
    "LANG",
    "TERM",
    "COLORTERM",
];

/// probe's configuration, in DIR (a temporary folder of its own): podman reads no containers.conf, a storage.conf
/// of pot's with its store in DIR, and no user file under a home in DIR; docker a config folder that does not exist.
pub fn probe_set(engine: Engine, dir: &str) -> Vec<(String, String)> {
    let v = |pairs: &[(&str, String)]| pairs.iter().map(|(k, v)| ((*k).to_string(), v.clone())).collect();
    match engine {
        Engine::Podman => v(&[
            ("CONTAINERS_CONF", "/dev/null".into()),
            ("CONTAINERS_STORAGE_CONF", format!("{dir}/storage.conf")),
            ("HOME", format!("{dir}/home")),
        ]),
        Engine::Docker => v(&[("DOCKER_CONFIG", "/nonexistent/pot-container".into())]),
    }
}

/// A variable the engine reads as configuration (or the dynamic loader, or Go's runtime, as the engine's own
/// process). A run's variables reach the container through the engine's environment (`--env NAME`, so that no
/// value is in an argument or a file), so a run may not be granted one of these: it would configure the engine
/// that confines it (audit N1).
pub fn configures_engine(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "CONTAINERS_",
        "_CONTAINERS_",
        "CONTAINER_",
        "DOCKER_",
        "PODMAN_",
        "BUILDAH_",
        "STORAGE_",
        "XDG_",
        "DBUS_",
        "LD_",
        // N5: socket activation; podman passes the caller's fds 3.. into the container
        "LISTEN_",
    ];
    const NAMES: &[&str] = &[
        "HOME",
        "PATH",
        "TMPDIR",
        "NOTIFY_SOCKET",
        "REGISTRY_AUTH_FILE",
        "SSH_AUTH_SOCK",
        "GODEBUG",
        "GOGC",
        "GOMAXPROCS",
        "GOMEMLIMIT",
        "GOTRACEBACK",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p)) || NAMES.contains(&name)
}

/// The run's storage.conf: native overlay (no `mount_program`), the store a folder of the run's own. A fresh store
/// per run costs about 10 ms against a persistent one (podman 5.4.1, 2026-11-21: 96 ms against 105 ms a start).
pub fn storage_conf(store: &str) -> String {
    format!(
        "# pot-container: the only storage.conf podman reads for this run (CONTAINERS_STORAGE_CONF).\n\
         [storage]\n\
         driver = \"overlay\"\n\
         rootless_storage_path = \"{store}\"\n"
    )
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
