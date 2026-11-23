//! Atomic edit operations.
//!
//! A request is parsed and validated as a whole before the first mutation, so a
//! rejected operation never leaves a half-applied revision. Each applied
//! operation advances the revision exactly once.

use std::collections::{HashMap, HashSet};

use document_ir::{Block, DocumentGraph, Inline, ListItem, NodeId, TableCell, TableRow};
use serde_json::{Value, json};

use crate::{ServiceError, replace};

#[derive(Clone)]
enum Operation {
    Paragraph(String),
    Heading { level: u8, text: String },
    HeadingLevel { section: NodeId, level: u8 },
    HeadingText { section: NodeId, text: String },
    InsertHeading { index: usize, level: u8, text: String },
    List { ordered: bool, items: Vec<String>, start: Option<u32> },
    SetListOrdered { list: NodeId, ordered: bool },
    SetListStart { list: NodeId, start: u32 },
    SetListItem { block: NodeId, item: usize, text: String },
    SetItemChecked { block: NodeId, item: usize, checked: bool },
    SetItemTask { block: NodeId, item: usize, task: bool, checked: Option<bool> },
    MoveListItem { block: NodeId, item: usize, index: usize },
    IndentListItem { block: NodeId, item: usize },
    InsertListItem { block: NodeId, item: usize, text: String },
    RemoveListItem { block: NodeId, item: usize },
    TaskList(Vec<(String, bool)>),
    Quote(String),
    SetQuoteText { block: NodeId, text: String },
    Code { text: String, language: Option<String> },
    SetCodeLanguage { block: NodeId, language: Option<String> },
    SetCodeText { block: NodeId, text: String },
    Table(Vec<Vec<String>>),
    SetCaption { table: usize, caption: Option<String> },
    SetTableHeader { table: usize, cells: Vec<String> },
    SetTableFooter { table: usize, cells: Vec<String> },
    SetTableHeaderRows { table: usize, rows: Vec<Vec<String>> },
    SetTableFooterRows { table: usize, rows: Vec<Vec<String>> },
    SetCellSpan { table: usize, row: usize, column: usize, colspan: u32, rowspan: u32 },
    Metadata { title: Option<String>, author: Option<String>, language: Option<String>, tags: Option<Vec<String>> },
    ReplaceText(replace::Request),
    RemoveBlock { id: NodeId },
    InsertParagraph { index: usize, text: String },
    SetCell { table: usize, row: usize, column: usize, text: String },
    InsertRow { table: usize, row: usize, cells: Option<Vec<String>> },
    DeleteRow { table: usize, row: usize },
    MoveTableRow { table: usize, row: usize, index: usize },
    DuplicateTableRow { table: usize, row: usize },
    InsertColumn { table: usize, column: usize, cells: Option<Vec<String>> },
    DeleteColumn { table: usize, column: usize },
    MoveColumn { table: usize, column: usize, index: usize },
    DuplicateColumn { table: usize, column: usize },
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
        "heading.insert" => {
            let level = operation
                .get("level")
                .and_then(Value::as_u64)
                .ok_or_else(|| crate::invalid("`heading.insert` needs a numeric `level`"))?;
            if !(1..=6).contains(&level) {
                return Err(crate::invalid("`level` must be between 1 and 6"));
            }
            Ok(Operation::InsertHeading {
                index: required_index(operation, "index")?,
                level: level as u8,
                text: required_text(operation, "text")?,
            })
        }
        "heading.setLevel" => {
            let level = operation
                .get("level")
                .and_then(Value::as_u64)
                .ok_or_else(|| crate::invalid("`heading.setLevel` needs a numeric `level`"))?;
            if !(1..=6).contains(&level) {
                return Err(crate::invalid("`level` must be between 1 and 6"));
            }
            Ok(Operation::HeadingLevel { section: parse_block_id(operation)?, level: level as u8 })
        }
        "heading.setText" => {
            let text = operation
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| crate::invalid("`heading.setText` needs a non-empty `text`"))?;
            Ok(Operation::HeadingText { section: parse_block_id(operation)?, text: text.to_owned() })
        }
        "list.append" => {
            let ordered = optional_bool(operation, "ordered")?.unwrap_or(false);
            let items = required_items(operation)?;
            let start = parse_list_start(operation, ordered, items.len())?;
            Ok(Operation::List { ordered, items, start })
        }
        "taskList.append" => Ok(Operation::TaskList(required_tasks(operation)?)),
        "list.setOrdered" => {
            let ordered = optional_bool(operation, "ordered")?
                .ok_or_else(|| crate::invalid("`list.setOrdered` needs a boolean `ordered`"))?;
            Ok(Operation::SetListOrdered { list: parse_block_id(operation)?, ordered })
        }
        "list.setStart" => {
            let start = required_index(operation, "start")?;
            if start == 0 || start > u32::MAX as usize {
                return Err(crate::invalid("`list.setStart` needs a positive integer `start` that fits in u32"));
            }
            Ok(Operation::SetListStart { list: parse_block_id(operation)?, start: start as u32 })
        }
        "list.setItem" => {
            let text = required_text(operation, "text")?;
            if text.trim().is_empty() {
                return Err(crate::invalid("`list.setItem` needs a non-empty `text`"));
            }
            Ok(Operation::SetListItem { block: parse_block_id(operation)?, item: required_index(operation, "item")?, text })
        }
        "list.setItemChecked" => {
            let checked = optional_bool(operation, "checked")?
                .ok_or_else(|| crate::invalid("`list.setItemChecked` needs a boolean `checked`"))?;
            Ok(Operation::SetItemChecked { block: parse_block_id(operation)?, item: required_index(operation, "item")?, checked })
        }
        "list.setItemTask" => {
            let task = optional_bool(operation, "task")?
                .ok_or_else(|| crate::invalid("`list.setItemTask` needs a boolean `task`"))?;
            let checked = optional_bool(operation, "checked")?;
            if !task && checked.is_some() {
                return Err(crate::invalid("`checked` is only valid when `task` is true"));
            }
            Ok(Operation::SetItemTask {
                block: parse_block_id(operation)?,
                item: required_index(operation, "item")?,
                task,
                checked,
            })
        }
        "list.moveItem" => Ok(Operation::MoveListItem {
            block: parse_block_id(operation)?,
            item: required_index(operation, "item")?,
            index: required_index(operation, "index")?,
        }),
        "list.indentItem" => Ok(Operation::IndentListItem {
            block: parse_block_id(operation)?,
            item: required_index(operation, "item")?,
        }),
        "list.insertItem" => {
            let text = required_text(operation, "text")?;
            if text.trim().is_empty() {
                return Err(crate::invalid("`list.insertItem` needs a non-empty `text`"));
            }
            Ok(Operation::InsertListItem { block: parse_block_id(operation)?, item: required_index(operation, "item")?, text })
        }
        "list.removeItem" => Ok(Operation::RemoveListItem {
            block: parse_block_id(operation)?,
            item: required_index(operation, "item")?,
        }),
        "quote.append" => {
            let text = required_text(operation, "text")?;
            if text.is_empty() {
                return Err(crate::invalid("`quote.append` needs a non-empty `text`"));
            }
            Ok(Operation::Quote(text))
        }
        "quote.setText" => {
            let text = required_text(operation, "text")?;
            if text.is_empty() {
                return Err(crate::invalid("`quote.setText` needs a non-empty `text`"));
            }
            Ok(Operation::SetQuoteText { block: parse_block_id(operation)?, text })
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
        "code.setLanguage" => Ok(Operation::SetCodeLanguage {
            block: parse_block_id(operation)?,
            language: optional_text(operation, "language")?.filter(|value| !value.trim().is_empty()),
        }),
        "code.setText" => {
            let text = required_text(operation, "text")?;
            if text.is_empty() {
                return Err(crate::invalid("`code.setText` needs a non-empty `text`"));
            }
            Ok(Operation::SetCodeText { block: parse_block_id(operation)?, text })
        }
        "table.setHeader" => Ok(Operation::SetTableHeader {
            table: required_index(operation, "table")?,
            cells: required_strings(operation, "cells")?,
        }),
        "table.setFooter" => Ok(Operation::SetTableFooter {
            table: required_index(operation, "table")?,
            cells: required_strings(operation, "cells")?,
        }),
        "table.setHeaderRows" => Ok(Operation::SetTableHeaderRows {
            table: required_index(operation, "table")?,
            rows: required_text_rows(operation)?,
        }),
        "table.setFooterRows" => Ok(Operation::SetTableFooterRows {
            table: required_index(operation, "table")?,
            rows: required_text_rows(operation)?,
        }),
        "table.setCellSpan" => {
            let colspan = required_index(operation, "colspan")?;
            let rowspan = required_index(operation, "rowspan")?;
            if colspan == 0 || colspan > u16::MAX as usize {
                return Err(crate::invalid("`colspan` must be between 1 and 65535"));
            }
            if rowspan == 0 || rowspan > u16::MAX as usize {
                return Err(crate::invalid("`rowspan` must be between 1 and 65535"));
            }
            Ok(Operation::SetCellSpan {
                table: required_index(operation, "table")?,
                row: required_index(operation, "row")?,
                column: required_index(operation, "column")?,
                colspan: colspan as u32,
                rowspan: rowspan as u32,
            })
        }
        "table.setCaption" => {
            let caption = optional_text(operation, "caption")?.map(|caption| caption.trim().to_owned());
            if caption.as_deref() == Some("") {
                return Err(crate::invalid(
                    "`table.setCaption` needs a non-empty `caption`, or omit it to clear the caption",
                ));
            }
            Ok(Operation::SetCaption { table: required_index(operation, "table")?, caption })
        }
        "metadata.set" => {
            let title = optional_text(operation, "title")?;
            let author = optional_text(operation, "author")?;
            let language = match operation.get("language") {
                None | Some(Value::Null) => None,
                Some(Value::String(text)) if text.trim().is_empty() => {
                    return Err(crate::invalid("`language` must be a non-empty tag such as `en`"));
                }
                Some(Value::String(text)) => Some(text.trim().to_owned()),
                Some(_) => return Err(crate::invalid("`language` must be a string")),
            };
            let tags = optional_tags(operation)?;
            if title.is_none() && author.is_none() && language.is_none() && tags.is_none() {
                return Err(crate::invalid("`metadata.set` needs `title`, `author`, `language` or `tags`"));
            }
            Ok(Operation::Metadata { title, author, language, tags })
        }
        "document.replaceText" => Ok(Operation::ReplaceText(replace::Request::parse(operation)?)),
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
        "table.moveRow" => Ok(Operation::MoveTableRow {
            table: required_index(operation, "table")?,
            row: required_index(operation, "row")?,
            index: required_index(operation, "index")?,
        }),
        "table.duplicateRow" => Ok(Operation::DuplicateTableRow {
            table: required_index(operation, "table")?,
            row: required_index(operation, "row")?,
        }),
        "table.insertColumn" => Ok(Operation::InsertColumn {
            table: required_index(operation, "table")?,
            column: required_index(operation, "column")?,
            cells: optional_cells(operation)?,
        }),
        "table.deleteColumn" => Ok(Operation::DeleteColumn {
            table: required_index(operation, "table")?,
            column: required_index(operation, "column")?,
        }),
        "table.moveColumn" => Ok(Operation::MoveColumn {
            table: required_index(operation, "table")?,
            column: required_index(operation, "column")?,
            index: required_index(operation, "index")?,
        }),
        "table.duplicateColumn" => Ok(Operation::DuplicateColumn {
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

/// Reads an optional string array, trimming entries and rejecting blanks.
///
/// An empty array is meaningful: it clears the previous tags.
fn optional_tags(operation: &Value) -> Result<Option<Vec<String>>, ServiceError> {
    let Some(value) = operation.get("tags").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let entries = value.as_array().ok_or_else(|| crate::invalid("`tags` must be an array of strings"))?;
    let mut tags = Vec::with_capacity(entries.len());
    for entry in entries {
        let text = entry.as_str().ok_or_else(|| crate::invalid("`tags` must be an array of strings"))?.trim();
        if text.is_empty() {
            return Err(crate::invalid("`tags` must not contain an empty entry"));
        }
        tags.push(text.to_owned());
    }
    Ok(Some(tags))
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

/// Reads a string array, allowing an empty array so callers can clear a field.
fn required_strings(operation: &Value, key: &str) -> Result<Vec<String>, ServiceError> {
    let values = operation
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| crate::invalid(format!("`{key}` must be an array of strings")))?;
    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| crate::invalid(format!("`{key}` must contain only strings")))
        })
        .collect()
}

