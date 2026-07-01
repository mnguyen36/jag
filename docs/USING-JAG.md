# Using JAG — operator's guide (humans & agents)

This is a self-contained guide to **using** the `jag` CLI: initialize a repo,
record work, run concurrent agents, merge, and sync over the network. If you are
an AI agent handed this file, you can operate `jag` from zero by following it.

JAG is an agent-first version-control tool. Unlike Git, many **agents** work
**concurrently in one folder** — each with its own staging area, HEAD, and lane
(branch) — and `jag reconcile` merges their work (with a real 3-way line merge).

---

## 0. Get the binary

`jag` is a single self-contained binary (no runtime, no plugins).

- **If it's already installed** (e.g. on this machine it lives at
  `C:\Users\ZION\bin\jag.exe` and is on `PATH`): just run `jag`. In a brand-new
  shell, confirm with `jag --version`. If `jag` isn't found, call it by full
  path or open a new terminal so `PATH` refreshes.
- **Build from source** (Windows / macOS / Linux):
  ```
  cargo build --release          # produces target/release/jag(.exe)
  cargo install --path .         # puts `jag` on your PATH
  ```

Verify: `jag --version` and `jag --help`.

---

## 1. The mental model (read this first)

| Term | What it is | Git analogue |
|------|-----------|--------------|
| **Repository** | one folder containing `.jag/` | a clone |
| **Agent** | a concurrent worker *inside* the folder, with its own index + HEAD | a separate clone + user |
| **Lane** | a line of work (each agent gets its own by default) | a branch |
| **Reconcile** | merge lanes together (3-way line merge) | `git merge` |
| **Contention** | the same lines changed two ways | a merge conflict |

The acting agent is chosen by (highest priority first): the `--agent <name>`
flag, the `JAG_AGENT` environment variable, then the repo default (`main`).

> **Important:** don't run `git` and `jag` as *both* the source of truth on the
> same working tree — a `jag checkout` rewrites files from jag's history and can
> clobber git-tracked edits (and vice-versa). Pick one VCS per working tree, or
> use separate folders.

---

## 2. Quickstart — init a repo and record work

```bash
mkdir myproject && cd myproject
jag init                       # creates .jag/, default agent 'main' on lane 'main'

echo "hello" > file.txt
jag push -m "first change"     # ONE command: stage all + commit + push (local-only if no remote)

jag log                        # see history
jag status                     # see what's changed
```

`jag push` is the everyday "save everything" verb. There is no remote yet, so it
just commits locally and tells you so. (Low-level `jag add` / `jag commit` still
exist if you want explicit staging.)

Undo / redo are an editor-style single step (not `git reset`/`reflog`):
```bash
jag undo        # step the current lane back one change; restores the files too
jag redo        # reapply
```

---

## 3. Concurrent agents in one folder (the point of JAG)

```bash
jag agent start alice          # new agent 'alice' on lane 'alice' (forked from main)
jag agent start bob            # new agent 'bob' on lane 'bob'
jag agent list

# Work as each agent via the JAG_AGENT env var (or --agent):
JAG_AGENT=alice  jag push -m "alice's work"
JAG_AGENT=bob    jag push -m "bob's work"

jag contention                 # paths multiple agents/lanes changed differently
jag reconcile                  # merge all agent lanes into main (see below)
```

### Merging (reconcile)

```bash
jag reconcile                  # preview the merge of all local lanes into main
jag reconcile alice bob        # merge only these lanes
jag merge alice bob            # 'merge' is an alias for 'reconcile'
jag reconcile alice bob --yes  # skip the confirmation prompt (use in scripts/agents)
```

`reconcile` first **prints a plan** and does nothing until you confirm (`-y` /
`--yes` to skip the prompt). Then it does a **3-way line merge**:

- file edits that don't overlap **auto-combine** — even within the same file;
- truly overlapping edits become a **conflict**: jag writes git-style markers
  (`<<<<<<<` / `=======` / `>>>>>>>`) into the working tree and pauses the merge.
  To finish: edit the marked files, then `jag add . && jag commit -m "resolve"`
  on the target lane — that commit becomes a proper merge commit.

After a successful reconcile, run `jag checkout main` to materialize the merged
files into the working tree.

---

## 4. Command reference

```
# core
jag init                       initialize a repository
jag status                     status for the current agent
jag add <paths...>             stage paths into the current agent's index
jag commit -m <msg>            commit staged changes on the current lane
jag log [-n N]                 history for the current lane
jag diff [--staged]            index vs working tree (or HEAD vs index)
jag cat-file <oid>             print an object (blobs/trees/commits are text)

# the everyday shortcut
jag push [-m <msg>] [--local]  stage all + commit + push to origin (or just commit with --local)
jag undo / jag redo            step the current lane back / forward one change

# agents & lanes
jag agent start <name> [--lane <l>] [--from <base>]
jag agent list | who | use <name>
jag lane list | new <name> [--from <base>]
jag checkout <lane>            materialize a lane into the working tree

# merging
jag reconcile [--into main] [-m <msg>] [-y] [lanes...]   (alias: jag merge)
jag contention                 paths changed differently across lanes/agents

# networking
jag serve [--addr 127.0.0.1:9418] [--threads N]   serve this repo + web dashboard
jag dl <url> [dir]             download (clone) from a jag server   (alias: clone)
jag remote add <name> <url> | list | remove <name>
jag fetch [remote]             pull objects + lanes (fast-forwards locals)

# natural language (optional) — use the `jags` command
jags <plain english>           e.g. jags save my work and push
jag model setup | status | on | off    attach a local model for free-form NL

# global
--agent <name>                 act as this agent (overrides JAG_AGENT and default)
```

---

## 5. Networking — clone / push between repos

One `jag serve` process hosts one repository over HTTP and also serves a web
dashboard at the same address.

```bash
# host:
cd my-repo && jag serve --addr 127.0.0.1:9418
# open http://127.0.0.1:9418 in a browser for the dashboard

# elsewhere:
jag dl http://127.0.0.1:9418 my-clone     # registers remote 'origin'
cd my-clone
echo hi >> file.txt
jag push -m "edit"                        # pushes the current lane to origin
```

Pushes are **fast-forward only**: if the remote lane diverged, the push is
rejected — `jag fetch`, then `jag reconcile`, then push again. (No auth/TLS yet;
keep servers on localhost / trusted networks.)

---

## 6. Natural language — the `jags` command (optional)

`jag` parses strictly as commands. For plain English, use **`jags`** (installed
alongside `jag`): everything after it is a request.

```
jags update my lane and then reconcile
jags save my work and push
```

`jags` prints the command it resolved to and confirms before mutating. A
built-in matcher handles common phrasings offline; `jag model setup` attaches a
small local model (via Ollama) for free-form requests.

---

## 7. A complete recipe for an agent: "init a jag and make a commit"

```bash
# 1. confirm the tool works
jag --version

# 2. create and initialize a repo
mkdir demo && cd demo
jag init

# 3. add content and record it (one command)
printf 'line one\nline two\n' > notes.txt
jag push -m "add notes"

# 4. verify
jag log -n 5
jag status        # should be clean
```

That's a working JAG repository with one commit on lane `main`, authored by
agent `main`. To go multi-agent, `jag agent start <name>` and work as each via
`JAG_AGENT=<name>`, then `jag reconcile`.

---

## More

- Project overview & rationale: [README.md](../README.md)
- Source & architecture (for developing jag itself): [CLAUDE.md](../CLAUDE.md)
