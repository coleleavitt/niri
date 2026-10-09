---
name: input-feature-or-bugfix
description: Workflow command scaffold for input-feature-or-bugfix in niri.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /input-feature-or-bugfix

Use this workflow when working on **input-feature-or-bugfix** in `niri`.

## Goal

Implements or fixes input-related features, often in response to hardware or user interaction needs.

## Common Files

- `src/input/mod.rs`
- `niri-config/src/*.rs`
- `src/ui/*.rs`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Modify src/input/mod.rs to implement or fix input logic
- Optionally update related configuration in niri-config/src/*.rs
- Optionally update UI overlays in src/ui/*.rs

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.