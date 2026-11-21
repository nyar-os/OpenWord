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
mod replace;
mod store;

use std::{fmt, path::Path};

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
#[derive(Debug)]
pub struct ServiceError {
    /// Stable, comparable code.
    pub code: String,
    /// Human-readable message; never carries binary payloads.
    pub message: String,
}

impl ServiceError {
    /// Builds a failure from a stable code and a message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

pub(crate) fn invalid(message: impl Into<String>) -> ServiceError {
    ServiceError::new("session/argument-invalid", message)
}

pub(crate) fn document_not_found(handle: u64) -> ServiceError {
    ServiceError::new("document/not-found", format!("no live document has handle {handle}"))
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
/// The request is `{ find, replace, caseSensitive?, wholeWord?, limit?,
/// expectedRevision? }`. `find` must be non-empty, `replace` may be empty to
/// delete matches, matching is case-sensitive by default, `wholeWord` defaults
/// to `false`, and `limit` caps the total number of replacements. Matching
/// covers the same text `find` searches: section titles, paragraphs, list items,
/// table cells and authored inline display text.
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
            return Err(ServiceError::new(
                "document/revision-conflict",
                format!("expected revision {expected} but the document is at revision {}", entry.graph.revision),
            ));
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
        return Err(ServiceError::new("object/unsupported", format!("merge mode `{mode}` is not supported")));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_matrix_reflects_wired_routes() {
        let matrix = capabilities();
        assert_eq!(matrix["engine"]["package"], json!("open-word-document"));
        assert_eq!(matrix["project"], json!("supported"));
        assert_eq!(matrix["formats"]["docx"]["import"], json!("supported"));
        assert_eq!(matrix["formats"]["docx"]["export"], json!("supported"));
        assert_eq!(matrix["formats"]["pdf"]["import"], json!("partial"));
        assert_eq!(matrix["formats"]["doc"]["import"], json!("unsupported"));
        assert_eq!(matrix["formats"]["txt"]["export"], json!("supported"));
        assert_eq!(matrix["checks"]["sections"], json!("supported"));
        assert_eq!(matrix["checks"]["fonts"], json!("supported"));
        assert_eq!(matrix["exportTargets"]["pdf"], json!("supported"));
        assert_eq!(matrix["exportTargets"]["txt"], json!("supported"));
        assert_eq!(matrix["exportTargets"]["dotx"], json!("unsupported"));
        assert_eq!(matrix["renderers"]["svg"], json!("unsupported"));
    }

    #[test]
    fn unknown_handle_is_a_stable_failure() {
        for error in [summary(999_999), inspect(999_999, None), edit(999_999, &json!({ "operations": [] }))] {
            assert_eq!(error.unwrap_err().code, "document/not-found");
        }
        let error = find(999_999, "text", None).unwrap_err();
        assert_eq!(error.code, "document/not-found");
        let error = find(999_999, "", None).unwrap_err();
        assert_eq!(error.code, "session/argument-invalid");
        let error = save(999_999, Path::new("report.oword"), true).unwrap_err();
        assert_eq!(error.code, "document/not-found");
        export(999_999, "docx", Path::new("report.docx"), true).unwrap_err();
    }

    #[test]
    fn close_is_idempotent() {
        let created = create(&json!({ "title": "Idempotent close" })).unwrap();
        let handle = created["handle"].as_u64().unwrap();
        close(handle).unwrap();
        close(handle).unwrap();
    }
}
