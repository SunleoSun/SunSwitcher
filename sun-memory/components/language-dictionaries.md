---
description: Maps canonical built-in RU/EN FST dictionary ownership, full-vocabulary exact/completion lookup, bounded correction subsets, source regeneration, and validation; read before changing system dictionaries, grammatical-form handling, or lexical candidate generation.
---

# Language Dictionaries

## Purpose

Built-in Russian and English dictionaries are immutable FST assets under `assets/dictionaries/`. They are the canonical shipped vocabulary for exact validity and system-word completion. Language morphology is represented by accepted surface forms; runtime suffix heuristics or a second spellchecker are not allowed.

## Built-in asset contract

Each built-in language has two FSTs with the same stored frequency scale:

- `ru.fst` / `en.fst`: full accepted alphabetic surface vocabulary. Exact validity and completion query these assets directly.
- `ru-correction.fst` / `en-correction.fst`: frequency-filtered subsets used only to build the typo `DeleteIndex`, so startup does not expand/index millions of rare forms.

Current full counts are 3,022,339 Russian forms and 88,750 English forms. Current correction subsets use score >= 4001 (approximately Zipf >= 4.00) and contain 9,484 Russian and 7,021 English forms. A correction-sidecar entry must exist in the full FST with exactly the same frequency or `LanguagePack` rejects the asset.

`LanguagePack` owns one shared immutable `LanguageDictionaryData`: the full `fst::Map`, the materialized correction entries/delete index, and the correction-set maximum frequency. `LanguagePack` clones are cheap through `Arc`. `contains_normalized` uses the full FST. `candidate_entries` uses only the correction delete index. `prefix_matches` streams the full FST prefix range but retains only the requested bounded top-frequency results, avoiding materializing every match for broad prefixes.

## Surface forms, not suffix heuristics

A valid grammatical form must exist exactly in the full asset. Russian cases/numbers/genders/adjective/verb forms and English plural/verb inflections are protected because the source dictionaries enumerate them. Exact membership is checked before typo candidate generation, so a valid inflected form is kept even when it is one edit from a more frequent lemma. Rare forms remain exact-valid even when deliberately excluded from typo generation by the correction frequency threshold.

Dictionary assets contain alphabetic words only. Hyphenated Russian forms and apostrophe-bearing English forms are excluded because current token ownership treats such punctuation as boundaries rather than part of one lexical token. The offline producer and Rust FST builder both enforce this contract.

## Sources and regeneration

`tools/dictionaries/build_scored_sources.py` is the deterministic source-to-TSV producer. Pinned generation dependencies are in `tools/dictionaries/requirements.txt`.

- Russian surface forms: OpenCorpora morphological data (CC BY-SA 3.0) via pinned `pymorphy3-dicts-ru==2.4.417150.4580142` and `pymorphy3==2.0.6`.
- English surface forms: SCOWLv2 commit `7e99edab8e32f9f9ea2b15f249ca8d4d67237410`, American size-60 word list, variant level 1, special categories disabled.
- Frequency ranking: `wordfreq==3.1.1`, stored as `max(1, round(zipf_frequency * 1000) + 1)`; generated frequency metadata carries wordfreq's upstream attribution/share-alike obligations documented in `assets/dictionaries/THIRD_PARTY_NOTICES.txt`.

The producer lowercases, keeps alphabetic forms, deduplicates, sorts, and writes scored TSV. `src/bin/dictionary_builder.rs` validates the normalized/sorted/unique/positive-frequency contract and builds FSTs; its optional minimum-frequency argument builds correction sidecars. Source notices and exact regeneration commands live in `assets/dictionaries/README.md` and `THIRD_PARTY_NOTICES.txt`.

## Persistence and layout ownership

SQLite `languages` remains the enablement/configuration authority for built-in language IDs. Built-in `ru`/`en` vocabulary content lives only in immutable FST assets; schema v5 drops the obsolete `dictionary_words` table and `Database::load_enabled_language_packs` fails closed on enabled non-built-in language IDs instead of rebuilding mutable custom dictionaries.

Keyboard-layout maps remain in `src/language/russian.rs` and `src/language/english.rs`; they are not dictionary data. The RU/EN maps include the ordinary punctuation tail plus shifted number-row punctuation that participates in wrong-layout words, so `rfr&` switches to `как?` instead of preserving a literal `&`; the physical-key fallback applies that shifted mapping only when the produced symbol is actually shifted, so an ordinary unshifted `7` remains `7`. `KeyboardLayoutMap::transform_text`, mixed-layout Double Shift behavior, and `TextCasePattern` remain independent typed behavior owners.

## Validation

High-value checks cover representative Russian inflections and `ё`, English inflections, separation of full exact vocabulary from the correction subset, correction-sidecar/full-asset consistency, schema-v5 removal of obsolete `dictionary_words`, unknown enabled-language fail-closed behavior, lexical correction behavior, completion behavior, and the normal full Rust validation suite.

## Related memory

- `components/persistence`
- `components/user-lexicon`
- `components/completion`
- `main/core-project-principles`