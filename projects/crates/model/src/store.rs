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

use crate::summary;

static HANDLES: Mutex<Option<HashMap<u64, Entry>>> = Mutex::new(None);
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static NEXT_DOCUMENT: AtomicU64 = AtomicU64::new(1);

/// Where an opened document came from, kept for source-aware inspection.
pub(crate) enum Source {
    /// A native `.oword` document envelope.
    Project,
    /// A Word OPC package whose bytes stay available for structural checks.
    Docx { label: String, bytes: Vec<u8> },
    /// Any other imported format, labelled with its canonical name.
    Other(&'static str),
}

/// One live document plus its provenance.
pub(crate) struct Entry {
    pub(crate) graph: DocumentGraph,
    pub(crate) source: Source,
}

impl Entry {
    pub(crate) fn new(graph: DocumentGraph, source: Source) -> Self {
        Self { graph, source }
    }
}

/// Registers one entry and returns the `{ handle, document }` response.
pub(crate) fn register(entry: Entry) -> Value {
    let summary = summary::project(&entry.graph);
    let handle = insert(entry);
    json!({ "handle": handle, "document": summary })
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
