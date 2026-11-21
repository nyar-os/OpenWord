//! Atomic edit operations.
//!
//! A request is parsed and validated as a whole before the first mutation, so a
//! rejected operation never leaves a half-applied revision. Each applied
//! operation advances the revision exactly once.

use document_ir::{Block, DocumentGraph, Inline, TableCell, TableRow};
use serde_json::Value;

use crate::ServiceError;

enum Operation {
    Paragraph(String),
    Heading { level: u8, text: String },
    Table(Vec<Vec<String>>),
    Metadata { title: Option<String>, author: Option<String> },
}

/// Parses, validates and applies one edit request.
pub(crate) fn apply(graph: &mut DocumentGraph, request: &Value) -> Result<(), ServiceError> {
    let expected = expected_revision(request)?;
    let operations = request
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| crate::invalid("`operations` must be an array of edit operations"))?;
    if operations.is_empty() {
        return Err(crate::invalid("`operations` must contain at least one edit operation"));
    }
    let parsed = operations.iter().map(parse).collect::<Result<Vec<_>, _>>()?;

    if let Some(expected) = expected
        && expected != graph.revision
    {
        return Err(ServiceError::new(
            "document/revision-conflict",
            format!("expected revision {expected} but the document is at revision {}", graph.revision),
        ));
    }

    for operation in parsed {
        apply_one(graph, operation);
    }
    Ok(())
}

fn expected_revision(request: &Value) -> Result<Option<u64>, ServiceError> {
    match request.get("expectedRevision") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => {
            number.as_u64().map(Some).ok_or_else(|| crate::invalid("`expectedRevision` must be a non-negative integer"))
        }
        Some(_) => Err(crate::invalid("`expectedRevision` must be a non-negative integer")),
    }
}

fn parse(operation: &Value) -> Result<Operation, ServiceError> {
    let kind = operation
        .get("type")
        .or_else(|| operation.get("op"))
        .and_then(Value::as_str)
        .ok_or_else(|| crate::invalid("each edit operation needs a `type` string"))?;

    match kind {
        "paragraph.append" => Ok(Operation::Paragraph(required_text(operation, "text")?)),
        "heading.append" => {
            let level = operation
                .get("level")
                .and_then(Value::as_u64)
                .ok_or_else(|| crate::invalid("`heading.append` needs a numeric `level`"))?;
            if !(1..=6).contains(&level) {
                return Err(crate::invalid("`level` must be between 1 and 6"));
            }
            Ok(Operation::Heading { level: level as u8, text: required_text(operation, "text")? })
        }
        "table.append" => Ok(Operation::Table(required_rows(operation)?)),
        "metadata.set" => {
            let title = optional_text(operation, "title")?;
            let author = optional_text(operation, "author")?;
            if title.is_none() && author.is_none() {
                return Err(crate::invalid("`metadata.set` needs `title` or `author`"));
            }
            Ok(Operation::Metadata { title, author })
        }
        other => Err(ServiceError::new("object/unsupported", format!("edit operation `{other}` is not supported"))),
    }
}

fn required_text(operation: &Value, key: &str) -> Result<String, ServiceError> {
    optional_text(operation, key)?.ok_or_else(|| crate::invalid(format!("this edit operation needs `{key}`")))
}

fn optional_text(operation: &Value, key: &str) -> Result<Option<String>, ServiceError> {
    match operation.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(crate::invalid(format!("`{key}` must be a string"))),
    }
}

fn required_rows(operation: &Value) -> Result<Vec<Vec<String>>, ServiceError> {
    let rows =
        operation.get("rows").and_then(Value::as_array).ok_or_else(|| crate::invalid("`table.append` needs a `rows` array"))?;
    if rows.is_empty() {
        return Err(crate::invalid("`rows` must contain at least one row"));
    }
    rows.iter()
        .map(|row| {
            let cells = row.as_array().ok_or_else(|| crate::invalid("each table row must be an array of strings"))?;
            if cells.is_empty() {
                return Err(crate::invalid("each table row needs at least one cell"));
            }
            cells
                .iter()
                .map(|cell| cell.as_str().map(str::to_string).ok_or_else(|| crate::invalid("each table cell must be a string")))
                .collect()
        })
        .collect()
}

fn apply_one(graph: &mut DocumentGraph, operation: Operation) {
    match operation {
        Operation::Paragraph(text) => append_block(graph, Block::Paragraph { content: vec![Inline::Text { text }] }),
        Operation::Heading { level, text } => {
            append_block(graph, Block::Section { level, title: vec![Inline::Text { text }], children: Vec::new() });
        }
        Operation::Table(rows) => {
            let total = rows.len();
            let mut head = Vec::new();
            let mut body = Vec::new();
            for (index, row) in rows.into_iter().enumerate() {
                let cells = row
                    .into_iter()
                    .map(|text| {
                        let child = graph.push_block(Block::Paragraph { content: vec![Inline::Text { text }] });
                        TableCell { children: vec![child], colspan: 1, rowspan: 1 }
                    })
                    .collect();
                let row = TableRow { cells };
                if index == 0 && total > 1 {
                    head.push(row);
                }
                else {
                    body.push(row);
                }
            }
            append_block(graph, Block::Table { caption: None, head, body, foot: Vec::new() });
        }
        Operation::Metadata { title, author } => {
            if let Some(title) = title {
                graph.metadata.title = Some(title);
            }
            if let Some(author) = author {
                graph.metadata.authors = vec![author];
            }
            graph.bump_revision();
        }
    }
}

/// Appends to the last section when the document has one, otherwise at top level.
fn append_block(graph: &mut DocumentGraph, block: Block) {
    let section = graph.blocks.iter().rev().find_map(|node| match node.block {
        Block::Section { .. } => Some(node.id),
        _ => None,
    });
    let id = graph.push_block(block);
    match section {
        Some(section) => {
            let _ = graph.append_child(section, id);
        }
        None => graph.bump_revision(),
    }
}