/// Reads the `rows` array used by the multi-row header and footer operations.
///
/// An empty array clears the band; otherwise every row must be non-empty and
/// all rows must have the same number of cells.
fn required_text_rows(operation: &Value) -> Result<Vec<Vec<String>>, ServiceError> {
    let rows = operation
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| crate::invalid("`rows` must be an array of string arrays"))?;
    let mut parsed: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    let mut width = None;
    for row in rows {
        let cells = row.as_array().ok_or_else(|| crate::invalid("`rows` must be an array of string arrays"))?;
        if cells.is_empty() {
            return Err(crate::invalid("each header or footer row needs at least one cell"));
        }
        let mut texts = Vec::with_capacity(cells.len());
        for cell in cells {
            texts.push(
                cell.as_str().ok_or_else(|| crate::invalid("`rows` must contain only strings"))?.to_owned(),
            );
        }
        match width {
            None => width = Some(texts.len()),
            Some(width) if width != texts.len() => {
                return Err(crate::invalid(format!(
                    "every header or footer row needs {width} cells, but one row has {}",
                    texts.len()
                )));
            }
            Some(_) => {}
        }
        parsed.push(texts);
    }
    Ok(parsed)
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

/// Parses `taskList.append` items. A string item is unchecked; an object may
/// carry `{ "text": "...", "checked": true }`.
fn required_tasks(operation: &Value) -> Result<Vec<(String, bool)>, ServiceError> {
    let items = operation
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| crate::invalid("`taskList.append` needs an `items` array"))?;
    if items.is_empty() {
        return Err(crate::invalid("`items` must contain at least one task item"));
    }
    items
        .iter()
        .map(|item| match item {
            Value::String(text) => Ok((text.clone(), false)),
            Value::Object(fields) => {
                let text = fields
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| crate::invalid("each task item object needs a `text` string"))?;
                match fields.get("checked") {
                    None | Some(Value::Null) => Ok((text, false)),
                    Some(Value::Bool(checked)) => Ok((text, *checked)),
                    Some(_) => Err(crate::invalid("task item `checked` must be a boolean")),
                }
            }
            _ => Err(crate::invalid("each task item must be a string or an object with `text`")),
        })
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
        Operation::InsertHeading { index, level, text } => {
            let id = insert_heading(graph, index, level, text)?;
            graph.bump_revision();
            Ok(json!({ "type": "heading.insert", "id": format!("section-{}", id.0) }))
        }
        Operation::HeadingLevel { section, level } => {
            let label = set_heading_level(graph, section, level)?;
            graph.bump_revision();
            Ok(json!({ "type": "heading.setLevel", "id": label, "level": level }))
        }
        Operation::HeadingText { section, text } => {
            let label = set_heading_text(graph, section, &text)?;
            graph.bump_revision();
            Ok(json!({ "type": "heading.setText", "id": label, "text": text }))
        }
        Operation::List { ordered, items, start } => {
            let count = items.len();
            append_list(graph, ordered, items, start);
            Ok(json!({ "type": "list.append", "ordered": ordered, "items": count, "start": start }))
        }
        Operation::SetListOrdered { list, ordered } => {
            let label = set_list_ordered(graph, list, ordered)?;
            graph.bump_revision();
            Ok(json!({ "type": "list.setOrdered", "id": label, "ordered": ordered }))
        }
        Operation::SetListStart { list, start } => {
            let label = set_list_start(graph, list, start)?;
            graph.bump_revision();
            Ok(json!({ "type": "list.setStart", "id": label, "start": start }))
        }
        Operation::SetListItem { block, item, text } => {
            let label = set_list_item(graph, block, item, text)?;
            graph.bump_revision();
            Ok(json!({ "type": "list.setItem", "id": label, "item": item }))
        }
        Operation::SetItemChecked { block, item, checked } => {
            let label = set_list_item_checked(graph, block, item, checked)?;
            graph.bump_revision();
            Ok(json!({ "type": "list.setItemChecked", "id": label, "item": item, "checked": checked }))
        }
        Operation::SetItemTask { block, item, task, checked } => {
            let label = set_list_item_task(graph, block, item, task, checked)?;
            graph.bump_revision();
            Ok(json!({
                "type": "list.setItemTask",
                "id": label,
                "item": item,
                "task": task,
                "checked": if task { Some(checked.unwrap_or(false)) } else { None },
            }))
        }
        Operation::MoveListItem { block, item, index } => {
            let label = move_list_item(graph, block, item, index)?;
            graph.bump_revision();
            Ok(json!({ "type": "list.moveItem", "id": label, "item": item, "index": index }))
        }
        Operation::IndentListItem { block, item } => {
            let label = indent_list_item(graph, block, item)?;
            graph.bump_revision();
            Ok(json!({ "type": "list.indentItem", "id": label, "item": item }))
        }
        Operation::InsertListItem { block, item, text } => {
            let label = insert_list_item(graph, block, item, text)?;
            graph.bump_revision();
            Ok(json!({ "type": "list.insertItem", "id": label, "item": item }))
        }
        Operation::RemoveListItem { block, item } => {
            let label = remove_list_item(graph, block, item)?;
            graph.bump_revision();
            Ok(json!({ "type": "list.removeItem", "id": label, "item": item }))
        }
        Operation::TaskList(items) => {
            let count = items.len();
            let checked = items.iter().filter(|(_, checked)| *checked).count();
            append_tasks(graph, items);
            Ok(json!({ "type": "taskList.append", "items": count, "checked": checked }))
        }
        Operation::Quote(text) => {
            append_quote(graph, text);
            Ok(json!({ "type": "quote.append" }))
        }
        Operation::SetQuoteText { block, text } => {
            let label = set_quote_text(graph, block, text)?;
            graph.bump_revision();
            Ok(json!({ "type": "quote.setText", "id": label }))
        }
        Operation::Code { text, language } => {
            append_block(graph, Block::Code { language: language.clone(), content: text });
            Ok(json!({ "type": "code.append", "language": language }))
        }
        Operation::SetCodeLanguage { block, language } => {
            let label = set_code_language(graph, block, language.clone())?;
            graph.bump_revision();
            Ok(json!({ "type": "code.setLanguage", "id": label, "language": language }))
        }
        Operation::SetCodeText { block, text } => {
            let label = set_code_text(graph, block, text)?;
            graph.bump_revision();
            Ok(json!({ "type": "code.setText", "id": label }))
        }
        Operation::Table(rows) => {
            append_table(graph, rows);
            Ok(json!({ "type": "table.append" }))
        }
        Operation::SetTableHeader { table, cells } => {
            set_table_header(graph, table, cells.clone())?;
            graph.bump_revision();
            Ok(json!({ "type": "table.setHeader", "table": table, "cells": cells }))
        }
        Operation::SetTableFooter { table, cells } => {
            set_table_footer(graph, table, cells.clone())?;
            graph.bump_revision();
            Ok(json!({ "type": "table.setFooter", "table": table, "cells": cells }))
        }
        Operation::SetTableHeaderRows { table, rows } => {
            let count = set_table_band_rows(graph, table, rows, true)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.setHeaderRows", "table": table, "rows": count }))
        }
        Operation::SetTableFooterRows { table, rows } => {
            let count = set_table_band_rows(graph, table, rows, false)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.setFooterRows", "table": table, "rows": count }))
        }
        Operation::SetCaption { table, caption } => {
            set_caption(graph, table, caption.clone())?;
            graph.bump_revision();
            Ok(json!({ "type": "table.setCaption", "table": table, "caption": caption }))
        }
        Operation::Metadata { title, author, language, tags } => {
            if let Some(title) = title.clone() {
                graph.metadata.title = Some(title);
            }
            if let Some(author) = author.clone() {
                graph.metadata.authors = vec![author];
            }
            if let Some(language) = language.clone() {
                graph.metadata.language = Some(language);
            }
            if let Some(tags) = tags.clone() {
                graph.metadata.tags = tags;
            }
            graph.bump_revision();
            Ok(json!({ "type": "metadata.set", "title": title, "author": author, "language": language, "tags": tags }))
        }
        Operation::ReplaceText(request) => {
            let replaced = replace::apply(graph, &request).replaced;
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
        Operation::SetCellSpan { table, row, column, colspan, rowspan } => {
            set_cell_span(graph, table, row, column, colspan, rowspan)?;
            graph.bump_revision();
            Ok(json!({
                "type": "table.setCellSpan",
                "table": table,
                "row": row,
                "column": column,
                "colspan": colspan,
                "rowspan": rowspan,
            }))
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
        Operation::MoveTableRow { table, row, index } => {
            let position = move_table_row(graph, table, row, index)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.moveRow", "table": table, "row": row, "index": position }))
        }
        Operation::DuplicateTableRow { table, row } => {
            let position = duplicate_table_row(graph, table, row)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.duplicateRow", "table": table, "row": row, "newRow": position }))
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
        Operation::MoveColumn { table, column, index } => {
            move_table_column(graph, table, column, index)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.moveColumn", "table": table, "column": column, "index": index }))
        }
        Operation::DuplicateColumn { table, column } => {
            let new_column = duplicate_table_column(graph, table, column)?;
            graph.bump_revision();
            Ok(json!({ "type": "table.duplicateColumn", "table": table, "column": column, "newColumn": new_column }))
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

/// Appends one unordered list whose items are tasks, preserving each check
/// state so Markdown and HTML exports keep their `[x]`/`[ ]` markers.
fn append_tasks(graph: &mut DocumentGraph, items: Vec<(String, bool)>) {
    let items = items
        .into_iter()
        .map(|(text, checked)| ListItem {
            content: vec![Inline::Text { text }],
            children: Vec::new(),
            number: None,
            is_task: true,
            is_checked: Some(checked),
        })
        .collect();
    append_block(graph, Block::List { ordered: false, items });
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

/// Inserts a heading before the block at flat index `index` and returns its id.
///
/// Mirrors `insert_paragraph`: when the target sits inside a container the new
/// section joins that container; otherwise it moves in front of the target at
/// the top level. The new section starts without children.
fn insert_heading(graph: &mut DocumentGraph, index: usize, level: u8, text: String) -> Result<NodeId, ServiceError> {
    let order = document_ir::flat_order(graph);
    let Some(target) = order.get(index).copied()
    else {
        return Err(crate::invalid(format!("`index` {index} is outside the document's {} blocks", order.len())));
    };
    let id = graph.push_block(Block::Section { level, title: vec![Inline::Text { text }], children: Vec::new() });
    if !insert_into_container(graph, target, id) {
        // A top-level block has no container list, so the node itself moves in
        // front of the target to keep document order.
        let position = graph.blocks.iter().position(|node| node.id == target).unwrap_or(graph.blocks.len());
        let node = graph.blocks.pop().expect("the new block was just pushed");
        graph.blocks.insert(position, node);
    }
    Ok(id)
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

/// Replaces the header row of one table.
///
/// An empty `cells` array removes the header, so the first body row becomes the
/// visual top; a non-empty array creates exactly one header row.
fn set_table_header(graph: &mut DocumentGraph, table: usize, cells: Vec<String>) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let head = table_text_rows(graph, cells);
    let old = {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head: current, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        std::mem::replace(current, head)
    };
    sweep_rows(graph, old);
    Ok(())
}

fn set_table_footer(graph: &mut DocumentGraph, table: usize, cells: Vec<String>) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let foot = table_text_rows(graph, cells);
    let old = {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { foot: current, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        std::mem::replace(current, foot)
    };
    sweep_rows(graph, old);
    Ok(())
}

/// Removes the cell content that a replaced header or footer row left behind.
fn sweep_rows(graph: &mut DocumentGraph, rows: Vec<TableRow>) {
    for row in rows {
        for cell in row.cells {
            for child in cell.children {
                sweep(graph, child);
            }
        }
    }
}

/// Replaces a header or footer band with any number of text rows.
///
/// The old rows are swept so no orphaned paragraph blocks survive the change.
fn set_table_band_rows(
    graph: &mut DocumentGraph,
    table: usize,
    rows: Vec<Vec<String>>,
    header: bool,
) -> Result<usize, ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let count = rows.len();
    let built = rows
        .into_iter()
        .map(|cells| TableRow { cells: cells.into_iter().map(|text| new_cell(graph, text)).collect() })
        .collect::<Vec<_>>();
    let old = {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, foot, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        if header {
            std::mem::replace(head, built)
        }
        else {
            std::mem::replace(foot, built)
        }
    };
    sweep_rows(graph, old);
    Ok(count)
}

/// Builds the single header or footer row that a `cells` array describes.
///
/// An empty array means "no row", which removes the band; otherwise each text
/// becomes a plain paragraph cell with span `1x1`.
fn table_text_rows(graph: &mut DocumentGraph, cells: Vec<String>) -> Vec<TableRow> {
    if cells.is_empty() {
        return Vec::new();
    }
    let cells = cells.into_iter().map(|text| new_cell(graph, text)).collect();
    vec![TableRow { cells }]
}

/// Sets or clears the caption of the table at `table` in document order.
fn set_caption(graph: &mut DocumentGraph, table: usize, caption: Option<String>) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
    let Block::Table { caption: current, .. } = &mut node.block
    else {
        unreachable!("the id was filtered to a table block");
    };
    *current = caption;
    Ok(())
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

/// Sets how many grid columns and rows one cell covers.
///
/// The cell is addressed by its top-left grid position. Spans keep their
/// content and children; setting a cell back to `1x1` unmerges it.
fn set_cell_span(
    graph: &mut DocumentGraph,
    table: usize,
    row: usize,
    column: usize,
    colspan: u32,
    rowspan: u32,
) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (total_rows, columns) = table_shape(graph, table_id);
    if row >= total_rows {
        return Err(crate::invalid(format!("`row` {row} is outside the table's {total_rows} rows")));
    }
    if column >= columns {
        return Err(crate::invalid(format!("`column` {column} is outside the table's {columns} grid columns")));
    }
    let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
    let Block::Table { head, body, foot, .. } = &mut node.block
    else {
        unreachable!("the id was filtered to a table block");
    };
    let target = row_at(head, body, foot, row).expect("the row was validated");
    let index = column_index(target, column);
    let Some(cell) = target.cells.get_mut(index)
    else {
        return Err(crate::invalid(format!("`column` {column} is outside row {row}'s grid width")));
    };
    cell.colspan = colspan;
    cell.rowspan = rowspan;
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

/// Moves one row so it lands at grid position `index` and returns that position.
///
/// Rows move across the head, body and footer bands, which keep their row
/// counts: a row moved into the header position becomes the header row and the
/// row it displaced continues in the body. Cells travel with their spans, so a
/// `rowspan` cell can misalign a merge; set the spans explicitly after moving
/// if the grid must stay exact.
fn move_table_row(graph: &mut DocumentGraph, table: usize, row: usize, index: usize) -> Result<usize, ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (total_rows, _) = table_shape(graph, table_id);
    if row >= total_rows {
        return Err(crate::invalid(format!("`row` {row} is outside the table's {total_rows} rows")));
    }
    if index > total_rows {
        return Err(crate::invalid(format!("`index` {index} is outside the table's {total_rows} insertion points")));
    }
    let position = {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, body, foot, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        let head_len = head.len();
        let foot_len = foot.len();
        let mut rows = std::mem::take(head);
        rows.append(body);
        rows.append(foot);
        let moved = rows.remove(row);
        let position = index.min(rows.len());
        rows.insert(position, moved);
        let head_len = head_len.min(rows.len());
        *head = rows.drain(..head_len).collect();
        let foot_start = rows.len().saturating_sub(foot_len);
        *foot = rows.split_off(foot_start);
        *body = rows;
        position
    };
    Ok(position)
}

/// Deep-copies one row directly after the source and returns the copy's position.
///
/// The copy stays inside the source's band and keeps each cell's spans. Cell
/// content is copied into fresh graph nodes, so editing one copy leaves the
/// other untouched.
fn duplicate_table_row(graph: &mut DocumentGraph, table: usize, row: usize) -> Result<usize, ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (total_rows, _) = table_shape(graph, table_id);
    if row >= total_rows {
        return Err(crate::invalid(format!("`row` {row} is outside the table's {total_rows} rows")));
    }
    let (cells, band, offset) = {
        let node = graph.blocks.iter().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, body, foot, .. } = &node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        if row < head.len() {
            (head[row].cells.clone(), 0usize, row)
        }
        else if row < head.len() + body.len() {
            (body[row - head.len()].cells.clone(), 1, row - head.len())
        }
        else {
            (foot[row - head.len() - body.len()].cells.clone(), 2, row - head.len() - body.len())
        }
    };
    let mut new_cells = Vec::with_capacity(cells.len());
    for cell in &cells {
        let children = cell
            .children
            .iter()
            .map(|child| duplicate_subtree(graph, *child, &mut HashMap::new()))
            .collect();
        new_cells.push(TableCell { children, colspan: cell.colspan, rowspan: cell.rowspan });
    }
    let copy = TableRow { cells: new_cells };
    {
        let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, body, foot, .. } = &mut node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        match band {
            0 => head.insert(offset + 1, copy),
            1 => body.insert(offset + 1, copy),
            _ => foot.insert(offset + 1, copy),
        }
    }
    Ok(row + 1)
}

