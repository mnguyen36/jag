# JAG — Just Another Git

**The Agent Era replacement for Git.**

JAG is a version control system designed from the ground up for multi-agent workflows. Where Git was built for individual developers working sequentially, JAG treats concurrent agentic work as the default — multiple AI agents proposing, reviewing, and reconciling changes in parallel.

## Why JAG?

Git's model was designed for humans committing discrete, intentional snapshots. In agentic workflows:

- Multiple agents work concurrently on overlapping areas of a codebase
- Revisions carry intent and context, not just diffs
- Review, reconciliation, and checkpointing need to be first-class operations — not conventions layered on top of branches and commits

JAG provides agent-native primitives that make these workflows natural.

## Core Concepts

| Concept | Description |
|---|---|
| **Proposal** | An agent's candidate change set, including intent and context |
| **Checkpoint** | A verified, reconciled snapshot of shared state |
| **Reconcile** | Merging concurrent proposals with awareness of agent intent |
| **Review** | Structured agent-to-agent (or human-to-agent) feedback on a proposal |

## Status

Early design and implementation phase.
