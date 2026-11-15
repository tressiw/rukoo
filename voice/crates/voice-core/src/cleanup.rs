//! Dictionary corrections and filler-word removal without a language model.
//!
//! A port of Ownkey's `text_cleanup.py`. Both steps are plain text rules, so they add no
//! measurable latency and run for every provider, local or cloud. Ownkey expressed them as
//! Python regexes with lookarounds; Rust's `regex` has no lookaround, so the matchers below walk
//! the text by hand with the same leftmost-first, backtracking order Python uses.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Clear hesitations per language: (code, label, words). Words that carry meaning ("like",
/// "well", "dus", "gewoon") are deliberately absent.
pub const FILLER_LANGUAGES: [(&str, &str, &[&str]); 5] = [
    (
        "en",
        "English",
        &["uh", "uhh", "um", "umm", "uhm", "erm", "er", "mhm"],
    ),
    (
        "nl",
        "Dutch",
        &["uh", "uhm", "eh", "ehm", "euh", "euhm", "hm"],
    ),
    ("de", "German", &["\u{e4}h", "\u{e4}hm", "ehm", "hm", "mh"]),
    ("fr", "French", &["euh", "heu", "hum"]),
    ("es", "Spanish", &["eh", "em", "ehm", "este"]),
];

/// Fillers in one language that are ordinary words in another. When a language here is enabled,
/// its words are never removed, whichever language lists them as a filler.
pub const PROTECTED_WORDS: [(&str, &[&str]); 5] = [
    ("en", &[]),
    ("nl", &["er", "este"]),
    ("de", &["um", "er", "este"]),
    ("fr", &["este"]),
    ("es", &["er"]),
];

pub const DEFAULT_FILLER_LANGUAGES: [&str; 2] = ["en", "nl"];

/// The sentence Settings shows under "Heard" in the filler-word preview.
pub const FILLER_SAMPLE: &str =
    "Um, I think, uh, we should ehm ship it on Tuesday. Er is nog \u{e9}\u{e9}n ding.";

/// The punctuation that ends a sentence: cleanup keeps it after a removed filler, and dictation
/// reuses it to decide on a trailing space.
pub const SENTENCE_END: [char; 4] = ['.', '!', '?', '\u{2026}'];
const POST_PUNCTUATION: [char; 7] = [',', ';', ':', '.', '!', '?', '\u{2026}'];
const MARK: char = '\u{0}';

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correction {
    pub from: String,
    pub to: String,
}

impl Correction {
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Correction {
        Correction {
            from: from.into(),
            to: to.into(),
        }
    }
}

/// The settings cleanup reads, taken from the config snapshot of one recording.
#[derive(Clone, Debug, PartialEq)]
pub struct CleanupConfig {
    pub remove_fillers: bool,
    pub filler_languages: Vec<String>,
    pub custom_fillers: String,
    pub corrections: Vec<Correction>,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        CleanupConfig {
            remove_fillers: true,
            filler_languages: DEFAULT_FILLER_LANGUAGES
                .iter()
                .map(|code| code.to_string())
                .collect(),
            custom_fillers: String::new(),
            corrections: Vec::new(),
        }
    }
}

/// Fillers first, then corrections. Only dictation is cleaned: spoken rewrite instructions and
/// meeting transcripts are not.
pub fn clean(text: &str, cfg: &CleanupConfig) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut text = text.to_string();
    if cfg.remove_fillers {
        text = remove_fillers(
            &text,
            &filler_words(&cfg.filler_languages, &cfg.custom_fillers),
        );
    }
    apply_corrections(&text, &cfg.corrections)
}

/// Collapses every whitespace run to one space and trims, like Python's `" ".join(s.split())`.
pub fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Python's `str()` for the JSON scalars Ownkey's config can hold.
fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        // Differs from Ownkey on purpose: Python turned a null item into the word "None".
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Unique, non-empty terms in their original order; the first spelling wins case-insensitively.
/// A string is split on commas and newlines; a list is used as is; anything else is empty.
pub fn normalize_vocabulary(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => normalize_vocabulary_str(s),
        Value::Array(items) => dedupe_terms(items.iter().filter_map(value_text)),
        _ => Vec::new(),
    }
}

pub fn normalize_vocabulary_str(text: &str) -> Vec<String> {
    dedupe_terms(text.replace('\n', ",").split(',').map(str::to_string))
}

