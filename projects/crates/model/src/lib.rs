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
mod rtf;
mod search;
mod store;
mod summary;

use std::{fmt, path::Path};

use serde_json::{Map, Value, json};

/// Every document format the product classifies.
pub const DOCUMENT_FORMATS: &[&str] = &["docx", "docm", "dotx", "dotm", "doc", "rtf", "odt", "txt", "md", "html", "pdf"];

/// Inspection rules the product declares.
const INSPECT_CHECKS: &[&str] = &["styles", "fonts", "images", "sections", "tables", "links"];

/// Per-document render targets.
const RENDER_FORMATS: &[&str] = &["svg", "png", "pdf"];

/// Export targets the writer can produce.
const EXPORT_TARGETS: &[&str] = &["docx", "dotx", "pdf", "html", "md", "txt"];

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
        ("rtf", "partial", "supported", "unsupported"),
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
            entry["lossCodes"] = json!(rtf::LOSS_CODES);
        }
        formats.insert((*format).to_string(), entry);
    }

    let mut checks = Map::new();
    for check in INSPECT_CHECKS {
        let level = if *check == "fonts" { "unsupported" } else { "supported" };
        checks.insert((*check).to_string(), json!(level));
    }

    let mut renderers = Map::new();
    for format in RENDER_FORMATS {
        renderers.insert((*format).to_string(), json!("unsupported"));
    }

    let mut export_targets = Map::new();
    for target in EXPORT_TARGETS {
        let level = match *target {
            "docx" | "pdf" | "html" | "md" | "txt" => "supported",
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

/// Creates a document from `{ title?, author? }` and returns its handle and summary.
pub fn create(request: &Value) -> Result<Value, ServiceError> {
    let title = files::optional_string(request, "title")?;
    let author = files::optional_string(request, "author")?;
    if files::optional_string(request, "template")?.is_some() {
        return Err(ServiceError::new("format/unsupported", "starting a document from a template is not implemented yet"));
    }
    let graph = files::create_graph(title, author);
    Ok(store::register(store::Entry::new(graph, store::Source::Project)))
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
    store::with(handle, |entry| summary::project(&entry.graph)).ok_or_else(|| document_not_found(handle))
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
    let limit = limit.map_or(search::DEFAULT_LIMIT, |value| value as usize);
    store::with(handle, |entry| Value::Array(search::find(&entry.graph, query, limit)))
        .ok_or_else(|| document_not_found(handle))
}

/// Applies a batch of edits and returns the new summary and revision.
pub fn edit(handle: u64, request: &Value) -> Result<Value, ServiceError> {
    store::with_mut(handle, |entry| {
        let results = edit::apply(&mut entry.graph, request)?;
        Ok(json!({
            "document": summary::project(&entry.graph),
            "revision": entry.graph.revision,
            "results": results,
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
        assert_eq!(matrix["checks"]["fonts"], json!("unsupported"));
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
