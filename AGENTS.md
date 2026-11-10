# SunSwitcher Project Instructions

SunSwitcher is a Windows text-input utility written in Rust. Its production paths cover global keyboard input, correction and keyboard-layout switching, autocomplete, persistence, clipboard behavior, and native UI/tray integration.

Current production code, schemas, persisted formats, generated contracts, and runtime artifacts describe the current system state. Explicit task requirements define the requested target state.

## Project invariants

- Preserve unrelated user changes. Do not reset, discard, rewrite history, or perform unrelated cleanup.
- Prefer the simplest complete design. Do not add compatibility layers, fallback wrappers, metadata, indirection, configuration, or extensibility without a concrete semantic need.
- Fail closed on promised effects: process or tool success is insufficient when the requested artifact, state, schema, parameters, or downstream effect can be verified.
- Preserve canonical production paths. Optimized, cached, snapshot-backed, asynchronous, or UI-projected implementations may differ internally but must preserve the same semantic result, identities, ordering/lifecycle contract, and failure behavior.
- Keyboard/input hot paths must not acquire SQLite I/O, blocking filesystem/process work, or another mutable authority when the required state can be projected into validated runtime memory.

## Single Source of Behavior + Typed Contracts

Treat Single Source of Behavior and Typed Contracts as one rule.

- Give every domain fact, invariant, routing rule, lifecycle rule, retry rule, threshold rule, scoring rule, validation rule, persisted meaning, and UI/runtime identity one authoritative semantic owner and one canonical implementation path.
- Consumers, caches, projections, summaries, indexes, UI views, and adapters may derive from the owner but must not reimplement, reinterpret, shadow, or synchronize copies of the same behavior.
- Encode semantic distinctions in types. Prefer enums, wrappers, dedicated IDs, structs, tagged values, validated types, and explicit units over booleans, magic strings, naming conventions, text matching, or duplicated validation.
- When producer and consumer behavior diverge, fix the semantic owner and make consumers depend on its canonical typed contract.
- A semantic owner is a production source module/file, not a feature, task, tool family, function cluster, or regression inside that file. If genuinely independent semantics need independent certification ownership, split the production code into separate owner modules/files first rather than inventing feature-level owners.
- Across runtime/UI/Windows/persistence boundaries, validate at the correct boundary and preserve meaning through typed or versioned contracts.
- A semantic change is incomplete until affected producers, consumers, persisted contracts, caches, projections, runtime artifacts, and bridges have been checked for stale or dropped meaning.

## Authority and precedence

When sources conflict, use this order for the question each source owns:

1. explicit user requirements and decisions for the requested target;
2. authoritative target specifications, schemas, persisted formats, platform contracts, and external APIs;
3. current source, generated contracts, runtime facts, and data artifacts for the current implementation;
4. durable project memory and maintained documentation;
5. historical notes, examples, experiments, and legacy behavior.

Do not let the current implementation silently override an explicit target.
Do not let stale memory or documentation override current factual state.
Do not preserve historical behavior unless compatibility is explicitly required.
If equally authoritative requirements genuinely conflict in a way that changes correctness, request clarification instead of choosing silently.

## Scope and autonomy

- Complete the assigned in-scope task rather than stopping at analysis, a plan, pseudo-code, or a first partial success when implementation or artifact change was requested.
- Make routine, reversible, in-scope engineering decisions autonomously from available evidence.
- Ask only when missing or conflicting information can materially change correctness, violate a hard requirement, expand scope, or cause destructive, irreversible, security-sensitive, externally consequential, or otherwise high-impact action.
- Do not expand scope merely because adjacent cleanup or modernization is available.
- For read-only review, audit, or investigation requests, do not mutate the workspace unless the user explicitly switches to implementation.

## Architecture and design discussion