pub fn normalize_vocabulary_list(items: &[String]) -> Vec<String> {
    dedupe_terms(items.iter().cloned())
}

fn dedupe_terms(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::<String>::new();
    let mut terms = Vec::new();
    for item in items {
        let term = collapse_whitespace(&item);
        if term.is_empty() {
            continue;
        }
        let key = term.to_lowercase();
        if !seen.contains(&key) {
            seen.push(key);
            terms.push(term);
        }
    }
    terms
}

/// Correction rules with both sides filled, different, and one rule per source (first wins,
/// case-insensitively). Items are `{"from", "to"}` objects or two-item lists.
pub fn normalize_corrections(value: &Value) -> Vec<Correction> {
    let Value::Array(items) = value else {
        return Vec::new();
    };
    let pairs = items.iter().filter_map(|item| match item {
        Value::Object(map) => {
            let side = |key: &str| map.get(key).and_then(value_text).unwrap_or_default();
            Some(Correction::new(side("from"), side("to")))
        }
        Value::Array(pair) if pair.len() == 2 => Some(Correction::new(
            value_text(&pair[0]).unwrap_or_default(),
            value_text(&pair[1]).unwrap_or_default(),
        )),
        _ => None,
    });
    dedupe_corrections(pairs)
}

pub fn normalize_corrections_list(rules: &[Correction]) -> Vec<Correction> {
    dedupe_corrections(rules.iter().cloned())
}

fn dedupe_corrections(rules: impl Iterator<Item = Correction>) -> Vec<Correction> {
    let mut seen = Vec::<String>::new();
    let mut result = Vec::new();
    for rule in rules {
        let from = collapse_whitespace(&rule.from);
        let to = collapse_whitespace(&rule.to);
        if from.is_empty() || to.is_empty() || from == to {
            continue;
        }
        let key = from.to_lowercase();
        if !seen.contains(&key) {
            seen.push(key);
            result.push(Correction { from, to });
        }
    }
    result
}

/// Known language codes in order. Defaults only when the setting is absent or malformed: an
/// explicit empty list stays empty, so a user can remove only their own extra words.
pub fn normalize_filler_languages(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => normalize_language_list(items.iter().filter_map(value_text)),
        _ => DEFAULT_FILLER_LANGUAGES
            .iter()
            .map(|code| code.to_string())
            .collect(),
    }
}

fn normalize_language_list(codes: impl Iterator<Item = String>) -> Vec<String> {
    let mut result = Vec::<String>::new();
    for code in codes {
        let code = code.to_lowercase();
        if FILLER_LANGUAGES.iter().any(|(known, _, _)| *known == code) && !result.contains(&code) {
            result.push(code);
        }
    }
    result
}

