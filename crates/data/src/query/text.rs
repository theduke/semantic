//! Full-text matching: the tokenizer shared by full-text indexes and
//! [`Expr::TextMatch`](super::Expr::TextMatch) evaluation.
//!
//! Text is split into tokens at every character that is not alphanumeric
//! (per [`char::is_alphanumeric`], so letters and digits of every script
//! form tokens) and lowercased with Unicode case mapping. A
//! [`TextAnalyzer`] can additionally drop short tokens and strip common
//! English suffixes. Documents and queries are tokenized by the same
//! analyzer, so a full-text index built with an analyzer answers exactly the
//! text matches using that analyzer.

use std::collections::BTreeSet;

use crate::value::Value;

/// How the query tokens of a text match combine.
#[derive(facet::Facet, Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
#[facet(traits(Default))]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum TextMatchMode {
    /// Every query token must occur in the text.
    #[default]
    All,
    /// At least one query token must occur in the text.
    Any,
}

impl TextMatchMode {
    pub fn is_all(&self) -> bool {
        *self == Self::All
    }
}

/// Tokenization options of full-text indexes and text matches.
///
/// The default analyzer lowercases and splits on non-alphanumeric
/// characters, without stemming or a minimum token length. Default fields
/// are omitted when serialized.
#[derive(facet::Facet, Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
#[facet(traits(Default))]
pub struct TextAnalyzer {
    /// Strip common English suffixes (`-ing`, `-ed`, `-ly`, plural `-s`,
    /// `-ies`, `-sses`) from tokens.
    #[facet(default)]
    #[facet(skip_serializing_if = is_false)]
    pub stemming: bool,
    /// Drop tokens with fewer characters (before stemming). `0` and `1`
    /// keep every token.
    #[facet(default)]
    #[facet(skip_serializing_if = is_zero)]
    pub min_token_len: u32,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// Shortest stem suffix stripping may leave, in characters.
const MIN_STEM_LEN: usize = 3;

impl TextAnalyzer {
    pub fn is_default(&self) -> bool {
        self.equivalent(&Self::default())
    }

    /// Whether both analyzers produce the same tokens for every text.
    pub fn equivalent(&self, other: &Self) -> bool {
        self.stemming == other.stemming && self.min_token_len.max(1) == other.min_token_len.max(1)
    }

    /// Tokens of `text`, in order of occurrence (with repetitions).
    pub fn tokens<'a>(&'a self, text: &'a str) -> impl Iterator<Item = String> + 'a {
        let min_len = usize::try_from(self.min_token_len).unwrap_or(usize::MAX);
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .filter(move |token| token.chars().count() >= min_len)
            .map(|token| if self.stemming { stem(token) } else { token })
    }

    /// Distinct tokens of the string content of `value`: a string, or the
    /// strings of a (nested) list. Other values have no tokens.
    pub fn value_tokens(&self, value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::String(text) => out.extend(self.tokens(text)),
            Value::List(items) => {
                for item in items {
                    self.value_tokens(item, out);
                }
            }
            _ => {}
        }
    }

    /// Distinct tokens of a text query.
    pub fn query_tokens(&self, query: &str) -> BTreeSet<String> {
        self.tokens(query).collect()
    }
}

/// Whether text with the distinct tokens `text` matches the query tokens
/// `query` under `mode`. A query without tokens matches nothing.
pub fn text_tokens_match(
    mode: TextMatchMode,
    text: &BTreeSet<String>,
    query: &BTreeSet<String>,
) -> bool {
    if query.is_empty() {
        return false;
    }
    match mode {
        TextMatchMode::All => query.is_subset(text),
        TextMatchMode::Any => !query.is_disjoint(text),
    }
}

/// Simple English suffix stripping for lowercase tokens.
///
/// Deliberately small (no dictionary, no Porter steps): it conflates the
/// most common inflections (`runs`, `running` -> `run`; `parties` ->
/// `party`) and never shortens a token below [`MIN_STEM_LEN`] characters.
fn stem(token: String) -> String {
    let len = token.chars().count();
    let strip = |suffix: &str, replacement: &str| -> Option<String> {
        let stem = token.strip_suffix(suffix)?;
        let stem_len = len - suffix.chars().count() + replacement.chars().count();
        (stem_len >= MIN_STEM_LEN).then(|| format!("{stem}{replacement}"))
    };
    let stemmed = if token.ends_with("sses") {
        strip("sses", "ss")
    } else if token.ends_with("ies") {
        strip("ies", "y")
    } else if token.ends_with("ing") {
        strip("ing", "").map(undouble)
    } else if token.ends_with("ed") {
        strip("ed", "").map(undouble)
    } else if token.ends_with("ly") {
        strip("ly", "")
    } else if token.ends_with('s') && !token.ends_with("ss") && !token.ends_with("us") {
        strip("s", "")
    } else {
        None
    };
    stemmed.unwrap_or(token)
}

