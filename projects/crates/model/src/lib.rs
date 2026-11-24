//! Document model and operations for OpenWord.
//!
//! The semantic model, importers and exporters live in the pinned upstream
//! document engine. This crate owns the product surface: a live document handle
//! table, format routing, summary projection, rule inspection, atomic edits and
//! export dispatch. Nothing here re-implements a format adapter.

#![deny(missing_docs)]

mod edit;
mod files;
mod inspect;
mod merge;
mod outline;
mod replace;
mod store;

use std::{fmt, io, path::Path};

use document_ir::DocumentGraph;
use serde_json::{Map, Value, json};

/// Every document format the product classifies.
pub const DOCUMENT_FORMATS: &[&str] = &["docx", "docm", "dotx", "dotm", "doc", "rtf", "odt", "txt", "md", "html", "pdf"];

/// Inspection rules the product declares.
const INSPECT_CHECKS: &[&str] = &["styles", "fonts", "images", "sections", "tables", "links"];

/// Loss codes the RTF route can report.
const RTF_LOSS_CODES: &[&str] = &[
    "import.rtf.table",
    "import.rtf.list",
    "import.rtf.image",
    "import.rtf.field",
    "import.rtf.color",
    "import.rtf.font",
    "export.rtf.table",
    "export.rtf.image",
    "export.rtf.link",
];

/// Projects one graph into the authoring summary shape.
pub(crate) fn summary_json(graph: &DocumentGraph) -> Value {
    serde_json::to_value(document_ir::summarize(graph)).expect("a document summary always serializes")
}

/// Projects the recorded loss markers into the report shape every surface reads.
pub(crate) fn loss_values(graph: &DocumentGraph) -> Vec<Value> {
    document_ir::losses(graph).into_iter().map(|entry| json!(entry)).collect()
}

/// Per-document render targets.
const RENDER_FORMATS: &[&str] = &["svg", "png", "pdf"];

/// Export targets the writer can produce.
const EXPORT_TARGETS: &[&str] = &["docx", "dotx", "pdf", "html", "md", "txt", "rtf"];

/// Stable failure of one binding operation.
///
/// Construction goes through the typed constructors below: each one fixes a
/// stable code and attaches the parameters that code's contract promises, so
/// the binding forwards `code`, `params` and the original OS error instead of
/// re-deriving them from prose. The free-form pair is crate-internal only, for
/// forwarding upstream codes that already carry their own contract.
#[derive(Debug)]
pub struct ServiceError {
    code: String,
    message: String,
    params: Value,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl ServiceError {
    fn build(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_owned(), message: message.into(), params: Value::Null, source: None }
    }

    fn with_params(mut self, params: Value) -> Self {
        self.params = params;
        self
    }

