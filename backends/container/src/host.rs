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
    /// The entries of the folder DIR, sorted by name, each a folder, a link (with what it says) or a file; never
    /// through a link. Empty when DIR can't be read.
    fn entries(&self, dir: &str) -> Vec<(String, Entry)>;
}

/// One entry of a folder, as [`Host::entries`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Folder,
    File,
    Link(String),
    /// X5: a name that is not UTF-8 (the listed name is its lossy form, for a message only).
    NotUtf8,
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

    fn entries(&self, dir: &str) -> Vec<(String, Entry)> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return vec![];
        };
        let mut out: Vec<(String, Entry)> = rd
            .flatten()
            .filter_map(|e| {
                let m = e.path().symlink_metadata().ok()?;
                let kind = if e.file_name().to_str().is_none() {
                    Entry::NotUtf8
                } else if m.file_type().is_symlink() {
                    Entry::Link(std::fs::read_link(e.path()).ok()?.to_string_lossy().into_owned())
                } else if m.is_dir() {
                    Entry::Folder
                } else {
                    Entry::File
                };
                Some((e.file_name().to_string_lossy().into_owned(), kind))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
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

#[cfg(test)]
mod tests {
    use super::*;

    /// X5: a name that is not UTF-8 is listed as such, never mangled into another name.
    #[test]
    fn entries_list_a_name_that_is_not_utf8_as_such() {
        use std::os::unix::ffi::OsStrExt as _;
        let d = std::env::temp_dir().join(format!("pot-container-entries-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(std::ffi::OsStr::from_bytes(b"bad\xffname")), "").unwrap();
        std::fs::write(d.join("good"), "").unwrap();
        let got = RealHost.entries(&d.display().to_string());
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(
            got,
            [
                ("bad\u{FFFD}name".to_string(), Entry::NotUtf8),
                ("good".to_string(), Entry::File)
            ]
        );
    }
}
