//! UI translations. The English text is the key: `tr("Save As…")`. To add a string, wrap it in
//! `tr`/`trf` and add a line to every `locales/*.tsv`; a test fails until all of them have it.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Language {
    English,
    Spanish,
    French,
    German,
    Italian,
    Portuguese,
    Polish,
    Dutch,
    Russian,
}

impl Language {
    pub const ALL: [Self; 9] = [Self::English, Self::Spanish, Self::French, Self::German, Self::Italian, Self::Portuguese, Self::Polish, Self::Dutch, Self::Russian];

    /// In its own language, so it can be found whatever the current one is.
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Spanish => "Español",
            Self::French => "Français",
            Self::German => "Deutsch",
            Self::Italian => "Italiano",
            Self::Portuguese => "Português",
            Self::Polish => "Polski",
            Self::Dutch => "Nederlands",
            Self::Russian => "Русский",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::English => "",
            Self::Spanish => include_str!("../locales/es.tsv"),
            Self::French => include_str!("../locales/fr.tsv"),
            Self::German => include_str!("../locales/de.tsv"),
            Self::Italian => include_str!("../locales/it.tsv"),
            Self::Portuguese => include_str!("../locales/pt.tsv"),
            Self::Polish => include_str!("../locales/pl.tsv"),
            Self::Dutch => include_str!("../locales/nl.tsv"),
            Self::Russian => include_str!("../locales/ru.tsv"),
        }
    }

    fn from_locale(locale: &str) -> Self {
        match locale.get(..2) {
            Some("es") => Self::Spanish,
            Some("fr") => Self::French,
            Some("de") => Self::German,
            Some("it") => Self::Italian,
            Some("pt") => Self::Portuguese,
            Some("pl") => Self::Polish,
            Some("nl") => Self::Dutch,
            Some("ru") => Self::Russian,
            _ => Self::English,
        }
    }
}

/// The desktop's language, the way gettext picks it; English if it isn't one of ours.
pub fn system_language() -> Language {
    let locale = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).find(|v| !v.is_empty()).or_else(crate::platform::locale);
    locale.map_or(Language::English, |l| Language::from_locale(&l))
}

static CURRENT: AtomicU8 = AtomicU8::new(0);
static TABLES: [OnceLock<HashMap<String, String>>; 9] = [const { OnceLock::new() }; 9];

