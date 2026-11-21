//! Atomic edit operations.
//!
//! A request is parsed and validated as a whole before the first mutation, so a
//! rejected operation never leaves a half-applied revision. Each applied
//! operation advances the revision exactly once.

use std::collections::HashSet;

use document_ir::{Block, DocumentGraph, Inline, NodeId, TableRow};
use serde_json::{Value, json};

use crate::{ServiceError, search};

#[derive(Clone)]
enum Operation {
    Paragraph(String),
    Heading { level: u8, text: String },
    Table(Vec<Vec<String>>),
    Metadata { title: Option<String>, author: Option<String> },
    ReplaceText { find: String, replace: String, limit: Option<usize> },
    RemoveBlock { id: NodeId },
    InsertParagraph { index: usize, text: String },
    SetCell { table: usize, row: usize, column: usize, text: String },
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
            let id = search::parse_block_id(&raw)
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
            let label = graph.blocks.iter().find(|node| node.id == id).map(|node| search::block_id(&node.block, id));
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
    }
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
    for id in search::flat_order(graph) {
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
        for child in search::children(&node.block) {
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
    let order = search::flat_order(graph);
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

fn set_cell(graph: &mut DocumentGraph, table: usize, row: usize, column: usize, text: String) -> Result<(), ServiceError> {
    let tables = search::flat_order(graph)
        .into_iter()
        .filter(|id| graph.blocks.iter().any(|node| node.id == *id && matches!(node.block, Block::Table { .. })))
        .collect::<Vec<_>>();
    let Some(table_id) = tables.get(table).copied()
    else {
        return Err(crate::invalid(format!("`table` {table} is outside the document's {} tables", tables.len())));
    };

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