- Optimize architecture/design explanations for understanding without reducing technical depth. Build the explanation progressively: establish the simplest useful mental model first, then add distinctions, types, contracts, edge cases, and future implications only as they become relevant.
- Do not force architecture answers into a fixed template or RFC structure. Choose the structure that best fits the question and keep it as small as the current decision allows.
- When introducing a term, type, or abstraction that is not already established in the conversation, briefly explain what it means and why it is needed before relying on it. Do not repeatedly re-explain established terminology.
- Prefer concrete SunSwitcher semantics when they make a generic abstraction easier to understand. Explain the concrete meaning first when that is clearer, then name the reusable abstraction.
- Keep architectural levels distinct when possible: input ownership, correction/completion semantics, Windows execution, persistence/runtime snapshots, and UI/tray projection should not be mixed merely because they are related.
- Clearly distinguish current facts and prior user decisions from new proposals, alternatives, open questions, and deliberately deferred work whenever confusing them could change the design.
- State the recommendation and the material tradeoff in ordinary language before expanding into detailed types or APIs. Avoid chains of unexplained technical nouns when a simpler explanation can establish the same concept first.
- Do not enumerate or design future extensions while the current decision is still being established unless those extensions materially constrain the current architecture.
- Respect existing abstraction boundaries during investigation and design. Internal implementation may be inspected to recover semantics, but do not turn internal modules into new dependencies when an existing public typed contract is the intended boundary.
- If an integration layer or production path does not yet exist, say so explicitly. Design and certify the independent core against synthetic/control spaces or existing public contracts rather than assuming unavailable pipeline capabilities.
- Match the user's language for conceptual explanation while preserving exact identifiers, types, filenames, and established technical terminology where translation would reduce precision.

## Workspace, tools, and transactions

- Use configured SunMCP tools for repository reading, search, edits, validation, and Git inspection. Do not substitute ad-hoc shell editing or built-in file mutation when SunMCP expresses the operation.
- Runtime tool schemas are authoritative. Discover live tools/signatures via list_tools/get_tool_info when exact arguments matter; do not maintain a static runtime tool catalog in repository instructions.
- Treat partial, truncated, skipped, or errored tool output as incomplete evidence and narrow the query before making exhaustive claims.
- Prefer semantic symbol navigation when it materially reduces discovery scope; use focused file reads and text search when they are the better fit. No fixed tool-call sequence is required.
- Capture initial Git state before the first modification so pre-existing work remains distinguishable.
- All task mutations must be made through one coherent active MCP transaction unless recovery from a failed transaction requires a replacement.
- Keep code, tests, documentation, imported skills, configuration, and related durable memory edits for one task in that transaction.
- Revert a transaction only when intentionally abandoning its task-owned changes. A repairable implementation or failing check is a reason to continue, not to discard the work.
- Before accepting a result, inspect the complete transaction change set and final Git state.
- commit_transaction accepts MCP rollback state only; it is not a Git commit and does not modify the Git index or history.

## Evidence and completion

Validate claims in proportion to consequence and uncertainty.

- Prefer evidence from the real production path or resulting artifact over reasoning about what should happen.
- Use the smallest evidence that proves the changed contract, then broaden when the affected boundary, compatibility risk, failure, or unresolved uncertainty justifies it.
- For bug fixes and semantic behavior changes, cover the relevant negative/failure path and realistic edge cases.
- For backend-equivalence or cross-boundary contracts, use independent or reference evidence where practical rather than comparing two paths that share the same defect.
- Do not claim a test, build, artifact, runtime effect, or file change succeeded unless it was actually verified.
- Completion requires the requested state to exist, relevant invariants to hold, and no known in-scope correctness issue to remain.
- Final coding replies normally report the changed areas, factual validation results, MCP transaction state, and any material remaining limitation.

## Project memory

Project memory under sun-memory/ stores durable architecture, ownership, lifecycle, data-flow, persistence, integration, invariant, and validation context.

- Read memory when project-specific prior context materially helps the active contract.
- Memory narrows discovery but never replaces current source, schema, configuration, generated contracts, or runtime evidence.
- Always update an existing owning memory entry in the same transaction when a durable fact it owns becomes stale.
- Do not store task progress, diffs, debugging notes, line numbers, temporary experiments, or speculation as durable memory.

## Code and test style

