//! Atomic edit operations.
//!
//! A request is parsed and validated as a whole before the first mutation, so a
//! rejected operation never leaves a half-applied revision. Each applied
//! operation advances the revision exactly once.

use std::collections::{HashMap, HashSet};

use document_ir::{Block, DocumentGraph, Inline, ListItem, NodeId, TableCell, TableRow};
use serde_json::{Value, json};

use crate::ServiceError;

#[derive(Clone)]
enum Operation {
    Paragraph(String),
    Heading { level: u8, text: String },
    List { ordered: bool, items: Vec<String>, start: Option<u32> },
    Quote(String),
    Code { text: String, language: Option<String> },
    Table(Vec<Vec<String>>),
    Metadata { title: Option<String>, author: Option<String> },
    ReplaceText { find: String, replace: String, limit: Option<usize> },
    RemoveBlock { id: NodeId },
    InsertParagraph { index: usize, text: String },
    SetCell { table: usize, row: usize, column: usize, text: String },
    InsertRow { table: usize, row: usize, cells: Option<Vec<String>> },
    DeleteRow { table: usize, row: usize },
    InsertColumn { table: usize, column: usize, cells: Option<Vec<String>> },
    DeleteColumn { table: usize, column: usize },
    UpdateParagraph { target: Target, text: String },
    MoveBlock { id: NodeId, index: usize },
    DuplicateBlock { id: NodeId },
    TemplateFill { values: Vec<(String, String)> },
}

#[derive(Clone)]
enum Target {
    Id(NodeId),
    Index(usize),
}

/// Parses, validates and applies one edit request, returning one report per
/// applied operation.
pub(crate) fn apply(graph: &mut DocumentGraph, request: &Value) -> Result<Vec<Value>, ServiceError> {
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

    // Operations that depend on graph state (a block id, an index, a table
    // coordinate) are replayed against a staging copy first, so a batch that
    // names something missing leaves the committed document untouched.
    let mut staged = graph.clone();
    for operation in &parsed {
        apply_one(&mut staged, operation.clone())?;
    }

    let mut results = Vec::with_capacity(parsed.len());
    for operation in parsed {
        results.push(apply_one(graph, operation)?);
    }
    Ok(results)
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
        "list.append" => {
            let ordered = optional_bool(operation, "ordered")?.unwrap_or(false);
            let items = required_items(operation)?;
            let start = parse_list_start(operation, ordered, items.len())?;
            Ok(Operation::List { ordered, items, start })
        }
        "quote.append" => {
            let text = required_text(operation, "text")?;
            if text.is_empty() {
                return Err(crate::invalid("`quote.append` needs a non-empty `text`"));
            }
            Ok(Operation::Quote(text))
        }
        "code.append" => {
            let text = required_text(operation, "text")?;
            if text.is_empty() {
                return Err(crate::invalid("`code.append` needs a non-empty `text`"));
            }
            let language = optional_text(operation, "language")?.filter(|value| !value.trim().is_empty());
            Ok(Operation::Code { text, language })
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
        "document.replaceText" => {
            let find = required_text(operation, "find")?;
            if find.is_empty() {
                return Err(crate::invalid("`find` must be a non-empty string"));
            }
            Ok(Operation::ReplaceText {
                find,
                replace: required_text(operation, "replace")?,
                limit: optional_index(operation, "limit")?,
            })
        }
        "block.remove" => {
            let raw = required_text(operation, "id")?;
            let id = document_ir::parse_block_address(&raw)
                .ok_or_else(|| crate::invalid("`id` must be a block id such as `node-3` or `section-1`"))?;
            Ok(Operation::RemoveBlock { id })
        }
        "paragraph.insert" => Ok(Operation::InsertParagraph {
            index: required_index(operation, "index")?,
            text: required_text(operation, "text")?,
        }),
        "table.setCell" => Ok(Operation::SetCell {
            table: required_index(operation, "table")?,
            row: required_index(operation, "row")?,
            column: required_index(operation, "column")?,
            text: required_text(operation, "text")?,
        }),
        "table.insertRow" => Ok(Operation::InsertRow {
            table: required_index(operation, "table")?,
            row: required_index(operation, "row")?,
            cells: optional_cells(operation)?,
        }),
        "table.deleteRow" => {
            Ok(Operation::DeleteRow { table: required_index(operation, "table")?, row: required_index(operation, "row")? })
        }
        "table.insertColumn" => Ok(Operation::InsertColumn {
            table: required_index(operation, "table")?,
            column: required_index(operation, "column")?,
            cells: optional_cells(operation)?,
        }),
        "table.deleteColumn" => Ok(Operation::DeleteColumn {
            table: required_index(operation, "table")?,
            column: required_index(operation, "column")?,
        }),
        "paragraph.update" => {
            Ok(Operation::UpdateParagraph { target: paragraph_target(operation)?, text: required_text(operation, "text")? })
        }
        "block.move" => Ok(Operation::MoveBlock { id: parse_block_id(operation)?, index: required_index(operation, "index")? }),
        "block.duplicate" => Ok(Operation::DuplicateBlock { id: parse_block_id(operation)? }),
        "template.fill" => Ok(Operation::TemplateFill { values: required_values(operation)? }),
        other => Err(ServiceError::new("object/unsupported", format!("edit operation `{other}` is not supported"))),
    }
}