/// `runn` -> `run`: drop a doubled final consonant left by `-ing`/`-ed`.
fn undouble(stem: String) -> String {
    let mut chars = stem.chars().rev();
    match (chars.next(), chars.next()) {
        (Some(last), Some(prev))
            if last == prev
                && last.is_ascii_alphabetic()
                && !matches!(last, 'a' | 'e' | 'i' | 'o' | 'u' | 'l' | 's' | 'z')
                && stem.chars().count() > MIN_STEM_LEN =>
        {
            let mut stem = stem;
            stem.pop();
            stem
        }
        _ => stem,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(analyzer: &TextAnalyzer, text: &str) -> Vec<String> {
        analyzer.tokens(text).collect()
    }

    #[test]
    fn splits_on_non_alphanumeric_and_lowercases() {
        let analyzer = TextAnalyzer::default();
        assert_eq!(
            tokens(&analyzer, "Hello, World! foo_bar-baz 42x  "),
            ["hello", "world", "foo", "bar", "baz", "42x"]
        );
        assert_eq!(tokens(&analyzer, "...--  "), Vec::<String>::new());
        assert_eq!(tokens(&analyzer, "RÉSUMÉ"), ["résumé"]);
    }

    #[test]
    fn keeps_unicode_letters_and_digits() {
        let analyzer = TextAnalyzer::default();
        assert_eq!(
            tokens(&analyzer, "Grüße aus Köln—Straße №5 日本語 ٣٤"),
            ["grüße", "aus", "köln", "straße", "5", "日本語", "٣٤"]
        );
        // Unicode case mapping, including characters whose lowercase form
        // differs in length.
        assert_eq!(tokens(&analyzer, "ΟΔΥΣΣΕΥΣ İstanbul"), {
            let expected: Vec<String> = vec!["οδυσσευς".into(), "i\u{307}stanbul".into()];
            expected
        });
    }

    #[test]
    fn min_token_len_counts_characters() {
        let analyzer = TextAnalyzer {
            min_token_len: 3,
            ..Default::default()
        };
        assert_eq!(tokens(&analyzer, "a an ant äöü xy"), ["ant", "äöü"]);
        assert!(TextAnalyzer::default().equivalent(&TextAnalyzer {
            min_token_len: 1,
            ..Default::default()
        }));
        assert!(!TextAnalyzer::default().equivalent(&analyzer));
    }

    #[test]
    fn stemming_strips_common_suffixes() {
        let analyzer = TextAnalyzer {
            stemming: true,
            ..Default::default()
        };
        assert_eq!(
            tokens(
                &analyzer,
                "running runs run jumped quickly parties classes glass bus is"
            ),
            [
                "run", "run", "run", "jump", "quick", "party", "class", "glass", "bus", "is"
            ]
        );
        // Stems never drop below three characters.
        assert_eq!(tokens(&analyzer, "sing bed ties"), ["sing", "bed", "ties"]);
    }

    #[test]
    fn value_tokens_cover_strings_and_lists() {
        let analyzer = TextAnalyzer::default();
        let mut out = BTreeSet::new();
        analyzer.value_tokens(
            &Value::List(vec![
                Value::String("Red apple".into()),
                Value::U64(7),
                Value::List(vec![Value::String("green APPLE".into())]),
            ]),
            &mut out,
        );
        assert_eq!(
            out.into_iter().collect::<Vec<_>>(),
            ["apple", "green", "red"]
        );
        let mut out = BTreeSet::new();
        analyzer.value_tokens(&Value::Bool(true), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn match_modes() {
        let text = BTreeSet::from(["a".to_string(), "b".to_string()]);
        let query = |tokens: &[&str]| tokens.iter().map(|t| t.to_string()).collect();
        assert!(text_tokens_match(
            TextMatchMode::All,
            &text,
            &query(&["a", "b"])
        ));
        assert!(!text_tokens_match(
            TextMatchMode::All,
            &text,
            &query(&["a", "c"])
        ));
        assert!(text_tokens_match(
            TextMatchMode::Any,
            &text,
            &query(&["a", "c"])
        ));
        assert!(!text_tokens_match(
            TextMatchMode::Any,
            &text,
            &query(&["c"])
        ));
        assert!(!text_tokens_match(TextMatchMode::All, &text, &query(&[])));
        assert!(!text_tokens_match(TextMatchMode::Any, &text, &query(&[])));
    }

    #[test]
    fn analyzer_serialization_omits_defaults() {
        assert_eq!(
            facet_json::to_string(&TextAnalyzer::default()).unwrap(),
            "{}"
        );
        let analyzer = TextAnalyzer {
            stemming: true,
            min_token_len: 2,
        };
        let encoded = facet_json::to_string(&analyzer).unwrap();
        assert_eq!(
            facet_json::from_str::<TextAnalyzer>(&encoded).unwrap(),
            analyzer
        );
        assert_eq!(
            facet_json::from_str::<TextAnalyzer>("{}").unwrap(),
            TextAnalyzer::default()
        );
    }
}
