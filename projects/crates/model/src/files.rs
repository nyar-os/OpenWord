//! File routing for project, document and export paths.
//!
//! Every importer and exporter comes from the pinned document engine; this
//! module only decides which route a path maps to, writes through a temporary
//! file and reports what the route can keep.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use document_formats::{FormatError, export, import};
use document_ir::{Block, BreakKind, DocumentEnvelope, DocumentGraph, Inline, NodeId, TableCell};
use serde_json::{Value, json};

use crate::{
    ServiceError, search,
    store::{self, Entry, Source},
    summary,
};

/// Result of writing a native project.
pub(crate) struct SaveOutcome {
    pub(crate) revision: u64,
    pub(crate) bytes: usize,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Reads one optional string field from a request object.
pub(crate) fn optional_string(request: &Value, key: &str) -> Result<Option<String>, ServiceError> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(crate::invalid(format!("`{key}` must be a string"))),
    }
}

/// Builds a new empty document with a process-unique identity.
pub(crate) fn create_graph(title: Option<String>, author: Option<String>) -> DocumentGraph {
    let mut graph = DocumentGraph::new(store::next_document_id());
    graph.metadata.title = title;
    if let Some(author) = author {
        graph.metadata.authors = vec![author];
    }
    graph
}

/// Loads one entry from `path` by extension.
pub(crate) fn load(path: &Path) -> Result<Entry, ServiceError> {
    let label = path.display().to_string();
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase();
    match extension.as_str() {
        "oword" => {
            let bytes = read(path)?;
            let mut envelope: DocumentEnvelope = serde_json::from_slice(&bytes)
                .map_err(|error| ServiceError::new("format/invalid", format!("{label} is not an OpenWord project: {error}")))?;
            envelope
                .ensure_supported()
                .map_err(|_| ServiceError::new("format/invalid", format!("{label} uses an unsupported document schema")))?;
            envelope.document.id = store::next_document_id();
            Ok(Entry::new(envelope.document, Source::Project))
        }
        "docx" | "docm" => {
            let bytes = read(path)?;
            let mut graph = import::docx::import_docx_bytes(&label, &bytes).map_err(format_failure)?;
            graph.id = store::next_document_id();
            Ok(Entry::new(graph, Source::Docx { label, bytes }))
        }
        "md" | "markdown" => {
            let text = read_text(path)?;
            let mut graph = import::markdown::import_markdown_bytes(&label, &text).map_err(format_failure)?;
            graph.id = store::next_document_id();
            Ok(Entry::new(graph, Source::Other("md")))
        }
        "html" | "htm" => {
            let bytes = read(path)?;
            let mut graph = import::html::import_html_bytes(&label, &bytes).map_err(format_failure)?;
            graph.id = store::next_document_id();
            Ok(Entry::new(graph, Source::Other("html")))
        }
        "txt" => {
            let text = read_text(path)?;
            let mut graph = text_graph(&text);
            graph.id = store::next_document_id();
            Ok(Entry::new(graph, Source::Other("txt")))
        }
        "rtf" => {
            let bytes = read(path)?;
            let graph = crate::rtf::import_rtf_bytes(&label, &bytes, store::next_document_id())?;
            Ok(Entry::new(graph, Source::Rtf { label }))
        }
        "pdf" => {
            let bytes = read(path)?;
            let mut graph = import::pdf::import_pdf_bytes(&label, &bytes).map_err(format_failure)?;
            graph.id = store::next_document_id();
            Ok(Entry::new(graph, Source::Other("pdf")))
        }
        other => Err(ServiceError::new(
            "format/unsupported",
            format!("`{}` has no import route in this build", if other.is_empty() { label.as_str() } else { other }),
        )),
    }
}

/// Writes one document envelope through a temporary file and a rename.
pub(crate) fn save_graph(graph: &DocumentGraph, path: &Path, overwrite: bool) -> Result<SaveOutcome, ServiceError> {
    let envelope = DocumentEnvelope::new(graph.clone());
    let bytes = serde_json::to_vec(&envelope)
        .map_err(|error| ServiceError::new("format/invalid", format!("failed to serialize the project: {error}")))?;
    write_atomically(path, &bytes, overwrite)?;
    Ok(SaveOutcome { revision: graph.revision, bytes: bytes.len() })
}

/// Exports one target and reports what the route kept.
pub(crate) fn export_graph(graph: &DocumentGraph, format: &str, target: &Path, overwrite: bool) -> Result<Value, ServiceError> {
    let (bytes, editable) = match format {
        "docx" => (export::docx::export_docx_bytes(graph).map_err(format_failure)?, true),
        "md" => (export::markdown::export_markdown(graph).map_err(format_failure)?.into_bytes(), true),
        "html" => (export::html::export_html(graph).map_err(format_failure)?.into_bytes(), true),
        "pdf" => (export::pdf::export_pdf_bytes(graph).map_err(format_failure)?, false),
        "txt" => (text_projection(graph).into_bytes(), true),
        other => {
            return Err(ServiceError::new("format/unsupported", format!("`{other}` has no export route in this build")));
        }
    };

    write_atomically(target, &bytes, overwrite)?;

    Ok(json!({
        "revision": graph.revision,
        "format": format,
        "target": target.display().to_string(),
        "losses": summary::loss_values(graph),
        "editable": editable,
    }))
}