fn parse_block_id(operation: &Value) -> Result<NodeId, ServiceError> {
    let raw = required_text(operation, "id")?;
    document_ir::parse_block_address(&raw).ok_or_else(|| crate::invalid("`id` must be a block id such as `node-3` or `section-1`"))
}

fn paragraph_target(operation: &Value) -> Result<Target, ServiceError> {
    if let Some(raw) = optional_text(operation, "id")? {
        return document_ir::parse_block_address(&raw)
            .map(Target::Id)
            .ok_or_else(|| crate::invalid("`id` must be a block id such as `node-3` or `section-1`"));
    }
    match optional_index(operation, "index")? {
        Some(index) => Ok(Target::Index(index)),
        None => Err(crate::invalid("`paragraph.update` needs `id` or `index`")),
    }
}

fn optional_cells(operation: &Value) -> Result<Option<Vec<String>>, ServiceError> {
    match operation.get("cells") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|cell| cell.as_str().map(str::to_string).ok_or_else(|| crate::invalid("each table cell must be a string")))
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(crate::invalid("`cells` must be an array of strings")),
    }
}

fn required_values(operation: &Value) -> Result<Vec<(String, String)>, ServiceError> {
    let values = operation
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| crate::invalid("`template.fill` needs a `values` object"))?;
    if values.is_empty() {
        return Err(crate::invalid("`values` must contain at least one placeholder value"));
    }
    values
        .iter()
        .map(|(key, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                Value::Bool(flag) => flag.to_string(),
                _ => return Err(crate::invalid(format!("`values.{key}` must be a string, number or boolean"))),
            };
            Ok((key.clone(), text))
        })
        .collect()
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

fn optional_index(operation: &Value, key: &str) -> Result<Option<usize>, ServiceError> {
    match operation.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(|value| Some(value as usize))
            .ok_or_else(|| crate::invalid(format!("`{key}` must be a non-negative integer"))),
        Some(_) => Err(crate::invalid(format!("`{key}` must be a non-negative integer"))),
    }
}

fn required_index(operation: &Value, key: &str) -> Result<usize, ServiceError> {
    optional_index(operation, key)?
        .ok_or_else(|| crate::invalid(format!("this edit operation needs a non-negative integer `{key}`")))
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

fn required_items(operation: &Value) -> Result<Vec<String>, ServiceError> {
    let items = operation
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| crate::invalid("`list.append` needs an `items` array"))?;
    if items.is_empty() {
        return Err(crate::invalid("`items` must contain at least one list item"));
    }
    items
        .iter()
        .map(|item| item.as_str().map(str::to_string).ok_or_else(|| crate::invalid("each list item must be a string")))
        .collect()
}

fn optional_bool(operation: &Value, key: &str) -> Result<Option<bool>, ServiceError> {
    match operation.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(crate::invalid(format!("`{key}` must be a boolean"))),
    }
}

/// Parses the optional first number of an ordered list.
///
/// `start` is only meaningful for ordered lists; unordered lists always keep
/// `number = None`. A valid start is a positive integer whose numbered range
/// still fits in `u32`, because item numbers are stored as `u32`.
fn parse_list_start(operation: &Value, ordered: bool, items: usize) -> Result<Option<u32>, ServiceError> {
    let Some(start) = optional_index(operation, "start")?
    else {
        return Ok(None);
    };
    if !ordered {
        return Err(crate::invalid("`start` is only valid for ordered lists"));
    }
    if start == 0 || start > u32::MAX as usize || start + items - 1 > u32::MAX as usize {
        return Err(crate::invalid("`start` must be a positive integer and the numbered range must fit in u32"));
    }
    Ok(Some(start as u32))
}

