# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**JAG** (Just Another Git) is an Agent Era replacement for Git, purpose-built for multi-agent workflows. Its core differentiator is treating concurrent, agentic work as a first-class citizen — where multiple AI agents can independently author, revise, and reconcile changes without the friction of traditional branch/merge models.

### Core Design Goals

- **Multi-agent concurrency**: Multiple agents can work on overlapping change sets simultaneously; the system coordinates rather than serializes.
- **Agentic revision workflows**: Revision history is structured to capture *intent* and *context* alongside diffs, so agents can understand not just what changed but why.
- **Agent-native primitives**: Operations like "propose", "review", "reconcile", and "checkpoint" are first-class — not bolted-on conventions over commits and branches.
