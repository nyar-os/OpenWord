//! Document statistics over one committed graph.
//!
//! The walk follows the same depth-first order the summary and search use, so
//! every count describes the document in reading order. Text is collected from
//! section titles, paragraphs, list items, code blocks and table cells; table
//! cell text lives in child paragraph nodes, which are visited once.

use std::collections::BTreeMap;

use document_ir::{Block, DocumentGraph};
use serde_json::{Map, Value, json};

use crate::{search, summary};

/// Reading speed used by `readingMinutes`.
const WORDS_PER_MINUTE: usize = 200;

/// Projects one graph into the statistics report.
pub(crate) fn project(graph: &DocumentGraph) -> Value {
    let mut blocks = 0usize;
    let mut paragraphs = 0usize;
    let mut tables = 0usize;
    let mut table_rows = 0usize;
    let mut headings = BTreeMap::<u8, usize>::new();
    let mut text = TextStats::default();

    for id in search::flat_order(graph) {
        let Some(node) = graph.blocks.iter().find(|node| node.id == id)
        else {
            continue;
        };
        blocks += 1;
        match &node.block {
            Block::Section { level, title, .. } => {
                *headings.entry(*level).or_insert(0) += 1;
                text.push(&summary::flatten(title));
            }
            Block::Paragraph { content } => {
                paragraphs += 1;
                text.push(&summary::flatten(content));
            }
            Block::List { items, .. } => {
                for item in items {
                    text.push(&summary::flatten(&item.content));
                }
            }
            Block::Code { content, .. } => text.push(content),
            Block::Table { head, body, foot, .. } => {
                tables += 1;
                table_rows += head.len() + body.len() + foot.len();
            }
            _ => {}
        }
    }

    let mut levels = Map::new();
    for (level, count) in headings {
        levels.insert(level.to_string(), json!(count));
    }

    json!({
        "revision": graph.revision,
        "blocks": blocks,
        "paragraphs": paragraphs,
        "headings": levels,
        "tables": tables,
        "tableRows": table_rows,
        "words": text.words,
        "characters": text.characters,
        "charactersWithoutWhitespace": text.characters_without_whitespace,
        "readingMinutes": text.words.div_ceil(WORDS_PER_MINUTE),
    })
}

#[derive(Default)]
struct TextStats {
    words: usize,
    characters: usize,
    characters_without_whitespace: usize,
}

impl TextStats {
    fn push(&mut self, text: &str) {
        self.words += text.split_whitespace().count();
        self.characters += text.chars().count();
        self.characters_without_whitespace += text.chars().filter(|character| !character.is_whitespace()).count();
    }
}