/// Moves one whole grid column to another position in every row.
///
/// A spanned cell makes a column boundary ambiguous, so a table whose cells all
/// occupy one column is required; nested content and row spans move with the
/// cell and are not rewritten.
fn move_table_column(graph: &mut DocumentGraph, table: usize, column: usize, index: usize) -> Result<(), ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (_, columns) = table_shape(graph, table_id);
    if column >= columns {
        return Err(crate::invalid(format!("`column` {column} is outside the table's {columns} grid columns")));
    }
    if index >= columns {
        return Err(crate::invalid(format!("`index` {index} is outside the table's {columns} column positions")));
    }
    let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
    let Block::Table { head, body, foot, .. } = &mut node.block
    else {
        unreachable!("the id was filtered to a table block");
    };
    for row in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
        if row.cells.len() != columns || row.cells.iter().any(|cell| cell.colspan > 1) {
            return Err(crate::invalid("`table.moveColumn` needs a rectangular table without spanning cells"));
        }
        let cell = row.cells.remove(column);
        row.cells.insert(index, cell);
    }
    Ok(())
}

/// Deep-copies one whole grid column directly after itself and returns its position.
///
/// Cell content is copied into fresh graph nodes, so editing one copy leaves the
/// other untouched. As with [`move_table_column`], spanning cells are refused
/// because the copied column would not have one grid boundary.
fn duplicate_table_column(graph: &mut DocumentGraph, table: usize, column: usize) -> Result<usize, ServiceError> {
    let table_id = table_id_at(graph, table)?;
    let (_, columns) = table_shape(graph, table_id);
    if column >= columns {
        return Err(crate::invalid(format!("`column` {column} is outside the table's {columns} grid columns")));
    }
    let sources = {
        let node = graph.blocks.iter().find(|node| node.id == table_id).expect("the table was just resolved");
        let Block::Table { head, body, foot, .. } = &node.block
        else {
            unreachable!("the id was filtered to a table block");
        };
        let mut cells = Vec::with_capacity(head.len() + body.len() + foot.len());
        for row in head.iter().chain(body.iter()).chain(foot.iter()) {
            if row.cells.len() != columns || row.cells.iter().any(|cell| cell.colspan > 1) {
                return Err(crate::invalid("`table.duplicateColumn` needs a rectangular table without spanning cells"));
            }
            cells.push(row.cells[column].clone());
        }
        cells
    };
    let mut copies = Vec::with_capacity(sources.len());
    for cell in sources {
        let children = cell
            .children
            .iter()
            .map(|child| duplicate_subtree(graph, *child, &mut HashMap::new()))
            .collect();
        copies.push(TableCell { children, colspan: 1, rowspan: cell.rowspan });
    }
    let node = graph.blocks.iter_mut().find(|node| node.id == table_id).expect("the table was just resolved");
    let Block::Table { head, body, foot, .. } = &mut node.block
    else {
        unreachable!("the id was filtered to a table block");
    };
    for (row, copy) in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()).zip(copies) {
        row.cells.insert(column + 1, copy);
    }
    Ok(column + 1)
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

