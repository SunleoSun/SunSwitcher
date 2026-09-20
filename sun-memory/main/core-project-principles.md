---
description: Durable architecture, ownership, typed-contract, DB-centric, colocated-test, certification, and fail-closed principles for SunSwitcher.
---

# SunSwitcher Core Project Principles

## Purpose

These are durable architecture rules for SunSwitcher. They apply to production code, tests, certifications, and future persistence/UI work.

## 1. Functional ownership: one source file per application functionality

Organize code by meaningful application functionality, not by type count.

A functional source file may own several tightly related structs, enums, helpers, and private implementation details when they implement one coherent behavior. Do not split one behavior into many files merely because it contains several types. Conversely, do not combine unrelated behaviors just to reduce file count.

The file/module name should make the behavior owner obvious. Examples: input buffering, correction decision, replacement planning, Windows keyboard runtime.

## 2. Single Source of Behavior + Typed Contracts

Treat these as one combined architecture rule.

Every domain decision, invariant, routing rule, lifecycle rule, validation rule, and side effect has one canonical semantic owner. Other components consume that owner's result and must not reimplement, reinterpret, shadow, synchronize, or compete with the owning behavior.

Boundaries between functional owners use typed contracts. When two values share a primitive representation but have different meanings, prefer enums, wrappers, dedicated IDs, structs, explicit fields/accessors, and validated types over booleans, magic strings, tuples, or repeated validation.

Before adding logic, ask:

1. Where is the single owner of this behavior?
2. Can the contract make misuse impossible or clearly invalid?

If ownership is ambiguous, consolidate ownership before extending behavior.

## 3. DB-centric architecture when persistence is introduced

When SunSwitcher gains persistent state, the database is the canonical persistent source of truth.

UI state and runtime state are derived views of the database and must not become competing authorities. UI mutations change canonical state first and then reflect it. Runtime snapshots/caches are allowed for latency-sensitive paths, but they contain no independent persistence semantics and must be fully reconstructable from database state.

The keyboard hot path must not perform database I/O. It consumes a validated in-memory runtime snapshot produced from canonical database state.

## 4. Tests live beside their functional owner

Unit tests and certification tests are colocated with the implementation they verify. Do not use a global `tests/` directory for tests that belong to one functional owner.

Example:

- `replacement/replacement_engine.rs`
- `replacement/replacement_engine_tests.rs`
- `replacement/replacement_engine_certification.rs`

A separate application-level test location is justified only when a test genuinely spans multiple functional owners or an external application/OS boundary.

## 5. Unit tests and behavioral certification have different jobs

Unit tests verify local invariants, edge cases, validation, and small operations.

Certification tests verify the behavior promised by a functional owner across representative conditions. Certification is contract-oriented: it tests externally observable semantics through the public/typed contract rather than depending on internal data structures or implementation details.

A meaningful semantic change is incomplete until the relevant unit tests and behavioral certification are updated and passing.

## 6. Fail closed when input state is uncertain

SunSwitcher modifies user text. If the runtime can no longer prove that its tracked input state corresponds to the user's current caret/text state, invalidate the tracked state and skip correction.

Missing a correction is acceptable. Editing or deleting unrelated user text is not.

The Windows keyboard runtime invalidates tracked token state when the foreground window changes or when command-modified editing keys have application-specific semantics (for example Ctrl+Backspace, Ctrl+Enter, or Ctrl+Tab). A physical key-up may be suppressed only after the corresponding synthetic replacement injection succeeds; a failed injection must leave the original key sequence intact.

## 7. Side effects execute decisions; they do not own them

OS integration code captures input and executes typed actions. It must not decide which word is correct, duplicate correction policy, or contain domain-specific word rules.

The correction engine owns correction decisions. The replacement layer owns replacement semantics. The Windows runtime owns Windows input capture/injection only.

## 8. Distinct replacement interactions remain distinct typed paths

Automatic replacement of a token immediately before the caret and explicit replacement of user-selected text are different interaction contracts. Do not force them through one ambiguous primitive merely because both eventually change text.

Typed-word replacement may use owned input-buffer state plus synthetic deletion/insertion. Selected-text replacement is initiated explicitly by the user, consumes the application's current selection, and executes the replacement directly over that selection; clipboard use, when required to capture the selected source text, is a separate temporary transport guarded by the preservation contract below. Both paths may consume the same future correction/language transformation logic, but each keeps its own typed action and OS execution contract.

Selected-text replacement must preserve the user's prior clipboard state within an explicit supported preservation contract or fail closed before changing user text. Clipboard use is limited to source-selection capture: snapshot the supported prior state, issue the temporary copy, read the selection, then restore and verify the prior clipboard before any text mutation. Replacement execution uses direct Unicode input over the active selection rather than transporting replacement text through the clipboard. The current preservation contract supports empty clipboard, Unicode text, copied-file state via the critical `CF_HDROP` payload plus `Preferred DropEffect` when present, and image/screenshot state via all available `CF_DIBV5`/`CF_DIB` payloads. Restore verification compares the preserved semantic payload before text mutation. Other clipboard states remain unsupported until they gain an explicit preservation contract. Temporary clipboard contents are execution details, never canonical application state.