fn text_graph(text: &str) -> DocumentGraph {
    let mut graph = DocumentGraph::new(store::next_document_id());
    for raw in text.split("\n\n") {
        let block = raw.trim_end_matches('\r');
        if block.trim().is_empty() {
            continue;
        }
        let mut content = Vec::new();
        for (index, line) in block.lines().enumerate() {
            if index > 0 {
                content.push(Inline::Break { kind: BreakKind::Hard });
            }
            content.push(Inline::Text { text: line.to_string() });
        }
        graph.push_block(Block::Paragraph { content });
    }
    graph
}

/// Plain-text projection of the committed graph.
///
/// The engine exposes no plain-text writer, so this route flattens the model:
/// headings keep their level as `#` markers, list items use `-`, and table rows
/// join cells with tabs. Table cell text appears in the row, never twice.
fn text_projection(graph: &DocumentGraph) -> String {
    let mut lines = Vec::new();
    let mut seen = HashSet::new();
    for root in search::roots(graph) {
        push_text(graph, root, &mut seen, &mut lines);
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn push_text(graph: &DocumentGraph, id: NodeId, seen: &mut HashSet<NodeId>, lines: &mut Vec<String>) {
    if !seen.insert(id) {
        return;
    }
    let Some(node) = graph.blocks.iter().find(|node| node.id == id)
    else {
        return;
    };
    match &node.block {
        Block::Section { level, title, children, .. } => {
            lines.push(format!("{} {}", "#".repeat(*level as usize), summary::flatten(title)));
            for child in children {
                push_text(graph, *child, seen, lines);
            }
        }
        Block::Paragraph { content } => lines.push(summary::flatten(content)),
        Block::List { items, .. } => {
            for item in items {
                lines.push(format!("- {}", summary::flatten(&item.content)));
                for child in &item.children {
                    push_text(graph, *child, seen, lines);
                }
            }
        }
        Block::Table { head, body, foot, .. } => {
            for row in head.iter().chain(body.iter()).chain(foot.iter()) {
                lines.push(row.cells.iter().map(|cell| cell_text(graph, cell)).collect::<Vec<_>>().join("\t"));
            }
        }
        Block::Quote { children } => {
            for child in children {
                push_text(graph, *child, seen, lines);
            }
        }
        Block::Code { content, .. } => lines.push(content.clone()),
        Block::Math { formula } => lines.push(formula.source.clone()),
        Block::Diagram { source, .. }
        | Block::Chart { source, .. }
        | Block::Engraving { source, .. }
        | Block::Depiction { source, .. } => lines.push(source.clone()),
        Block::RawHtml { html } => lines.push(html.clone()),
        Block::Opaque { payload_hint, .. } if !payload_hint.is_empty() => lines.push(payload_hint.clone()),
        _ => {}
    }
}

fn cell_text(graph: &DocumentGraph, cell: &TableCell) -> String {
    cell.children
        .iter()
        .filter_map(|id| graph.blocks.iter().find(|node| node.id == *id))
        .filter_map(|node| match &node.block {
            Block::Paragraph { content } => Some(summary::flatten(content)),
            _ => None,
        })
        .map(|text| text.replace('\n', " "))
        .collect::<Vec<_>>()
        .join(" ")
}

fn read(path: &Path) -> Result<Vec<u8>, ServiceError> {
    fs::read(path).map_err(|error| ServiceError::new("format/invalid", format!("failed to read {}: {error}", path.display())))
}

fn read_text(path: &Path) -> Result<String, ServiceError> {
    fs::read_to_string(path)
        .map_err(|error| ServiceError::new("format/invalid", format!("failed to read {} as text: {error}", path.display())))
}

fn format_failure(error: FormatError) -> ServiceError {
    let message = error.to_string();
    let code = match &error {
        FormatError::NotImplemented { .. } | FormatError::Unsupported { .. } => "format/unsupported",
        FormatError::Budget { .. } => "session/budget-exceeded",
        FormatError::InvalidInput { .. } | FormatError::Parse { .. } => "format/invalid",
    };
    ServiceError::new(code, message)
}

fn write_atomically(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), ServiceError> {
    if path.exists() && !overwrite {
        return Err(crate::invalid(format!("{} already exists; pass overwrite to replace it", path.display())));
    }
    let temporary = temporary_path(path);
    fs::write(&temporary, bytes).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        ServiceError::new("format/invalid", format!("failed to write {}: {error}", temporary.display()))
    })?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| ServiceError::new("format/invalid", format!("failed to replace {}: {error}", path.display())))?;
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        ServiceError::new("format/invalid", format!("failed to write {}: {error}", path.display()))
    })?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("open-word");
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{counter}.tmp", std::process::id()))
}
