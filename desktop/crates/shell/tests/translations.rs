//! Every string a person reads, in both languages.
//!
//! `STACK.md` §3.2 ships English and Simplified Chinese from day one, and a translation that
//! silently falls back to English is the way that stops being true: nothing fails, the window
//! just quietly reverts a word at a time as the markup grows. This reads the markup and the
//! catalogue and insists they still describe the same window.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn ui() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui")
}

/// Every string the markup marks for translation.
fn marked() -> BTreeSet<String> {
    let markup = match std::fs::read_to_string(ui().join("app.slint")) {
        Ok(markup) => markup,
        Err(error) => panic!("cannot read the window markup: {error}"),
    };

    let mut found = BTreeSet::new();
    let mut rest = markup.as_str();
    while let Some(at) = rest.find("@tr(\"") {
        rest = &rest[at + 5..];
        match rest.find('"') {
            Some(end) => {
                found.insert(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => panic!("a translated string is never closed"),
        }
    }
    found
}

/// Every string a catalogue gives an answer for, ignoring the ones left empty.
fn translated(language: &str) -> BTreeSet<String> {
    let path = ui()
        .join("translations")
        .join(language)
        .join("LC_MESSAGES/photo-sync.po");
    let catalogue = match std::fs::read_to_string(&path) {
        Ok(catalogue) => catalogue,
        Err(error) => panic!("cannot read {}: {error}", path.display()),
    };

    let mut answered = BTreeSet::new();
    let mut waiting: Option<String> = None;
    for line in catalogue.lines() {
        if let Some(rest) = line.strip_prefix("msgid \"") {
            let text = rest.trim_end_matches('"').to_string();
            waiting = (!text.is_empty()).then_some(text);
        } else if let Some(rest) = line.strip_prefix("msgstr \"") {
            let text = rest.trim_end_matches('"');
            if let Some(waited) = waiting.take()
                && !text.is_empty()
            {
                answered.insert(waited);
            }
        }
    }
    answered
}

#[test]
fn every_string_in_the_window_has_a_chinese_translation() {
    let missing: Vec<String> = marked().difference(&translated("zh_CN")).cloned().collect();

    assert!(
        missing.is_empty(),
        "these would fall back to English: {missing:?}"
    );
}

#[test]
fn the_catalogue_has_not_outlived_the_window() {
    let marked = marked();
    let stale: Vec<String> = translated("zh_CN")
        .into_iter()
        .filter(|line| !marked.contains(line))
        .collect();

    assert!(
        stale.is_empty(),
        "these are translated and no longer shown: {stale:?}"
    );
}

#[test]
fn the_window_says_something_worth_translating() {
    // A guard on the two tests above: if the extraction ever stopped finding anything, both
    // would pass by comparing nothing with nothing.
    assert!(marked().len() > 8, "the markup has stopped being read");
}
