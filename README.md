# JAG — Just Another Git

**The Agent Era replacement for Git.** A version control system whose default
unit of work is the *agent*, not the developer — so many agents can stage,
commit, and reconcile **concurrently inside a single folder** instead of cloning
the repo N times.

```
$ jag init
$ jag add . && jag commit -m "base"
$ jag agent start alice          # independent worker, its own lane
$ jag agent start bob            # ...in the SAME folder
$ JAG_AGENT=alice jag commit -m "alice's work"   # concurrent
$ JAG_AGENT=bob   jag commit -m "bob's work"     # no clones
$ jag reconcile                  # N-way merge of the agent lanes
```

## The problem it solves

With Git, parallel work means parallel *clones*: one working tree per line of
work, then a dance of remotes and merges to bring them back. For a fleet of
agents operating on one codebase that overhead dominates.

JAG removes the single-tenant bottleneck. Git keeps **one** index and **one**
HEAD per working tree; JAG gives **every agent its own index, HEAD, and lane**
over **one shared object store and working folder**. Agents work side by side;
JAG tracks who changed what and reconciles them.

## Concepts (and their Git analogues)

| JAG            | Git analogue | What's different |
|----------------|--------------|------------------|
| **Agent**      | a clone + user | A first-class concurrent worker living *inside* one folder, with its own index/HEAD. |
| **Lane**       | branch       | A line of work. Each agent gets its own by default. |
| **Reconcile**  | merge        | An N-way, authorship-aware merge of lanes. Non-overlapping work flows in automatically. |
| **Contention** | merge conflict | Surfaced *across agents* (even before commit) instead of discovered at merge time. |

## Commands

```
jags <plain english>          natural language, e.g. jags save my work and push
jag model setup               download a local model (via Ollama) for free-form NL

jag push [-m <msg>]           SAVE EVERYTHING: stage all + commit + push to origin
jag undo                      step the current lane back one change (restores files)
jag redo                      reapply the last undone change

jag init                      initialize a repository
jag status                    status for the current agent (+ concurrent agents)
jag add <paths...>            stage into the current agent's index (low-level)
jag commit -m <msg>           commit on the current agent's lane (low-level)
jag log [-n N]                history for the current lane
jag diff [--staged]           index vs working tree (or HEAD vs index)
jag cat-file <oid>            inspect an object (blobs/trees/commits are text)

jag agent start <name>        new agent: own index, HEAD, and lane
jag agent list                agents, their lanes, and tips
jag agent who                 current agent
jag agent use <name>          set the default agent

jag lane list                 lanes and their tips
jag lane new <name>           create a lane from a base
jag checkout <lane>           materialize a lane into the shared working tree

jag merge [lanes...]          preview the merge into main, then confirm (-y to skip)
jag reconcile [--into main]   same as merge (lower-level name)
jag contention                paths multiple agents/lanes changed differently

jag serve [--addr H:P]        serve this repo over HTTP for dl/fetch/push
jag dl <url> [dir]            download (clone) from a jag server  (alias: clone)
jag remote add <name> <url>   register a remote (dl adds 'origin')
jag remote list | remove      manage remotes
jag fetch [remote]            download objects + lanes (fast-forwards locals)
```

## Everyday flow

The whole save → share loop is one command. Edit files, then:

```
$ jag push -m "did the thing"      # stage all + commit + push to origin
```

Changed your mind?

```
$ jag undo      # back one step — lane tip and working files both revert
$ jag redo      # forward again
```

`undo`/`redo` are a single pointer walking the lane's history (like an editor),
not git's `reset`/`revert`/`reflog`. Merging concurrent agents is `jag reconcile`
(aka `jag merge`): disjoint work auto-combines, real collisions surface as
`contention` instead of halting everything.

Select the acting agent with `--agent <name>` or the `JAG_AGENT` environment
variable; otherwise the configured default (`main`) is used.

## Plain English — the `jags` command

`jag` parses strictly as commands. For plain English use **`jags`** (installed
alongside `jag`) — everything after it is a request:

```
$ jags save my work and push
Interpreted as:  jag push -m "my work"
$ jags what changed
Interpreted as:  jag status
$ jags bring feature-login into main
Interpreted as:  jag merge feature-login
```

`jags` always prints the command it resolved to, and confirms before mutating.
A built-in matcher handles the common phrasings instantly and offline. For
free-form requests, attach a small **local model** (no cloud):

```
$ jag model setup            # downloads qwen2.5:0.5b via Ollama and enables it
$ jag model status           # show config + reachability
$ jag model off              # back to the built-in matcher only
```

The model runs locally (default Ollama at `localhost:11434`); jag asks it for a
single command and runs that. Config lives at `~/.jag/nl.json` and belongs to
the installed `jag`, not to any repo.

## Networking (local jag server)

JAG syncs over plain HTTP. One `jag serve` process serves one repository.

```
# machine/terminal A — host a repo
$ cd my-repo && jag serve --addr 127.0.0.1:9418

# machine/terminal B — download, change, push back
$ jag dl http://127.0.0.1:9418 my-clone
$ cd my-clone
$ echo hi >> file.txt && jag add file.txt && jag commit -m "edit"
$ jag push origin main
```

Push only fast-forwards: if the remote lane has diverged the push is **rejected**
(non-fast-forward) — `jag fetch` then `jag reconcile`, and push again. Sync is
object-negotiated (only missing objects move) and integrity-checked (received
objects are re-hashed before they're stored).

## Install

JAG is a single self-contained binary (pure Rust, no runtime, no extra DLLs).
It runs on **Windows, macOS, and Linux** — same source, same commands.

```
cargo build --release          # native binary for the current OS
cargo install --path .         # put `jag` (or jag.exe) on your PATH
```

Cross-compiling (e.g. building the Windows binary from Linux/WSL):

```
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu   # -> jag.exe
```

The Windows binary links only against system DLLs, so it's a single ~2 MB
`jag.exe` you can drop on your PATH.

## Documentation

- **[docs/USING-JAG.md](docs/USING-JAG.md)** — operator's guide: how to init a
  repo, run concurrent agents, merge, and sync. Point a fresh agent here and it
  can drive `jag` from zero.
- [CLAUDE.md](CLAUDE.md) — source layout and architecture (for developing `jag`).

## Status

Early but working: content-addressable store, per-agent index/HEAD/lanes,
concurrent commits in one folder, path-level reconcile, contention detection,
and HTTP sync (`serve`/`clone`/`fetch`/`push` with object negotiation and
fast-forward rules) are implemented and covered by end-to-end tests. Line-level
merge resolution is the next milestone.
