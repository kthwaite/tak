# RFC 0005: Work-Loop UX Friction Reduction Bundle

- **Status:** Draft
- **Date:** 2026-02-10
- **Author:** spry-otter-6a99
- **Related epic:** [#6609522224539167459] Reduce tak dogfooding friction in work-loop UX and coordination ergonomics
- **Related tasks:**
  - [#15906214680224180686] Draft RFC for work-loop UX friction reduction bundle
  - [#10996821707303780651] Deterministic `tak work` entry snapshot + safe-resume anti-thrash
  - [#10543929956773346429] `tak work done` finish+cleanup+pause helper
  - [#7375938272634783085] `tak takeover` stale in-progress ownership transfer
  - [#8246513620076065537] Scoped `tak verify` + reservation-aware blocker diagnostics
  - [#3967726589393726993] Blackboard status-note supersede/auto-close hygiene
  - [#12865599711298777539] Low-noise personal/time-window filters (blackboard/inbox/feed)
  - [#2296521817245445668] Phase-2 canonical task-ID display rollout
- **Related docs:**
  - [`docs/rfcs/0004-cooperative-blocker-workflow.md`](./0004-cooperative-blocker-workflow.md)
  - [`docs/how/channel-contract.md`](../how/channel-contract.md)
  - [`docs/how/isolated-verification.md`](../how/isolated-verification.md)

---

## Summary

This RFC proposes a staged bundle of CLI-first UX improvements to reduce friction in active multi-agent work loops while preserving Tak safety guarantees.

The bundle targets common pain points observed during dogfooding:

1. stale in-progress ownership requires manual handoff choreography,
2. finishing one loop unit involves repeated cleanup commands,
3. repeated `tak work` entry/status calls can thrash claim behavior,
4. verification in shared trees needs explicit scoped diagnostics,
5. coordination views (blackboard/inbox/feed) can be noisy.

The proposal keeps core correctness in `tak` CLI and persisted state; extension UX layers remain optional accelerators.

---

## Problem statement

Current flows are correct but operationally heavy in busy lanes:

- **Takeover friction:** reclaiming stale in-progress tasks is possible but not first-class.
- **Closeout friction:** finishing current work often requires separate finish/release/stop/status commands.
- **Re-entry friction:** repeated `tak work` invocations can produce ambiguous “what should I do next?” states and rapid reclaim loops after handoff/block.
- **Verify friction:** isolated verification decisions are hard to scope quickly under reservation overlap.
- **Signal-to-noise friction:** operators spend time scanning unrelated updates in blackboard/inbox/feed streams.

---

## Goals

1. Reduce command ceremony for common work-loop transitions.
2. Preserve existing lifecycle safety and coordination invariants.
3. Keep machine-readable output compatibility explicit.
4. Make blocker and verify decisions more deterministic and auditable.
5. Improve coordination signal quality without reducing traceability.

## Non-goals

1. Replacing `.tak/tasks/*.json` as source-of-truth.
2. Introducing a centralized scheduler/daemon.
3. Forcing pi extension runtime for correctness.
4. Breaking current JSON output contracts without explicit migration/versioning.
5. Shipping all changes in one release train.

---

## Proposed command-surface bundle

## A) `tak takeover TASK_ID` (stale in-progress ownership transfer)

### Semantics

- Reassign a stale in-progress task to the requesting agent through one auditable command.
- Emit previous owner, decision path, and resulting state in output.

### Safety constraints

- Reject when current owner is active/recent unless explicit override conditions are met.
- Preserve lifecycle/history logging and assignment invariants.
- Keep race safety under concurrent takeover attempts.

### Compatibility strategy

- Additive command; no change to existing `start`/`handoff` behavior.
- Existing scripts continue to work unchanged.

---

## B) `tak work done` (finish + cleanup + optional pause)

### Semantics

- One command to finish the current loop unit, release owned reservations, and optionally pause/deactivate auto-claim.
- Return structured sub-action report (finish result, reservation release result, loop active flag).

### Safety constraints

- Must preserve current lifecycle transition guards (`in_progress -> done`).
- Cleanup actions should be idempotent and auditable.

### Compatibility strategy

- Additive helper over existing primitives (`finish`, `mesh release`, `work stop/status`).
- Does not remove or redefine existing commands.

---

## C) Deterministic `tak work` snapshot + safe-resume anti-thrash

### Semantics

- `tak work` and `tak work status` return deterministic snapshots with explicit next action hints.
- Resume logic avoids immediate reclaim churn after handoff/block unless blocker predicates changed (or explicit override).

### Safety constraints

- Preserve claim ordering policy and lock safety.
- Never hide true blocker/availability state.

### Compatibility strategy

- Keep existing command entrypoints.
- Tighten behavior contract and output fields in a backward-compatible additive manner where possible.

---

## D) Scoped `tak verify` + reservation-aware diagnostics

### Semantics

- Add explicit verify scope selectors and report effective scope.
- On reservation overlap, emit blocker owner/path/reason/age plus remediation hints.

### Safety constraints

- Reuse canonical reservation conflict predicate for overlap decisions.
- Maintain verification-result storage compatibility.

### Compatibility strategy

- Existing unscoped verify path remains valid.
- Scoped mode is additive and explicit.

---

## E) Blackboard supersede/auto-close note hygiene

### Semantics

- Support low-friction supersede/auto-close flows for status/completion notes.
- Preserve linkage/auditability between old and replacement notes.

### Safety constraints

- No silent destructive mutation; closure reason and actor required.
- Query/list semantics remain deterministic.

### Compatibility strategy

- Existing free-form notes remain supported.
- Structured hygiene semantics are additive.

---

## F) Low-noise filters for blackboard/inbox/feed

### Semantics

- Personal and time-window filters across `tak blackboard list`, `tak mesh inbox`, and `tak mesh feed`.
- Default views should suppress low-value noise (for example heartbeat chatter) unless explicitly requested.

### Safety constraints

- Filtering must not alter underlying stored events.
- Ordering and cursor/ack semantics must remain explicit.

### Compatibility strategy

- Additive filter flags; existing default invocation behavior preserved unless RFC-approved default changes are staged.

---

## G) Phase-2 canonical task-ID display defaults (deferred lane)

This remains a follow-on phase after higher-impact UX lanes close.

### Constraints

- Human-facing surfaces can migrate first.
- Machine-facing compatibility (especially JSON consumers) must be explicitly protected.

---

## Cross-cutting safety contract

All lanes must preserve:

1. lifecycle state-machine integrity,
2. dependency/blocked derivation correctness,
3. lock-safe claim/takeover behavior,
4. structured history/audit trails,
5. explicit coordination channel roles.

---

## Rollout order (strict chain)

1. **RFC + contract lock** ([#15906214680224180686])
2. **Work snapshot/safe-resume** ([#10996821707303780651])
3. **`tak work done` helper** ([#10543929956773346429])
4. **`tak takeover`** ([#7375938272634783085])
5. **Scoped `tak verify` diagnostics** ([#8246513620076065537])
6. **Blackboard supersede hygiene** ([#3967726589393726993])
7. **Low-noise coordination filters** ([#12865599711298777539])
8. **Phase-2 ID display rollout** ([#2296521817245445668])

This order matches dependency edges and minimizes semantic churn.

---

## Out-of-scope for this bundle

- Full rewrite of work-loop strategy models.
- New persistence backend or daemonized orchestration.
- Breaking task-ID input compatibility.
- Non-CLI-only correctness rules that require extension runtimes.

---

## Success criteria

1. Reduced average command count for “finish current unit and clean up”.
2. Faster, lower-friction stale-owner reconciliation.
3. Fewer immediate re-entry reclaim loops after blocker/handoff transitions.
4. Faster diagnosis of isolated verify blockers in shared trees.
5. Lower coordination scanning overhead while preserving auditability.

---

## Open questions

1. Should `tak takeover` default guardrail be TTL-based, heartbeat-based, or both?
2. Should `tak work done` support explicit handoff/cancel submodes in the same command or remain finish-only initially?
3. How much default noise suppression is acceptable before users must opt in to full streams?
4. Which output additions require explicit versioning for strict machine consumers?

---

## Initial recommendation

Adopt this bundle as a strict staged roadmap with additive command surfaces and explicit compatibility guardrails.

Delivering the first five lanes provides the largest operator impact while preserving Tak’s existing safety and storage invariants.
