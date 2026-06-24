//! End-to-end tests exercising JAG's reason for existing: multiple agents
//! working concurrently in a single folder, then reconciling.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Run the built `jag` binary in `dir` as `agent`, returning (stdout, ok).
fn jag(dir: &Path, agent: Option<&str>, args: &[&str]) -> (String, bool) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_jag"));
    cmd.current_dir(dir);
    if let Some(a) = agent {
        cmd.env("JAG_AGENT", a);
    }
    cmd.args(args);
    let out = cmd.output().expect("failed to run jag");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.success(),
    )
}

fn fresh(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jagtest-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn base_repo(tag: &str) -> std::path::PathBuf {
    let dir = fresh(tag);
    assert!(jag(&dir, None, &["init"]).1);
    fs::write(dir.join("shared.txt"), "base\n").unwrap();
    assert!(jag(&dir, None, &["add", "shared.txt"]).1);
    assert!(jag(&dir, None, &["commit", "-m", "base"]).1);
    assert!(jag(&dir, None, &["agent", "start", "alice"]).1);
    assert!(jag(&dir, None, &["agent", "start", "bob"]).1);
    dir
}

#[test]
fn concurrent_agents_share_one_folder() {
    let dir = base_repo("concurrent");

    // Alice and Bob touch different files in the SAME folder, no clones.
    fs::write(dir.join("alice.txt"), "from alice\n").unwrap();
    assert!(jag(&dir, Some("alice"), &["add", "alice.txt"]).1);
    assert!(jag(&dir, Some("alice"), &["commit", "-m", "alice work"]).1);

    fs::write(dir.join("bob.txt"), "from bob\n").unwrap();
    assert!(jag(&dir, Some("bob"), &["add", "bob.txt"]).1);
    assert!(jag(&dir, Some("bob"), &["commit", "-m", "bob work"]).1);

    // Non-overlapping work reconciles automatically.
    let (out, ok) = jag(&dir, None, &["reconcile"]);
    assert!(ok, "reconcile should succeed for disjoint work: {out}");

    assert!(jag(&dir, None, &["checkout", "main"]).1);
    assert!(dir.join("alice.txt").exists());
    assert!(dir.join("bob.txt").exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn overlapping_edits_are_flagged_as_contention() {
    let dir = base_repo("contention");

    fs::write(dir.join("shared.txt"), "alice change\n").unwrap();
    assert!(jag(&dir, Some("alice"), &["add", "shared.txt"]).1);
    assert!(jag(&dir, Some("alice"), &["commit", "-m", "alice"]).1);

    fs::write(dir.join("shared.txt"), "bob change\n").unwrap();
    assert!(jag(&dir, Some("bob"), &["add", "shared.txt"]).1);
    assert!(jag(&dir, Some("bob"), &["commit", "-m", "bob"]).1);

    // The same path changed two ways must NOT auto-merge.
    let (out, ok) = jag(&dir, None, &["reconcile"]);
    assert!(!ok, "reconcile must refuse contended paths");
    assert!(out.contains("contention"), "expected a contention report: {out}");

    let _ = fs::remove_dir_all(&dir);
}
