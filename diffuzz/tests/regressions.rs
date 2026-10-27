//! Replays every stored regression case against the real targets and
//! requires that each one still reproduces its signature.
//!
//! Needs two binaries, given by environment variables:
//! * `DIFFUZZ_MCP_SERVER_RS`: mcp-server-rs (see `scripts/build-targets.sh`);
//! * `DIFFUZZ_RMCP`: the `rmcp-echo` target of this workspace.
//!
//! If they are missing the test FAILS (it is never skipped silently).

use std::path::{Path, PathBuf};

use mcp_diffuzz::corpus::{self, Case};
use mcp_diffuzz::target::{TargetSpec, Timing};

fn binary(var: &str) -> String {
    let path = std::env::var(var).unwrap_or_else(|_| {
        panic!("{var} is not set: build the targets with scripts/build-targets.sh and export it")
    });
    assert!(Path::new(&path).is_file(), "{var}={path} does not exist");
    path
}

fn targets() -> Vec<TargetSpec> {
    vec![
        TargetSpec {
            name: "mcp-server-rs".into(),
            argv: vec![binary("DIFFUZZ_MCP_SERVER_RS")],
        },
        TargetSpec {
            name: "rmcp".into(),
            argv: vec![binary("DIFFUZZ_RMCP")],
        },
    ]
}

fn cases() -> Vec<(PathBuf, Case)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../regressions");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("regressions/ directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let c = Case::load(&p).unwrap();
            (p, c)
        })
        .collect()
}

#[test]
fn corpus_is_not_empty_and_well_formed() {
    let cases = cases();
    assert!(cases.len() >= 3, "only {} regression cases", cases.len());
    for (path, c) in &cases {
        assert!(!c.steps.is_empty(), "{}", path.display());
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            c.file_name(),
            "file name does not match the signature hash"
        );
    }
}

#[test]
fn every_regression_case_still_reproduces() {
    let targets = targets();
    let timing = Timing::default();
    let mut gone = Vec::new();
    let cases = cases();
    for (path, c) in &cases {
        if !corpus::reproduces(&targets, &c.steps, &c.signature, &timing) {
            gone.push(format!("{}: {}", path.display(), c.signature));
        }
    }
    assert!(
        gone.is_empty(),
        "{} of {} cases no longer reproduce:\n{}",
        gone.len(),
        cases.len(),
        gone.join("\n")
    );
}