/// Changes the level of one existing heading section and returns its address.
///
/// Only the level changes; the title, children and authored spans stay as they
/// are, so a document that addressed a heading can be re-levelled in place.
fn set_heading_level(graph: &mut DocumentGraph, id: NodeId, level: u8) -> Result<String, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let label =
        graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id)).unwrap_or_default();
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the block was just resolved");
    match &mut node.block {
        Block::Section { level: current, .. } => {
            *current = level;
            Ok(label)
        }
        _ => Err(crate::invalid("`heading.setLevel` needs a heading section; the addressed block is not one")),
    }
}

/// Replaces the content of one quote with a single paragraph and returns its address.
///
/// Only the content changes; the quote's id and position stay as they are. The
/// previous children are physically removed, so a quote that held several blocks
/// collapses to the one new paragraph.
fn set_quote_text(graph: &mut DocumentGraph, id: NodeId, text: String) -> Result<String, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let label =
        graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id)).unwrap_or_default();
    let old_children = {
        let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the block was just resolved");
        match &mut node.block {
            Block::Quote { children } => std::mem::take(children),
            _ => return Err(crate::invalid("`quote.setText` needs a quote; the addressed block is not one")),
        }
    };
    let child = graph.push_block(Block::Paragraph { content: vec![Inline::Text { text }] });
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the quote was just resolved");
    let Block::Quote { children } = &mut node.block
    else {
        unreachable!("the block was checked to be a quote");
    };
    children.push(child);
    for old in old_children {
        sweep(graph, old);
    }
    Ok(label)
}