    fn source(mut self, error: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(error));
        self
    }

    /// Crate-internal forwarder for an upstream code that already carries its own contract.
    pub(crate) fn forward(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), params: Value::Null, source: None }
    }

    /// `session/argument-invalid`: a request parameter was rejected.
    pub fn argument(message: impl Into<String>) -> Self {
        Self::build("session/argument-invalid", message)
    }

    /// `document/not-found`: no live document has this handle.
    pub fn document_not_found(handle: u64) -> Self {
        Self::build("document/not-found", format!("no live document has handle {handle}")).with_params(json!({ "handle": handle }))
    }

    /// `document/revision-conflict`: the document moved past the expected revision.
    pub fn revision_conflict(expected: u64, actual: u64) -> Self {
        Self::build(
            "document/revision-conflict",
            format!("expected revision {expected} but the document is at revision {actual}"),
        )
        .with_params(json!({ "expected": expected, "actual": actual }))
    }

    /// `object/not-found`: the addressed object is absent from the document.
    pub fn not_found(kind: &str, id: impl fmt::Display) -> Self {
        let id = id.to_string();
        Self::build("object/not-found", format!("`{kind}` `{id}` does not exist")).with_params(json!({ "kind": kind, "id": id }))
    }

    /// `object/not-found`: no block carries this id.
    pub fn block_not_found(id: impl fmt::Display) -> Self {
        let id = id.to_string();
        Self::build("object/not-found", format!("no block has id {id}")).with_params(json!({ "kind": "block", "id": id }))
    }

    /// `object/unsupported`: the edit operation name is not implemented.
    pub fn operation_unsupported(operation: &str) -> Self {
        Self::build("object/unsupported", format!("edit operation `{operation}` is not supported"))
            .with_params(json!({ "operation": "edit operation", "name": operation }))
    }

    /// `object/unsupported`: the merge mode is not implemented.
    pub fn merge_mode_unsupported(mode: &str) -> Self {
        Self::build("object/unsupported", format!("merge mode `{mode}` is not supported"))
            .with_params(json!({ "operation": "merge mode", "name": mode }))
    }

    /// `format/invalid`: an input or serialization failed format validation.
    pub fn invalid(format: &str, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::build("format/invalid", format!("`{format}` input is invalid: {reason}"))
            .with_params(json!({ "format": format, "reason": reason }))
    }

    /// `format/unsupported`: the requested format has no route in this build.
    pub fn unsupported(format: &str) -> Self {
        Self::build("format/unsupported", format!("`{format}` has no route in this build")).with_params(json!({ "format": format }))
    }

    /// `session/budget-exceeded`: an upstream budget refused the operation.
    pub fn budget(message: impl Into<String>) -> Self {
        Self::build("session/budget-exceeded", message)
    }

    /// `io/read`: reading a file failed; the OS error kind is preserved.
    pub fn io_read(path: &Path, error: io::Error) -> Self {
        let kind = format!("{:?}", error.kind());
        Self::build("io/read", format!("could not read `{}`: {error}", path.display()))
            .with_params(json!({ "path": path.display().to_string(), "kind": kind }))
            .source(error)
    }

    /// `io/write`: publishing a file failed.
    pub fn io_write(path: &Path, reason: impl Into<String>) -> Self {
        Self::build("io/write", format!("could not publish `{}`: {}", path.display(), reason.into()))
            .with_params(json!({ "path": path.display().to_string() }))
    }

    /// `io/exists`: the target is present and overwrite was not granted.
    pub fn io_exists(path: &Path) -> Self {
        Self::build("io/exists", format!("`{}` already exists; pass overwrite to replace it", path.display()))
            .with_params(json!({ "path": path.display().to_string() }))
    }

    /// Stable, comparable code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Human-readable message; never carries binary payloads.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Typed per-code parameters, or `Value::Null` when the code carries none.
    pub fn params(&self) -> &Value {
        &self.params
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|error| error.as_ref() as &(dyn std::error::Error + 'static))
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> ServiceError {
    ServiceError::argument(message)
}

pub(crate) fn document_not_found(handle: u64) -> ServiceError {
    ServiceError::document_not_found(handle)
}

/// What this build can actually do.
///
/// The matrix is derived from the wired engine routes: a format is `supported`
/// only when `open`/`export` route it, `partial` when the adapter declares loss,
/// and `unsupported` when no route exists. Classification is never a support
/// claim by itself.
pub fn capabilities() -> Value {
    let matrix: [(&str, &str, &str, &str); 11] = [
        ("docx", "supported", "supported", "supported"),
        ("docm", "supported", "supported", "partial"),
        ("dotx", "unsupported", "unsupported", "unsupported"),
        ("dotm", "unsupported", "unsupported", "unsupported"),
        ("doc", "unsupported", "unsupported", "unsupported"),
        ("rtf", "partial", "supported", "partial"),
        ("odt", "unsupported", "unsupported", "unsupported"),
        ("txt", "supported", "supported", "supported"),
        ("md", "supported", "supported", "supported"),
        ("html", "supported", "supported", "supported"),
        ("pdf", "partial", "partial", "supported"),
    ];

    let mut formats = Map::new();
    for (format, import, edit_level, export) in matrix {
        let mut entry = json!({ "import": import, "edit": edit_level, "export": export });
        if format == "rtf" {
            entry["lossCodes"] = json!(RTF_LOSS_CODES);
        }
        formats.insert((*format).to_string(), entry);
    }

    let mut checks = Map::new();
    for check in INSPECT_CHECKS {
        checks.insert((*check).to_string(), json!("supported"));
    }

    let mut renderers = Map::new();
    for format in RENDER_FORMATS {
        renderers.insert((*format).to_string(), json!("unsupported"));
    }

    let mut export_targets = Map::new();
    for target in EXPORT_TARGETS {
        let level = match *target {
            "docx" | "pdf" | "html" | "md" | "txt" => "supported",
            "rtf" => "partial",
            _ => "unsupported",
        };
        export_targets.insert((*target).to_string(), json!(level));
    }

    json!({
        "engine": { "package": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION") },
        "project": "supported",
        "formats": formats,
        "checks": checks,
        "renderers": renderers,
        "exportTargets": export_targets,
    })
}

/// Creates a document from `{ title?, author?, template? }` and returns its
/// handle and summary.
///
/// A `template` is any path `open` can route; its blocks become the new
/// document, and `title`/`author` are applied through the same metadata edit
/// an explicit `metadata.set` uses.
pub fn create(request: &Value) -> Result<Value, ServiceError> {
    let title = files::optional_string(request, "title")?;
    let author = files::optional_string(request, "author")?;
    match files::optional_string(request, "template")? {
        Some(template) => {
            let mut entry = files::load(Path::new(&template))?;
            if title.is_some() || author.is_some() {
                edit::apply(
                    &mut entry.graph,
                    &json!({
                        "operations": [{ "type": "metadata.set", "title": title, "author": author }],
                    }),
                )?;
            }
            Ok(store::register(entry))
        }
        None => {
            let graph = files::create_graph(title, author);
            Ok(store::register(store::Entry::new(graph, store::Source::Project)))
        }
    }
}

/// Opens a project, Word, Markdown, HTML, plain-text or PDF file.
pub fn open(path: &Path) -> Result<Value, ServiceError> {
    Ok(store::register(files::load(path)?))
}

/// Writes one document identified by `handle` to `path` as a native project.
pub fn save(handle: u64, path: &Path, overwrite: bool) -> Result<Value, ServiceError> {
    let result = store::with(handle, |entry| files::save_graph(&entry.graph, path, overwrite))
        .ok_or_else(|| document_not_found(handle))??;
    Ok(json!({ "path": path.display().to_string(), "revision": result.revision, "bytes": result.bytes }))
}

/// Runs rule checks against one document handle.
pub fn inspect(handle: u64, checks: Option<Vec<String>>) -> Result<Value, ServiceError> {
    store::with(handle, |entry| inspect::report(&entry.graph, &entry.source, checks.as_deref()))
        .ok_or_else(|| document_not_found(handle))
}

/// Returns the committed summary of one document handle.
pub fn summary(handle: u64) -> Result<Value, ServiceError> {
    store::with(handle, |entry| summary_json(&entry.graph)).ok_or_else(|| document_not_found(handle))
}

/// Returns the nested heading outline of one document handle.
///
/// Every section appears once, nested under the closest preceding heading with a
/// lower level. Each item carries its `section-N` id, heading level, title
/// (`null` when empty), depth-first `blockIndex` and child items, so a caller can
/// render a table of contents or address a heading without walking the summary.
pub fn outline(handle: u64) -> Result<Value, ServiceError> {
    store::with(handle, |entry| outline::report(&entry.graph)).ok_or_else(|| document_not_found(handle))
}

/// Returns block, paragraph, heading, table, word and character counts for one
/// document handle.
pub fn stats(handle: u64) -> Result<Value, ServiceError> {
    store::with(handle, |entry| serde_json::to_value(document_ir::statistics(&entry.graph)).expect("statistics always serialize"))
        .ok_or_else(|| document_not_found(handle))
}

/// Writes one export target from a document handle.
pub fn export(handle: u64, format: &str, target: &Path, overwrite: bool) -> Result<Value, ServiceError> {
    let report = store::with(handle, |entry| files::export_graph(&entry.graph, format, target, overwrite))
        .ok_or_else(|| document_not_found(handle))??;
    Ok(report)
}

/// Searches one document for text across section titles, paragraphs, list
/// items and table cells, returning at most `limit` hits (default 100).
pub fn find(handle: u64, query: &str, limit: Option<u32>) -> Result<Value, ServiceError> {
    if query.is_empty() {
        return Err(invalid("`query` must be a non-empty string"));
    }
    let limit = limit.map_or(100, |value| value as usize);
    store::with(handle, |entry| serde_json::to_value(document_ir::find(&entry.graph, query, limit)).expect("hits always serialize"))
        .ok_or_else(|| document_not_found(handle))
}

/// Replaces text in one document and reports the affected blocks.
///
/// The request is `{ find, replace, caseSensitive?, wholeWord?, regex?, limit?,
/// expectedRevision? }`. `find` must be non-empty, `replace` may be empty to
/// delete matches, matching is case-sensitive by default, `wholeWord` defaults
/// to `false`, and `limit` caps the total number of replacements. With `regex`
/// set, `find` is a pattern whose capture groups are referenced as `$1` or
/// `$name` in `replace`; `wholeWord` is refused because the boundary belongs in
/// the pattern. Matching covers the same text `find` searches: section titles,
/// paragraphs, list items, table cells and authored inline display text.
///
/// A committing pass advances the revision once and records the prior graph for
/// undo. A pass with no match changes nothing, so its revision and history stay
/// untouched. `expectedRevision` rejects a stale caller with
/// `document/revision-conflict` before any text changes.
pub fn replace(handle: u64, request: &Value) -> Result<Value, ServiceError> {
    let request = replace::Request::parse(request)?;
    store::with_history(handle, |entry| {
        if let Some(expected) = request.expected_revision
            && expected != entry.graph.revision
        {
            return Err(ServiceError::revision_conflict(expected, entry.graph.revision));
        }
        let before = entry.state().clone();
        let outcome = replace::apply(&mut entry.graph, &request);
        if outcome.replaced > 0 {
            entry.graph.bump_revision();
            entry.commit(before);
        }
        Ok(json!({
            "document": summary_json(&entry.graph),
            "revision": entry.graph.revision,
            "replaced": outcome.replaced,
            "affected": outcome.affected,
        }))
    })
    .ok_or_else(|| document_not_found(handle))?
}

/// Applies a batch of edits and returns the new summary and revision.
///
/// A successful batch records the prior graph for undo and clears the redo
/// path; a rejected batch leaves both the document and its history untouched.
pub fn edit(handle: u64, request: &Value) -> Result<Value, ServiceError> {
    store::with_history(handle, |entry| {
        let before = entry.state().clone();
        let results = edit::apply(&mut entry.graph, request)?;
        entry.commit(before);
        Ok(json!({
            "document": summary_json(&entry.graph),
            "revision": entry.graph.revision,
            "results": results,
        }))
    })
    .ok_or_else(|| document_not_found(handle))?
}

/// Undoes `steps` committed mutations (default 1) and returns the new summary.
///
/// Asking for more steps than the history holds is rejected with
/// `session/argument-invalid` before any state changes. Undo history is
/// process-local: it is never written to a `.oword` container.
pub fn undo(handle: u64, steps: Option<u32>) -> Result<Value, ServiceError> {
    let steps = steps.unwrap_or(1) as usize;
    if steps == 0 {
        return Err(invalid("`steps` must be a positive integer"));
    }
    store::with_history(handle, |entry| {
        entry.undo(steps)?;
        Ok(json!({
            "document": summary_json(&entry.graph),
            "revision": entry.graph.revision,
            "undone": steps,
            "redoAvailable": entry.can_redo(),
        }))
    })
    .ok_or_else(|| document_not_found(handle))?
}

/// Replays `steps` undone mutations (default 1) and returns the new summary.
pub fn redo(handle: u64, steps: Option<u32>) -> Result<Value, ServiceError> {
    let steps = steps.unwrap_or(1) as usize;
    if steps == 0 {
        return Err(invalid("`steps` must be a positive integer"));
    }
    store::with_history(handle, |entry| {
        entry.redo(steps)?;
        Ok(json!({
            "document": summary_json(&entry.graph),
            "revision": entry.graph.revision,
            "redone": steps,
            "undoAvailable": entry.can_undo(),
        }))
    })
    .ok_or_else(|| document_not_found(handle))?
}

/// Appends every block of another document to one handle.
///
/// The donor is opened through the same file routing `open` uses, so `docx`,
/// `docm`, `md`, `html`, `rtf`, `txt` and `.oword` all work. Merging a document
/// with the path it came from is allowed. Only `append` is supported.
pub fn merge(handle: u64, path: &Path, mode: Option<&str>) -> Result<Value, ServiceError> {
    let mode = mode.unwrap_or("append");
    if mode != "append" {
        return Err(ServiceError::merge_mode_unsupported(mode));
    }
    store::with(handle, |_| ()).ok_or_else(|| document_not_found(handle))?;
    let donor = files::load(path)?;
    store::with_history(handle, |entry| {
        let before = entry.state().clone();
        let outcome = merge::append(&mut entry.graph, &donor.graph);
        entry.commit(before);
        Ok(json!({
            "blocksAdded": outcome.blocks_added,
            "summary": summary_json(&entry.graph),
            "revision": entry.graph.revision,
            "losses": loss_values(&entry.graph),
        }))
    })
    .ok_or_else(|| document_not_found(handle))?
}

/// Releases one document handle. Closing an unknown or already closed handle is
/// a no-op so cleanup paths never fail.
pub fn close(handle: u64) -> Result<(), ServiceError> {
    store::remove(handle);
    Ok(())
}
