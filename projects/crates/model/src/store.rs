//! Live document handle table.
//!
//! Handles, revisions and the bounded undo stack live in the shared
//! `document-session` store. This module keeps only the product state and the
//! response envelope every open/import operation returns.

use std::sync::{
    LazyLock,
    atomic::{AtomicU64, Ordering},
};

use document_ir::{DocumentGraph, DocumentId};
use document_session::Store;
use serde_json::{Value, json};

use crate::{ServiceError, loss_values, summary_json};

/// Where an opened document came from, kept for source-aware inspection.
#[derive(Clone)]
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

/// One live document plus its provenance.
#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) graph: DocumentGraph,
    pub(crate) source: Source,
}

impl Entry {
    pub(crate) fn new(graph: DocumentGraph, source: Source) -> Self {
        Self { graph, source }
    }
}

impl document_session::Revisioned for Entry {
    fn revision(&self) -> u64 {
        self.graph.revision
    }

    fn set_revision(&mut self, revision: u64) {
        self.graph.revision = revision;
    }
}

impl From<document_session::SessionError> for ServiceError {
    fn from(error: document_session::SessionError) -> Self {
        // Absence is reported through `Option` at every call site, so a
        // session-level failure is always a rejected request, never a handle
        // without its frozen payload.
        ServiceError::argument(error.message)
    }
}

static STORE: LazyLock<Store<Entry>> = LazyLock::new(Store::new);
static NEXT_DOCUMENT: AtomicU64 = AtomicU64::new(1);

/// Registers one entry and returns the `{ handle, document, complete, losses }` response.
pub(crate) fn register(entry: Entry) -> Value {
    let document = summary_json(&entry.graph);
    let complete = entry.graph.coverage.complete;
    let losses = loss_values(&entry.graph);
    let handle = insert(entry);
    json!({ "handle": handle, "document": document, "complete": complete, "losses": losses })
}

/// Allocates one process-unique document identity.
pub(crate) fn next_document_id() -> DocumentId {
    DocumentId(NEXT_DOCUMENT.fetch_add(1, Ordering::Relaxed))
}

/// Inserts one entry and returns its handle.
pub(crate) fn insert(entry: Entry) -> u64 {
    STORE.insert(entry)
}

/// Runs `work` against one entry.
pub(crate) fn with<R>(handle: u64, work: impl FnOnce(&Entry) -> R) -> Option<R> {
    STORE.with(handle, work)
}

/// Runs `work` against one entry with undo and redo access.
pub(crate) fn with_history<R>(handle: u64, work: impl FnOnce(&mut document_session::Session<'_, Entry>) -> R) -> Option<R> {
    STORE.with_session(handle, work)
}

/// Removes one handle; returns whether it was live.
pub(crate) fn remove(handle: u64) -> bool {
    STORE.remove(handle)
}
