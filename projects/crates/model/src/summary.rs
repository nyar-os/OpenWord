//! Summary projection from the semantic graph.
//!
//! Sections expand recursively through their child ids; a paragraph or table is
//! attributed to the nearest enclosing section, or to `-1` when the document has
//! no section around it. Only committed state is read, so a summary always
//! matches a revision.

use std::collections::HashSet;

use document_ir::{Block, DocumentGraph, Inline, NodeId, SemanticStatus, TableRow};
use serde_json::{Value, json};

/// Projects one graph into the authoring summary shape.
pub(crate) fn project(graph: &DocumentGraph) -> Value {
    let mut walker = Walker::new(graph);
    walker.walk();
    json!({
        "id": format!("doc-{}", graph.id.0),
        "revision": graph.revision,
        "title": graph.metadata.title,
        "sections": walker.sections,
        "paragraphs": walker.paragraphs,
        "tables": walker.tables,
    })
}

struct Walker<'a> {
    graph: &'a DocumentGraph,
    roots: Vec<NodeId>,
    visited: HashSet<NodeId>,
    sections: Vec<Value>,
    paragraphs: Vec<Value>,
    tables: Vec<Value>,
}

impl<'a> Walker<'a> {
    fn new(graph: &'a DocumentGraph) -> Self {
        let mut referenced = HashSet::new();
        for node in &graph.blocks {
            match &node.block {
                Block::Section { children, .. } | Block::Quote { children } => referenced.extend(children.iter().copied()),
                Block::List { items, .. } => {
                    for item in items {
                        referenced.extend(item.children.iter().copied());
                    }
                }
                Block::Table { head, body, foot, .. } => {
                    for row in head.iter().chain(body.iter()).chain(foot.iter()) {
                        for cell in &row.cells {
                            referenced.extend(cell.children.iter().copied());
                        }
                    }
                }
                _ => {}
            }
        }
        let roots = graph.blocks.iter().map(|node| node.id).filter(|id| !referenced.contains(id)).collect();
        Self { graph, roots, visited: HashSet::new(), sections: Vec::new(), paragraphs: Vec::new(), tables: Vec::new() }
    }

    fn walk(&mut self) {
        for root in self.roots.clone() {
            self.visit(root, None);
        }
    }

    fn visit(&mut self, id: NodeId, section: Option<usize>) {
        if !self.visited.insert(id) {
            return;
        }
        let Some(node) = self.graph.blocks.iter().find(|node| node.id == id)
        else {
            return;
        };
        match &node.block {
            Block::Section { title, children, .. } => {
                let index = self.sections.len();
                self.sections.push(json!({
                    "index": index,
                    "id": format!("section-{}", id.0),
                    "title": title_text(title),
                }));
                for child in children {
                    self.visit(*child, Some(index));
                }
            }
            Block::Paragraph { content } => {
                let index = self.paragraphs.len();
                self.paragraphs.push(json!({
                    "index": index,
                    "sectionIndex": section.map_or(-1, |value| value as i64),
                    "style": Value::Null,
                    "text": flatten(content),
                }));
            }
            Block::Table { head, body, foot, .. } => {
                let rows = head.len() + body.len() + foot.len();
                let mut columns = 0u32;
                for row in head.iter().chain(body.iter()).chain(foot.iter()) {
                    columns = columns.max(row_columns(row));
                }
                let index = self.tables.len();
                self.tables.push(json!({
                    "index": index,
                    "sectionIndex": section.map_or(-1, |value| value as i64),
                    "rows": rows,
                    "columns": columns,
                }));
                for row in head.iter().chain(body.iter()).chain(foot.iter()) {
                    for cell in &row.cells {
                        for child in &cell.children {
                            self.visit(*child, section);
                        }
                    }
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    for child in &item.children {
                        self.visit(*child, section);
                    }
                }
            }
            Block::Quote { children } => {
                for child in children {
                    self.visit(*child, section);
                }
            }
            _ => {}
        }
    }
}

fn row_columns(row: &TableRow) -> u32 {
    row.cells.iter().fold(0u32, |total, cell| total.saturating_add(cell.colspan))
}

/// Projects the recorded import/export loss markers into the report shape every
/// surface reads.
pub(crate) fn loss_values(graph: &DocumentGraph) -> Vec<Value> {
    graph
        .coverage
        .loss
        .iter()
        .map(|marker| {
            let severity = if matches!(marker.status, SemanticStatus::Unsupported) { "error" } else { "warning" };
            json!({ "code": marker.code, "severity": severity, "message": marker.message })
        })
        .collect()
}

/// Flattens inline content into the text every surface reads.
pub(crate) fn flatten(inlines: &[Inline]) -> String {
    inlines.iter().map(Inline::plain_text).collect()
}

fn title_text(title: &[Inline]) -> Value {
    let text = flatten(title);
    if text.trim().is_empty() { Value::Null } else { Value::String(text) }
}
