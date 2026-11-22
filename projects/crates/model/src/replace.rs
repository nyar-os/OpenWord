//! Case-, word- and pattern-aware text replacement.
//!
//! The replacement walks the same text surfaces as [`document_ir::find`] —
//! section titles, paragraphs, list items, table cell paragraphs and authored
//! inline display text — so a query that can be found can also be replaced.
//! Matching stays inside one inline run, so styles and links are never split.

use document_ir::{Block, DocumentGraph, Inline};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

use crate::ServiceError;

/// Parsed replacement request.
#[derive(Clone)]
pub(crate) struct Request {
    pub(crate) find: String,
    pub(crate) replacement: String,
    pub(crate) case_sensitive: bool,
    pub(crate) whole_word: bool,
    /// Compiled pattern when `regex` was requested; literal matching otherwise.
    pub(crate) regex: Option<Regex>,
    pub(crate) limit: Option<usize>,
    pub(crate) expected_revision: Option<u64>,
}

impl Request {
    /// Parses and validates one replacement request.
    pub(crate) fn parse(request: &Value) -> Result<Self, ServiceError> {
        let find = match request.get("find") {
            Some(Value::String(text)) => text.clone(),
            Some(_) => return Err(crate::invalid("`find` must be a string")),
            None => return Err(crate::invalid("`find` must be a non-empty string")),
        };
        if find.is_empty() {
            return Err(crate::invalid("`find` must be a non-empty string"));
        }
        let replacement = match request.get("replace") {
            Some(Value::String(text)) => text.clone(),
            Some(_) => return Err(crate::invalid("`replace` must be a string")),
            None => return Err(crate::invalid("`replace` must be a string")),
        };
        let case_sensitive = optional_bool(request, "caseSensitive")?.unwrap_or(true);
        let whole_word = optional_bool(request, "wholeWord")?.unwrap_or(false);
        let pattern = optional_bool(request, "regex")?.unwrap_or(false);
        if pattern && whole_word {
            return Err(crate::invalid("`wholeWord` cannot be combined with `regex`; add the boundary to the pattern"));
        }
        let regex = if pattern {
            Some(
                RegexBuilder::new(&find)
                    .case_insensitive(!case_sensitive)
                    .build()
                    .map_err(|error| crate::invalid(format!("`find` is not a valid regular expression: {error}")))?,
            )
        }
        else {
            None
        };
        Ok(Self {
            find,
            replacement,
            case_sensitive,
            whole_word,
            regex,
            limit: optional_limit(request)?,
            expected_revision: optional_revision(request)?,
        })
    }
}

/// What one replacement pass changed.
pub(crate) struct Outcome {
    pub(crate) replaced: usize,
    pub(crate) affected: Vec<Value>,
}

/// Rewrites every match in document order, honouring the optional total limit.
pub(crate) fn apply(graph: &mut DocumentGraph, request: &Request) -> Outcome {
    let matcher = Matcher {
        find: &request.find,
        case_sensitive: request.case_sensitive,
        whole_word: request.whole_word,
        regex: request.regex.as_ref(),
    };
    let mut budget = request.limit.unwrap_or(usize::MAX);
    let mut replaced = 0;
    let mut affected = Vec::new();
    for (index, id) in document_ir::flat_order(graph).into_iter().enumerate() {
        if budget == 0 {
            break;
        }
        let Some(node) = graph.blocks.iter_mut().find(|node| node.id == id)
        else {
            continue;
        };
        let count = match &mut node.block {
            Block::Section { title, .. } => replace_in_inlines(title, &matcher, &request.replacement, &mut budget),
            Block::Paragraph { content } => replace_in_inlines(content, &matcher, &request.replacement, &mut budget),
            Block::List { items, .. } => items
                .iter_mut()
                .map(|item| replace_in_inlines(&mut item.content, &matcher, &request.replacement, &mut budget))
                .sum(),
            _ => 0,
        };
        if count > 0 {
            affected.push(json!({
                "blockId": document_ir::block_address(&node.block, id),
                "blockIndex": index,
                "replaced": count,
            }));
            replaced += count;
        }
    }
    Outcome { replaced, affected }
}

/// One matcher shared by every text surface.
struct Matcher<'a> {
    find: &'a str,
    case_sensitive: bool,
    whole_word: bool,
    regex: Option<&'a Regex>,
}

