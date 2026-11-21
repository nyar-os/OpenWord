//! Rule inspection over the semantic graph.
//!
//! Checks are structural rules on the IR, not aesthetic scores, and every issue
//! names the revision it was observed at. DOCX sources additionally merge the
//! container facts the format layer verifies from the original bytes.

use document_formats::{InspectBudget, verify_docx_bytes};
use document_ir::{Block, DocumentGraph, Inline, NodeId, RelationEndpoint, TableRow};
use serde_json::{Value, json};

use crate::store::Source;

const CHECKER_VERSION: &str = "open-word/1";

const ALL_CHECKS: &[&str] = &["sections", "tables", "links", "images", "styles"];

/// Runs the requested checks and returns the report.
pub(crate) fn report(graph: &DocumentGraph, source: &Source, checks: Option<&[String]>) -> Value {
    let mut issues = Vec::new();
    let requested: Vec<&str> = match checks {
        Some(names) if !names.is_empty() => names.iter().map(String::as_str).collect(),
        _ => ALL_CHECKS.to_vec(),
    };

    for name in &requested {
        if !ALL_CHECKS.contains(name) {
            issues.push(
                Issue::warning(
                    "check/unsupported",
                    format!("the `{name}` check is not supported by this build"),
                    graph.revision,
                )
                .into_value(),
            );
        }
    }

    let enabled = Enabled::resolve(&requested);
    if enabled.sections || enabled.tables || enabled.links || enabled.images || enabled.styles {
        walk(graph, &enabled, &mut issues);
    }

    if let Source::Docx { label, bytes } = source {
        match verify_docx_bytes(label, bytes, InspectBudget::default()) {
            Ok(verification) if !verification.main_part_present => issues.push(
                Issue::error("docx/main-part-missing", "the Word package has no word/document.xml main part", graph.revision)
                    .into_value(),
            ),
            Ok(_) => {}
            Err(error) => issues.push(
                Issue::error(
                    "docx/structure-invalid",
                    format!("the Word package failed structural verification: {error}"),
                    graph.revision,
                )
                .into_value(),
            ),
        }
    }
    else if let Source::Other("pdf") = source {
        issues.push(
            Issue::warning(
                "source/partial-import",
                "PDF import is a projection: text, tables and layout can be partial",
                graph.revision,
            )
            .into_value(),
        );
    }

    let passed = !issues.iter().any(|issue| issue["severity"] == json!("error"));
    json!({
        "revision": graph.revision,
        "checkerVersion": CHECKER_VERSION,
        "passed": passed,
        "issues": issues,
    })
}

struct Enabled {
    sections: bool,
    tables: bool,
    links: bool,
    images: bool,
    styles: bool,
}

impl Enabled {
    fn resolve(requested: &[&str]) -> Self {
        Self {
            sections: requested.contains(&"sections"),
            tables: requested.contains(&"tables"),
            links: requested.contains(&"links"),
            images: requested.contains(&"images"),
            styles: requested.contains(&"styles"),
        }
    }
}

struct Issue {
    code: &'static str,
    severity: &'static str,
    recoverable: bool,
    message: String,
    revision: u64,
    section_id: Option<String>,
    object_id: Option<String>,
}

impl Issue {
    fn error(code: &'static str, message: impl Into<String>, revision: u64) -> Self {
        Self {
            code,
            severity: "error",
            recoverable: false,
            message: message.into(),
            revision,
            section_id: None,
            object_id: None,
        }
    }

    fn warning(code: &'static str, message: impl Into<String>, revision: u64) -> Self {
        Self {
            code,
            severity: "warning",
            recoverable: true,
            message: message.into(),
            revision,
            section_id: None,
            object_id: None,
        }
    }

    fn at(mut self, node: NodeId) -> Self {
        self.object_id = Some(format!("node-{}", node.0));
        self
    }

    fn in_section(mut self, section: Option<&str>) -> Self {
        self.section_id = section.map(str::to_string);
        self
    }

    fn into_value(self) -> Value {
        json!({
            "code": self.code,
            "severity": self.severity,
            "revision": self.revision,
            "sectionId": self.section_id,
            "objectId": self.object_id,
            "recoverable": self.recoverable,
            "message": self.message,
        })
    }
}