/// Fillers to remove for the enabled languages, minus words another enabled language needs,
/// then the user's own words, which bypass that protection.
pub fn filler_words<S: AsRef<str>>(languages: &[S], custom: &str) -> Vec<String> {
    let languages = normalize_language_list(languages.iter().map(|code| code.as_ref().to_string()));
    let protected: Vec<&str> = PROTECTED_WORDS
        .iter()
        .filter(|(code, _)| languages.iter().any(|enabled| enabled == code))
        .flat_map(|(_, words)| words.iter().copied())
        .collect();
    let mut words = Vec::<String>::new();
    for code in &languages {
        let Some((_, _, list)) = FILLER_LANGUAGES.iter().find(|(known, _, _)| known == code) else {
            continue;
        };
        for word in list.iter() {
            if !protected.contains(word) && !words.iter().any(|w| w == word) {
                words.push(word.to_string());
            }
        }
    }
    for term in normalize_vocabulary_str(custom) {
        let term = term.to_lowercase();
        if !words.contains(&term) {
            words.push(term);
        }
    }
    words
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `[\w'’-]`: a filler next to one of these is part of a longer word ("uh-huh", "uh’s").
fn is_word_edge(c: char) -> bool {
    is_word_char(c) || c == '\'' || c == '\u{2019}' || c == '-'
}

fn same_char(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

fn matches_at(text: &[char], at: usize, pattern: &[char]) -> bool {
    at + pattern.len() <= text.len()
        && pattern
            .iter()
            .zip(&text[at..])
            .all(|(p, t)| same_char(*p, *t))
}

struct FillerMatch {
    start: usize,
    pre: bool,
    lead_start: usize,
    lead_end: usize,
    post_start: usize,
    end: usize,
}

/// Drops hesitations and repairs the surrounding punctuation and spacing.
///
/// Ownkey's pattern, which this matcher follows step by step:
/// `(,)?(^|\s+)(?<![\w'’-])(A(?:\s+A)*)(?![\w'’-])([,;:.!?…]*)(?=\s|$)` with `A` the escaped
/// fillers, longest first, ignoring case.
pub fn remove_fillers<S: AsRef<str>>(text: &str, words: &[S]) -> String {
    let mut alternatives: Vec<Vec<char>> = words
        .iter()
        .map(|w| w.as_ref().chars().collect::<Vec<_>>())
        .filter(|w| !w.is_empty())
        .collect();
    if text.is_empty() || alternatives.is_empty() {
        return text.to_string();
    }
    // Python sorts by length with a stable sort, so ties keep the list order.
    alternatives.sort_by_key(|w| std::cmp::Reverse(w.len()));

    let mut current = text.to_string();
    let mut removed_any = false;
    loop {
        let next = filler_pass(&current, &alternatives);
        if next == current {
            break;
        }
        removed_any = true;
        current = next;
    }
    if !removed_any {
        // Differs from Ownkey on purpose: without a filler the text is left exactly as it was,
        // instead of losing French spacing before "?" and "!" and doubled spaces.
        return text.to_string();
    }
    finish_marks(&current)
}

fn filler_pass(text: &str, alternatives: &[Vec<char>]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < chars.len() {
        let Some(m) = filler_match_at(&chars, i, alternatives) else {
            i += 1;
            continue;
        };
        out.extend(&chars[copied..m.start]);
        let replacement = filler_replacement(&out, &chars, &m);
        out.push_str(&replacement);
        copied = m.end;
        i = m.end;
    }
    out.extend(&chars[copied..]);
    out
}

fn filler_match_at(chars: &[char], at: usize, alternatives: &[Vec<char>]) -> Option<FillerMatch> {
    // `(,)?` is greedy: the comma is tried first.
    if chars[at] == ',' {
        if let Some(m) = filler_lead(chars, at, at + 1, true, alternatives) {
            return Some(m);
        }
    }
    filler_lead(chars, at, at, false, alternatives)
}

fn filler_lead(
    chars: &[char],
    start: usize,
    lead_start: usize,
    pre: bool,
    alternatives: &[Vec<char>],
) -> Option<FillerMatch> {
    let build = |lead_end: usize, (word_end, post_end): (usize, usize)| FillerMatch {
        start,
        pre,
        lead_start,
        lead_end,
        post_start: word_end,
        end: post_end,
    };
    // `^` comes first in the alternation and matches only at the very start of the text.
    if lead_start == 0 {
        if let Some(found) = filler_words_at(chars, 0, alternatives) {
            return Some(build(0, found));
        }
    }
    let mut run_end = lead_start;
    while run_end < chars.len() && chars[run_end].is_whitespace() {
        run_end += 1;
    }
    for lead_end in (lead_start + 1..=run_end).rev() {
        if lead_end > 0 && is_word_edge(chars[lead_end - 1]) {
            continue;
        }
        if let Some(found) = filler_words_at(chars, lead_end, alternatives) {
            return Some(build(lead_end, found));
        }
    }
    None
}

/// `A(?:\s+A)*` followed by the lookahead, the post punctuation and the trailing lookahead.
/// Returns (end of the fillers, end of the post punctuation).
fn filler_words_at(
    chars: &[char],
    at: usize,
    alternatives: &[Vec<char>],
) -> Option<(usize, usize)> {
    for alternative in alternatives {
        if !matches_at(chars, at, alternative) {
            continue;
        }
        let word_end = at + alternative.len();
        let mut space_end = word_end;
        while space_end < chars.len() && chars[space_end].is_whitespace() {
            space_end += 1;
        }
        // The repetition is greedy: another filler after the whitespace is tried first.
        for next in (word_end + 1..=space_end).rev() {
            if let Some(found) = filler_words_at(chars, next, alternatives) {
                return Some(found);
            }
        }
        if let Some(post_end) = filler_tail(chars, word_end) {
            return Some((word_end, post_end));
        }
    }
    None
}

fn filler_tail(chars: &[char], word_end: usize) -> Option<usize> {
    if word_end < chars.len() && is_word_edge(chars[word_end]) {
        return None;
    }
    let mut post_end = word_end;
    while post_end < chars.len() && POST_PUNCTUATION.contains(&chars[post_end]) {
        post_end += 1;
    }
    (word_end..=post_end)
        .rev()
        .find(|&end| end == chars.len() || chars[end].is_whitespace())
}

fn filler_replacement(out: &str, chars: &[char], m: &FillerMatch) -> String {
    // Differs from Ownkey on purpose: "before" is the text this pass has already produced, not
    // the text it started from, so a filler removed earlier in the same pass counts as gone.
    // Ownkey left a stray "." for "Uh, um." and a lowercase word after "Uh. uh. hello".
    let before = out.trim_end();
    let lead: String = chars[m.lead_start..m.lead_end].iter().collect();
    let line_start = lead.contains('\n');
    let mut sentence_start =
        line_start || before.is_empty() || before.ends_with(SENTENCE_END) || before.ends_with(MARK);
    // Differs from Ownkey on purpose: the whole run of sentence punctuation stays, so "uh..."
    // keeps its ellipsis and "uh?!" keeps both marks, where Ownkey kept only the first character.
    let ending: String = chars[m.post_start..m.end]
        .iter()
        .filter(|c| SENTENCE_END.contains(c))
        .collect();
    if m.pre && !line_start {
        sentence_start = false;
    }
    let comma = if m.pre { "," } else { "" };
    if line_start {
        // Keep the line break; the word that now opens the line gets its capital.
        format!("{comma}{lead}{MARK}")
    } else if sentence_start {
        // Mark the spot so the word that now opens the sentence gets its capital.
        let lead = if before.is_empty() { "" } else { lead.as_str() };
        format!("{lead}{MARK}")
    } else if !ending.is_empty() {
        ending
    } else {
        comma.to_string()
    }
}

/// Capitalizes the word after each mark and removes the marks with the whitespace they leave.
fn finish_marks(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != MARK {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut run_end = i;
        let mut breaks = String::new();
        while run_end < chars.len() && (chars[run_end] == MARK || chars[run_end].is_whitespace()) {
            if chars[run_end] == '\n' || chars[run_end] == '\r' {
                breaks.push(chars[run_end]);
            }
            run_end += 1;
        }
        let kept = out.trim_end_matches([' ', '\t']).len();
        let at_line_start = kept == 0 || out[..kept].ends_with('\n');
        if at_line_start {
            // The filler opened a line, so its indentation and the break after it go with it.
            out.truncate(kept);
        } else if !breaks.is_empty() {
            // Differs from Ownkey on purpose: a filler at the end of a sentence no longer takes
            // the line break after it along ("Hello. Um\nworld" became "Hello. World").
            out.truncate(kept);
            out.push_str(&breaks);
        }
        let mut word_end = run_end;
        while word_end < chars.len() && !chars[word_end].is_whitespace() && chars[word_end] != MARK
        {
            word_end += 1;
        }
        out.push_str(&capitalize(&chars[run_end..word_end]));
        i = word_end;
    }
    out.trim().to_string()
}

/// Capitalizes a plain lowercase word and leaves intentional casing such as iPhone alone.
fn capitalize(word: &[char]) -> String {
    let Some((first, rest)) = word.split_first() else {
        return String::new();
    };
    if !first.is_lowercase() || rest.iter().any(|c| c.is_uppercase()) {
        return word.iter().collect();
    }
    let mut upper = first.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) => std::iter::once(single)
            .chain(rest.iter().copied())
            .collect(),
        // Differs from Ownkey on purpose: "ße" stays as it is instead of growing into "SSe".
        _ => word.iter().collect(),
    }
}

/// Replaces whole-word misspellings with their preferred spelling, ignoring case. Rules run in
/// order on the result of the previous rule, and the replacement is inserted literally.
pub fn apply_corrections(text: &str, rules: &[Correction]) -> String {
    let mut text = text.to_string();
    for rule in normalize_corrections_list(rules) {
        text = replace_whole_word(&text, &rule.from, &rule.to);
    }
    text
}

fn replace_whole_word(text: &str, from: &str, to: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let pattern: Vec<char> = from.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let starts_word = i == 0 || !is_word_char(chars[i - 1]);
        let end = i + pattern.len();
        if starts_word
            && matches_at(&chars, i, &pattern)
            && (end == chars.len() || !is_word_char(chars[end]))
        {
            out.push_str(to);
            i = end;
        } else {
            // A rejected candidate moves on by one character, as the regex engine does.
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}