fn apply_one(graph: &mut DocumentGraph, operation: Operation) -> Result<Value, ServiceError> {
    match operation {
        Operation::Paragraph(text) => {
            append_block(graph, Block::Paragraph { content: vec![Inline::Text { text }] });
            Ok(json!({ "type": "paragraph.append" }))
        }
        Operation::Heading { level, text } => {
            append_block(graph, Block::Section { level, title: vec![Inline::Text { text }], children: Vec::new() });
            Ok(json!({ "type": "heading.append" }))
        }
        Operation::List { ordered, items, start } => {
            let count = items.len();
            append_list(graph, ordered, items, start);
            Ok(json!({ "type": "list.append", "ordered": ordered, "items": count, "start": start }))
        }
        Operation::Quote(text) => {
            append_quote(graph, text);
            Ok(json!({ "type": "quote.append" }))
        }
        Operation::Code { text, language } => {
            append_block(graph, Block::Code { language: language.clone(), content: text });
            Ok(json!({ "type": "code.append", "language": language }))
        }
        Operation::Table(rows) => {
            append_table(graph, rows);
            Ok(json!({ "type": "table.append" }))
        }
        Operation::Metadata { title, author } => {
            if let Some(title) = title {
                graph.metadata.title = Some(title);
            }
            if let Some(author) = author {
                graph.metadata.authors = vec![author];
            }
            graph.bump_revision();
            Ok(json!({ "type": "metadata.set" }))
        }
        Operation::ReplaceText { find, replace, limit } => {
            let replaced = replace_text(graph, &find, &replace, limit);
            graph.bump_revision();
            Ok(json!({ "type": "document.replaceText", "replaced": replaced }))
        }
        Operation::RemoveBlock { id } => {
            let label = graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id));
            let removed = remove_block(graph, id)?;
            graph.bump_revision();
            Ok(json!({ "type": "block.remove", "id": label, "removed": removed }))
        }
        Operation::InsertParagraph { index, text } => {
            let id = insert_paragraph(graph, index, text)?;
            graph.bump_revision();
            Ok(json!({ "type": "paragraph.insert", "id": format!("node-{}", id.0) }))
        }
        Operation::SetCell { table, row, column, text } => {
            set_cell(graph, table, row, column, text)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.setCell", "table": table, "row": row, "column": column }))
        }
        Operation::InsertRow { table, row, cells } => {
            insert_row(graph, table, row, cells)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.insertRow", "table": table, "row": row }))
        }
        Operation::DeleteRow { table, row } => {
            delete_row(graph, table, row)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.deleteRow", "table": table, "row": row }))
        }
        Operation::InsertColumn { table, column, cells } => {
            insert_column(graph, table, column, cells)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.insertColumn", "table": table, "column": column }))
        }
        Operation::DeleteColumn { table, column } => {
            delete_column(graph, table, column)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.deleteColumn", "table": table, "column": column }))
        }
        Operation::UpdateParagraph { target, text } => {
            let label = update_paragraph(graph, target, text)?;
            graph.bump_revision();
            Ok(json!({ "type": "paragraph.update", "id": label }))
        }
        Operation::MoveBlock { id, index } => {
            let label = graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id));
            move_block(graph, id, index)?;
            graph.bump_revision();
            Ok(json!({ "type": "block.move", "id": label, "index": index }))
        }
        Operation::DuplicateBlock { id } => {
            let label = graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id));
            let new_id = duplicate_block(graph, id)?;
            graph.bump_revision();
            let index = document_ir::flat_order(graph).iter().position(|node| *node == new_id).unwrap_or_default();
            Ok(json!({ "type": "block.duplicate", "id": label, "newId": format!("node-{}", new_id.0), "index": index }))
        }
        Operation::TemplateFill { values } => {
            let replaced = fill_template(graph, &values);
            graph.bump_revision();
            Ok(json!({ "type": "template.fill", "replaced": replaced }))
        }
    }
}

/// Appends one list block whose items carry plain-text content. Ordered items
/// record explicit numbers so numbering survives export and edit; numbering
/// begins at `start` (1 when omitted) and increases by one per item.
fn append_list(graph: &mut DocumentGraph, ordered: bool, items: Vec<String>, start: Option<u32>) {
    let first = start.unwrap_or(1);
    let items = items
        .into_iter()
        .enumerate()
        .map(|(index, text)| ListItem {
            content: vec![Inline::Text { text }],
            children: Vec::new(),
            number: ordered.then_some(first + index as u32),
            is_task: false,
            is_checked: None,
        })
        .collect();
    append_block(graph, Block::List { ordered, items });
}

/// Appends one quote block wrapping a single paragraph child.
fn append_quote(graph: &mut DocumentGraph, text: String) {
    let child = graph.push_block(Block::Paragraph { content: vec![Inline::Text { text }] });
    append_block(graph, Block::Quote { children: vec![child] });
}

