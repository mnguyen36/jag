//! JAG — Just Another Git. Agent-first version control.
//!
//! CLI surface and dispatch. Every subcommand resolves to a handler in
//! `commands`. The global `--agent` flag (and the `JAG_AGENT` env var) select
//! which concurrent agent the command acts as inside the shared folder.

mod agent;
mod commands;
mod commit;
mod index;
mod objects;
mod reconcile;
mod refs;
mod repo;
mod status;
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
    Reconcile {
        /// Target lane to reconcile into
        #[arg(long, default_value = "main")]
        into: String,
        #[arg(short = 'm', long)]
        message: Option<String>,
        /// Source lanes (default: every lane except the target)
        lanes: Vec<String>,
    },
    /// Show paths multiple agents/lanes have changed (merge hot-spots)
    Contention,
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
    // init is the only command that runs without an existing repository.
    if let Command::Init = cli.command {
        return commands::init(std::env::current_dir()?);
    }

    let repo = Repo::find(None, cli.agent.clone())?;
    match cli.command {
        Command::Init => unreachable!(),
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
            lanes,
        } => commands::reconcile(&repo, &into, message, lanes),
        Command::Contention => commands::contention(&repo),
    }
}