- Prefer clear names, strong types, schemas, validation, constants, and executable proof over explanatory comments.
- Do not add comments or explanatory contracts inside tests; encode intent through names, typed fixtures, assertions, and structured evidence.
- Add comments/docstrings only for non-obvious domain rules, persisted formats, bridge boundaries, side effects, ordering/idempotency/concurrency assumptions, performance traps, or known regression hazards.
- Non-trivial Rust structs/enums should have a concise role description when their meaning is not obvious locally.
- Prefer a flat module layout within a semantic area. Related sibling owners should use a stable semantic/category prefix such as `<category>_<owner>.*` instead of shallow directories created only to group a few files or shorten names.
- Use a subdirectory only when it represents a genuine semantic subsystem/boundary or substantial owner-private implementation structure. Keep semantic context in the owner basename; adjacent artifacts derive from the full basename, for example `<owner>.rs` -> `<owner>_certification.rs`.
- Do not create pass-through wrappers that only forward the same parameters.
- Avoid magic numbers when the value has domain meaning.
- Speed priority is CPU before memory; prefer straightforward single-pass logic and avoid unnecessary allocations or indirection.
- Do not redirect build/target folders into the repository when an external build directory is configured.
- SunObservations is the canonical target for project-owned runtime/diagnostic telemetry and certification evidence. Do not add a parallel logging framework. Existing legacy diagnostics do not justify unrelated migration; when such an owner is in scope, converge rather than creating another authority.
- When inspecting SunObservations JSONL v1 logs, use the SunMCP read_log and stats_log capabilities (discover their live aliases at runtime) for filtered reads and process, namespace, span, field, or comparison analysis. Prefer them over raw JSONL reads/parsing unless they cannot express the required query.

## Agent skills

Skills provide specialized domain/API context, not a mandatory generic development itinerary.

- Load a skill only when its domain is materially involved.
- SunObservations owns these canonical skills:
  - `certifications-methodology`
  - `certifications-api`
  - `domain-api`
  - `observations-api`
- The copies under `.agents/skills/` are package-owned mirrors for the pinned `sun-observations` version. Synchronize them from the resolved package/version; do not hand-maintain divergent SunSwitcher-specific copies.
- `sun-observations` is pinned in `Cargo.toml`. Certification-only use belongs in `[dev-dependencies]`; if production owners begin emitting Observations, move the same pinned dependency to `[dependencies]` rather than adding another telemetry library.

### Certification routing

Use semantic certifications when a task creates, changes, reviews, or migrates semantic behavior whose correctness benefits from executable proof.

- Read `certifications-methodology` for proof ownership, evidence choice, negative paths, Domain/Observation decisions, statistical/anomaly proof, causality/alignment, and migration from ordinary tests.
- Also read `certifications-api` when authoring or changing certification code.
- Before authoring family-wide certification inputs, classify the contract as example-specific or family-wide. When a hand-written matrix, repeated parameterized cases, or loops over semantic variants represent a family that the Domain DSL can faithfully express, read `domain-api` and use Domain for that family.
- Domain and checks are complementary: Domain defines where the contract must hold; checks define what must hold for every generated positive case. Never reduce the semantic check set merely because Domain coverage is added.
- Keep concrete non-Domain certification functions for independently meaningful regressions, exact boundary values, externally defined examples, or cases the Domain DSL cannot faithfully represent. For family-wide contracts with important named regressions or boundaries, use both Domain-based family proof and concrete exact cases; neither replaces the other.
- Prefer several focused Domain families when independent semantic dimensions would make one Domain combinatorially large, obscure the contract being proved, or generate many low-value combinations. Each Domain should describe one coherent input family; do not split a naturally unified family merely to reduce case count.
- Also read `observations-api` when Observations, capture, query/decode, spans, routing, or provenance are involved.
- Pure Observation API work without certification proof may use `observations-api` alone.
- The target test architecture has no standalone ordinary test files or modules. Existing verification is retained only when it proves useful behavior, a regression, a boundary, a failure path, or another meaningful contract; migrate that coverage into the owning certification and rewrite it against the certification API. Remove checks that only duplicate an already-proven contract or assert non-contractual implementation detail.

### Observation evidence and namespaces

