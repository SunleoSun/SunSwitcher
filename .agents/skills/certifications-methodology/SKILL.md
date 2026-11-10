---
name: certifications-methodology
description: Design, review, or migrate semantic certifications. Use when deciding proof ownership, evidence, Domain coverage, Observations, negative paths, statistical or anomaly proof, or how existing tests should change.
---

# Semantic certification methodology

For proof code read certifications-api, for generated inputs read domain-api, and for production Observation capture/query/decode read observations-api.

## Core model

**Certify semantic behavior, not implementation shape.**

A certification is executable proof of one semantic contract on the real production path. Start from the **semantic owner**: the component that owns the domain decision, invariant, transformation, validation rule, side effect, or producer-consumer boundary.


Unit, integration, and end-to-end are scales, not different philosophies. A component deserves its own certification only when it owns independent semantics.

Prefer one authoritative proof over several tests that restate the same contract. Do not certify temporary containers, helper call counts, loop shape, cache representation, or other implementation details unless the detail is itself promised behavior such as exactly-once persistence or bounded work.

## Evidence priority

Use the most authoritative evidence that can prove the contract:

1. canonical returned result or persisted artifact;
2. promised external effect/state;
3. typed production Observation;
4. independent reference/oracle;
5. statistical/anomaly evidence.

Use **certification only** when canonical input, result, artifact, or promised state exposes enough evidence.

Use **certification + Observations** when a proof-critical semantic fact otherwise disappears inside the production path, such as a meaningful owner boundary transfer, domain-significant decision, causal identity, checkpoint choice, or provenance. Emit the minimum evidence needed; do not duplicate an already canonical result, artifact, or effect.

Observation roles:
- **Boundary Observation**: meaningful transfer between owners or stages; default for cross-owner correspondence.
- **Decision Observation**: domain-significant accept/reject/select/suppress branch that changes downstream behavior.
- **Diagnostic Observation**: metric or state needed by a specific certification or diagnostic workflow.

Use Observations without certification for diagnostics or telemetry when no acceptance proof is being made. Observations record facts; certification owns expectations and pass/fail policy.

## Domain as input-proof strategy

Use a Domain when the owner promises behavior over a **family of inputs** that can be faithfully described by the Domain DSL. Domain describes where the contract must hold; checks describe what must hold after execution.

Keep concrete data_source cases when a specific case has independent regression, boundary, fixture, external-data, or diagnostic meaning. Prefer exact fixtures when the contract is inherently example-specific, expensive, externally defined, or cannot be faithfully expressed by the DSL.

Generate invalid states only at the semantic owner that owns validation or rejection of those states. Downstream owners receiving already validated or typed inputs should be certified over reachable valid states, not impossible raw states.

When migrating a single-case certification:
- **keep the concrete case** when it has independent semantic or diagnostic value;
- **Concrete case + Domain** when that case should remain and the contract is also family-wide;
- **Domain only** when the old input was arbitrary and the contract is family-wide;
- **remove the certification** only when it owns no independent semantic contract or another certification already proves the same contract without losing diagnostic value.

Do not rewrite a good semantic certification solely to use newer DSL.

## Owner and composition proof

An **owner certification** proves semantics owned by one semantic owner. It may combine deterministic examples, Domain-generated cases, regressions, oracle checks, statistical/anomaly checks, and metamorphic checks; proof technique does not create a new owner.

A **composition certification** proves a semantic contract across multiple owners or an explicit producer-consumer boundary. Prove only the cross-owner promise instead of re-testing owner-local details.

## Proof dimensions

For each semantic contract, include only dimensions that materially strengthen the proof: canonical outcome, what **must happen** and **must never happen**, producer-consumer correspondence, typed identity, causal or temporal alignment, failure/rejection semantics, natural semantic groups, statistical/anomaly behavior, independent oracle, or metamorphic/control scenarios.

**Adversarially validate the proof boundary itself.** When a mechanism decides what evidence or obligations are admissible, reachable, feasible, complete, covered, matched, or evaluable, pair a representative state that must contribute with the closest state that must not. Check the evidence set and proof obligations themselves: invalid inclusion, required omission, or impossible obligations can falsify a proof even when every local check is correct. Prefer positive/negative controls that distinguish plausible false PASS and false FAIL.

### Failure, identity, and causality

