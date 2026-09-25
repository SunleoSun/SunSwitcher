# Built-in dictionary assets

`ru.fst` and `en.fst` are SunSwitcher's immutable built-in system vocabularies. They own exact-word validity and system-word completion. `ru-correction.fst` and `en-correction.fst` are frequency-filtered subsets used to build the typo candidate delete index without scanning/indexing the entire natural-language vocabulary at startup.

Current asset counts:

- Russian full: 3,022,339 alphabetic surface forms; correction subset: 9,484.
- English full: 88,750 alphabetic surface forms; correction subset: 7,021.
- The correction subset threshold is score >= 4001, approximately Zipf frequency >= 4.00.

## Sources

Russian surface forms come from the OpenCorpora morphological dictionary (CC BY-SA 3.0), consumed through the pinned `pymorphy3-dicts-ru==2.4.417150.4580142` package and `pymorphy3==2.0.6`. English surface forms come from SCOWLv2 commit `7e99edab8e32f9f9ea2b15f249ca8d4d67237410`, using the American size-60 word list with variant level 1 and special categories disabled:

```text
./scowl --db scowl.db word-list 60 A 1 --categories= > en-forms.txt
```

Both source sets are normalized to lowercase, filtered to alphabetic surface forms, deduplicated, sorted, and scored with `wordfreq==3.1.1` as `max(1, round(zipf_frequency * 1000) + 1)`. This keeps frequency ranking deterministic while preserving every accepted surface form in the full asset. The generated frequency metadata is derived from wordfreq data and therefore carries its upstream attribution/share-alike obligations; see `THIRD_PARTY_NOTICES.txt` before redistribution.

## Regeneration

Install the pinned offline-generation dependencies from `tools/dictionaries/requirements.txt`, generate scored TSVs, then build the FSTs:

```text
python tools/dictionaries/build_scored_sources.py --english-wordlist <en-forms.txt> --output-dir <work-dir>
cargo run --bin dictionary_builder -- <work-dir>/ru-scored.tsv assets/dictionaries/ru.fst
cargo run --bin dictionary_builder -- <work-dir>/en-scored.tsv assets/dictionaries/en.fst
cargo run --bin dictionary_builder -- <work-dir>/ru-scored.tsv assets/dictionaries/ru-correction.fst 4001
cargo run --bin dictionary_builder -- <work-dir>/en-scored.tsv assets/dictionaries/en-correction.fst 4001
```

The Rust builder rejects unsorted, duplicated, non-lowercase, nonalphabetic/NUL-invalid producer mistakes that violate the runtime asset contract. The correction FST is additionally checked at runtime to be an exact frequency-preserving subset of the full FST.

See `THIRD_PARTY_NOTICES.txt` in this directory for source notices.