fn append_table(graph: &mut DocumentGraph, rows: Vec<Vec<String>>) {
    let total = rows.len();
    let mut head = Vec::new();
    let mut body = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        let cells = row
            .into_iter()
            .map(|text| {
                let child = graph.push_block(Block::Paragraph { content: vec![Inline::Text { text }] });
                document_ir::TableCell { children: vec![child], colspan: 1, rowspan: 1 }
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

/// Replaces plain-text occurrences across titles, paragraphs and list items in
/// document order, honouring an optional total limit.
fn replace_text(graph: &mut DocumentGraph, find: &str, replace: &str, limit: Option<usize>) -> usize {
    let mut budget = limit.unwrap_or(usize::MAX);
    let mut replaced = 0;
    for id in document_ir::flat_order(graph) {
        if budget == 0 {
            break;
        }
        let Some(node) = graph.blocks.iter_mut().find(|node| node.id == id)
        else {
            continue;
        };
        replaced += match &mut node.block {
            Block::Section { title, .. } => replace_in_inlines(title, find, replace, &mut budget),
            Block::Paragraph { content } => replace_in_inlines(content, find, replace, &mut budget),
            Block::List { items, .. } => {
                items.iter_mut().map(|item| replace_in_inlines(&mut item.content, find, replace, &mut budget)).sum()
            }
            _ => 0,
        };
    }
    replaced
}

fn replace_in_inlines(inlines: &mut [Inline], find: &str, replace: &str, budget: &mut usize) -> usize {
    let mut replaced = 0;
    for inline in inlines {
        match inline {
            Inline::Text { text } => {
                let (rewritten, count) = replace_in_text(text, find, replace, budget);
                if count > 0 {
                    *text = rewritten;
                }
                replaced += count;
            }
            Inline::Styled { children, .. } | Inline::Attributed { children, .. } => {
                replaced += replace_in_inlines(children, find, replace, budget);
            }
            Inline::Link { display, .. } => replaced += replace_in_inlines(display, find, replace, budget),
            Inline::Image { alt, .. } => replaced += replace_in_inlines(alt, find, replace, budget),
            Inline::Note { display: Some(display), .. } | Inline::Citation { display: Some(display), .. } => {
                replaced += replace_in_inlines(display, find, replace, budget);
            }
            _ => {}
        }
    }
    replaced
}

/// Replaces whole matches inside one text run. `match_indices` yields byte
/// offsets on character boundaries, so multi-byte text never splits a scalar.
fn replace_in_text(text: &str, find: &str, replace: &str, budget: &mut usize) -> (String, usize) {
    if *budget == 0 {
        return (text.to_string(), 0);
    }
    let mut output = String::with_capacity(text.len());
    let mut last = 0;
    let mut replaced = 0;
    for (start, matched) in text.match_indices(find) {
        if *budget == 0 {
            break;
        }
        output.push_str(&text[last..start]);
        output.push_str(replace);
        last = start + matched.len();
        *budget -= 1;
        replaced += 1;
    }
    if replaced == 0 {
        return (text.to_string(), 0);
    }
    output.push_str(&text[last..]);
    (output, replaced)
}

/// Substitutes `{{key}}` placeholders across titles, paragraphs, list items and
/// table cells (cell text is stored in child paragraphs) in document order.
/// Unknown placeholders stay verbatim.
fn fill_template(graph: &mut DocumentGraph, values: &[(String, String)]) -> usize {
    let mut replaced = 0;
    for id in document_ir::flat_order(graph) {
        let Some(node) = graph.blocks.iter_mut().find(|node| node.id == id)
        else {
            continue;
        };
        replaced += match &mut node.block {
            Block::Section { title, .. } => fill_in_inlines(title, values),
            Block::Paragraph { content } => fill_in_inlines(content, values),
            Block::List { items, .. } => items.iter_mut().map(|item| fill_in_inlines(&mut item.content, values)).sum(),
            _ => 0,
        };
    }
    replaced
}

fn fill_in_inlines(inlines: &mut [Inline], values: &[(String, String)]) -> usize {
    let mut replaced = 0;
    for inline in inlines {
        match inline {
            Inline::Text { text } => {
                let (rewritten, count) = fill_in_text(text, values);
                if count > 0 {
                    *text = rewritten;
                }
                replaced += count;
            }
            Inline::Styled { children, .. } | Inline::Attributed { children, .. } => {
                replaced += fill_in_inlines(children, values);
            }
            Inline::Link { display, .. } => replaced += fill_in_inlines(display, values),
            Inline::Image { alt, .. } => replaced += fill_in_inlines(alt, values),
            Inline::Note { display: Some(display), .. } | Inline::Citation { display: Some(display), .. } => {
                replaced += fill_in_inlines(display, values);
            }
            _ => {}
        }
    }
    replaced
}

/// Replaces `{{key}}` occurrences inside one text run. Inner whitespace around
/// the key is ignored, so `{{ key }}` matches the same value as `{{key}}`.
fn fill_in_text(text: &str, values: &[(String, String)]) -> (String, usize) {
    let mut output = String::with_capacity(text.len());
    let mut replaced = 0;
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}")
        else {
            break;
        };
        let key = after[..end].trim();
        match values.iter().find(|(name, _)| name == key) {
            Some((_, value)) => {
                output.push_str(&rest[..start]);
                output.push_str(value);
                replaced += 1;
            }
            None => output.push_str(&rest[..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    output.push_str(rest);
    (output, replaced)
}

fn remove_block(graph: &mut DocumentGraph, id: NodeId) -> Result<usize, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let before = graph.blocks.len();
    sweep(graph, id);
    Ok(before - graph.blocks.len())
}

/// Physically removes one block and everything nested under it, then drops the
/// references its containers still hold.
fn sweep(graph: &mut DocumentGraph, id: NodeId) {
    let mut doomed = HashSet::new();
    collect_subtree(graph, id, &mut doomed);
    for node in &mut graph.blocks {
        prune_children(&mut node.block, &doomed);
    }
    graph.blocks.retain(|node| !doomed.contains(&node.id));
}

fn collect_subtree(graph: &DocumentGraph, id: NodeId, doomed: &mut HashSet<NodeId>) {
    if !doomed.insert(id) {
        return;
    }
    if let Some(node) = graph.blocks.iter().find(|node| node.id == id) {
        for child in document_ir::child_ids(&node.block) {
            collect_subtree(graph, child, doomed);
        }
    }
}

fn prune_children(block: &mut Block, doomed: &HashSet<NodeId>) {
    match block {
        Block::Section { children, .. } | Block::Quote { children } => {
            children.retain(|child| !doomed.contains(child));
        }
        Block::List { items, .. } => {
            for item in items {
                item.children.retain(|child| !doomed.contains(child));
            }
        }
        Block::Table { head, body, foot, .. } => {
            for row in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
                for cell in &mut row.cells {
                    cell.children.retain(|child| !doomed.contains(child));
                }
            }
        }
        _ => {}
    }
}

fn insert_paragraph(graph: &mut DocumentGraph, index: usize, text: String) -> Result<NodeId, ServiceError> {
    let order = document_ir::flat_order(graph);
    let Some(target) = order.get(index).copied()
    else {
        return Err(crate::invalid(format!("`index` {index} is outside the document's {} blocks", order.len())));
    };
    let id = graph.push_block(Block::Paragraph { content: vec![Inline::Text { text }] });
    if !insert_into_container(graph, target, id) {
        // A top-level block has no container list, so the node itself moves in
        // front of the target to keep document order.
        let position = graph.blocks.iter().position(|node| node.id == target).unwrap_or(graph.blocks.len());
        let node = graph.blocks.pop().expect("the new block was just pushed");
        graph.blocks.insert(position, node);
    }
    Ok(id)
}

/// Inserts `new_id` before `target` inside the container that owns `target`.
fn insert_into_container(graph: &mut DocumentGraph, target: NodeId, new_id: NodeId) -> bool {
    for node in &mut graph.blocks {
        match &mut node.block {
            Block::Section { children, .. } | Block::Quote { children } => {
                if let Some(position) = children.iter().position(|child| *child == target) {
                    children.insert(position, new_id);
                    return true;
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    if let Some(position) = item.children.iter().position(|child| *child == target) {
                        item.children.insert(position, new_id);
                        return true;
                    }
                }
            }
            Block::Table { head, body, foot, .. } => {
                for row in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
                    for cell in &mut row.cells {
                        if let Some(position) = cell.children.iter().position(|child| *child == target) {
                            cell.children.insert(position, new_id);
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Deep-copies one block and everything nested under it, then inserts the copy
/// directly after the source inside the same container. Every copied block gets
/// a fresh node id, so the copy never aliases a source node, while inline
/// content, structure and the authored source spans are kept. A top-level
/// source is followed at the top level, because it has no container list.
fn duplicate_block(graph: &mut DocumentGraph, id: NodeId) -> Result<NodeId, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let new_id = duplicate_subtree(graph, id, &mut HashMap::new());
    if !insert_after_source(graph, id, new_id) {
        let position = graph.blocks.iter().position(|node| node.id == id).expect("the source block exists");
        let copied = graph.blocks.iter().position(|node| node.id == new_id).expect("the copy was just pushed");
        let node = graph.blocks.remove(copied);
        graph.blocks.insert(position + 1, node);
    }
    Ok(new_id)
}

/// Copies one node and its children depth-first, recording `source -> copy` in
/// `ids` so child references can be remapped once the whole subtree exists.
fn duplicate_subtree(graph: &mut DocumentGraph, id: NodeId, ids: &mut HashMap<NodeId, NodeId>) -> NodeId {
    let source =
        graph.blocks.iter().find(|node| node.id == id).map(|node| node.block.clone()).expect("the subtree was resolved");
    let span = graph.block_span(id);
    let new_id = graph.push_block_at(source.clone(), span);
    ids.insert(id, new_id);
    for child in document_ir::child_ids(&source) {
        if graph.blocks.iter().any(|node| node.id == child) {
            duplicate_subtree(graph, child, ids);
        }
    }
    let node = graph.blocks.iter_mut().find(|node| node.id == new_id).expect("the copy was just pushed");
    remap_copy_children(&mut node.block, ids);
    new_id
}

/// Rewrites child references of a copied block to their fresh ids. A reference
/// without a copy is dropped rather than left pointing into the source subtree.
fn remap_copy_children(block: &mut Block, ids: &HashMap<NodeId, NodeId>) {
    match block {
        Block::Section { children, .. } | Block::Quote { children } => retain_copies(children, ids),
        Block::List { items, .. } => {
            for item in items {
                retain_copies(&mut item.children, ids);
            }
        }
        Block::Table { head, body, foot, .. } => {
            for row in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
                for cell in &mut row.cells {
                    retain_copies(&mut cell.children, ids);
                }
            }
        }
        _ => {}
    }
}

fn retain_copies(children: &mut Vec<NodeId>, ids: &HashMap<NodeId, NodeId>) {
    children.retain_mut(|child| match ids.get(child) {
        Some(mapped) => {
            *child = *mapped;
            true
        }
        None => false,
    });
}

/// Inserts `new_id` directly after `target` inside the container that owns
/// `target`, returning `false` when `target` is a top-level block.
fn insert_after_source(graph: &mut DocumentGraph, target: NodeId, new_id: NodeId) -> bool {
    for node in &mut graph.blocks {
        match &mut node.block {
            Block::Section { children, .. } | Block::Quote { children } => {
                if let Some(position) = children.iter().position(|child| *child == target) {
                    children.insert(position + 1, new_id);
                    return true;
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    if let Some(position) = item.children.iter().position(|child| *child == target) {
                        item.children.insert(position + 1, new_id);
                        return true;
                    }
                }
            }
            Block::Table { head, body, foot, .. } => {
                for row in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
                    for cell in &mut row.cells {
                        if let Some(position) = cell.children.iter().position(|child| *child == target) {
                            cell.children.insert(position + 1, new_id);
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn set_cell(graph: &mut DocumentGraph, table: usize, row: usize, column: usize, text: String) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;

    let old_children = {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, body, foot, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        let total_rows = head.len() + body.len() + foot.len();
        let Some(target) = row_at(head, body, foot, row)
        else {
            return Err(crate::invalid(format!("`row` {row} is outside the table's {total_rows} rows")));
        };
        let total_cells = target.cells.len();
        let Some(cell) = target.cells.get_mut(column)
        else {
            return Err(crate::invalid(format!("`column` {column} is outside row {row}'s {total_cells} cells")));
        };
        std::mem::take(&mut cell.children)
    };

    let id = graph.push_block(Block::Paragraph { content: vec![Inline::Text { text }] });
    {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, body, foot, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        let target = row_at(head, body, foot, row).expect("the row was validated");
        target.cells[column].children = vec![id];
    }
    for child in old_children {
        sweep(graph, child);
    }
    Ok(())
}

fn row_at<'a>(
    head: &'a mut [TableRow],
    body: &'a mut [TableRow],
    foot: &'a mut [TableRow],
    row: usize,
) -> Option<&'a mut TableRow> {
    let head_len = head.len();
    if row < head_len {
        return head.get_mut(row);
    }
    let body_len = body.len();
    if row < head_len + body_len {
        return body.get_mut(row - head_len);
    }
    foot.get_mut(row - head_len - body_len)
}

fn table_id_at(graph: &DocumentGraph, table: usize) -> Result<NodeId, ServiceError> {
    let tables = document_ir::flat_order(graph)
        .into_iter()
        .filter(|id| graph.blocks.iter().any(|node| node.id == *id && matches!(node.block, Block::Table { .. })))
        .collect::<Vec<_>>();
    tables
        .get(table)
        .copied()
        .ok_or_else(|| crate::invalid(format!("`table` {table} is outside the document's {} tables", tables.len())))
}

fn table_shape(graph: &DocumentGraph, id: NodeId) -> (usize, usize) {
    let Some(node) = graph.blocks.iter().find(|node| node.id == id)
    else {
        return (0, 0);
    };
    let Block::Table { head, body, foot, .. } = &node.block
    else {
        return (0, 0);
    };
    let rows = head.len() + body.len() + foot.len();
    let columns = head.iter().chain(body.iter()).chain(foot.iter()).map(row_width).max().unwrap_or(0);
    (rows, columns)
}

fn row_width(row: &TableRow) -> usize {
    row.cells.iter().fold(0usize, |total, cell| total.saturating_add(cell.colspan.max(1) as usize))
}

fn new_cell(graph: &mut DocumentGraph, text: String) -> TableCell {
    let child = graph.push_block(Block::Paragraph { content: vec![Inline::Text { text }] });
    TableCell { children: vec![child], colspan: 1, rowspan: 1 }
}

fn insert_row(graph: &mut DocumentGraph, table: usize, row: usize, cells: Option<Vec<String>>) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (total_rows, columns) = table_shape(graph, table_id);
    if columns == 0 {
        return Err(crate::invalid("`table.insertRow` needs a table with at least one column"));
    }
    if row > total_rows {
        return Err(crate::invalid(format!("`row` {row} is outside the table's {total_rows} rows")));
    }
    let texts = match cells {
        Some(values) if values.len() != columns => {
            return Err(crate::invalid(format!("`cells` needs {columns} entries for the table grid but has {}", values.len())));
        }
        Some(values) => values,
        None => vec![String::new(); columns],
    };
    let new_row = TableRow { cells: texts.into_iter().map(|text| new_cell(graph, text)).collect() };

    let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
    let Block::Table { head, body, foot, .. } = &mut node.block
    else {
        unreachable!("the id was filtered to a table block");
    };
    let head_len = head.len();
    let body_len = body.len();
    if row < head_len {
        head.insert(row, new_row);
    }
    else if row < head_len + body_len {
        body.insert(row - head_len, new_row);
    }
    else if row < head_len + body_len + foot.len() {
        foot.insert(row - head_len - body_len, new_row);
    }
    else if !foot.is_empty() {
        foot.push(new_row);
    }
    else {
        body.push(new_row);
    }
    Ok(())
}

fn delete_row(graph: &mut DocumentGraph, table: usize, row: usize) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (total_rows, _) = table_shape(graph, table_id);
    if row >= total_rows {
        return Err(crate::invalid(format!("`row` {row} is outside the table's {total_rows} rows")));
    }
    if total_rows == 1 {
        return Err(crate::invalid("cannot delete the table's last row"));
    }
    let removed = {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, body, foot, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        let removed = if row < head.len() {
            head.remove(row)
        }
        else if row < head.len() + body.len() {
            body.remove(row - head.len())
        }
        else {
            foot.remove(row - head.len() - body.len())
        };
        removed.cells.into_iter().flat_map(|cell| cell.children).collect::<Vec<_>>()
    };
    for child in removed {
        sweep(graph, child);
    }
    Ok(())
}

/// Grid index at which a new cell starts, given a target grid column.
fn column_index(row: &TableRow, column: usize) -> usize {
    let mut start = 0usize;
    for (index, cell) in row.cells.iter().enumerate() {
        let span = cell.colspan.max(1) as usize;
        if column < start + span {
            return index;
        }
        start += span;
    }
    row.cells.len()
}

fn insert_column(
    graph: &mut DocumentGraph,
    table: usize,
    column: usize,
    cells: Option<Vec<String>>,
) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (rows, columns) = table_shape(graph, table_id);
    if columns == 0 {
        return Err(crate::invalid("`table.insertColumn` needs a table with at least one column"));
    }
    if column > columns {
        return Err(crate::invalid(format!("`column` {column} is outside the table's {columns} grid columns")));
    }
    let texts = match cells {
        Some(values) if values.len() != rows => {
            return Err(crate::invalid(format!(
                "`cells` needs {rows} entries for the table's {rows} rows but has {}",
                values.len()
            )));
        }
        Some(values) => values,
        None => vec![String::new(); rows],
    };
    let mut new_cells = texts.into_iter().map(|text| new_cell(graph, text)).collect::<std::collections::VecDeque<_>>();

    let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
    let Block::Table { head, body, foot, .. } = &mut node.block
    else {
        unreachable!("the id was filtered to a table block");
    };
    for target in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
        let index = column_index(target, column);
        let cell = new_cells.pop_front().expect("one new cell was built per row");
        target.cells.insert(index, cell);
    }
    Ok(())
}

fn delete_column(graph: &mut DocumentGraph, table: usize, column: usize) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (_, columns) = table_shape(graph, table_id);
    if column >= columns {
        return Err(crate::invalid(format!("`column` {column} is outside the table's {columns} grid columns")));
    }
    if columns == 1 {
        return Err(crate::invalid("cannot delete the table's last column"));
    }
    let mut removed = Vec::new();
    {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, body, foot, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        for target in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
            let mut start = 0usize;
            let mut found = None;
            for (index, cell) in target.cells.iter().enumerate() {
                let span = cell.colspan.max(1) as usize;
                if column >= start && column < start + span {
                    found = Some(index);
                    break;
                }
                start += span;
            }
            let Some(index) = found
            else {
                continue;
            };
            if target.cells[index].colspan > 1 {
                target.cells[index].colspan -= 1;
            }
            else {
                let cell = target.cells.remove(index);
                removed.extend(cell.children);
            }
        }
    }
    for child in removed {
        sweep(graph, child);
    }
    Ok(())
}

fn update_paragraph(graph: &mut DocumentGraph, target: Target, text: String) -> Result<String, ServiceError> {
    let id = match target {
        Target::Id(id) => {
            if !graph.blocks.iter().any(|node| node.id == id) {
                return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
            }
            id
        }
        Target::Index(index) => {
            let order = document_ir::flat_order(graph);
            *order
                .get(index)
                .ok_or_else(|| crate::invalid(format!("`index` {index} is outside the document's {} blocks", order.len())))?
        }
    };
    let label =
        graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id)).unwrap_or_default();
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the block was just resolved");
    match &mut node.block {
        Block::Paragraph { content } => *content = vec![Inline::Text { text }],
        _ => return Err(crate::invalid("`paragraph.update` needs a paragraph; the addressed block is not one")),
    }
    Ok(label)
}

fn move_block(graph: &mut DocumentGraph, id: NodeId, index: usize) -> Result<(), ServiceError> {
    let order = document_ir::flat_order(graph);
    let Some(position) = order.iter().position(|node| *node == id)
    else {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    };
    if index > order.len() {
        return Err(crate::invalid(format!("`index` {index} is outside the document's {} block positions", order.len())));
    }
    let mut subtree = HashSet::new();
    collect_subtree(graph, id, &mut subtree);
    if index < order.len() {
        let target = order[index];
        if target != id && subtree.contains(&target) {
            return Err(crate::invalid("a block cannot move inside its own subtree"));
        }
    }
    if index == position {
        return Ok(());
    }

    detach_block(graph, id);
    let remaining = order.into_iter().filter(|node| !subtree.contains(node)).collect::<Vec<_>>();
    if index >= remaining.len() {
        append_root(graph, id);
    }
    else {
        let target = remaining[index];
        if !insert_into_container(graph, target, id) {
            reorder_root_before(graph, target, id);
        }
    }
    Ok(())
}

/// Removes one node from every container list, without deleting the node.
fn detach_block(graph: &mut DocumentGraph, id: NodeId) {
    for node in &mut graph.blocks {
        match &mut node.block {
            Block::Section { children, .. } | Block::Quote { children } => children.retain(|child| *child != id),
            Block::List { items, .. } => {
                for item in items {
                    item.children.retain(|child| *child != id);
                }
            }
            Block::Table { head, body, foot, .. } => {
                for row in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
                    for cell in &mut row.cells {
                        cell.children.retain(|child| *child != id);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Moves one block node to the end of the top-level block order.
fn append_root(graph: &mut DocumentGraph, id: NodeId) {
    let Some(position) = graph.blocks.iter().position(|node| node.id == id)
    else {
        return;
    };
    let node = graph.blocks.remove(position);
    graph.blocks.push(node);
}

/// Moves one block node directly in front of a top-level target.
fn reorder_root_before(graph: &mut DocumentGraph, target: NodeId, id: NodeId) {
    let Some(position) = graph.blocks.iter().position(|node| node.id == id)
    else {
        return;
    };
    let node = graph.blocks.remove(position);
    let target_position = graph.blocks.iter().position(|node| node.id == target).unwrap_or(graph.blocks.len());
    graph.blocks.insert(target_position, node);
}