Failure-path proof is stronger than checking only Result::is_err. When semantically promised, prove the rejection owner/category and absence of forbidden downstream events, artifacts, state changes, duplicate actions, or other side effects.

Join evidence by explicit domain identity, never incidental record order, Vec index, sink order, or formatted field names unless that ordering is contractual.

Value agreement does not prove causality. Certify row, timestamp, horizon, and decision-entry offsets when known. Use lag analysis when diagnosing an unknown stale, off-by-one, or future dependency.

### Hard, statistical, anomaly, and evaluation failure

A **hard invariant** is logically incompatible with a correct system when violated.

A **statistical expectation** is scenario-owned acceptance policy such as correlation, slope, grouped deviation, outlier count, or lag. It is not a warning and does not belong in production Observation metadata.

First prove exact invariants whenever the semantic contract permits exact proof. **Statistical or anomaly evidence is required when correctness also has a measurable distributional, relational, temporal, or aggregate property whose violation is a reachable defect not already ruled out by exact checks.**

If correct behavior permits no domain-defined anomalies or deviations, certify that explicitly. Prefer a direct hard invariant when the forbidden deviation can be expressed exactly; use zero-tolerance anomaly acceptance such as AtMostAnomalies(0) only when the detector itself matches the semantic definition of deviation.

Do not add a statistical metric that is logically implied by stronger exact checks. Statistical bounds and anomaly limits must come from the semantic contract, an independent reference, or a justified control scenario, not from the observed production result itself.

Global aggregates can hide broken folds, targets, directions, episodes, or runs. Check completeness of natural semantic groups before per-group expectations.

An **evaluation error** means the proof could not be validly evaluated: incomplete capture, decode failure, undefined required metric, invalid bounds, incomplete correspondence, or invalid configuration. Proof fails closed.

### Oracle, metamorphic, and control evidence

For optimized or mathematically complex behavior, prefer a simple independent oracle that uses a meaningfully different path. Do not compute expected values with the same production helper.

When exact expected output is awkward, use metamorphic relations with known semantic consequences such as scaling, sign reversal, partition invariance, or shuffled-association collapse.

Strengthen statistical evidence with a null/control scenario when it distinguishes the intended relationship from an accidentally high metric.

## Execution and completion

Certification names should state the semantic promise, not the implementation mechanism. For large composition proofs, group checks by semantic pipeline stages so failures point to the responsible owner or boundary without re-testing owner-local details.

### Per-case and aggregate checks

Ordinary runner.check(...) proves each successful positive execution. Use runner.check_aggregate(...) once after exploration whenever the contract spans multiple executions—not only profiling: cross-case invariants, grouped distributions, scaling relationships, aggregate correspondence, or cross-case anomalies. Aggregate evidence preserves distinct execution identity even for equal inputs; correctly rejected generated negatives are excluded, while invalid positive execution evidence makes required aggregate access fail closed. Use the cases root and PreparedSource analysis described by certifications-api.

### Profiling

Profile when performance or resource behavior is promised semantics or a known regression risk whose violation is a real defect; incidental tuning and non-contractual wall-clock thresholds remain benchmark/diagnostic work. Apply the general hard/statistical/anomaly policy above unchanged. For cross-case profiling use the aggregate mechanism above over comparable span evidence: Domain owns only the input family; span runtime owns measurement emission; evidence owns pairing/duration/completeness/provenance; PreparedSource owns generic metrics; certifications own acceptance. Use an evidence set that actually contains comparable samples. proc_ns preserves available monotonic precision, not nanosecond hardware resolution; avoid unjustified brittle wall-clock thresholds for extremely short operations and hidden overhead subtraction.

When production code changes, preserve expectations that encode independent intent. Do not make a broken certification green by deriving expected values from the implementation under test or rewriting the contract to match the defect.

Run the real production path. Prefer deterministic fixtures; when production randomness matters, use deterministic seeds or invariants independent of the sample.

Collect independent checks so one execution yields a useful bounded semantic report instead of stopping at the first unrelated failure. Dependent analysis that cannot be evaluated should become an evaluation error; unrelated checks should still run.

Instrumentation must observe production semantics, never become a second behavior owner or a test-only execution path.

A certification is complete when its smallest sufficient evidence set proves the intended semantic contract, realistic negative or failure behavior is covered at the owning boundary, required statistical/anomaly properties are covered where applicable, and the proof remains valid across implementation refactors that preserve semantics.
