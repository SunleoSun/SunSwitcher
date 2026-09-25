use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use fst::MapBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(args.next().ok_or("missing scored TSV input path")?);
    let output = PathBuf::from(args.next().ok_or("missing FST output path")?);
    let min_frequency = args
        .next()
        .map(|value| value.to_string_lossy().parse::<u32>())
        .transpose()?
        .unwrap_or(1);
    if min_frequency == 0 || args.next().is_some() {
        return Err(
            "usage: dictionary_builder <scored.tsv> <output.fst> [min_frequency>=1]".into(),
        );
    }

    let reader = BufReader::new(File::open(&input)?);
    let mut builder = MapBuilder::memory();
    let mut count = 0_u64;
    let mut previous = String::new();

    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        let (word, frequency) = line.split_once('\t').ok_or_else(|| {
            format!(
                "{}:{}: expected WORD\\tFREQUENCY",
                input.display(),
                line_number + 1
            )
        })?;
        if word.is_empty()
            || word.contains('\0')
            || !word.chars().all(char::is_alphabetic)
            || word.chars().flat_map(char::to_lowercase).ne(word.chars())
        {
            return Err(format!(
                "{}:{}: word must be non-empty, NUL-free, alphabetic, and normalized lowercase",
                input.display(),
                line_number + 1
            )
            .into());
        }
        if !previous.is_empty() && previous.as_str() >= word {
            return Err(format!(
                "{}:{}: words must be unique and strictly sorted",
                input.display(),
                line_number + 1
            )
            .into());
        }
        let frequency: u32 = frequency.parse()?;
        if frequency == 0 {
            return Err(format!(
                "{}:{}: frequency must be positive",
                input.display(),
                line_number + 1
            )
            .into());
        }
        previous.clear();
        previous.push_str(word);
        if frequency < min_frequency {
            continue;
        }
        builder.insert(word, u64::from(frequency))?;
        count += 1;
    }
    if count == 0 {
        return Err("dictionary must contain at least one word at the requested frequency".into());
    }
    let bytes = builder.into_inner()?;
    std::fs::write(&output, bytes)?;
    eprintln!("built {} entries -> {}", count, output.display());
    Ok(())
}
