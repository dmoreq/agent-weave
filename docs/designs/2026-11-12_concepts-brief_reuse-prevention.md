# Concepts Brief — Prevent-Reinvention v2

- Date: 2026-11-12
- Status: Approved (Phase 1 spec; owner approval 2026-11-12)
- Source artifacts: `.tmp/prevent-reinvention/design-r4-b.md`, `.tmp/prevent-reinvention/audit.md`
- Approved spec: `docs/designs/2026-11-12_reuse-prevention.md`

## Problem

Agents still reinvent helpers because the current prevention loop is half a loop: `eg search` is post-hoc lexical search and `eg reuse` only sees code that already exists. The design adds a pre-finalize proposal mode, precision guards, provisional confidence, staleness awareness, an opt-in allowlist, opt-in coverage, judge hardening, and an oracle protocol.

## Consumers

- Coding agents using CLI or MCP.
- Human reviewers using `eg reuse` and the dashboard.
- Build hints emitted during incremental `eg build`.
- Maintainers of the reuse/judge/memory subsystems.

## Hard constraints

- Local-first, zero network; no new schema, dependency, daemon, or hook.
- CLI and MCP byte parity through `src/service.rs`.
- Module target 200–500 LOC; MSRV 1.90; zero unsafe.
- Current D1–D4 scan remains the default until a gate approves a new tier.
- No runtime Python; offline scripts live under `tests/scripts/` or `bench/`.

## Options considered

### A. Phase 1 scan-only first

- Pros: smallest shippable slice; closes 5 P0 + 28 P1 Phase 1 blockers; ~700–1,100 LOC.
- Cons: leaves proposal, allowlist, coverage, and judge unbuilt; does not satisfy the owner's full-design request.

### B. Revision-only spec first

- Pros: exactly follows the audit verdict; no code until all 8 P0 + 50 P1 close.
- Cons: does not deliver the full design; defers implementation by a full review cycle.

### C. Full design with Phase 0 spec remediation (chosen)

- Pros: preserves the owner's full scope; closes the audit findings before each phase; keeps all phases.
- Cons: large; requires Gate 1 and Gate 2 before code; oracle and judge remain schedule risks.
- Decision: full design, but spec remediation is mandatory. No phase ships with an open P0.

## Strongest objections from the final audit

- 8 P0 and 50 P1 unresolved; verdict `needs-one-more-revision`.
- Phase 1 “Medium” is uncalibrated; don't print it as Medium.
- Composition-call has two contradictory dispositions.
- Oracle arithmetic, power, and denominators do not close.
- Phase 1 performance gate has no fixed harness.
- Judge and allowlist trust boundaries are not closed.
- Scan path/truncation can silently mislead.

## Gate plan

1. Gate 1: owner approves the dated spec and the five decisions in §Gate 1 Decisions.
2. Gate 2: five-lens review of `FeatureRange` + `DetailedRange`; owner resolves findings.
3. Phase 0: oracle, labels, SAP, perf harness; no product code.
4. Phase 1a/1b, then Phase 2–5 only after their memo gates.
