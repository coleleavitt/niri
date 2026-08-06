---
name: feature-implementation-with-config-and-docs
description: Workflow command scaffold for feature-implementation-with-config-and-docs in niri.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /feature-implementation-with-config-and-docs

Use this workflow when working on **feature-implementation-with-config-and-docs** in `niri`.

## Goal

Implements a new feature that requires changes to configuration, code, and documentation.

## Common Files

- `niri-config/src/*.rs`
- `src/**/*.rs`
- `docs/wiki/*.md`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Add or update configuration code in niri-config/src/*.rs
- Update or add implementation in src/**/*.rs
- Update documentation in docs/wiki/*.md

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.