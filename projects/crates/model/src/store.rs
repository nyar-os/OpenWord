//! Live document handle table.
//!
//! Handles are opaque process-local integers. The table never panics on a
//! poisoned lock, and `remove` is infallible so `close` stays idempotent.

use std::{
    collections::HashMap,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use document_ir::{DocumentGraph, DocumentId};
use serde_json::{Value, json};

use crate::{ServiceError, summary};

/// In-memory undo depth. History never reaches the `.oword` container.
const HISTORY_LIMIT: usize = 50;

static HANDLES: Mutex<Option<HashMap<u64, Entry>>> = Mutex::new(None);
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static NEXT_DOCUMENT: AtomicU64 = AtomicU64::new(1);

/// Where an opened document came from, kept for source-aware inspection.
pub(crate) enum Source {
    /// A native `.oword` document envelope.
    Project,
    /// A Word OPC package whose bytes stay available for structural checks.
    Docx { label: String, bytes: Vec<u8> },
    /// An RTF stream, labelled with the path it was read from.
    Rtf { label: String },
    /// Any other imported format, labelled with its canonical name.
    Other(&'static str),
}

/// One live document plus its provenance and in-memory history.
pub(crate) struct Entry {
    pub(crate) graph: DocumentGraph,
    pub(crate) source: Source,
    /// Graphs as they were before each committed mutation, oldest first.
    undo: Vec<DocumentGraph>,
    /// Graphs discarded by undo, most recently undone last.
    redo: Vec<DocumentGraph>,
}

impl Entry {
    pub(crate) fn new(graph: DocumentGraph, source: Source) -> Self {
        Self { graph, source, undo: Vec::new(), redo: Vec::new() }
    }

    /// Records one committed mutation: the prior state joins the undo stack and
    /// the redo path is discarded, because the future it described is gone.
    pub(crate) fn commit_history(&mut self, before: DocumentGraph) {
        push_capped(&mut self.undo, before);
        self.redo.clear();
    }

    /// Restores the state `steps` mutations back and bumps the revision once.
    ///
    /// The revision is not restored from the snapshot: revisions are monotonic,
    /// so an undone document stays distinguishable from the one it replaced.
    pub(crate) fn undo(&mut self, steps: usize) -> Result<(), ServiceError> {
        if steps == 0 || steps > self.undo.len() {
            return Err(crate::invalid(format!("cannot undo {steps} step(s); {} available", self.undo.len())));
        }
        let revision = self.graph.revision;
        for _ in 0..steps {
            let snapshot = self.undo.pop().expect("the step count was checked against the stack");
            push_capped(&mut self.redo, std::mem::replace(&mut self.graph, snapshot));
        }
        self.graph.revision = revision + 1;
        Ok(())
    }

    /// Replays `steps` undone mutations and bumps the revision once.
    pub(crate) fn redo(&mut self, steps: usize) -> Result<(), ServiceError> {
        if steps == 0 || steps > self.redo.len() {
            return Err(crate::invalid(format!("cannot redo {steps} step(s); {} available", self.redo.len())));
        }
        let revision = self.graph.revision;
        for _ in 0..steps {
            let snapshot = self.redo.pop().expect("the step count was checked against the stack");
            push_capped(&mut self.undo, std::mem::replace(&mut self.graph, snapshot));
        }
        self.graph.revision = revision + 1;
        Ok(())
    }

    /// Whether an undo snapshot is available.
    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether a redo snapshot is available.
    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

fn push_capped(stack: &mut Vec<DocumentGraph>, snapshot: DocumentGraph) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(snapshot);
}

/// Registers one entry and returns the `{ handle, document, complete, losses }` response.
pub(crate) fn register(entry: Entry) -> Value {
    let document = summary::project(&entry.graph);
    let complete = entry.graph.coverage.complete;
    let losses = summary::loss_values(&entry.graph);
    let handle = insert(entry);
    json!({ "handle": handle, "document": document, "complete": complete, "losses": losses })
}

/// Allocates one process-unique document identity.
pub(crate) fn next_document_id() -> DocumentId {
    DocumentId(NEXT_DOCUMENT.fetch_add(1, Ordering::Relaxed))
}

fn handles() -> MutexGuard<'static, Option<HashMap<u64, Entry>>> {
    HANDLES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Inserts one entry and returns its handle.
pub(crate) fn insert(entry: Entry) -> u64 {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    handles().get_or_insert_with(HashMap::new).insert(handle, entry);
    handle
}

/// Runs `work` against one entry without taking a second lock.
pub(crate) fn with<R>(handle: u64, work: impl FnOnce(&Entry) -> R) -> Option<R> {
    handles().as_ref().and_then(|table| table.get(&handle)).map(work)
}

/// Runs `work` against one mutable entry.
pub(crate) fn with_mut<R>(handle: u64, work: impl FnOnce(&mut Entry) -> R) -> Option<R> {
    handles().as_mut().and_then(|table| table.get_mut(&handle)).map(work)
}

/// Removes one handle; returns whether it was live.
pub(crate) fn remove(handle: u64) -> bool {
    handles().as_mut().and_then(|table| table.remove(&handle)).is_some()
}
