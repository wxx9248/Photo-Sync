//! Which tests claim which requirements.
//!
//! Declarations are read from the sources rather than from a run, so the matrix exists before
//! any test executes. Rust tests declare with `covers!`, Kotlin tests with `@Covers`, and
//! scenarios with a `covers` key.

use std::collections::BTreeMap;
use std::path::Path;

use crate::requirements::{Registry, Status};
use crate::workspace;

pub struct Matrix {
    pub declarations: BTreeMap<String, Vec<String>>,
    pub active: Vec<ActiveRequirement>,
    pub deferred: usize,
}

/// The part of a requirement the matrix needs in order to explain a gap.
pub struct ActiveRequirement {
    pub id: String,
    pub area: String,
    pub section: String,
    pub note: String,
}

impl Matrix {
    /// Active requirements that no test claims. These fail the full tier.
    pub fn unclaimed(&self) -> Vec<&ActiveRequirement> {
        self.active
            .iter()
            .filter(|requirement| !self.declarations.contains_key(&requirement.id))
            .collect()
    }

    pub fn areas(&self) -> BTreeMap<&str, usize> {
        let mut counts = BTreeMap::new();
        for requirement in &self.active {
            *counts.entry(requirement.area.as_str()).or_default() += 1;
        }
        counts
    }
}

pub fn build(root: &Path) -> Result<Matrix, String> {
    let registry = Registry::load(&workspace::requirements_file(root))?;

    let mut declarations = BTreeMap::new();
    collect_from_rust(&root.join("desktop/crates"), &mut declarations)?;
    collect_from_kotlin(&root.join("android"), &mut declarations)?;
    collect_from_scenarios(&workspace::scenarios_directory(root), &mut declarations)?;

    Ok(Matrix {
        declarations,
        active: registry
            .active()
            .map(|item| ActiveRequirement {
                id: item.id.clone(),
                area: item.area.clone(),
                section: item.section.clone(),
                note: item.note.clone(),
            })
            .collect(),
        deferred: registry
            .requirements
            .iter()
            .filter(|i| i.status == Status::Deferred)
            .count(),
    })
}

pub fn print_matrix(root: &Path) -> Result<bool, String> {
    let matrix = build(root)?;
    let unclaimed = matrix.unclaimed();

    let areas: Vec<String> = matrix
        .areas()
        .into_iter()
        .map(|(area, count)| format!("{area} {count}"))
        .collect();

    println!(
        "matrix: {} active ({}), {} deferred, {} claimed by tests",
        matrix.active.len(),
        if areas.is_empty() {
            "none yet".to_string()
        } else {
            areas.join(", ")
        },
        matrix.deferred,
        matrix.declarations.len()
    );

    for requirement in &unclaimed {
        println!(
            "  no test claims {} ({})",
            requirement.id, requirement.section
        );
        if !requirement.note.is_empty() {
            println!("    {}", requirement.note);
        }
    }

    Ok(unclaimed.is_empty())
}

fn collect_from_rust(
    directory: &Path,
    into: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    for file in rust_sources(directory)? {
        let text = read(&file)?;
        let mut enclosing = String::from("unknown");

        for line in text.lines() {
            if let Some(name) = function_name(line) {
                enclosing = name;
            }
            for id in covered_identifiers(line) {
                into.entry(id).or_default().push(enclosing.clone());
            }
        }
    }

    Ok(())
}

/// The same, for the phone.
///
/// Half of the specification is the phone's, and a requirement covered by a Kotlin test is
/// covered whether or not this end can run it. Leaving these out would show the phone's rules
/// as unverified for as long as the Android half exists, which is the opposite of what a
/// traceability matrix is for.
fn collect_from_kotlin(
    directory: &Path,
    into: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    for file in sources_with(directory, "kt")? {
        let text = read(&file)?;
        // A Kotlin annotation sits above the function it belongs to, so the identifiers are
        // held until the name that earns them turns up.
        let mut pending: Vec<String> = Vec::new();

        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("@Covers") {
                pending.extend(quoted_identifiers(line));
                continue;
            }
            if let Some(name) = kotlin_function_name(trimmed) {
                for id in pending.drain(..) {
                    into.entry(id).or_default().push(name.clone());
                }
            }
        }
    }

    Ok(())
}

/// The name of a Kotlin test, including the backticked kind that reads as a sentence.
fn kotlin_function_name(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("fun ")
        .or_else(|| line.find(" fun ").map(|at| &line[at + 5..]))?;
    let rest = rest.trim_start();
    if let Some(quoted) = rest.strip_prefix('`') {
        let end = quoted.find('`')?;
        return Some(quoted[..end].to_string());
    }
    let end = rest.find(['(', '<'])?;
    Some(rest[..end].trim().to_string())
}

fn collect_from_scenarios(
    directory: &Path,
    into: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    if !directory.is_dir() {
        return Ok(());
    }

    for entry in read_directory(directory)? {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "toml") {
            continue;
        }

        let text = read(&path)?;
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        for line in text
            .lines()
            .filter(|line| line.trim_start().starts_with("covers"))
        {
            for id in quoted_identifiers(line) {
                into.entry(id).or_default().push(name.clone());
            }
        }
    }

    Ok(())
}

fn function_name(line: &str) -> Option<String> {
    let start = line.find("fn ")? + 3;
    let rest = &line[start..];
    let end = rest.find(['(', '<'])?;
    Some(rest[..end].trim().to_string())
}

fn covered_identifiers(line: &str) -> Vec<String> {
    if !line.contains("covers!") {
        return Vec::new();
    }

    quoted_identifiers(line)
}

fn quoted_identifiers(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .filter(|candidate| candidate.starts_with("R-"))
        .map(str::to_string)
        .collect()
}

fn rust_sources(directory: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    sources_with(directory, "rs")
}

/// Every source file of one kind under a directory, ignoring build output.
fn sources_with(directory: &Path, extension: &str) -> Result<Vec<std::path::PathBuf>, String> {
    let mut found = Vec::new();
    if !directory.is_dir() {
        return Ok(found);
    }

    for entry in read_directory(directory)? {
        let path = entry.path();
        // Generated code declares nothing and there is a great deal of it.
        if path
            .file_name()
            .is_some_and(|name| name == "build" || name == "target")
        {
            continue;
        }
        if path.is_dir() {
            found.extend(sources_with(&path, extension)?);
        } else if path.extension().is_some_and(|found| found == extension) {
            found.push(path);
        }
    }

    Ok(found)
}

fn read_directory(directory: &Path) -> Result<Vec<std::fs::DirEntry>, String> {
    std::fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_covers_line_yields_every_identifier_on_it() {
        let ids = covered_identifiers(r#"    covers!("R-COMMIT-014", "R-COMMIT-015");"#);

        assert_eq!(ids, vec!["R-COMMIT-014", "R-COMMIT-015"]);
    }

    #[test]
    fn an_ordinary_string_is_not_mistaken_for_a_declaration() {
        assert!(covered_identifiers(r#"let message = "R-COMMIT-014 failed";"#).is_empty());
    }

    #[test]
    fn the_enclosing_test_name_is_taken_from_the_function_line() {
        assert_eq!(
            function_name("    fn commit_orders_fsync_before_donemark() {"),
            Some("commit_orders_fsync_before_donemark".to_string())
        );
    }
}
