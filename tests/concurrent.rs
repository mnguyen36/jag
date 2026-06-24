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

    // Non-overlapping work reconciles automatically (--yes skips the prompt).
    let (out, ok) = jag(&dir, None, &["reconcile", "--yes"]);
    assert!(ok, "reconcile should succeed for disjoint work: {out}");

    assert!(jag(&dir, None, &["checkout", "main"]).1);
    assert!(dir.join("alice.txt").exists());
    assert!(dir.join("bob.txt").exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn undo_and_redo_move_the_lane_and_working_tree() {
    let dir = fresh("undoredo");
    assert!(jag(&dir, None, &["init"]).1);

    fs::write(dir.join("f.txt"), "v1\n").unwrap();
    assert!(jag(&dir, None, &["add", "f.txt"]).1);
    assert!(jag(&dir, None, &["commit", "-m", "v1"]).1);

    fs::write(dir.join("f.txt"), "v2\n").unwrap();
    assert!(jag(&dir, None, &["add", "f.txt"]).1);
    assert!(jag(&dir, None, &["commit", "-m", "v2"]).1);
    assert_eq!(fs::read_to_string(dir.join("f.txt")).unwrap(), "v2\n");

    // undo restores both the lane tip and the working tree to v1.
    assert!(jag(&dir, None, &["undo"]).1);
    assert_eq!(fs::read_to_string(dir.join("f.txt")).unwrap(), "v1\n");

    // redo reapplies v2.
    assert!(jag(&dir, None, &["redo"]).1);
    assert_eq!(fs::read_to_string(dir.join("f.txt")).unwrap(), "v2\n");

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

    // The same line changed two ways must NOT auto-merge.
    let (out, ok) = jag(&dir, None, &["reconcile"]);
    assert!(!ok, "reconcile must refuse overlapping edits");
    assert!(out.contains("conflict"), "expected a conflict report: {out}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reconcile_auto_merges_nonoverlapping_edits() {
    let dir = fresh("automerge");
    assert!(jag(&dir, None, &["init"]).1);
    fs::write(dir.join("f.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
    assert!(jag(&dir, None, &["add", "f.txt"]).1);
    assert!(jag(&dir, None, &["commit", "-m", "base"]).1);
    assert!(jag(&dir, None, &["agent", "start", "a"]).1);
    assert!(jag(&dir, None, &["agent", "start", "b"]).1);

    // Agent a edits the first line; agent b edits the last — different regions.
    fs::write(dir.join("f.txt"), "A1\nl2\nl3\nl4\nl5\n").unwrap();
    assert!(jag(&dir, Some("a"), &["add", "f.txt"]).1);
    assert!(jag(&dir, Some("a"), &["commit", "-m", "a edit"]).1);

    fs::write(dir.join("f.txt"), "l1\nl2\nl3\nl4\nB5\n").unwrap();
    assert!(jag(&dir, Some("b"), &["add", "f.txt"]).1);
    assert!(jag(&dir, Some("b"), &["commit", "-m", "b edit"]).1);

    // JAG's own 3-way merge combines the two edits — no git, no conflict.
    let (out, ok) = jag(&dir, None, &["reconcile", "a", "b", "--yes"]);
    assert!(ok, "non-overlapping edits should auto-merge: {out}");

    assert!(jag(&dir, None, &["checkout", "main"]).1);
    assert_eq!(
        fs::read_to_string(dir.join("f.txt")).unwrap(),
        "A1\nl2\nl3\nl4\nB5\n"
    );

    let _ = fs::remove_dir_all(&dir);
}