/// Changes the language tag of one code block and returns its address.
///
/// Only the tag changes; the source text and position stay as they are. A
/// missing or empty `language` clears the tag, so the fence renders bare.
fn set_code_language(graph: &mut DocumentGraph, id: NodeId, language: Option<String>) -> Result<String, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let label =
        graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id)).unwrap_or_default();
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the block was just resolved");
    match &mut node.block {
        Block::Code { language: current, .. } => {
            *current = language;
            Ok(label)
        }
        _ => Err(crate::invalid("`code.setLanguage` needs a code block; the addressed block is not one")),
    }
}

/// Replaces the source text of one code block and returns its address.
///
/// Only the content changes; the id, position and language tag stay as they are.
fn set_code_text(graph: &mut DocumentGraph, id: NodeId, text: String) -> Result<String, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let label =
        graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id)).unwrap_or_default();
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the block was just resolved");
    match &mut node.block {
        Block::Code { content, .. } => *content = text,
        _ => return Err(crate::invalid("`code.setText` needs a code block; the addressed block is not one")),
    }
    Ok(label)
}

/// Switches one existing list between bullets and numbers and returns its address.
///
/// Only the marker style changes; items and nesting stay as they are. Numbers
/// restart at 1 when the list becomes ordered and are cleared when it becomes
/// unordered, because the IR requires explicit numbers on ordered items.
fn set_list_ordered(graph: &mut DocumentGraph, id: NodeId, ordered: bool) -> Result<String, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let label =
        graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id)).unwrap_or_default();
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the block was just resolved");
    match &mut node.block {
        Block::List { ordered: current, items, .. } => {
            *current = ordered;
            for (index, item) in items.iter_mut().enumerate() {
                item.number = if ordered { Some(index as u32 + 1) } else { None };
            }
            Ok(label)
        }
        _ => Err(crate::invalid("`list.setOrdered` needs a list; the addressed block is not one")),
    }
}