pub fn set(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

fn parse(source: &str) -> HashMap<String, String> {
    source.lines().filter_map(|l| l.split_once('\t')).map(|(k, v)| (k.replace("\\n", "\n"), v.replace("\\n", "\n"))).collect()
}

fn lookup(language: Language, key: &'static str) -> &'static str {
    if language == Language::English {
        return key;
    }
    let table = TABLES[language as usize].get_or_init(|| parse(language.source()));
    table.get(key).map_or(key, String::as_str)
}

/// Marks text in a `const` table for translation; show it through `tr`.
pub const fn tr_key(key: &'static str) -> &'static str {
    key
}

pub fn tr(key: &'static str) -> &'static str {
    lookup(Language::ALL[CURRENT.load(Ordering::Relaxed) as usize], key)
}

/// `tr` with `{name}` placeholders, so a translation can reorder them.
pub fn trf(key: &'static str, args: &[(&str, &dyn Display)]) -> String {
    fill(tr(key), args)
}

/// In one pass: a file name containing `{name}` isn't a placeholder.
fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, from_brace) = rest.split_at(open);
        out.push_str(before);
        let value = from_brace.find('}').and_then(|close| args.iter().find(|(n, _)| *n == &from_brace[1..close]).map(|(_, v)| (close, v)));
        match value {
            Some((close, v)) => {
                out.push_str(&v.to_string());
                rest = &from_brace[close + 1..];
            }
            None => {
                out.push('{');
                rest = &from_brace[1..];
            }
        }
    }
    out + rest
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const TRANSLATED: [Language; 8] = [Language::Spanish, Language::French, Language::German, Language::Italian, Language::Portuguese, Language::Polish, Language::Dutch, Language::Russian];

    /// Every literal passed to `tr`/`trf` in the sources, as written there.
    fn keys_in_sources() -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        let mut files: Vec<_> = std::fs::read_dir("src").unwrap().map(|e| e.unwrap().path()).collect();
        while let Some(path) = files.pop() {
            if path.is_dir() {
                files.extend(std::fs::read_dir(path).unwrap().map(|e| e.unwrap().path()));
                continue;
            }
            if path.ends_with("i18n.rs") {
                continue;
            }
            let source = std::fs::read_to_string(path).unwrap();
            let code = source.split("#[cfg(test)]").next().unwrap();
            for call in ["tr(\"", "trf(\"", "tr_key(\""] {
                for (at, _) in code.match_indices(call) {
                    if code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let rest = &code[at + call.len()..];
                    let mut end = 0;
                    while rest.as_bytes()[end] != b'"' {
                        end += if rest.as_bytes()[end] == b'\\' { 2 } else { 1 };
                    }
                    keys.insert(rest[..end].replace("\\\"", "\""));
                }
            }
        }
        keys
    }

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{').skip(1).filter_map(|p| p.split_once('}')).map(|(name, _)| name).collect()
    }

    #[test]
    fn every_language_has_exactly_the_strings_in_the_code() {
        let keys = keys_in_sources();
        assert!(keys.len() > 100, "the source scan found only {}", keys.len());
        for language in TRANSLATED {
            let table: BTreeSet<String> = language.source().lines().filter_map(|l| l.split_once('\t')).map(|(k, _)| k.to_string()).collect();
            assert_eq!(table.len(), language.source().lines().count(), "{language:?}: a line is malformed or repeated");
            let missing: Vec<_> = keys.difference(&table).collect();
            let stale: Vec<_> = table.difference(&keys).collect();
            assert!(missing.is_empty() && stale.is_empty(), "{language:?}: missing {missing:#?}, stale {stale:#?}");
        }
    }

    #[test]
    fn translations_keep_their_placeholders() {
        for language in TRANSLATED {
            for line in language.source().lines() {
                let (key, value) = line.split_once('\t').unwrap();
                assert_eq!(placeholders(key), placeholders(value), "{language:?}: {line}");
                assert!(!value.trim().is_empty(), "{language:?}: {line}");
            }
        }
    }

    #[test]
    fn the_fonts_can_draw_every_translation() {
        use ab_glyph::{Font, FontRef};
        let roboto = FontRef::try_from_slice(include_bytes!("../fonts/Roboto-Regular.ttf")).unwrap();
        let icons = FontRef::try_from_slice(include_bytes!("../fonts/emoji-icon-font.ttf")).unwrap();
        for language in TRANSLATED {
            for c in language.source().chars().filter(|c| !c.is_control()) {
                assert!(roboto.glyph_id(c).0 != 0 || icons.glyph_id(c).0 != 0, "{language:?}: no glyph for {c}");
            }
        }
    }

    #[test]
    fn unknown_text_stays_english() {
        assert_eq!(lookup(Language::German, "Not a key"), "Not a key");
        assert_eq!(lookup(Language::English, "Save As…"), "Save As…");
    }

    #[test]
    fn placeholders_fill_once_in_any_order() {
        assert_eq!(fill("{b} then {a}", &[("a", &1), ("b", &"x")]), "x then 1");
        assert_eq!(fill("Saved {name}", &[("name", &"{name}.png")]), "Saved {name}.png");
        assert_eq!(fill("keeps {other} and {", &[("name", &1)]), "keeps {other} and {");
    }

    #[test]
    fn locale_names_map_to_languages() {
        assert_eq!(Language::from_locale("es_ES.UTF-8"), Language::Spanish);
        assert_eq!(Language::from_locale("es_MX"), Language::Spanish);
        assert_eq!(Language::from_locale("pt_BR.UTF-8"), Language::Portuguese);
        assert_eq!(Language::from_locale("de:en"), Language::German);
        assert_eq!(Language::from_locale("pl_PL.UTF-8"), Language::Polish);
        assert_eq!(Language::from_locale("nl_BE"), Language::Dutch);
        assert_eq!(Language::from_locale("ru_RU.UTF-8"), Language::Russian);
        assert_eq!(lookup(Language::Russian, "Open…"), "Открыть…");
        assert_eq!(Language::ALL.map(|l| l as usize), std::array::from_fn(|i| i), "ALL is in declaration order");
        assert_eq!(Language::ALL.len(), TABLES.len());
        assert_eq!(Language::from_locale("ja_JP"), Language::English);
        assert_eq!(Language::from_locale("C"), Language::English);
    }

    #[test]
    fn newlines_are_escaped_in_the_tables() {
        assert_eq!(parse("One\\nTwo\tUno\\nDos")["One\nTwo"], "Uno\nDos");
    }
}
