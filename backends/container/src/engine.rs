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

/// A variable bash can't hand over faithfully to the command (T1): read-only in bash (`UID`, `EUID`, `PPID`,
/// `BASHOPTS`, `SHELLOPTS`), set by bash at its start, its exec or after every command (`SHLVL`, `PWD`, `OLDPWD`,
/// `_`, `PIPESTATUS`), unset by a
/// non-interactive bash (`PS1`, `PS2`), one of bash's own (`BASH_*`), or a name of the script's (`__pot_*`). Refused for
/// every `env` item, whatever the engine, so a run's environment does not depend on whether it holds a secret.
pub fn bash_sets(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "UID",
        "EUID",
        "PPID",
        "BASHOPTS",
        "SHELLOPTS",
        "SHLVL",
        "PWD",
        "OLDPWD",
        "_",
        "PS1",
        "PS2",
        "PIPESTATUS",
    ];
    NAMES.contains(&name) || name.starts_with("BASH_") || name.starts_with("__pot_")
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

/// The run's containers.conf: nothing in it adds to a container. The seccomp profile is the run's own (L1:
/// `--security-opt seccomp=RUNDIR/seccomp.json`), never podman's default.
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

/// The longest `TMPDIR` the engine keeps (I1): with `--tty`, podman's conmon makes its console socket in `TMPDIR`,
/// as `conmon-term.XXXXXX`, and a unix socket's path holds 107 bytes (`sun_path`, 108 with its NUL); crun then fails
/// to open a socket conmon could not make (`open console socket: ... No such file or directory`).
pub const TMPDIR_MAX: usize = 107 - "/conmon-term.XXXXXX".len();

/// The `TMPDIR` the engine gets for the caller's VALUE: VALUE, or `/tmp` when it is longer than [`TMPDIR_MAX`].
pub fn tmpdir(value: &std::ffi::OsStr) -> std::ffi::OsString {
    if value.len() > TMPDIR_MAX {
        "/tmp".into()
    } else {
        value.to_os_string()
    }
}

/// The engine's command: the caller's environment cleared, then KEEP (and EXTRA) from it, then SET. A `TMPDIR` too
/// long for a socket's path under it is `/tmp` ([`tmpdir`]).
pub fn command(program: &str, extra: &[String], set: &[(String, String)]) -> Command {
    let mut c = Command::new(program);
    c.env_clear();
    for k in KEEP.iter().copied().chain(extra.iter().map(String::as_str)) {
        if let Some(v) = std::env::var_os(k) {
            if k == "TMPDIR" {
                c.env(k, tmpdir(&v));
            } else {
                c.env(k, v);
            }
        }
    }
    for (k, v) in set {
        c.env(k, v);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tmpdir_too_long_for_conmons_socket_is_tmp() {
        let at = "/".to_string() + &"t".repeat(TMPDIR_MAX - 1);
        assert_eq!(tmpdir(at.as_ref()), std::ffi::OsString::from(&at));
        assert_eq!((at.clone() + "/conmon-term.XXXXXX").len(), 107);
        let over = at + "t";
        assert_eq!(tmpdir(over.as_ref()), std::ffi::OsString::from("/tmp"));
    }
}
