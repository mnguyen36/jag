//! End-to-end tests for the sync layer: a real `jag serve` child process plus
//! clone / push against it, including the non-fast-forward rejection path.

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_jag")
}

/// Run jag in `dir`, returning (combined output, success).
fn jag(dir: &Path, args: &[&str]) -> (String, bool) {
    let out = Command::new(bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to run jag");
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (s, out.status.success())
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_ready(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("jag server never became ready on port {port}");
}

/// Kills the server child when the test ends.
struct Serving(Child);
impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_server(dir: &Path) -> (Serving, String) {
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let child = Command::new(bin())
        .current_dir(dir)
        .args(["serve", "--addr", &addr, "--threads", "2"])
        .spawn()
        .expect("spawn jag serve");
    wait_ready(port);
    (Serving(child), format!("http://127.0.0.1:{port}"))
}

fn fresh(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jagnet-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn clone_then_push_roundtrip() {
    let root = fresh("roundtrip");
    let server = root.join("server");
    fs::create_dir_all(&server).unwrap();
    assert!(jag(&server, &["init"]).1);
    fs::write(server.join("a.txt"), "one\n").unwrap();
    assert!(jag(&server, &["add", "a.txt"]).1);
    assert!(jag(&server, &["commit", "-m", "init"]).1);

    let (_srv, url) = start_server(&server);

    // Clone brings the content across.
    assert!(jag(&root, &["clone", &url, "client"]).1);
    let client = root.join("client");
    assert_eq!(fs::read_to_string(client.join("a.txt")).unwrap(), "one\n");

    // Edit in the clone and push back.
    fs::write(client.join("a.txt"), "one\ntwo\n").unwrap();
    assert!(jag(&client, &["add", "a.txt"]).1);
    assert!(jag(&client, &["commit", "-m", "client edit"]).1);
    let (out, ok) = jag(&client, &["push"]);
    assert!(ok, "push should succeed: {out}");

    // A second clone observes the pushed change — proof the server advanced.
    assert!(jag(&root, &["clone", &url, "client2"]).1);
    assert_eq!(
        fs::read_to_string(root.join("client2/a.txt")).unwrap(),
        "one\ntwo\n"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn diverged_push_is_rejected() {
    let root = fresh("diverged");
    let server = root.join("server");
    fs::create_dir_all(&server).unwrap();
    assert!(jag(&server, &["init"]).1);
    fs::write(server.join("a.txt"), "base\n").unwrap();
    assert!(jag(&server, &["add", "a.txt"]).1);
    assert!(jag(&server, &["commit", "-m", "base"]).1);

    let (_srv, url) = start_server(&server);

    // Two clones, both at the same base tip.
    assert!(jag(&root, &["clone", &url, "c1"]).1);
    assert!(jag(&root, &["clone", &url, "c2"]).1);
    let (c1, c2) = (root.join("c1"), root.join("c2"));

    // c1 edits and pushes — fast-forward, accepted.
    fs::write(c1.join("a.txt"), "c1\n").unwrap();
    assert!(jag(&c1, &["add", "a.txt"]).1);
    assert!(jag(&c1, &["commit", "-m", "c1"]).1);
    assert!(jag(&c1, &["push"]).1);

    // c2 (still at base) edits and pushes — must be rejected as non-fast-forward.
    fs::write(c2.join("a.txt"), "c2\n").unwrap();
    assert!(jag(&c2, &["add", "a.txt"]).1);
    assert!(jag(&c2, &["commit", "-m", "c2"]).1);
    let (out, ok) = jag(&c2, &["push"]);
    assert!(!ok, "diverged push must fail");
    assert!(
        out.contains("reject") || out.contains("fast-forward"),
        "expected a non-fast-forward rejection, got: {out}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn push_to_a_slashed_lane_name() {
    let root = fresh("slashed");
    let server = root.join("server");
    fs::create_dir_all(&server).unwrap();
    assert!(jag(&server, &["init"]).1);
    fs::write(server.join("a.txt"), "base\n").unwrap();
    assert!(jag(&server, &["add", "a.txt"]).1);
    assert!(jag(&server, &["commit", "-m", "base"]).1);

    let (_srv, url) = start_server(&server);

    assert!(jag(&root, &["dl", &url, "c"]).1);
    let c = root.join("c");

    // Namespaced lane (contains '/'), like git's feature/x.
    assert!(jag(&c, &["lane", "new", "team/feature"]).1);
    assert!(jag(&c, &["checkout", "team/feature"]).1);
    fs::write(c.join("a.txt"), "feature\n").unwrap();

    // Before the routing fix this 404'd on POST /ref/team/feature.
    let (out, ok) = jag(&c, &["push"]);
    assert!(ok, "push to a slashed lane should succeed: {out}");

    // A fresh clone sees the slashed lane the server now advertises.
    assert!(jag(&root, &["dl", &url, "c2"]).1);
    let (lanes, _) = jag(&root.join("c2"), &["lane", "list"]);
    assert!(
        lanes.contains("team/feature"),
        "server should advertise the slashed lane: {lanes}"
    );

    let _ = fs::remove_dir_all(&root);
}
