//! `jags` — the natural-language front-end to jag.
//!
//! Everything after `jags` is a plain-English request:
//!
//!     jags update my lane and then reconcile
//!     jags save my work and push
//!
//! It forwards the words to `jag do <words>`, which resolves the request to a
//! command (a built-in matcher first, an optional local model as fallback),
//! prints the command it resolved to, and confirms before mutating. Running the
//! child with inherited stdio means those confirmation prompts work normally.

use std::process::Command;

fn find_jag() -> String {
    let exe = if cfg!(windows) { "jag.exe" } else { "jag" };
    // Prefer the jag binary installed right next to this one.
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let sibling = dir.join(exe);
            if sibling.exists() {
                return sibling.to_string_lossy().into_owned();
            }
        }
    }
    exe.to_string() // fall back to PATH
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help") {
        eprintln!("jags — say what you want in plain English:");
        eprintln!("  jags update my lane and then reconcile");
        eprintln!("  jags save my work and push");
        eprintln!("\n(jags always shows the command it resolved to and confirms before changing anything.)");
        std::process::exit(if args.is_empty() { 2 } else { 0 });
    }

    let jag = find_jag();
    match Command::new(&jag).arg("do").args(&args).status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("jags: could not run '{jag}': {e}");
            eprintln!("      is jag installed and on your PATH?");
            std::process::exit(1);
        }
    }
}