/// Checks the addressed block is a list and returns its address.
///
/// Unknown ids report `object/not-found`; other block types are refused because
/// items only live inside lists.
fn resolve_list(graph: &DocumentGraph, id: NodeId, operation: &str) -> Result<String, ServiceError> {
    let Some(node) = graph.blocks.iter().find(|node| node.id == id) else {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    };
    if !matches!(node.block, Block::List { .. }) {
        return Err(crate::invalid(format!("`{operation}` needs a list; the addressed block is not one")));
    }
    Ok(document_ir::block_address(&node.block, id))
}

/// Rewrites an ordered list's numbers as one consecutive run from `base`.
///
/// The item count decides whether the range fits in `u32`; unordered lists keep
/// their items untouched because their items carry no numbers.
fn renumber_items(items: &mut [ListItem], base: u32) -> Result<(), ServiceError> {
    for (index, item) in items.iter_mut().enumerate() {
        let offset = u32::try_from(index).unwrap_or(u32::MAX);
        let Some(number) = base.checked_add(offset) else {
            return Err(crate::invalid(format!("editing this list would push its numbering beyond the {} maximum", u32::MAX)));
        };
        item.number = Some(number);
    }
    Ok(())
}

/// Replaces one list item's text with a single plain-text inline and returns
/// the list's address.
///
/// Only the text changes; the item keeps its number, nesting and check state.
fn set_list_item(graph: &mut DocumentGraph, id: NodeId, item: usize, text: String) -> Result<String, ServiceError> {
    let label = resolve_list(graph, id, "`list.setItem`")?;
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the list was just resolved");
    let Block::List { items, .. } = &mut node.block
    else {
        unreachable!("the list was just checked");
    };
    let count = items.len();
    let target = items
        .get_mut(item)
        .ok_or_else(|| crate::invalid(format!("item {item} is outside the list's {count} items")))?;
    target.content = vec![Inline::Text { text }];
    Ok(label)
}

