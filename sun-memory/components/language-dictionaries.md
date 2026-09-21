---
description: Maps canonical RU/EN dictionary surface-form ownership, runtime exact/delete indexes, inflection protection, and bootstrap persistence; read before changing system dictionaries, grammatical-form handling, or lexical candidate generation.
---

# Language Dictionaries

## Purpose

System language dictionaries are the canonical vocabulary used to decide whether a token is already valid and to generate typo candidates. The persistence layer stores normalized surface forms and frequencies; `LanguagePack` turns those rows into immutable exact/delete indexes for the keyboard hot path.

## Surface forms, not suffix heuristics

A dictionary row represents an accepted surface form. Russian cases, numbers, genders, and adjective forms and English plural/verb inflections that should be accepted must exist as exact rows, for example `слово`, `слову`, `словом`, `words`, and `working`. The correction provider must not infer "valid morphology" from ad-hoc suffix rules because that would create a second language-specific correction authority and can protect real typos.

Exact dictionary membership is checked before typo candidate generation. Therefore a valid grammatical form must be kept unchanged even when it is one edit away from a more frequent lemma. Surface forms also participate in the same delete-index typo correction, so a typo of an inflected form can correct back to that inflected form rather than being collapsed to the lemma.

## Ownership boundary

SQLite `languages` and `dictionary_words` are the persistent authority. `src/persistence/database.rs` owns the current bootstrap rows and stored-row validation. `src/language/language_pack.rs` owns immutable runtime indexes. `src/correction/lexical_provider.rs` consumes those indexes and remains language-agnostic. RU/EN keyboard-layout maps remain in `src/language/russian.rs` and `src/language/english.rs`; they are not dictionary data.

While no released database compatibility contract exists, shipped vocabulary lives directly in the single bootstrap schema. Once a schema version is shipped to real users, later dictionary additions that must reach existing databases should use forward migrations rather than silently depending on a fresh install.

## Scope

The current built-in dictionary is still a representative embedded lexicon, not a production-complete natural-language corpus. When a broader licensed dictionary is introduced, import its accepted surface forms into the same canonical `dictionary_words` contract rather than adding a parallel spellchecker or morphology authority.

## Validation

Changes to shipped dictionary data should certify that fresh databases contain the intended rows, valid representative surface forms are kept exactly, and representative typos can still correct to the intended surface form. After a persistent compatibility baseline is released, add upgrade-path certification as well. Finish with the full lexical-provider and persistence certifications plus the normal project validation suite.

## Related memory

- `components/persistence`
- `main/core-project-principles`
- `components/user-lexicon`