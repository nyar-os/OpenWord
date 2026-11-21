//! Whole-document merge.
//!
//! Merging copies every block of the donor graph into the target, appending the
//! donor's roots after the target's roots. Node identities are reassigned so the
//! copied subtree never aliases a target node, while the donor's container lists
//! (sections, quotes, list items, table cells) keep their structure. The target
//! revision advances exactly once per merge.

use std::collections::HashMap;

use document_ir::{Block, DocumentGraph, NodeId};

/// What one merge appended.
pub(crate) struct MergeOutcome {
    /// Number of block nodes copied, including nested children.
    pub(crate) blocks_added: usize,
}

/// Appends every block of `donor` to `target` with fresh node ids.
pub(crate) fn append(target: &mut DocumentGraph, donor: &DocumentGraph) -> MergeOutcome {
    let start = target.blocks.len();
    let mut ids = HashMap::with_capacity(donor.blocks.len());
    for node in &donor.blocks {
        let id = target.push_block(node.block.clone());
        ids.insert(node.id, id);
    }
    for node in &mut target.blocks[start..] {
        remap_children(&mut node.block, &ids);
    }
    for marker in &donor.coverage.loss {
        target.push_loss(marker.clone());
    }
    target.bump_revision();
    MergeOutcome { blocks_added: target.blocks.len() - start }
}

/// Rewrites every child reference of one copied block through the id map.
fn remap_children(block: &mut Block, ids: &HashMap<NodeId, NodeId>) {
    match block {
        Block::Section { children, .. } | Block::Quote { children } => remap_ids(children, ids),
        Block::List { items, .. } => {
            for item in items {
                remap_ids(&mut item.children, ids);
            }
        }
        Block::Table { head, body, foot, .. } => {
            for row in head.iter_mut().chain(body.iter_mut()).chain(foot.iter_mut()) {
                for cell in &mut row.cells {
                    remap_ids(&mut cell.children, ids);
                }
            }
        }
        _ => {}
    }
}

fn remap_ids(children: &mut [NodeId], ids: &HashMap<NodeId, NodeId>) {
    for child in children {
        if let Some(mapped) = ids.get(child) {
            *child = *mapped;
        }
    }
}
