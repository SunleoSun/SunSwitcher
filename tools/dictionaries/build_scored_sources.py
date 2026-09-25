#!/usr/bin/env python3
"""Build deterministic scored surface-form TSV files for SunSwitcher dictionary assets."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Iterable


def normalize_surface(value: str) -> str | None:
    word = value.strip().lower()
    if not word or not word.isalpha():
        return None
    return word


def score_surface(word: str, language: str) -> int:
    from wordfreq import zipf_frequency

    return max(1, round(zipf_frequency(word, language) * 1000) + 1)


def russian_surfaces() -> Iterable[str]:
    import pymorphy3

    analyzer = pymorphy3.MorphAnalyzer()
    for entry in analyzer.dictionary.iter_known_words(""):
        yield entry[0]


def english_surfaces(path: Path) -> Iterable[str]:
    with path.open("r", encoding="utf-8-sig") as source:
        yield from source


def write_scored(output: Path, surfaces: Iterable[str], language: str) -> tuple[int, int]:
    words = sorted(
        {
            normalized
            for surface in surfaces
            if (normalized := normalize_surface(surface)) is not None
        }
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    correction_count = 0
    with output.open("w", encoding="utf-8", newline="\n") as destination:
        for word in words:
            score = score_surface(word, language)
            correction_count += score >= 4001
            destination.write(f"{word}\t{score}\n")
    return len(words), correction_count


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--english-wordlist",
        required=True,
        type=Path,
        help="SCOWL word-list output generated with size 60, spelling A, variant level 1, categories disabled",
    )
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()

    ru_total, ru_correction = write_scored(
        args.output_dir / "ru-scored.tsv", russian_surfaces(), "ru"
    )
    en_total, en_correction = write_scored(
        args.output_dir / "en-scored.tsv",
        english_surfaces(args.english_wordlist),
        "en",
    )
    print(f"ru: {ru_total} full, {ru_correction} correction")
    print(f"en: {en_total} full, {en_correction} correction")


if __name__ == "__main__":
    main()