fn walk(graph: &DocumentGraph, enabled: &Enabled, issues: &mut Vec<Value>) {
    let mut previous_level: u8 = 1;
    let mut current_section: Option<String> = None;
    let inline_checks = enabled.links || enabled.images || enabled.styles;

    for node in &graph.blocks {
        match &node.block {
            Block::Section { level, title, .. } => {
                let section_id = format!("section-{}", node.id.0);
                if enabled.sections {
                    if title_text(title).trim().is_empty() {
                        issues.push(
                            Issue::error("section/empty-title", "a section has an empty title", graph.revision)
                                .at(node.id)
                                .into_value(),
                        );
                    }
                    if *level > previous_level.saturating_add(1) {
                        issues.push(
                            Issue::error(
                                "section/level-jump",
                                format!("heading level {level} follows level {previous_level}"),
                                graph.revision,
                            )
                            .at(node.id)
                            .in_section(Some(&section_id))
                            .into_value(),
                        );
                    }
                    previous_level = *level;
                }
                if inline_checks {
                    check_inlines(title, node.id, Some(&section_id), graph.revision, enabled, issues);
                }
                current_section = Some(section_id);
            }
            Block::Paragraph { content } => {
                if inline_checks {
                    check_inlines(content, node.id, current_section.as_deref(), graph.revision, enabled, issues);
                }
            }
            Block::Table { head, body, foot, .. } => {
                if enabled.tables {
                    check_table(head, body, foot, node.id, graph.revision, issues);
                }
            }
            Block::List { items, .. } => {
                if inline_checks {
                    for item in items {
                        check_inlines(&item.content, node.id, current_section.as_deref(), graph.revision, enabled, issues);
                    }
                }
            }
            Block::Quote { .. } => {}
            _ => {}
        }
    }
}

fn check_table(head: &[TableRow], body: &[TableRow], foot: &[TableRow], node: NodeId, revision: u64, issues: &mut Vec<Value>) {
    let rows: Vec<&TableRow> = head.iter().chain(body.iter()).chain(foot.iter()).collect();
    if rows.is_empty() {
        return;
    }
    let total = rows.len();

    for (row_index, row) in rows.iter().enumerate() {
        for cell in &row.cells {
            if cell.colspan == 0 || cell.rowspan == 0 {
                issues.push(
                    Issue::error("table/span-zero", "a table cell declares a zero colspan or rowspan", revision)
                        .at(node)
                        .into_value(),
                );
                continue;
            }
            if row_index as u32 + cell.rowspan > total as u32 {
                issues.push(
                    Issue::error(
                        "table/span-overflow",
                        format!("a cell starting at row {row_index} spans {} rows past the grid", cell.rowspan),
                        revision,
                    )
                    .at(node)
                    .into_value(),
                );
            }
        }
    }

    let expected = match head.first() {
        Some(row) => row_columns(row),
        None => row_columns(rows[0]),
    };
    for (index, row) in rows.iter().enumerate() {
        let columns = row_columns(row);
        if columns != expected {
            issues.push(
                Issue::error(
                    "table/column-mismatch",
                    format!("row {index} declares {columns} columns but the table grid has {expected}"),
                    revision,
                )
                .at(node)
                .into_value(),
            );
            break;
        }
    }
}

fn row_columns(row: &TableRow) -> u32 {
    row.cells.iter().fold(0u32, |total, cell| total.saturating_add(cell.colspan.max(1)))
}

fn check_inlines(
    inlines: &[Inline],
    node: NodeId,
    section_id: Option<&str>,
    revision: u64,
    enabled: &Enabled,
    issues: &mut Vec<Value>,
) {
    for inline in inlines {
        match inline {
            Inline::Styled { style, children } => {
                if enabled.styles && !known_style(style) {
                    issues.push(
                        Issue::error("style/unknown", format!("unknown inline style `{style}`"), revision)
                            .at(node)
                            .in_section(section_id)
                            .into_value(),
                    );
                }
                check_inlines(children, node, section_id, revision, enabled, issues);
            }
            Inline::Attributed { children, .. } => check_inlines(children, node, section_id, revision, enabled, issues),
            Inline::Link { target, display, .. } => {
                if enabled.links {
                    let empty = match target {
                        RelationEndpoint::ExternalUri(uri) => uri.trim().is_empty(),
                        RelationEndpoint::Unresolved { label } => label.trim().is_empty(),
                        _ => false,
                    };
                    if empty {
                        issues.push(
                            Issue::error("link/empty-target", "a link has an empty destination", revision)
                                .at(node)
                                .in_section(section_id)
                                .into_value(),
                        );
                    }
                }
                check_inlines(display, node, section_id, revision, enabled, issues);
            }
            Inline::Image { source, alt, asset, .. } => {
                if enabled.images && asset.is_none() {
                    issues.push(
                        Issue::error(
                            "image/unbound-asset",
                            format!("an image is not bound to an asset (source `{source}`)"),
                            revision,
                        )
                        .at(node)
                        .in_section(section_id)
                        .into_value(),
                    );
                }
                check_inlines(alt, node, section_id, revision, enabled, issues);
            }
            Inline::Note { display: Some(display), .. } | Inline::Citation { display: Some(display), .. } => {
                check_inlines(display, node, section_id, revision, enabled, issues);
            }
            _ => {}
        }
    }
}

/// Style names the importers normalize to. Compound declarations such as
/// `color:FF0000` keep a known prefix and are accepted as normalized spellings.
fn known_style(style: &str) -> bool {
    const EXACT: &[&str] = &["bold", "italic", "strike", "superscript", "subscript", "underline"];
    const PREFIXES: &[&str] = &["color:", "highlight:", "font-family:", "font-size:"];
    EXACT.contains(&style) || PREFIXES.iter().any(|prefix| style.len() > prefix.len() && style.starts_with(prefix))
}

fn title_text(title: &[Inline]) -> String {
    title.iter().map(Inline::plain_text).collect()
}