/// Sets the check state of one task item and returns the list's address.
///
/// Only task lists carry a check state, so a bullet or numbered list is refused;
/// the item's text, number and nested children stay as they are.
fn set_list_item_checked(graph: &mut DocumentGraph, id: NodeId, item: usize, checked: bool) -> Result<String, ServiceError> {
    let label = resolve_list(graph, id, "`list.setItemChecked`")?;
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the list was just resolved");
    let Block::List { items, .. } = &mut node.block
    else {
        unreachable!("the list was just checked");
    };
    let count = items.len();
    let target =
        items.get_mut(item).ok_or_else(|| crate::invalid(format!("item {item} is outside the list's {count} items")))?;
    if !target.is_task {
        return Err(crate::invalid("`list.setItemChecked` needs a task list; append one with `taskList.append` first"));
    }
    target.is_checked = Some(checked);
    Ok(label)
}

/// Turns one list item into a task item, or back into a plain item.
///
/// The text and nested children stay; making an item a task also stores its
/// check state, which defaults to unchecked. Turning the task flag off clears
/// the check state because plain items carry none.
fn set_list_item_task(
    graph: &mut DocumentGraph,
    id: NodeId,
    item: usize,
    task: bool,
    checked: Option<bool>,
) -> Result<String, ServiceError> {
    let label = resolve_list(graph, id, "`list.setItemTask`")?;
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the list was just resolved");
    let Block::List { items, .. } = &mut node.block
    else {
        unreachable!("the list was just resolved");
    };
    let count = items.len();
    let target =
        items.get_mut(item).ok_or_else(|| crate::invalid(format!("item {item} is outside the list's {count} items")))?;
    target.is_task = task;
    target.is_checked = if task { Some(checked.unwrap_or(false)) } else { None };
    Ok(label)
}

/// Moves one item to a new position inside its list and returns the list's address.
///
/// The item keeps its text, check state and nested children; only its position
/// changes. An ordered list is renumbered from the number its first item carried,
/// so the run stays consecutive.
fn move_list_item(graph: &mut DocumentGraph, id: NodeId, item: usize, index: usize) -> Result<String, ServiceError> {
    let label = resolve_list(graph, id, "`list.moveItem`")?;
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the list was just resolved");
    let Block::List { ordered, items, .. } = &mut node.block
    else {
        unreachable!("the list was just checked");
    };
    let count = items.len();
    if item >= count {
        return Err(crate::invalid(format!("item {item} is outside the list's {count} items")));
    }
    if index >= count {
        return Err(crate::invalid(format!("`index` {index} is outside the list's {count} item positions")));
    }
    let base = if *ordered { items.first().and_then(|entry| entry.number).unwrap_or(1) } else { 0 };
    let moved = items.remove(item);
    items.insert(index, moved);
    if *ordered {
        renumber_items(items, base)?;
    }
    Ok(label)
}

