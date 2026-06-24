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
jag init                      initialize a repository
jag status                    status for the current agent (+ concurrent agents)
jag add <paths...>            stage into the current agent's index
jag commit -m <msg>           commit on the current agent's lane
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

jag reconcile [--into main]   merge agent lanes; auto-resolves disjoint work
jag contention                paths multiple agents/lanes changed differently

jag serve [--addr H:P]        serve this repo over HTTP for clone/fetch/push
jag clone <url> [dir]         clone from a jag server
jag remote add <name> <url>   register a remote (clone adds 'origin')
jag remote list | remove      manage remotes
jag fetch [remote]            download objects + lanes (fast-forwards locals)
jag push [remote] [lane]      upload a lane (default: current agent's lane)
```

Select the acting agent with `--agent <name>` or the `JAG_AGENT` environment
variable; otherwise the configured default (`main`) is used.

## Networking (local jag server)

JAG syncs over plain HTTP. One `jag serve` process serves one repository.

```
# machine/terminal A — host a repo
$ cd my-repo && jag serve --addr 127.0.0.1:9418

# machine/terminal B — clone, change, push back
$ jag clone http://127.0.0.1:9418 my-clone
$ cd my-clone
$ echo hi >> file.txt && jag add file.txt && jag commit -m "edit"
$ jag push origin main
```

Push only fast-forwards: if the remote lane has diverged the push is **rejected**
(non-fast-forward) — `jag fetch` then `jag reconcile`, and push again. Sync is
object-negotiated (only missing objects move) and integrity-checked (received
objects are re-hashed before they're stored).

## Install

JAG is a single self-contained binary built with Rust.

```
cargo build --release
cargo install --path .        # puts `jag` on your PATH
```

See [CLAUDE.md](CLAUDE.md) for the architecture and development workflow.

## Status

Early but working: content-addressable store, per-agent index/HEAD/lanes,
concurrent commits in one folder, path-level reconcile, contention detection,
and HTTP sync (`serve`/`clone`/`fetch`/`push` with object negotiation and
fast-forward rules) are implemented and covered by end-to-end tests. Line-level
merge resolution is the next milestone.