impl Matcher<'_> {
    /// Byte length of the match starting at `start`, when one begins there.
    fn match_len(&self, text: &str, start: usize) -> Option<usize> {
        let rest = &text[start..];
        if self.case_sensitive {
            return rest.starts_with(self.find).then_some(self.find.len());
        }
        let mut length = 0;
        let mut chars = rest.chars();
        for query in self.find.chars() {
            let found = chars.next()?;
            if !char_eq_ci(found, query) {
                return None;
            }
            length += found.len_utf8();
        }
        Some(length)
    }

    /// Whether the match respects the optional whole-word boundary rule.
    fn whole_word_ok(&self, text: &str, start: usize, end: usize) -> bool {
        if !self.whole_word {
            return true;
        }
        if self.find.chars().next().is_some_and(is_word_char)
            && text[..start].chars().next_back().is_some_and(is_word_char)
        {
            return false;
        }
        if self.find.chars().next_back().is_some_and(is_word_char) && text[end..].chars().next().is_some_and(is_word_char) {
            return false;
        }
        true
    }
}

fn replace_in_inlines(inlines: &mut [Inline], matcher: &Matcher<'_>, replacement: &str, budget: &mut usize) -> usize {
    let mut replaced = 0;
    for inline in inlines {
        if *budget == 0 {
            break;
        }
        match inline {
            Inline::Text { text } | Inline::InlineCode { text } => {
                replaced += rewrite(text, matcher, replacement, budget);
            }
            Inline::Reference { display, .. } => replaced += rewrite(display, matcher, replacement, budget),
            Inline::Styled { children, .. } | Inline::Attributed { children, .. } => {
                replaced += replace_in_inlines(children, matcher, replacement, budget);
            }
            Inline::Link { display, .. } => replaced += replace_in_inlines(display, matcher, replacement, budget),
            Inline::Image { alt, .. } => replaced += replace_in_inlines(alt, matcher, replacement, budget),
            Inline::Note { display: Some(display), .. } | Inline::Citation { display: Some(display), .. } => {
                replaced += replace_in_inlines(display, matcher, replacement, budget);
            }
            _ => {}
        }
    }
    replaced
}

/// Rewrites one text run in place, returning how many matches were replaced.
fn rewrite(text: &mut String, matcher: &Matcher<'_>, replacement: &str, budget: &mut usize) -> usize {
    if *budget == 0 {
        return 0;
    }
    if let Some(regex) = matcher.regex {
        let mut output = String::with_capacity(text.len());
        let mut last = 0;
        let mut replaced = 0;
        for capture in regex.captures_iter(text) {
            if replaced >= *budget {
                break;
            }
            let whole = capture.get(0).expect("group 0 always matches");
            output.push_str(&text[last..whole.start()]);
            capture.expand(replacement, &mut output);
            last = whole.end();
            replaced += 1;
        }
        if replaced == 0 {
            return 0;
        }
        output.push_str(&text[last..]);
        *text = output;
        *budget -= replaced;
        return replaced;
    }
    let mut output = String::with_capacity(text.len());
    let mut last = 0;
    let mut cursor = 0;
    let mut replaced = 0;
    while cursor < text.len() && *budget > 0 {
        match matcher.match_len(text, cursor) {
            Some(length) if matcher.whole_word_ok(text, cursor, cursor + length) => {
                output.push_str(&text[last..cursor]);
                output.push_str(replacement);
                cursor += length;
                last = cursor;
                *budget -= 1;
                replaced += 1;
            }
            _ => {
                let step = text[cursor..].chars().next().expect("cursor stays on a character boundary").len_utf8();
                cursor += step;
            }
        }
    }
    if replaced == 0 {
        return 0;
    }
    output.push_str(&text[last..]);
    *text = output;
    replaced
}

/// Case-insensitive comparison of one character pair.
///
/// Comparing the lowercase sequences keeps multi-character mappings such as
/// `İ` consistent instead of truncating them to one scalar.
fn char_eq_ci(left: char, right: char) -> bool {
    left == right || left.to_lowercase().eq(right.to_lowercase())
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn optional_bool(request: &Value, key: &str) -> Result<Option<bool>, ServiceError> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(crate::invalid(format!("`{key}` must be a boolean"))),
    }
}

fn optional_limit(request: &Value) -> Result<Option<usize>, ServiceError> {
    match request.get("limit") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => match number.as_u64() {
            Some(0) | None => Err(crate::invalid("`limit` must be a positive integer")),
            Some(value) => usize::try_from(value)
                .map(Some)
                .map_err(|_| crate::invalid("`limit` must be a positive integer")),
        },
        Some(_) => Err(crate::invalid("`limit` must be a positive integer")),
    }
}

fn optional_revision(request: &Value) -> Result<Option<u64>, ServiceError> {
    match request.get("expectedRevision") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| crate::invalid("`expectedRevision` must be a non-negative integer")),
        Some(_) => Err(crate::invalid("`expectedRevision` must be a non-negative integer")),
    }
}