- Certifications may use canonical results, artifacts, state, and Observations from both visibility scopes. Prefer canonical evidence first; when Observation evidence is needed, reuse an existing suitable Observation from either scope before adding another. If a new Observation is required, use `production::` when the fact also belongs in ordinary runtime telemetry and `certifications::` when it is certification-only evidence. Do not duplicate an already sufficient semantic fact merely to expose it through another evidence channel.
- Observation scope controls visibility, not declaration location or ownership: `production::<module_path>` may appear in ordinary runtime logs and may also be consumed by certifications; `certifications::<module_path>` is certification-only evidence and must not appear in ordinary production logs.
- Certification scope may change Observation routing or visibility, but must not change the owner's semantic execution path or computed result.
- Observation types and emission points belong to the semantic owner whose facts they describe and execute on that owner's canonical path. Certification modules may select, capture, decode, analyze, and assert over Observations, but must not declare owner-semantic Observation types or own their emission.
- Observation payloads should expose stable owner semantics rather than transient internal representations. Prefer domain identities, decisions, boundaries, provenance, quantities, and relations over cache layouts, buffer shapes, helper calls, loop counters, or intermediate storage; a refactor that preserves owner semantics should normally not require the evidence contract to change.
- Observations record semantic facts; certifications own expectations and pass/fail policy. Do not instrument low-level implementation details merely to make them testable or create an alternate verification-only execution path.
- Runtime production owners must not contain verification-only behavior, state, counters, hooks, helpers, or alternate paths under `#[cfg(test)]`. The only permitted `#[cfg(test)]` usage in a production owner is wiring its owner-adjacent certification module.
- Observation namespaces identify the canonical semantic owner; the typed Observation/event identifies what happened inside that owner. Do not encode certification function names, tasks, features, regressions, event kinds, or temporary implementation details into the namespace.
- Namespace form is `<scope>::<module_path>`, where `<scope>` is `production` or `certifications`. Namespace segments are lowercase `snake_case`; for Rust, derive `<module_path>` as `sunswitcher::<module path relative to src>`, omitting technical `lib`, `main`, and `mod` filenames.
- For nested Rust modules, preserve the source module hierarchy after `src`. A certification module follows its owner's path and does not add its own `_certification` segment.
- Tags are stable lowercase `snake_case` filter dimensions; do not duplicate namespace segments, event/type, level, certification/check names, tasks, or implementation details.
- Certification-scope Observations require a functional semantic tag for the owner-local behavior. Production-scope Observations require exactly one role tag: `boundary`, `decision`, or `diagnostic`; add a stable flow tag when they belong to a named cross-owner end-to-end flow. A production functional tag is optional when owner-local filtering is useful.
- `diagnostic` does not make certification-only or hot-path evidence appropriate for production. Per-candidate, per-character, per-inner-loop, per-cache-access, or similar high-cardinality counters that exist to prove certifications belong under `certifications::`; production diagnostics must be operationally meaningful and bounded or aggregated at a lifecycle/domain boundary.
- Durable research/evidence logs should not persist production `diagnostic` telemetry by default. Persist it only when the concrete research contract requires that diagnostic evidence; ordinary durable evidence should favor boundary, decision, lifecycle, anomaly, and result facts.

### Certification file ownership

Keep certifications with the semantics they prove.

- Do not create new standalone `*_test.rs`, `*_certification_test.rs`, or equivalent ordinary test modules. The target form is `<owner>_certification.rs` for one production semantic owner or a composition `_certification.rs` module for a genuine cross-owner contract.
- When an in-scope legacy test file is encountered, migrate every still-valid regression/example/edge/failure case into the appropriate certification and express its expectations through the certification API; then remove the legacy test module. A migration is incomplete while the in-scope old test file remains.
- In owner certification naming, `<owner>` is the basename of the production source module/file that owns the behavior, not the feature, task, tool family, function cluster, or regression being certified.
- Each production owner has at most one owner certification module, normally owner-adjacent as `<owner>_certification.rs`. Multiple certification functions inside that module may prove different deterministic cases, Domain families, regressions, boundaries, oracle checks, statistical/anomaly checks, causal/alignment checks, and metamorphic properties without creating new owners.
- If one production file appears to need multiple independently named owner certification modules, split the production ownership first; do not simulate multiple owners with feature-named certification files.
- Cross-owner contracts within this crate belong under `src/composition_certifications/` and use names that describe the semantic relation with the `_certification.rs` suffix.
- `tests/` is not a certification source or certification-entrypoint location. Composition certifications that require Cargo integration-test semantics must be registered directly from `src/composition_certifications/` through explicit Cargo test targets rather than wrapper files under `tests/`.
- Prove only the cross-owner promise in a composition certification; do not duplicate owner-local proofs.
- Prefer one authoritative semantic proof over several tests that merely restate the same contract.
