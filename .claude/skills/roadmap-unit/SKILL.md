---
name: roadmap-unit
description: Implement a roadmap unit (D24, P13, C15, F5, S3, …) end to end — read the unit, branch, implement, verify, and open the PR. Use whenever the user asks to implement, close, or open a PR for a roadmap unit by its ID.
---

# Implementing a roadmap unit

`$ARGUMENTS` names the unit (e.g. `D24`, `P10-perf`, `C8 stage 3`).

## 1. Read the unit before touching code

Units are defined in `docs/roadmap-2026-07-work-units.md` (index and phasing in
`docs/roadmap-2026-07.md`). Find the unit and read it in full, including:

- its **Depends on / Blocks** lines — a unit whose dependency has not landed is usually
  not ready to start, and units that share a doc are sequenced to avoid conflicts;
- any `[DECISION]` marker — those units require a decision from the maintainer before
  implementation, not just code;
- the findings docs it cites (`docs/findings-*.md`).

If the unit's charter and the code disagree, say so before implementing — a stale unit
description is itself a finding.

## 2. Branch

Never commit to `main`. Branch from an up-to-date `main`:

```
git checkout main && git pull
git checkout -b <type>/<unit-id-lowercase>-<slug>
```

`<type>` matches the nature of the work: `feat`, `fix`, `perf`, `refactor`, `docs`,
`test`, `chore`. Examples from this repo:

```
feat/d21-correction-surface-angular-resolution
fix/p13-pre-gate
perf/p10-perf-mode-integrator
docs/d5-design-docs-truth-sweep
test/c15-option3-example-schema-validation
```

**One roadmap unit per PR.** If the work uncovers a second defect, file it as a new unit
in the work-units doc rather than widening the branch — unless the two are genuinely
inseparable, in which case both IDs go in the commit subject.

## 3. Implement

Follow the path-scoped rules in `.claude/rules/` for whatever you touch. Two that bite
here specifically:

- A change to served physics results bumps `PHYSICS_MODEL_VERSION`
  (`antenna-core/src/model/mod.rs`).
- A change to the artifact bumps the container axis, the schema axis, or both — see
  `.claude/rules/calibration.md` for which case is which.

If the unit produces a non-obvious finding (a measurement that contradicts a prior
explanation, a guard that had stopped guarding), write it up as
`docs/findings-<YYYY-MM-DD>-<topic>.md` and cite it from the code and docs that depend
on it. That is how this repo keeps its invariants explicable.

## 4. Verify before the PR — actually run it

```
./scripts/check.sh
```

This is the single entrypoint and runs exactly what CI runs, including the two
package-scoped checks no workspace command can perform. Report the real output. If
something fails, fix it or say plainly that it failed — never describe an unrun check
as passing.

## 5. Commit and open the PR

Commit subject:

```
<type>(<UNIT>): <imperative summary>
```

e.g. `fix(P14): scale the Miller seed offset with the turning-point width`, or
`fix(D26,D27): serve a negative-cone calibration; close the dep guard that failed open`.
The `(#NN)` suffix seen in `git log` is added by the squash merge — do not write it by
hand.

The body should say what changed and **why the old behaviour was wrong**, with the
measured numbers where there are any; that is the house style, and those bodies are
what later findings docs cite.

Then:

```
gh pr create --title "<same as commit subject>" --body "..."
```

Confirm with the user before pushing or opening the PR unless they have already asked
for it in this session.
