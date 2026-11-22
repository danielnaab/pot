//! `prepare` on a real run folder (it starts no container): it can run again, never writes through a link
//! planted in the run folder, and names the file it could not write (audit F9).

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

fn id(field: &str) -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with(field)).unwrap();
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

/// A run folder whose every bind source exists: a workspace, a state directory and the config's files.
fn world(name: &str) -> (PathBuf, PathBuf) {
    let top = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&top);
    let (ws, run, store) = (top.join("ws"), top.join("run"), top.join("store"));
    for d in [&ws.join(".git"), &run, &store, &top.join("state")] {
        fs::create_dir_all(d).unwrap();
    }
    for f in ["sh", "bash", "env", "ca.crt", "base.txt"] {
        fs::write(store.join(f), "").unwrap();
    }
    fs::create_dir_all(store.join("terminfo")).unwrap();
    fs::create_dir_all(store.join("zoneinfo")).unwrap();
    let s = |f: &str| store.join(f).display().to_string();
    let config = serde_json::json!({ "engine": "podman", "sh": s("sh"), "shell": s("bash"), "env": s("env"),
        "cacert": s("ca.crt"), "terminfo": s("terminfo"), "tzdir": s("zoneinfo"), "baseClosure": s("base.txt") });
    fs::write(top.join("config.json"), config.to_string()).unwrap();
    let docs =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/container/prepare.in/read-only");
    let mut p: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(docs.join("policy.json")).unwrap()).unwrap();
    let w = ws.display().to_string();
    p["workspace"][0]["path"] = w.clone().into();
    p["workspace"][0]["common"] = format!("{w}/.git").into();
    p["workspace"][0]["id"] = format!("repo:{w}").into();
    p["mounts"]["state"][0]["dir"] = top.join("state").display().to_string().into();
    let items: Vec<serde_json::Value> = p["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] != "file" && i["kind"] != "read")
        .cloned()
        .collect();
    p["items"] = items.into();
    fs::write(run.join("policy.json"), p.to_string()).unwrap();
    let mut l: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(docs.join("launch.json")).unwrap()).unwrap();
    l["uid"] = id("Uid:").into();
    l["gid"] = id("Gid:").into();
    l["cwd"] = w.into();
    l["home"] = top.join("home").display().to_string().into();
    l["closure"] = serde_json::json!([]);
    l["potBin"] = serde_json::Value::Null;
    fs::write(run.join("launch.json"), l.to_string()).unwrap();
    (top, run)
}

fn prepare(top: &Path, run: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pot-container"))
        .arg("--config")
        .arg(top.join("config.json"))
        .arg("prepare")
        .arg(run)
        .output()
        .unwrap()
}

#[test]
fn prepare_runs_again_and_never_writes_through_a_link() {
    let (top, run) = world("again");
    let first = prepare(&top, &run);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let again = prepare(&top, &run);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    // a link planted where prepare writes: replaced, never followed
    let outside = top.join("outside");
    fs::write(&outside, "keep").unwrap();
    fs::remove_file(run.join("hosts")).unwrap();
    symlink(&outside, run.join("hosts")).unwrap();
    let planted = prepare(&top, &run);
    assert!(
        planted.status.success(),
        "{}",
        String::from_utf8_lossy(&planted.stderr)
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    assert!(
        run.join("hosts")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_file()
    );
}

#[test]
fn prepare_names_the_file_it_could_not_write() {
    let (top, run) = world("blocked");
    fs::create_dir_all(run.join("passwd/x")).unwrap();
    let out = prepare(&top, &run);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains(&format!("{}/passwd", fs::canonicalize(&run).unwrap().display()))
            || err.contains("run/passwd"),
        "{err}"
    );
}
