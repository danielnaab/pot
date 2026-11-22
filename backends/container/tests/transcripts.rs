//! The reference in `docs/reference/container/` is the spec: each page's `console` blocks run against the binary,
//! in the page's `.in` folder (its run directories). `TRYCMD=overwrite cargo test` rewrites the expected output;
//! review that diff like code. Nothing here starts a container.

use std::path::{Path, PathBuf};

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/container")
}

#[test]
fn transcripts() {
    let cases = trycmd::TestCases::new();
    cases.register_bin("pot-container", Path::new(env!("CARGO_BIN_EXE_pot-container")));
    // a run directory's absolute path shows as [CWD]/NAME (each page runs in its .in folder)
    cases.case(docs().join("*.md"));
}
