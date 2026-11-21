//! Editing, search and plain-text export behaviour of the document model.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{close, create, edit, export, find, open, save, summary};
use serde_json::{Value, json};

const FIXTURE: &str = "# Overview

Intro paragraph.

## Details

- alpha
- beta

| Name | Value |
| --- | --- |
| alpha | 1 |
";

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("open-word-edit-{name}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary workspace");
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn markdown(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, text).expect("write markdown");
        path
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

fn revision_of(value: &Value) -> u64 {
    value["revision"].as_u64().expect("a revision")
}

fn assert_exists(path: &Path) {
    assert!(path.is_file(), "{} should exist", path.display());
}

#[test]
fn replace_text_counts_titles_paragraphs_and_table_cells() {
    let workspace = Workspace::new("replace");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let replaced =
        edit(handle, &json!({ "operations": [{ "type": "document.replaceText", "find": "alpha", "replace": "ALPHA" }] }))
            .expect("replace text");
    assert_eq!(replaced["results"][0]["replaced"], json!(2));
    assert_eq!(replaced["revision"], json!(1));
    let text = summary_text(&replaced["document"]);
    assert!(text.contains("ALPHA"), "table cell must be replaced: {text}");
    assert!(!text.contains("alpha"), "no plain occurrence may survive: {text}");

    let titled =
        edit(handle, &json!({ "operations": [{ "type": "document.replaceText", "find": "Overview", "replace": "Summary" }] }))
            .expect("replace title");
    assert_eq!(titled["results"][0]["replaced"], json!(1));
    assert_eq!(titled["document"]["sections"][0]["title"], json!("Summary"));

    let limited = edit(
        handle,
        &json!({ "operations": [{ "type": "document.replaceText", "find": "ALPHA", "replace": "x", "limit": 1 }] }),
    )
    .expect("limited replace");
    assert_eq!(limited["results"][0]["replaced"], json!(1));

    let missing =
        edit(handle, &json!({ "operations": [{ "type": "document.replaceText", "find": "absent", "replace": "x" }] }))
            .expect("replace without a match");
    assert_eq!(missing["results"][0]["replaced"], json!(0));
    assert_eq!(revision_of(&missing), revision_of(&limited) + 1);

    let unicode = create(&json!({ "title": "Unicode" })).expect("create");
    let unicode_handle = handle_of(&unicode);
    edit(unicode_handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "a😀b😀c" }] }))
        .expect("append unicode");
    let rewritten =
        edit(unicode_handle, &json!({ "operations": [{ "type": "document.replaceText", "find": "😀", "replace": "哈" }] }))
            .expect("replace emoji");
    assert_eq!(rewritten["results"][0]["replaced"], json!(2));
    assert_eq!(summary_text(&rewritten["document"]), "a哈b哈c");

    close(unicode_handle).expect("close unicode");
    close(handle).expect("close fixture");
}

