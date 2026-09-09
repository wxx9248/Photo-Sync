//! Running seeds, and remembering the ones that ever failed.
//!
//! A campaign makes up sessions nobody wrote down and checks the desktop against the model.
//! A seed that fails is written into `verification/corpus/` and replayed by the fastest tier
//! from then on, which is the discipline that makes a fixed bug stay fixed.

use std::path::{Path, PathBuf};

use photo_sync_sim::campaign;

/// What one campaign run found.
pub(crate) struct Outcome {
    pub seeds: usize,
    pub failures: Vec<campaign::Failure>,
}

impl Outcome {
    #[must_use]
    pub(crate) fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Runs a stretch of seeds, starting from zero.
#[must_use]
pub(crate) fn run(seeds: usize) -> Outcome {
    let failures = (0..seeds as u64)
        .filter_map(|seed| campaign::run(seed).err())
        .collect();
    Outcome { seeds, failures }
}

/// Runs every seed that has ever failed. `AGENTS.md` commits each of these beside its fix.
///
/// # Errors
/// When the corpus cannot be read.
pub(crate) fn replay_corpus(directory: &Path) -> Result<Outcome, String> {
    let seeds = corpus(directory)?;
    let failures = seeds
        .iter()
        .filter_map(|(seed, _)| campaign::run(*seed).err())
        .collect();
    Ok(Outcome {
        seeds: seeds.len(),
        failures,
    })
}

/// Runs one seed and says what it did.
///
/// # Errors
/// When the token is not a seed.
pub(crate) fn replay_one(token: &str) -> Result<Outcome, String> {
    let seed = parse(token).ok_or_else(|| format!("{token} is not a seed"))?;
    let failures: Vec<campaign::Failure> = campaign::run(seed).err().into_iter().collect();
    if failures.is_empty() {
        println!("{token}: nothing to see");
    }
    Ok(Outcome { seeds: 1, failures })
}

/// The seeds the corpus holds, with the note each was filed under.
fn corpus(directory: &Path) -> Result<Vec<(u64, PathBuf)>, String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", directory.display())),
    };

    let mut found: Vec<(u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "seed"))
        .filter_map(|path| {
            let token = path.file_stem()?.to_str()?;
            parse(token).map(|seed| (seed, path.clone()))
        })
        .collect();
    found.sort();
    Ok(found)
}

/// Reads `0x` followed by hexadecimal, which is how a seed is written everywhere it appears.
fn parse(token: &str) -> Option<u64> {
    u64::from_str_radix(token.strip_prefix("0x").unwrap_or(token), 16).ok()
}

pub(crate) fn print_failures(outcome: &Outcome) {
    for failure in &outcome.failures {
        println!("  {failure}");
        println!("    reproduce with: ./verify replay {:#018x}", failure.seed);
        println!(
            "    then keep it: touch verification/corpus/{:#018x}.seed",
            failure.seed
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_is_read_from_the_way_it_is_written() {
        assert_eq!(parse("0x000000000000002f"), Some(47));
        assert_eq!(parse("2f"), Some(47));
        assert_eq!(parse("not a seed"), None);
    }
}
