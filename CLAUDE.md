# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**JAG** (Just Another Git) is an agent-first replacement for Git, written in
Rust. Its differentiator: instead of cloning a repo N times to run N concurrent
efforts, JAG lets many **agents** stage, commit, and reconcile **inside one
shared folder**. Each agent has its own index, HEAD, and lane over a single
shared object store.

## Environment

This is a **WSL (Ubuntu) project**. The source lives on the Windows side at
`C:\dev\jag` (i.e. `/mnt/c/dev/jag` from WSL); build and run from WSL where the
Rust toolchain and a C linker are installed. To avoid slow `/mnt/c` builds, the
working convention is a native target dir:

```bash
source $HOME/.cargo/env
export CARGO_TARGET_DIR=$HOME/.cache/jag-target
```

## Common commands

```bash
cargo build                       # debug build
cargo build --release             # optimized single binary (LTO)
cargo test                        # run all tests
cargo test concurrent_agents_share_one_folder   # run ONE test by name
cargo test -- --nocapture         # show stdout from tests
cargo install --path . --root $HOME/.local      # install `jag` on PATH
cargo clippy                      # lint (if installed)
```

Tests are end-to-end (`tests/concurrent.rs`): they invoke the built binary via
`env!("CARGO_BIN_EXE_jag")` against a temp repo, so they exercise the real CLI.
**Dogfood on this repo itself** — `jag init` here creates `.jag/` (gitignored)
and coexists with `.git/`.

## Architecture

JAG is a Git-shaped core with an agent-first layer on top.

### Storage layout (`.jag/`)
- `objects/ab/cdef…` — content-addressable, SHA-256 + zlib. Header is
  `"<kind> <len>\0"` then payload, exactly like Git.
- `refs/lanes/<name>` — lane tips (commit oids). **Shared by all agents.**
- `agents/<name>/{HEAD,index,meta.json}` — **per-agent** state.
- `config` — JSON: `default_agent`, `default_lane`, `version`.

### The core idea, in code
The single-tenant pieces of Git are made per-agent:
- **Index** (`index.rs`) — each agent stages into `agents/<name>/index`, so
  concurrent staging never collides.
- **HEAD** (`refs.rs`) — `agents/<name>/HEAD` points at a lane
  (`ref: refs/lanes/<lane>`) or a detached oid.
- **Lane** — agents fork their own lane off a base (default `main`) on
  `agent start`, then commit to it independently.

Everything else (objects, trees, lane refs, the working tree) is **shared**.
That sharing is the whole point: one folder, many workers.

### Object model
- `objects.rs` — `hash_object` / `read_object`; idempotent writes make
  concurrent writers safe.
- `tree.rs` — trees serialized as **readable text** (`<mode> <kind> <oid>\t<name>`),
  not packed binary, so `jag cat-file <tree>` is human-inspectable.
  `build_tree_from_paths` / `flatten_tree` convert between nested trees and flat
  `path -> (mode, oid)` maps — the lingua franca for status/diff/reconcile.
- `commit.rs` — commits carry **`agent` and `lane`** alongside tree+parents;
  that authorship is what reconcile/contention reason over. >1 parent = a merge.

### Agent-first operations
- `reconcile.rs` — `plan_reconcile` does an **N-way, path-level** merge of lane
  tip trees vs the target: a path changed by exactly one source is taken
  automatically; a path changed differently by ≥2 sources becomes **contention**
  and aborts (no silent resolution). `apply_reconcile` writes a merge commit
  with all source tips as parents. `find_contention` runs the same comparison
  live, including uncommitted staged work across agents.
- `agent.rs` — `create_agent` forks the lane and seeds the index from the lane
  tip tree (so the index mirrors HEAD, Git-style).

### Networking / sync (local jag server)
Plain HTTP, blocking, single binary (`tiny_http` server + `ureq` client, TLS
off). One `jag serve` process serves one repo.
- `protocol.rs` — JSON wire types. Endpoints: `GET /refs`, `GET /closure/<oid>`,
  `GET|POST /object/<oid>`, `POST /missing`, `POST /ref/<lane>`.
- `objects.rs` — `read_raw`/`write_raw` move the **on-disk compressed bytes**
  verbatim (the store is content-addressed identically on both ends);
  `write_raw` re-hashes before storing (integrity) and is atomic via temp+rename.
- `graph.rs` — `reachable` (object closure from tips) and `is_ancestor`
  (fast-forward checks).
- `server.rs` — N worker threads share the listener; object writes are
  idempotent, lane updates serialized behind a `Mutex` with an FF/contention
  rule (non-fast-forward → HTTP 409).
- `client.rs` / `sync.rs` — push computes its closure, asks `/missing`, uploads
  the gap, then `/ref`; fetch pulls each lane's closure and fast-forwards local
  lanes (a diverged lane is imported as `<remote>/<lane>` for reconcile).
- `remote.rs` — remotes stored as `name -> url` JSON in `.jag/remotes`.

Note: `refs::list_lanes` is recursive so remote-tracking lanes (`origin/main`)
are listed; `reconcile` excludes `/`-containing lanes from its default sources.

### CLI flow
`main.rs` (clap) parses, resolves the acting agent (`--agent` > `JAG_AGENT` >
config default), then dispatches to a handler in `commands.rs`. `init` and
`clone` are the only commands that run without an existing repo.

## Conventions / gotchas
- Paths are stored with `/` separators everywhere (normalized on the way in).
- Reconcile and contention are **path-level**, not line-level — two edits to the
  same file are contention even if they wouldn't textually conflict. Line-level
  merge is the next milestone; keep the path-level plan as the fallback.
- `reconcile` updates refs/commits only; it intentionally does **not** touch the
  shared working tree. Run `jag checkout <lane>` to materialize.