#[test]
fn removing_a_section_deletes_its_subtree() {
    let created = create(&json!({ "title": "Cascade" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Overview" },
                { "type": "paragraph.append", "text": "Intro paragraph." },
                { "type": "heading.append", "level": 2, "text": "Details" },
                { "type": "paragraph.append", "text": "Nested paragraph." },
                { "type": "table.append", "rows": [["Name", "Value"], ["alpha", "1"]] },
            ],
        }),
    )
    .expect("seed nested document");

    let document = summary(handle).expect("summary");
    let sections = document["sections"].as_array().expect("sections");
    assert_eq!(sections.len(), 2);
    assert_eq!(document["tables"].as_array().unwrap().len(), 1);
    let details = sections[1]["id"].as_str().expect("section id").to_string();
    let revision_before = revision_of(&document);

    let removed = edit(handle, &json!({ "operations": [{ "type": "block.remove", "id": details }] })).expect("remove section");
    assert!(removed["results"][0]["removed"].as_u64().unwrap() >= 3);
    assert_eq!(removed["document"]["sections"].as_array().unwrap().len(), 1);
    assert_eq!(removed["document"]["tables"].as_array().unwrap().len(), 0);
    assert_eq!(revision_of(&removed), revision_before + 1);
    let text = summary_text(&removed["document"]);
    assert!(text.contains("Intro paragraph."), "remaining paragraph: {text}");
    assert!(!text.contains("Nested paragraph."), "section children must be gone: {text}");
    assert!(!text.contains("alpha"), "table text must be gone: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    let error = edit(handle, &json!({ "operations": [{ "type": "block.remove", "id": "node-999999" }] }))
        .expect_err("missing block must fail");
    assert_eq!(error.code, "object/not-found");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn paragraph_insert_keeps_document_and_parent_order() {
    let created = create(&json!({ "title": "Insert" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Alpha" },
                { "type": "paragraph.append", "text": "one" },
            ],
        }),
    )
    .expect("seed document");

    let top = edit(handle, &json!({ "operations": [{ "type": "paragraph.insert", "index": 0, "text": "before all" }] }))
        .expect("insert at top level");
    let paragraphs = top["document"]["paragraphs"].as_array().unwrap();
    assert_eq!(paragraphs[0]["text"], json!("before all"));
    assert_eq!(paragraphs[0]["sectionIndex"], json!(-1));
    assert!(top["results"][0]["id"].as_str().unwrap_or_default().starts_with("node-"));

    // Flat order is now: inserted paragraph, section, its paragraph.
    let nested = edit(handle, &json!({ "operations": [{ "type": "paragraph.insert", "index": 2, "text": "two" }] }))
        .expect("insert inside the section");
    let paragraphs = nested["document"]["paragraphs"].as_array().unwrap();
    assert_eq!(paragraphs[0]["text"], json!("before all"));
    assert_eq!(paragraphs[1]["text"], json!("two"));
    assert_eq!(paragraphs[1]["sectionIndex"], json!(0));
    assert_eq!(paragraphs[2]["text"], json!("one"));
    assert_eq!(paragraphs[2]["sectionIndex"], json!(0));
    assert_eq!(revision_of(&nested), 4);

    let before = revision_of(&summary(handle).expect("summary"));
    let error = edit(handle, &json!({ "operations": [{ "type": "paragraph.insert", "index": 99, "text": "late" }] }))
        .expect_err("out-of-range index must fail");
    assert_eq!(error.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn table_set_cell_replaces_content_and_checks_bounds() {
    let workspace = Workspace::new("cell");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let updated =
        edit(handle, &json!({ "operations": [{ "type": "table.setCell", "table": 0, "row": 1, "column": 1, "text": "2" }] }))
            .expect("set cell");
    assert_eq!(updated["results"][0]["table"], json!(0));
    assert_eq!(updated["revision"], json!(1));

    let txt = workspace.path("report.txt");
    export(handle, "txt", &txt, false).expect("export txt");
    assert_exists(&txt);
    let text = fs::read_to_string(&txt).expect("read txt");
    assert!(text.contains("alpha\t2"), "cell must hold the new text: {text}");

    // Save the project and confirm the span fields survived the replacement.
    let project = workspace.path("report.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let blocks = envelope["document"]["blocks"].as_array().expect("blocks");
    let table = blocks.iter().find(|node| node["block"].get("Table").is_some()).expect("table block");
    let cell = &table["block"]["Table"]["body"][0]["cells"][1];
    assert_eq!(cell["colspan"], json!(1));
    assert_eq!(cell["rowspan"], json!(1));
    let children = cell["children"].as_array().expect("cell children");
    assert_eq!(children.len(), 1);
    let child_id = children[0].as_u64().expect("child id");
    let child = blocks.iter().find(|node| node["id"] == json!(child_id)).expect("child block");
    assert_eq!(child["block"]["Paragraph"]["content"][0]["Text"]["text"], json!("2"));

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "table.setCell", "table": 9, "row": 0, "column": 0, "text": "x" }),
        json!({ "type": "table.setCell", "table": 0, "row": 9, "column": 0, "text": "x" }),
        json!({ "type": "table.setCell", "table": 0, "row": 0, "column": 9, "text": "x" }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("out-of-range cell must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn find_reports_character_offsets() {
    let workspace = Workspace::new("find");
    let opened =
        open(&workspace.markdown("report.md", "# 标题你好\n\n你好世界😀你好\n\n| 名称 | 值 |\n| --- | --- |\n| 你好 | 1 |\n"))
            .expect("open fixture");
    let handle = handle_of(&opened);

    let hits = find(handle, "你好", None).expect("find");
    let hits = hits.as_array().expect("hits array");
    assert_eq!(hits.len(), 4, "title, paragraph and table cell occurrences: {hits:?}");
    assert_eq!(hits[0]["text"], json!("你好"));
    assert_eq!(hits[0]["path"], Value::Null);
    assert_eq!(hits[1]["start"], json!(0));
    assert_eq!(hits[1]["end"], json!(2));
    assert_eq!(hits[1]["blockIndex"], json!(1));
    assert_eq!(hits[2]["start"], json!(5));
    assert_eq!(hits[2]["end"], json!(7));
    assert_eq!(hits[2]["blockId"], hits[1]["blockId"]);

    let limited = find(handle, "你好", Some(1)).expect("limited find");
    assert_eq!(limited.as_array().unwrap().len(), 1);

    let error = find(handle, "", None).expect_err("empty query must fail");
    assert_eq!(error.code, "session/argument-invalid");

    close(handle).expect("close");
}

#[test]
fn txt_export_writes_document_order() {
    let workspace = Workspace::new("txt");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let target = workspace.path("report.txt");
    let report = export(handle, "txt", &target, false).expect("export txt");
    assert_eq!(report["format"], json!("txt"));
    assert_eq!(report["editable"], json!(true));
    assert_exists(&target);

    let text = fs::read_to_string(&target).expect("read txt");
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "# Overview");
    assert_eq!(lines[1], "Intro paragraph.");
    assert_eq!(lines[2], "## Details");
    assert!(lines.contains(&"- alpha"));
    assert!(lines.contains(&"- beta"));
    assert!(lines.contains(&"Name\tValue"));
    assert!(lines.contains(&"alpha\t1"));
    assert_eq!(text.matches("alpha").count(), 2, "each occurrence must appear once: {text}");

    close(handle).expect("close");
}

#[test]
fn edits_bump_once_per_operation_and_reject_stale_batches() {
    let workspace = Workspace::new("revision");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let report = edit(
        handle,
        &json!({
            "operations": [
                { "type": "document.replaceText", "find": "alpha", "replace": "ALPHA" },
                { "type": "table.setCell", "table": 0, "row": 1, "column": 1, "text": "2" },
                { "type": "paragraph.insert", "index": 0, "text": "lead" },
            ],
        }),
    )
    .expect("three operations");
    assert_eq!(report["results"].as_array().unwrap().len(), 3);
    assert_eq!(revision_of(&report), 3);

    let stale = edit(
        handle,
        &json!({
            "expectedRevision": 0,
            "operations": [{ "type": "paragraph.append", "text": "stale" }],
        }),
    )
    .expect_err("stale write must be rejected");
    assert_eq!(stale.code, "document/revision-conflict");

    let before = summary(handle).expect("summary");
    let failed = edit(
        handle,
        &json!({
            "operations": [
                { "type": "paragraph.append", "text": "half" },
                { "type": "block.remove", "id": "node-999999" },
            ],
        }),
    )
    .expect_err("a batch naming a missing block must fail as a whole");
    assert_eq!(failed.code, "object/not-found");
    let after = summary(handle).expect("summary");
    assert_eq!(revision_of(&after), revision_of(&before));
    assert_eq!(after, before);

    close(handle).expect("close");
}
