//! JAG — Just Another Git. Agent-first version control.
//!
//! CLI surface and dispatch. Every subcommand resolves to a handler in
//! `commands`. The global `--agent` flag (and the `JAG_AGENT` env var) select
//! which concurrent agent the command acts as inside the shared folder.

mod agent;
mod client;
mod commands;
mod commit;
mod graph;
mod index;
mod journal;
mod objects;
mod protocol;
mod reconcile;
mod refs;
mod remote;
mod repo;
mod server;
mod status;
mod sync;
mod tree;
mod worktree;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::repo::Repo;

#[derive(Parser)]
#[command(
    name = "jag",
    version,
    about = "Just Another Git — agent-first version control for concurrent multi-agent workflows"
)]
struct Cli {
    /// Act as this agent (overrides JAG_AGENT and the configured default)
    #[arg(long, global = true)]
    agent: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new JAG repository
    Init,
    /// Show working-tree status for the current agent
    Status,
    /// Stage file contents into the current agent's index
    Add {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Record staged changes as a commit on the current agent's lane
    Commit {
        #[arg(short = 'm', long)]
        message: String,
    },
    /// Show commit history for the current agent's lane
    Log {
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Show changes (index vs working tree, or --staged for HEAD vs index)
    Diff {
        #[arg(long)]
        staged: bool,
    },
    /// Print the type and contents of an object
    CatFile { oid: String },
    /// Manage agents — independent concurrent workers in one folder
    Agent {
        #[command(subcommand)]
        action: AgentCmd,
    },
    /// Manage lanes — independent lines of work (JAG's branches)
    Lane {
        #[command(subcommand)]
        action: LaneCmd,
    },
    /// Materialize a lane into the shared working tree for this agent
    Checkout { lane: String },
    /// Merge concurrent lanes, auto-resolving non-overlapping work
    #[command(visible_alias = "merge")]
    Reconcile {
        /// Target lane to reconcile into
        #[arg(long, default_value = "main")]
        into: String,
        #[arg(short = 'm', long)]
        message: Option<String>,
        /// Apply without confirming (skip the interactive preview prompt)
        #[arg(short = 'y', long)]
        yes: bool,
        /// Source lanes (default: every lane except the target)
        lanes: Vec<String>,
    },
    /// Show paths multiple agents/lanes have changed (merge hot-spots)
    Contention,
    /// Manage remotes (named jag server URLs)
    Remote {
        #[command(subcommand)]
        action: RemoteCmd,
    },
    /// Download (clone) a repository from a jag server URL
    #[command(name = "dl", visible_alias = "clone")]
    Dl {
        /// Server URL, e.g. http://127.0.0.1:9418
        url: String,
        /// Destination directory (default: jag-clone)
        dir: Option<String>,
    },
    /// Download objects and lanes from a remote
    Fetch {
        #[arg(default_value = "origin")]
        remote: String,
    },
    /// Save everything: stage all changes, commit, and push to the remote
    Push {
        /// Commit message (default: an auto timestamped checkpoint)
        #[arg(short = 'm', long)]
        message: Option<String>,
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Commit locally only; don't push
        #[arg(long)]
        local: bool,
    },
    /// Undo the last change on the current lane (restores the working tree)
    Undo,
    /// Reapply the last undone change
    Redo,
    /// Serve this repository over HTTP for clone/fetch/push
    Serve {
        #[arg(long, default_value = "127.0.0.1:9418")]
        addr: String,
        #[arg(long, default_value_t = 4)]
        threads: usize,
    },
}

#[derive(Subcommand)]
enum RemoteCmd {
    /// Add or update a remote
    Add { name: String, url: String },
    /// List remotes
    List,
    /// Remove a remote
    Remove { name: String },
}

#[derive(Subcommand)]
enum AgentCmd {
    /// Start a new agent with its own index, HEAD, and lane
    Start {
        name: String,
        /// Lane to place the agent on (default: a new lane named after it)
        #[arg(long)]
        lane: Option<String>,
        /// Base lane to fork from when the lane is new
        #[arg(long = "from", default_value = "main")]
        base: String,
    },
    /// List agents in this repository
    List,
    /// Print the current agent
    Who,
    /// Set the default agent for this repository
    Use { name: String },
}

#[derive(Subcommand)]
enum LaneCmd {
    /// List lanes and their tips
    List,
    /// Create a new lane from a base lane's tip
    New {
        name: String,
        #[arg(long = "from", default_value = "main")]
        base: String,
    },
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("jag: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    // init and clone are the only commands that run without an existing repo.
    match &cli.command {
        Command::Init => return commands::init(std::env::current_dir()?),
        Command::Dl { url, dir } => return commands::clone(url, dir.clone()),
        _ => {}
    }

    let repo = Repo::find(None, cli.agent.clone())?;
    match cli.command {
        Command::Init => unreachable!(),
        Command::Dl { .. } => unreachable!(),
        Command::Status => commands::status(&repo),
        Command::Add { paths } => commands::add(&repo, &paths),
        Command::Commit { message } => commands::commit(&repo, &message),
        Command::Log { limit } => commands::log(&repo, limit),
        Command::Diff { staged } => commands::diff(&repo, staged),
        Command::CatFile { oid } => commands::cat_file(&repo, &oid),
        Command::Agent { action } => match action {
            AgentCmd::Start { name, lane, base } => {
                commands::agent_start(&repo, &name, lane, &base)
            }
            AgentCmd::List => commands::agent_list(&repo),
            AgentCmd::Who => commands::agent_who(&repo),
            AgentCmd::Use { name } => commands::agent_use(&repo, &name),
        },
        Command::Lane { action } => match action {
            LaneCmd::List => commands::lane_list(&repo),
            LaneCmd::New { name, base } => commands::lane_new(&repo, &name, &base),
        },
        Command::Checkout { lane } => commands::checkout(&repo, &lane),
        Command::Reconcile {
            into,
            message,
            yes,
            lanes,
        } => commands::reconcile(&repo, &into, message, lanes, yes),
        Command::Contention => commands::contention(&repo),
        Command::Remote { action } => match action {
            RemoteCmd::Add { name, url } => commands::remote_add(&repo, &name, &url),
            RemoteCmd::List => commands::remote_list(&repo),
            RemoteCmd::Remove { name } => commands::remote_remove(&repo, &name),
        },
        Command::Fetch { remote } => commands::fetch(&repo, &remote),
        Command::Push {
            message,
            remote,
            local,
        } => commands::push_all(&repo, message, &remote, local),
        Command::Undo => commands::undo(&repo),
        Command::Redo => commands::redo(&repo),
        Command::Serve { addr, threads } => commands::serve(&repo, &addr, threads),
    }
}
