//! Nested heading outline over one committed graph.
//!
//! The summary lists sections flat. Navigation surfaces need the hierarchy the
//! heading levels describe, so this projection nests each section under the
//! closest preceding heading with a lower level.

use document_ir::{Block, DocumentGraph};
use serde_json::{Value, json};

struct Heading {
    id: String,
    level: u8,
    title: Option<String>,
    block_index: usize,
}

/// Projects one graph into its nested heading outline.
pub(crate) fn report(graph: &DocumentGraph) -> Value {
    let mut headings = Vec::new();
    for (block_index, id) in document_ir::flat_order(graph).into_iter().enumerate() {
        let Some(node) = graph.block(id)
        else {
            continue;
        };
        let Block::Section { level, title, .. } = &node.block
        else {
            continue;
        };
        let text = document_ir::plain_text(title);
        headings.push(Heading {
            id: format!("section-{}", id.0),
            level: *level,
            title: (!text.trim().is_empty()).then_some(text),
            block_index,
        });
    }

    let mut position = 0;
    let items = nest(&headings, &mut position, 0);
    json!({ "revision": graph.revision, "items": items })
}

/// Builds one level's items, consuming every heading that is deeper than
/// `parent_level`. A heading at or above the parent closes the current level.
fn nest(headings: &[Heading], position: &mut usize, parent_level: u8) -> Vec<Value> {
    let mut items = Vec::new();
    while let Some(heading) = headings.get(*position) {
        if heading.level <= parent_level {
            break;
        }
        *position += 1;
        let children = nest(headings, position, heading.level);
        items.push(json!({
            "id": heading.id,
            "level": heading.level,
            "title": heading.title,
            "blockIndex": heading.block_index,
            "children": children,
        }));
    }
    items
}
