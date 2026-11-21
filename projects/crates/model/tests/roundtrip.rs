//! Round-trip, inspection and edit behaviour of the document model.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{close, create, edit, export, inspect, open, save, summary};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("open-word-{name}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary workspace");
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn handle_of(value: &Value) -> u64 {
    value["handle"].as_u64().expect("a handle")
}

fn summary_text(document: &Value) -> String {
    document["paragraphs"]
        .as_array()
        .expect("paragraphs")
        .iter()
        .filter_map(|paragraph| paragraph["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_exists(path: &Path) {
    assert!(path.is_file(), "{} should exist", path.display());
}

#[test]
fn create_save_open_round_trip() {
    let workspace = Workspace::new("project");
    let created = create(&json!({ "title": "Round trip", "author": "Ada" })).expect("create");
    assert_eq!(created["document"]["title"], json!("Round trip"));
    let source_handle = handle_of(&created);

    let project = workspace.path("report.oword");
    let saved = save(source_handle, &project, false).expect("save");
    assert_eq!(saved["revision"], json!(0));
    assert!(saved["bytes"].as_u64().unwrap() > 0);
    assert_exists(&project);

    let reopened = open(&project).expect("open project");
    assert_eq!(reopened["document"]["title"], json!("Round trip"));
    assert_eq!(reopened["document"]["revision"], json!(0));

    let reopened_handle = handle_of(&reopened);
    assert!(summary(reopened_handle).is_ok());
    close(source_handle).expect("close source");
    close(reopened_handle).expect("close reopened");
}

#[test]
fn markdown_imports_inspects_and_exports_docx() {
    let workspace = Workspace::new("markdown");
    let markdown = workspace.path("notes.md");
    fs::write(
        &markdown,
        "# Structure\n\nIntro paragraph with **bold** text.\n\n## Details\n\n| Name | Value |\n| --- | --- |\n| alpha | 1 |\n",
    )
    .expect("write markdown");

    let opened = open(&markdown).expect("open markdown");
    let markdown_handle = handle_of(&opened);
    assert_eq!(opened["document"]["sections"].as_array().unwrap().len(), 2);

    let report = inspect(markdown_handle, None).expect("inspect");
    assert_eq!(report["checkerVersion"], json!("open-word/1"));
    assert_eq!(report["passed"], json!(true), "clean markdown must pass: {report}");

    let docx = workspace.path("notes.docx");
    let exported = export(markdown_handle, "docx", &docx, false).expect("export docx");
    assert_eq!(exported["editable"], json!(true));
    assert_eq!(exported["format"], json!("docx"));

    let bytes = fs::read(&docx).expect("read docx");
    assert!(bytes.starts_with(b"PK\x03\x04"), "docx export must be a ZIP package");

    let reopened = open(&docx).expect("reopen docx");
    let document = &reopened["document"];
    let inserted = summary_text(document);
    assert!(inserted.contains("Intro paragraph with"), "docx text: {inserted}");
    assert!(inserted.contains("alpha"), "table cell text: {inserted}");
    assert_eq!(document["tables"].as_array().unwrap().len(), 1);

    close(markdown_handle).expect("close markdown");
    close(handle_of(&reopened)).expect("close docx");
}

#[test]
fn edits_advance_the_revision_and_reject_stale_writes() {
    let created = create(&json!({ "title": "Edited" })).expect("create");
    let document_handle = handle_of(&created);
    assert_eq!(created["document"]["revision"], json!(0));

    let appended = edit(
        document_handle,
        &json!({
            "operations": [{ "type": "paragraph.append", "text": "First paragraph" }],
        }),
    )
    .expect("paragraph");
    assert_eq!(appended["revision"], json!(1));
    assert_eq!(appended["document"]["paragraphs"].as_array().unwrap().len(), 1);

    let headed = edit(
        document_handle,
        &json!({
            "operations": [{ "type": "heading.append", "level": 2, "text": "Second section" }],
        }),
    )
    .expect("heading");
    assert_eq!(headed["revision"], json!(2));
    assert_eq!(headed["document"]["sections"].as_array().unwrap().len(), 1);

    let tabled = edit(
        document_handle,
        &json!({
            "operations": [{ "type": "table.append", "rows": [["Name", "Value"], ["alpha", "1"]] }],
        }),
    )
    .expect("table");
    assert_eq!(tabled["revision"], json!(3));
    assert_eq!(tabled["document"]["tables"].as_array().unwrap().len(), 1);
    assert_eq!(tabled["document"]["tables"][0]["rows"], json!(2));

    let renamed = edit(
        document_handle,
        &json!({
            "operations": [{ "type": "metadata.set", "title": "Renamed", "author": "Grace" }],
        }),
    )
    .expect("metadata");
    assert_eq!(renamed["revision"], json!(4));
    assert_eq!(renamed["document"]["title"], json!("Renamed"));

    let stale = edit(
        document_handle,
        &json!({
            "expectedRevision": 1,
            "operations": [{ "type": "paragraph.append", "text": "stale" }],
        }),
    )
    .expect_err("stale write must be rejected");
    assert_eq!(stale.code, "document/revision-conflict");
    assert_eq!(summary(document_handle).unwrap()["revision"], json!(4));

    let unknown = edit(document_handle, &json!({ "operations": [{ "type": "paragraph.remove", "text": "x" }] }))
        .expect_err("unknown operation must be rejected");
    assert_eq!(unknown.code, "object/unsupported");

    close(document_handle).expect("close");
}
