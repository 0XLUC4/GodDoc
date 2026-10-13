//! Spelling: the words of a document looked up in the dictionary of a language. The
//! dictionaries are those LibreOffice publishes: fetched the first time a language is asked
//! for, then kept on this computer, where the words are checked.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

/// Where the dictionaries are published.
const SOURCE: &str = "https://raw.githubusercontent.com/LibreOffice/dictionaries/master/";
/// How many other spellings a wrong word is offered.
const SUGGESTIONS: usize = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    French,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::French];

    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::French => "Français",
        }
    }

    /// Its files, without their extension: as kept here, and as published.
    fn files(self) -> (&'static str, &'static str) {
        match self {
            Self::English => ("en_US", "en/en_US"),
            Self::French => ("fr", "fr_FR/dictionaries/fr"),
        }
    }
}

/// The dictionary of a language, and what it said of the words met so far.
pub struct Speller {
    pub language: Language,
    dictionary: spellbook::Dictionary,
    known: HashMap<String, bool>,
    /// Words the user said to leave alone.
    ignored: HashSet<String>,
}

/// Folder the dictionaries are kept in.
fn folder() -> PathBuf {
    let home = |name: &str| std::env::var_os(name).map(PathBuf::from);
    match home("APPDATA") {
        Some(data) => data.join("GodDoc").join("dictionaries"),
        None => home("HOME")
            .unwrap_or_default()
            .join(".local/share/goddoc/dictionaries"),
    }
}

fn fetch(url: &str) -> Result<String, String> {
    let failed = |err: ureq::Error| format!("Cannot get the dictionary: {err}");
    let mut response = ureq::get(url)
        .header("User-Agent", crate::APP_NAME)
        .call()
        .map_err(failed)?;
    response.body_mut().read_to_string().map_err(failed)
}

/// The dictionary of `language`, fetched if it is not here yet. Slow: to be called away from
/// the window.
pub fn load(language: Language) -> Result<Speller, String> {
    let dir = folder();
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Cannot write {}: {err}", dir.display()))?;
    let (here, published) = language.files();
    let part = |extension: &str| -> Result<String, String> {
        let path = dir.join(format!("{here}.{extension}"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Ok(text);
        }
        let text = fetch(&format!("{SOURCE}{published}.{extension}"))?;
        // Kept for the next time; without it, it is only fetched again.
        let _ = std::fs::write(&path, &text);
        Ok(text)
    };
    let (rules, words) = (part("aff")?, part("dic")?);
    let dictionary = spellbook::Dictionary::new(&rules, &words)
        .map_err(|err| format!("Cannot read the dictionary: {err}"))?;
    Ok(Speller {
        language,
        dictionary,
        known: HashMap::new(),
        ignored: HashSet::new(),
    })
}

impl Speller {
    /// The words of `text` the dictionary does not know: where each is, in characters.
    pub fn misspelled(&mut self, text: &str) -> Vec<Range<usize>> {
        let mut found = Vec::new();
        let mut word = String::new();
        let mut start = 0;
        // A space after the text ends its last word.
        for (index, letter) in text.chars().chain([' ']).enumerate() {
            // Apostrophes and hyphens belong to the word they are in.
            let joins = matches!(letter, '\'' | '\u{2019}' | '-') && !word.is_empty();
            if letter.is_alphabetic() || joins {
                if word.is_empty() {
                    start = index;
                }
                word.push(letter);
                continue;
            }
            let whole = word.trim_end_matches(['\'', '\u{2019}', '-']);
            let letters = whole.chars().count();
            if letters > 1 && !self.knows(whole) {
                found.push(start..start + letters);
            }
            word.clear();
        }
        found
    }

    fn knows(&mut self, word: &str) -> bool {
        if self.ignored.contains(word) {
            return true;
        }
        if let Some(known) = self.known.get(word) {
            return *known;
        }
        let plain = word.replace('\u{2019}', "'");
        // "l'heure", "arc-en-ciel": right when each of its parts is.
        let mut parts = plain.split(['\'', '-']);
        let known = self.dictionary.check(&plain)
            || parts.all(|part| part.chars().count() <= 1 || self.dictionary.check(part));
        self.known.insert(word.to_owned(), known);
        known
    }

    /// Other spellings for `word`, the likeliest first.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut found = Vec::new();
        self.dictionary.suggest(word, &mut found);
        found.truncate(SUGGESTIONS);
        found
    }

    /// Leave `word` alone from now on.
    pub fn ignore(&mut self, word: &str) {
        self.ignored.insert(word.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fetches the dictionaries the first time: needs the network.
    #[test]
    #[ignore = "needs the network, the first time"]
    fn wrong_words_are_found_in_both_languages() {
        let mut english = load(Language::English).unwrap();
        let text = "The quick brwn fox, well-known and don't jmps.";
        let wrong: Vec<String> = english
            .misspelled(text)
            .into_iter()
            .map(|range| text.chars().skip(range.start).take(range.len()).collect())
            .collect();
        assert_eq!(wrong, ["brwn", "jmps"]);
        assert!(english.suggest("brwn").contains(&"brown".to_owned()));
        english.ignore("brwn");
        assert_eq!(english.misspelled(text).len(), 1);

        let mut french = load(Language::French).unwrap();
        let text = "L\u{2019}heure est venue, aujourd\u{2019}hui, de corriger les fotes.";
        let wrong: Vec<String> = french
            .misspelled(text)
            .into_iter()
            .map(|range| text.chars().skip(range.start).take(range.len()).collect())
            .collect();
        assert_eq!(wrong, ["fotes"]);
    }
}
