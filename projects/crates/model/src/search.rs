//! Read-only document traversal and text search.
//!
//! Search addresses blocks by the same depth-first order the summary uses, so a
//! `blockIndex` reported here is the index `paragraph.insert` expects. Offsets
//! are Unicode scalar counts inside one block's plain text, never byte offsets.

use std::collections::{HashMap, HashSet};

use document_ir::{Block, DocumentGraph, NodeId};
use serde_json::{Value, json};

use crate::summary;

/// Default number of hits one search returns.
pub(crate) const DEFAULT_LIMIT: usize = 100;

/// Direct child block ids of one block, in document order.
pub(crate) fn children(block: &Block) -> Vec<NodeId> {
    let mut out = Vec::new();
    match block {
        Block::Section { children, .. } => out.extend(children.iter().copied()),
        Block::Quote { children } => out.extend(children.iter().copied()),
        Block::List { items, .. } => {
            for item in items {
                out.extend(item.children.iter().copied());
            }
        }
        Block::Table { head, body, foot, .. } => {
            for row in head.iter().chain(body.iter()).chain(foot.iter()) {
                for cell in &row.cells {
                    out.extend(cell.children.iter().copied());
                }
            }
        }
        _ => {}
    }
    out
}

/// Top-level blocks no container references, in graph order.
pub(crate) fn roots(graph: &DocumentGraph) -> Vec<NodeId> {
    let mut referenced = HashSet::new();
    for node in &graph.blocks {
        referenced.extend(children(&node.block));
    }
    graph.blocks.iter().map(|node| node.id).filter(|id| !referenced.contains(id)).collect()
}

/// Depth-first document order, parents before their children.
pub(crate) fn flat_order(graph: &DocumentGraph) -> Vec<NodeId> {
    let mut order = Vec::with_capacity(graph.blocks.len());
    let mut seen = HashSet::new();
    for root in roots(graph) {
        push_pre_order(graph, root, &mut seen, &mut order);
    }
    order
}

fn push_pre_order(graph: &DocumentGraph, id: NodeId, seen: &mut HashSet<NodeId>, order: &mut Vec<NodeId>) {
    if !seen.insert(id) {
        return;
    }
    order.push(id);
    if let Some(node) = graph.blocks.iter().find(|node| node.id == id) {
        for child in children(&node.block) {
            push_pre_order(graph, child, seen, order);
        }
    }
}

/// Stable address of one block: `section-N` for sections, `node-N` otherwise.
pub(crate) fn block_id(block: &Block, id: NodeId) -> String {
    match block {
        Block::Section { .. } => format!("section-{}", id.0),
        _ => format!("node-{}", id.0),
    }
}

/// Parses `node-N`, `section-N` or a bare `N` into a node id.
pub(crate) fn parse_block_id(raw: &str) -> Option<NodeId> {
    let trimmed = raw.trim();
    let digits = trimmed.strip_prefix("node-").or_else(|| trimmed.strip_prefix("section-")).unwrap_or(trimmed);
    digits.parse().ok().map(NodeId)
}

/// Finds every occurrence of `query` in block text, up to `limit` hits.
pub(crate) fn find(graph: &DocumentGraph, query: &str, limit: usize) -> Vec<Value> {
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut indices = HashMap::new();
    for (index, id) in flat_order(graph).into_iter().enumerate() {
        indices.insert(id, index);
    }
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for root in roots(graph) {
        collect(graph, root, None, &indices, query, limit, &mut seen, &mut hits);
    }
    hits
}

fn collect(
    graph: &DocumentGraph,
    id: NodeId,
    section: Option<NodeId>,
    indices: &HashMap<NodeId, usize>,
    query: &str,
    limit: usize,
    seen: &mut HashSet<NodeId>,
    hits: &mut Vec<Value>,
) {
    if hits.len() >= limit || !seen.insert(id) {
        return;
    }
    let Some(node) = graph.blocks.iter().find(|node| node.id == id)
    else {
        return;
    };
    let Some(index) = indices.get(&id).copied()
    else {
        return;
    };
    let block = block_id(&node.block, id);
    let path = section.map(|value| format!("section-{}", value.0));
    match &node.block {
        Block::Section { title, children, .. } => {
            push_hits(&block, index, path.as_deref(), &summary::flatten(title), query, limit, hits);
            for child in children {
                collect(graph, *child, Some(id), indices, query, limit, seen, hits);
            }
        }
        Block::Paragraph { content } => {
            push_hits(&block, index, path.as_deref(), &summary::flatten(content), query, limit, hits);
        }
        Block::List { items, .. } => {
            for item in items {
                push_hits(&block, index, path.as_deref(), &summary::flatten(&item.content), query, limit, hits);
                for child in &item.children {
                    collect(graph, *child, section, indices, query, limit, seen, hits);
                }
            }
        }
        Block::Table { head, body, foot, .. } => {
            for row in head.iter().chain(body.iter()).chain(foot.iter()) {
                for cell in &row.cells {
                    for child in &cell.children {
                        collect(graph, *child, section, indices, query, limit, seen, hits);
                    }
                }
            }
        }
        Block::Quote { children } => {
            for child in children {
                collect(graph, *child, section, indices, query, limit, seen, hits);
            }
        }
        _ => {}
    }
}

fn push_hits(block: &str, index: usize, path: Option<&str>, text: &str, query: &str, limit: usize, hits: &mut Vec<Value>) {
    for (byte, matched) in text.match_indices(query) {
        if hits.len() >= limit {
            break;
        }
        let start = text[..byte].chars().count();
        let end = start + matched.chars().count();
        hits.push(json!({
            "blockId": block,
            "blockIndex": index,
            "path": path,
            "start": start,
            "end": end,
            "text": matched,
        }));
    }
}