/// Nests one item under the item before it and returns the list's address.
///
/// The IR stores nesting as child blocks, so the moved item leaves the parent
/// run and becomes an item of a nested list attached to its new parent's
/// children. When the previous item already ends with a nested list of the same
/// marker style the item joins that list, keeping one nested run per level. An
/// ordered list is renumbered on both levels so each run stays consecutive. The
/// first item has no previous sibling and is refused.
fn indent_list_item(graph: &mut DocumentGraph, id: NodeId, item: usize) -> Result<String, ServiceError> {
    let label = resolve_list(graph, id, "`list.indentItem`")?;
    if item == 0 {
        return Err(crate::invalid("`list.indentItem` needs `item` to be at least 1; the first item has no previous sibling"));
    }
    let (ordered, removed, host) = {
        let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the list was just resolved");
        let Block::List { ordered, items, .. } = &mut node.block
        else {
            unreachable!("the list was just resolved");
        };
        let count = items.len();
        if item >= count {
            return Err(crate::invalid(format!("item {item} is outside the list's {count} items")));
        }
        let base = if *ordered { items.first().and_then(|entry| entry.number).unwrap_or(1) } else { 0 };
        let removed = items.remove(item);
        if *ordered {
            renumber_items(items, base)?;
        }
        // The item before the moved one becomes its parent after removal.
        let host = items.get(item - 1).and_then(|entry| entry.children.last().copied());
        (*ordered, removed, host)
    };
    let nested = host.filter(|child| {
        graph
            .blocks
            .iter()
            .find(|node| node.id == *child)
            .is_some_and(|node| matches!(&node.block, Block::List { ordered: nested, .. } if *nested == ordered))
    });
    if let Some(child) = nested {
        let node = graph.blocks.iter_mut().find(|node| node.id == child).expect("the nested list was just resolved");
        let Block::List { items, .. } = &mut node.block
        else {
            unreachable!("the nested list was just resolved");
        };
        let base = items.first().and_then(|entry| entry.number).unwrap_or(1);
        items.push(removed);
        if ordered {
            renumber_items(items, base)?;
        }
        return Ok(label);
    }
    // A fresh nested run restarts at 1, so ordered items keep consecutive numbers.
    let mut nested = removed;
    if ordered {
        nested.number = Some(1);
    }
    let child = graph.push_block(Block::List { ordered, items: vec![nested] });
    let parent = graph.blocks.iter_mut().find(|node| node.id == id).expect("the list is still there");
    let Block::List { items, .. } = &mut parent.block
    else {
        unreachable!("the list is still a list");
    };
    let host = items.get_mut(item - 1).expect("the previous item still exists");
    host.children.push(child);
    Ok(label)
}

/// Inserts a new plain item before position `item` and returns the list's address.
///
/// An ordered list is renumbered so the run stays consecutive, keeping the
/// number its first item carried before the insertion.
fn insert_list_item(graph: &mut DocumentGraph, id: NodeId, item: usize, text: String) -> Result<String, ServiceError> {
    let label = resolve_list(graph, id, "`list.insertItem`")?;
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the list was just resolved");
    let Block::List { ordered, items, .. } = &mut node.block
    else {
        unreachable!("the list was just checked");
    };
    if item > items.len() {
        return Err(crate::invalid(format!("item {item} is outside the list's {} insertion points", items.len())));
    }
    let base = if *ordered { items.first().and_then(|entry| entry.number).unwrap_or(1) } else { 0 };
    items.insert(
        item,
        ListItem { content: vec![Inline::Text { text }], children: Vec::new(), number: None, is_task: false, is_checked: None },
    );
    if *ordered {
        renumber_items(items, base)?;
    }
    Ok(label)
}

/// Removes one item, with everything nested under it, and returns the list's address.
///
/// An ordered list is renumbered so the run stays consecutive. The last item
/// cannot be removed because that would leave an empty list block; remove the
/// whole block instead.
fn remove_list_item(graph: &mut DocumentGraph, id: NodeId, item: usize) -> Result<String, ServiceError> {
    let label = resolve_list(graph, id, "`list.removeItem`")?;
    let removed = {
        let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the list was just resolved");
        let Block::List { ordered, items, .. } = &mut node.block
        else {
            unreachable!("the list was just checked");
        };
        if item >= items.len() {
            return Err(crate::invalid(format!("item {item} is outside the list's {} items", items.len())));
        }
        if items.len() == 1 {
            return Err(crate::invalid("`list.removeItem` would leave the list empty; remove the whole list block instead"));
        }
        let base = if *ordered { items.first().and_then(|entry| entry.number).unwrap_or(1) } else { 0 };
        let removed = items.remove(item);
        if *ordered {
            renumber_items(items, base)?;
        }
        removed
    };
    for child in removed.children {
        sweep(graph, child);
    }
    Ok(label)
}

/// Renumbers one ordered list from `start` and returns its address.
///
/// The item count decides whether the range fits in `u32`; unordered lists are
/// refused because their items carry no numbers to move.
fn set_list_start(graph: &mut DocumentGraph, id: NodeId, start: u32) -> Result<String, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let label =
        graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id)).unwrap_or_default();
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the block was just resolved");
    match &mut node.block {
        Block::List { ordered: true, items, .. } => {
            for (index, item) in items.iter_mut().enumerate() {
                item.number = Some(
                    start
                        .checked_add(index as u32)
                        .ok_or_else(|| crate::invalid("the renumbered range would overflow u32"))?,
                );
            }
            Ok(label)
        }
        Block::List { ordered: false, .. } => {
            Err(crate::invalid("`list.setStart` needs an ordered list; switch it with `list.setOrdered` first"))
        }
        _ => Err(crate::invalid("`list.setStart` needs a list; the addressed block is not one")),
    }
}

/// Replaces the title of one existing heading section and returns its address.
///
/// The level, children and position stay as they are; the title becomes one
/// plain inline run so callers can rely on the exact text they passed.
fn set_heading_text(graph: &mut DocumentGraph, id: NodeId, text: &str) -> Result<String, ServiceError> {
    if !graph.blocks.iter().any(|node| node.id == id) {
        return Err(ServiceError::new("object/not-found", format!("no block has id {}", id.0)));
    }
    let label =
        graph.blocks.iter().find(|node| node.id == id).map(|node| document_ir::block_address(&node.block, id)).unwrap_or_default();
    let node = graph.blocks.iter_mut().find(|node| node.id == id).expect("the block was just resolved");
    match &mut node.block {
        Block::Section { title, .. } => {
            title.clear();
            title.push(Inline::Text { text: text.to_owned() });
            Ok(label)
        }
        _ => Err(crate::invalid("`heading.setText` needs a heading section; the addressed block is not one")),
    }
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
