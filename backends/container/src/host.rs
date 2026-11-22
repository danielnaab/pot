//! What `prepare` reads of the host's file system, behind one trait so the rendering stays a pure function of
//! (policy, launch, config, engine, these facts) and its tests use a fake host.

use std::collections::BTreeSet;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;

pub trait Host {
    /// The path exists (a dangling link counts, as bwrap's `lexists`).
    fn lexists(&self, p: &str) -> bool;
    /// A real directory: not a link to one.
    fn is_real_dir(&self, p: &str) -> bool;
    /// Every socket below DIR, on DIR's file system, never through a link (bwrap's `find -xdev -type s`). Sorted.
    fn sockets_under(&self, dir: &str) -> Vec<String>;
    /// The path with every link and `..` resolved, when it exists.
    fn realpath(&self, p: &str) -> Option<String>;
}

pub struct RealHost;

impl Host for RealHost {
    fn realpath(&self, p: &str) -> Option<String> {
        std::fs::canonicalize(p)
            .ok()
            .map(|r| r.to_string_lossy().into_owned())
    }

    fn lexists(&self, p: &str) -> bool {
        Path::new(p).symlink_metadata().is_ok()
    }

    fn is_real_dir(&self, p: &str) -> bool {
        Path::new(p).symlink_metadata().is_ok_and(|m| m.is_dir())
    }

    fn sockets_under(&self, dir: &str) -> Vec<String> {
        let Ok(top) = Path::new(dir).symlink_metadata() else {
            return vec![];
        };
        if !top.is_dir() {
            return vec![];
        }
        let mut out = BTreeSet::new();
        let mut todo = vec![Path::new(dir).to_path_buf()];
        while let Some(d) = todo.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                let Ok(m) = e.path().symlink_metadata() else {
                    continue;
                };
                if m.file_type().is_socket() {
                    out.insert(e.path().to_string_lossy().into_owned());
                } else if m.is_dir() && m.dev() == top.dev() {
                    todo.push(e.path());
                }
            }
        }
        out.into_iter().collect()
    }
}

/// The caller's effective uid: the owner of /proc/self (std has no getuid, and unsafe is forbidden).
pub fn euid() -> Option<u32> {
    Path::new("/proc/self").metadata().ok().map(|m| m.uid())
}
