# Architecture decision records

One ADR per architectural choice ([PLAN §2.11](../PLAN.md#211-repository-layout-languages-key-dependencies-coding-standards)). Numbers run in order and are never reused. An accepted ADR is not rewritten: a new ADR supersedes it, and both link to each other. Changing a house rule, adding a network host or adding an elevated component needs an ADR and the owner's approval ([PLAN §2.10](../PLAN.md#210-safety-privacy-and-security-rules), [CLAUDE.md](../../CLAUDE.md)).

| ADR | Title | Status |
|---|---|---|
| [0001](0001-architecture-and-stack.md) | Architecture and stack | Accepted, 2026-10-21 |

## Template

Save as `docs/adr/NNNN-short-title.md` and add a row above.

```markdown
# ADR-NNNN: Title

- **Status:** Proposed | Accepted | Superseded by ADR-NNNN
- **Date:** YYYY-MM-DD
- **Decided by:** the owner (decision or question number) | Claude Code, within the plan
- **Sources:** plan sections, research, spike reports

## Context
What forces the decision: constraints, measurements, options.

## Decision
What we do, precisely enough to check in review.

## Consequences
What gets better, what it costs, what we must now watch.
```
