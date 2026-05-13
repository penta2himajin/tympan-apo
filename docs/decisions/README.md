# Architecture Decision Records

This directory contains the project's settled architectural decisions.

## Why ADRs

`CLAUDE.md` § SSOT precedence lists ADRs as a top-tier source of truth,
ranked above handoff issues. When the same question recurs across
sessions or PRs, the answer belongs here so it does not get rewritten
each time.

## Lifecycle

- ADRs are numbered sequentially. Filenames follow
  `NNNN-kebab-case-summary.md`.
- An ADR is **Accepted** when merged. Status may later move to
  **Superseded by ADR NNNN** if a follow-up decision overturns it.
  The superseded record stays in the tree for historical context — it
  is not deleted.
- ADRs are short. A single page of context, decision, consequences,
  and (where useful) a documented reversal trigger.

## Index

| ID | Title | Status |
|---:|---|---|
| [0001](0001-ci-verification-strategy.md) | CI verification strategy and scope boundary | Accepted |
