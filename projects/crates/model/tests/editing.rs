//! Editing, search and plain-text export behaviour of the document model.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{close, create, edit, export, find, open, outline, save, stats, summary};
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

fn find_heading(items: &[Value], title: &str) -> Option<(String, u64)> {
    for item in items {
        if item["title"].as_str() == Some(title) {
            return Some((item["id"].as_str()?.to_owned(), item["level"].as_u64()?));
        }
        if let Some(children) = item["children"].as_array()
            && let Some(found) = find_heading(children, title)
        {
            return Some(found);
        }
    }
    None
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
fn set_heading_level_relevels_only_the_addressed_section() {
    let workspace = Workspace::new("heading-level");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let before = outline(handle).expect("outline");
    let items = before["items"].as_array().expect("items");
    let (details_id, details_level) = find_heading(items, "Details").expect("Details heading");
    assert_eq!(details_level, 2);
    let (overview_id, _) = find_heading(items, "Overview").expect("Overview heading");

    let result =
        edit(handle, &json!({ "operations": [{ "type": "heading.setLevel", "id": details_id, "level": 3 }] }))
            .expect("set heading level");
    assert_eq!(result["results"][0], json!({ "type": "heading.setLevel", "id": details_id, "level": 3 }));
    assert_eq!(revision_of(&result), 1);
    assert_eq!(result["document"]["sections"][1]["title"], json!("Details"));

    let after = outline(handle).expect("outline after");
    let items = after["items"].as_array().expect("items");
    assert_eq!(find_heading(items, "Details").expect("Details after").1, 3);
    assert_eq!(find_heading(items, "Overview").expect("Overview after"), (overview_id.clone(), 1));

    let inserted =
        edit(handle, &json!({ "operations": [{ "type": "paragraph.insert", "index": 0, "text": "Top" }] }))
            .expect("insert paragraph");
    let paragraph_id = inserted["results"][0]["id"].as_str().expect("paragraph id").to_owned();
    let not_heading =
        edit(handle, &json!({ "operations": [{ "type": "heading.setLevel", "id": paragraph_id, "level": 2 }] }))
            .expect_err("a paragraph is not a heading");
    assert_eq!(not_heading.code, "session/argument-invalid");

    let missing =
        edit(handle, &json!({ "operations": [{ "type": "heading.setLevel", "id": "section-9999", "level": 2 }] }))
            .expect_err("missing heading");
    assert_eq!(missing.code, "object/not-found");

    let out_of_range =
        edit(handle, &json!({ "operations": [{ "type": "heading.setLevel", "id": overview_id, "level": 7 }] }))
            .expect_err("level is out of range");
    assert_eq!(out_of_range.code, "session/argument-invalid");

    close(handle).expect("close");
}

#[test]
fn set_heading_text_keeps_level_children_and_position() {
    let workspace = Workspace::new("heading-text");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let before = outline(handle).expect("outline");
    let items = before["items"].as_array().expect("items");
    let (details_id, details_level) = find_heading(items, "Details").expect("Details heading");

    let result =
        edit(handle, &json!({ "operations": [{ "type": "heading.setText", "id": details_id, "text": "  Background  " }] }))
            .expect("set heading text");
    assert_eq!(
        result["results"][0],
        json!({ "type": "heading.setText", "id": details_id, "text": "Background" })
    );
    assert_eq!(revision_of(&result), 1);
    assert_eq!(result["document"]["sections"][1]["title"], json!("Background"));

    let after = outline(handle).expect("outline after");
    let items = after["items"].as_array().expect("items");
    assert_eq!(find_heading(items, "Details"), None);
    let renamed = find_heading(items, "Background").expect("renamed heading");
    assert_eq!(renamed.0, details_id);
    assert_eq!(renamed.1, details_level);

    let blank = edit(handle, &json!({ "operations": [{ "type": "heading.setText", "id": details_id, "text": "   " }] }))
        .expect_err("blank title");
    assert_eq!(blank.code, "session/argument-invalid");

    let missing =
        edit(handle, &json!({ "operations": [{ "type": "heading.setText", "id": "section-9999", "text": "X" }] }))
            .expect_err("missing heading");
    assert_eq!(missing.code, "object/not-found");

    close(handle).expect("close");
}

#[test]
fn replace_text_accepts_regular_expressions_and_capture_groups() {
    let created = create(&json!({ "title": "Pattern" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "Order 12, order 34 and 56." }] }))
        .expect("append paragraph");

    let rewritten = edit(
        handle,
        &json!({ "operations": [{
            "type": "document.replaceText",
            "find": "(?i)order\\s+(\\d+)",
            "replace": "item $1",
            "regex": true,
        }] }),
    )
    .expect("regex replace");
    assert_eq!(rewritten["results"][0]["replaced"], json!(2));
    assert_eq!(summary_text(&rewritten["document"]), "item 12, item 34 and 56.");

    let insensitive = edit(
        handle,
        &json!({ "operations": [{
            "type": "document.replaceText",
            "find": "ITEM",
            "replace": "line",
            "caseSensitive": false,
        }] }),
    )
    .expect("case-insensitive literal replace");
    assert_eq!(insensitive["results"][0]["replaced"], json!(2));
    assert_eq!(summary_text(&insensitive["document"]), "line 12, line 34 and 56.");

    close(handle).expect("close");
}

#[test]
fn replace_text_rejects_invalid_patterns_and_conflicting_options() {
    let created = create(&json!({ "title": "Pattern errors" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "alpha" }] })).expect("append");
    let before = summary(handle).expect("summary");

    for request in [
        json!({ "operations": [{ "type": "document.replaceText", "find": "(", "replace": "x", "regex": true }] }),
        json!({ "operations": [{
            "type": "document.replaceText",
            "find": "a",
            "replace": "x",
            "regex": true,
            "wholeWord": true,
        }] }),
        json!({ "operations": [{ "type": "document.replaceText", "find": "a", "replace": "x", "regex": "yes" }] }),
    ] {
        let error = edit(handle, &request).expect_err("invalid pattern request");
        assert_eq!(error.code, "session/argument-invalid", "{request}");
        assert_eq!(summary(handle).expect("summary"), before);
    }

    close(handle).expect("close");
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
fn table_set_caption_sets_trims_and_clears_the_caption() {
    let workspace = Workspace::new("caption");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let updated = edit(
        handle,
        &json!({ "operations": [{ "type": "table.setCaption", "table": 0, "caption": "  Quarterly results  " }] }),
    )
    .expect("set caption");
    assert_eq!(updated["results"][0]["type"], json!("table.setCaption"));
    assert_eq!(updated["results"][0]["table"], json!(0));
    assert_eq!(updated["results"][0]["caption"], json!("Quarterly results"));
    assert_eq!(updated["revision"], json!(1));

    let project = workspace.path("report.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let blocks = envelope["document"]["blocks"].as_array().expect("blocks");
    let table = blocks.iter().find(|node| node["block"].get("Table").is_some()).expect("table block");
    assert_eq!(table["block"]["Table"]["caption"], json!("Quarterly results"));

    let cleared =
        edit(handle, &json!({ "operations": [{ "type": "table.setCaption", "table": 0 }] })).expect("clear caption");
    assert_eq!(cleared["results"][0]["caption"], Value::Null);

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "table.setCaption", "table": 7, "caption": "late" }),
        json!({ "type": "table.setCaption", "table": 0, "caption": "   " }),
        json!({ "type": "table.setCaption" }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid caption must fail");
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

fn table_grid(envelope: &Value) -> Vec<u64> {
    let blocks = envelope["document"]["blocks"].as_array().expect("blocks");
    let table = blocks.iter().find(|node| node["block"].get("Table").is_some()).expect("table block");
    let table = &table["block"]["Table"];
    let mut widths = Vec::new();
    for band in ["head", "body", "foot"] {
        let Some(rows) = table.get(band).and_then(Value::as_array)
        else {
            continue;
        };
        for row in rows {
            let width = row["cells"]
                .as_array()
                .expect("cells")
                .iter()
                .map(|cell| cell["colspan"].as_u64().expect("colspan"))
                .sum::<u64>();
            widths.push(width);
        }
    }
    widths
}

#[test]
fn table_row_and_column_edits_keep_the_grid_consistent() {
    let workspace = Workspace::new("grid");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let inserted = edit(
        handle,
        &json!({
            "operations": [
                { "type": "table.insertRow", "table": 0, "row": 1, "cells": ["beta", "2"] },
                { "type": "table.insertColumn", "table": 0, "column": 2, "cells": ["note", "v1", "v2"] },
            ],
        }),
    )
    .expect("insert row and column");
    assert_eq!(inserted["document"]["tables"][0]["rows"], json!(3));
    assert_eq!(inserted["document"]["tables"][0]["columns"], json!(3));
    assert_eq!(revision_of(&inserted), 2);

    let project = workspace.path("grid.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    assert_eq!(table_grid(&envelope), vec![3, 3, 3], "every row must span three grid columns");

    let txt = workspace.path("grid.txt");
    export(handle, "txt", &txt, false).expect("export txt");
    let text = fs::read_to_string(&txt).expect("read txt");
    assert!(text.contains("Name\tValue\tnote"), "header row: {text}");
    assert!(text.contains("beta\t2\tv1"), "inserted row: {text}");
    assert!(text.contains("alpha\t1\tv2"), "body row: {text}");

    let deleted = edit(
        handle,
        &json!({
            "operations": [
                { "type": "table.deleteRow", "table": 0, "row": 1 },
                { "type": "table.deleteColumn", "table": 0, "column": 2 },
            ],
        }),
    )
    .expect("delete row and column");
    assert_eq!(deleted["document"]["tables"][0]["rows"], json!(2));
    assert_eq!(deleted["document"]["tables"][0]["columns"], json!(2));
    assert_eq!(revision_of(&deleted), 4);

    let before = summary(handle).expect("summary");
    for operation in [
        json!({ "type": "table.insertRow", "table": 0, "row": 9 }),
        json!({ "type": "table.deleteRow", "table": 0, "row": 9 }),
        json!({ "type": "table.insertColumn", "table": 0, "column": 9 }),
        json!({ "type": "table.deleteColumn", "table": 0, "column": 9 }),
        json!({ "type": "table.insertRow", "table": 0, "row": 1, "cells": ["only-one"] }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid table edit must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }

    let failed = edit(
        handle,
        &json!({
            "operations": [
                { "type": "table.insertRow", "table": 0, "row": 1, "cells": ["x", "y"] },
                { "type": "table.deleteRow", "table": 0, "row": 99 },
            ],
        }),
    )
    .expect_err("a batch with one invalid row must fail as a whole");
    assert_eq!(failed.code, "session/argument-invalid");
    assert_eq!(summary(handle).expect("summary"), before);

    close(handle).expect("close");
}

#[test]
fn table_edits_reject_an_empty_grid() {
    let created = create(&json!({ "title": "Single row" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "table.append", "rows": [["only"]] }] })).expect("append table");

    let before = revision_of(&summary(handle).expect("summary"));
    let error = edit(handle, &json!({ "operations": [{ "type": "table.deleteRow", "table": 0, "row": 0 }] }))
        .expect_err("the last row cannot be deleted");
    assert_eq!(error.code, "session/argument-invalid");
    let error = edit(handle, &json!({ "operations": [{ "type": "table.deleteColumn", "table": 0, "column": 0 }] }))
        .expect_err("the last column cannot be deleted");
    assert_eq!(error.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    let column = edit(handle, &json!({ "operations": [{ "type": "table.insertColumn", "table": 0, "column": 1 }] }))
        .expect("insert an empty column");
    assert_eq!(column["document"]["tables"][0]["columns"], json!(2));
    assert_eq!(revision_of(&column), before + 1);

    close(handle).expect("close");
}

#[test]
fn paragraph_update_replaces_text_and_keeps_the_paragraph() {
    let created = create(&json!({ "title": "Update" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Alpha" },
                { "type": "paragraph.append", "text": "old text" },
            ],
        }),
    )
    .expect("seed document");

    let hits = find(handle, "old text", None).expect("find");
    let id = hits.as_array().expect("hits")[0]["blockId"].as_str().expect("block id").to_string();
    let updated = edit(handle, &json!({ "operations": [{ "type": "paragraph.update", "id": id, "text": "new text" }] }))
        .expect("update by id");
    assert_eq!(updated["document"]["paragraphs"].as_array().unwrap().len(), 1);
    assert_eq!(updated["document"]["paragraphs"][0]["text"], json!("new text"));
    assert_eq!(updated["document"]["paragraphs"][0]["style"], Value::Null);
    assert_eq!(updated["document"]["paragraphs"][0]["sectionIndex"], json!(0));
    assert_eq!(revision_of(&updated), 3);

    // Flat order is the section at index 0 and its paragraph at index 1.
    let by_index = edit(handle, &json!({ "operations": [{ "type": "paragraph.update", "index": 1, "text": "third" }] }))
        .expect("update by index");
    assert_eq!(by_index["document"]["paragraphs"][0]["text"], json!("third"));
    assert_eq!(revision_of(&by_index), 4);

    let section_id = by_index["document"]["sections"][0]["id"].as_str().expect("section id").to_string();
    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "paragraph.update", "id": section_id, "text": "not a paragraph" }),
        json!({ "type": "paragraph.update", "text": "missing target" }),
        json!({ "type": "paragraph.update", "index": 99, "text": "outside" }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid target must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    let error = edit(handle, &json!({ "operations": [{ "type": "paragraph.update", "id": "node-999999", "text": "x" }] }))
        .expect_err("unknown block id must fail");
    assert_eq!(error.code, "object/not-found");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn block_move_keeps_section_subtrees_and_rejects_cycles() {
    let created = create(&json!({ "title": "Move" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Alpha" },
                { "type": "paragraph.append", "text": "one" },
                { "type": "heading.append", "level": 1, "text": "Beta" },
                { "type": "paragraph.append", "text": "two" },
            ],
        }),
    )
    .expect("seed sections");
    let seeded = summary(handle).expect("summary");
    let alpha = seeded["sections"][0]["id"].as_str().expect("alpha id").to_string();
    let beta = seeded["sections"][1]["id"].as_str().expect("beta id").to_string();

    let before = revision_of(&seeded);
    let error = edit(handle, &json!({ "operations": [{ "type": "block.move", "id": alpha, "index": 1 }] }))
        .expect_err("a section cannot move into its own subtree");
    assert_eq!(error.code, "session/argument-invalid");
    let error = edit(handle, &json!({ "operations": [{ "type": "block.move", "id": "node-999999", "index": 0 }] }))
        .expect_err("unknown block id must fail");
    assert_eq!(error.code, "object/not-found");
    let error = edit(handle, &json!({ "operations": [{ "type": "block.move", "id": alpha, "index": 99 }] }))
        .expect_err("out-of-range position must fail");
    assert_eq!(error.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    let moved = edit(handle, &json!({ "operations": [{ "type": "block.move", "id": beta, "index": 0 }] }))
        .expect("move section with subtree");
    assert_eq!(moved["document"]["sections"][0]["title"], json!("Beta"));
    assert_eq!(moved["document"]["sections"][1]["title"], json!("Alpha"));
    assert_eq!(summary_text(&moved["document"]), "two\none");
    assert_eq!(moved["document"]["paragraphs"][0]["sectionIndex"], json!(0));
    assert_eq!(moved["document"]["paragraphs"][1]["sectionIndex"], json!(1));
    assert_eq!(revision_of(&moved), before + 1);

    let hits = find(handle, "one", None).expect("find");
    let one = hits.as_array().expect("hits")[0]["blockId"].as_str().expect("block id").to_string();
    let promoted = edit(handle, &json!({ "operations": [{ "type": "block.move", "id": one, "index": 0 }] }))
        .expect("promote a paragraph to the top level");
    assert_eq!(summary_text(&promoted["document"]), "one\ntwo");
    assert_eq!(promoted["document"]["paragraphs"][0]["sectionIndex"], json!(-1));
    assert_eq!(promoted["document"]["sections"][0]["title"], json!("Beta"));
    assert_eq!(revision_of(&promoted), before + 2);

    close(handle).expect("close");
}

#[test]
fn template_fill_replaces_known_placeholders_in_one_revision() {
    let created = create(&json!({ "title": "Letter" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Dear {{ name }}" },
                { "type": "paragraph.append", "text": "{{name}} owes {{amount}} {{ note }} and {{missing}}." },
                { "type": "table.append", "rows": [["Item", "{{item}}"], ["{{amount}}", "1"]] },
            ],
        }),
    )
    .expect("seed placeholders");
    let before = revision_of(&summary(handle).expect("summary"));

    let filled = edit(
        handle,
        &json!({
            "operations": [{ "type": "template.fill", "values": { "name": "Ada", "amount": 42, "note": true } }],
        }),
    )
    .expect("fill template");
    assert_eq!(filled["results"][0]["replaced"], json!(5));
    assert_eq!(revision_of(&filled), before + 1);
    assert_eq!(filled["document"]["sections"][0]["title"], json!("Dear Ada"));
    let text = summary_text(&filled["document"]);
    assert!(text.contains("Ada owes 42 true and {{missing}}."), "unknown placeholders must stay: {text}");
    assert!(text.contains("42"), "table cell placeholder must be replaced: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    for invalid in [
        json!({ "operations": [{ "type": "template.fill", "values": {} }] }),
        json!({ "operations": [{ "type": "template.fill", "values": { "name": ["Ada"] } }] }),
        json!({ "operations": [{ "type": "template.fill", "values": "Ada" }] }),
    ] {
        let error = edit(handle, &invalid).expect_err("invalid values must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn list_and_quote_appends_extend_counts_and_export_markdown() {
    let workspace = Workspace::new("listquote");
    let created = create(&json!({ "title": "Structures" })).expect("create");
    let handle = handle_of(&created);
    let before = stats(handle).expect("stats");

    let bullets = edit(handle, &json!({ "operations": [{ "type": "list.append", "items": ["alpha", "beta"] }] }))
        .expect("append unordered list");
    assert_eq!(bullets["results"][0]["type"], json!("list.append"));
    assert_eq!(bullets["results"][0]["items"], json!(2));
    assert_eq!(bullets["results"][0]["ordered"], json!(false));
    assert_eq!(revision_of(&bullets), 1);
    let after_bullets = stats(handle).expect("stats");
    assert_eq!(after_bullets["blocks"].as_u64().unwrap(), before["blocks"].as_u64().unwrap() + 1);
    assert_eq!(after_bullets["words"].as_u64().unwrap(), before["words"].as_u64().unwrap() + 2);

    let numbered =
        edit(handle, &json!({ "operations": [{ "type": "list.append", "ordered": true, "items": ["one", "two"] }] }))
            .expect("append ordered list");
    assert_eq!(numbered["results"][0]["items"], json!(2));
    assert_eq!(numbered["results"][0]["ordered"], json!(true));
    let after_numbered = stats(handle).expect("stats");
    assert_eq!(after_numbered["blocks"].as_u64().unwrap(), after_bullets["blocks"].as_u64().unwrap() + 1);
    assert_eq!(after_numbered["words"].as_u64().unwrap(), after_bullets["words"].as_u64().unwrap() + 2);

    let quoted = edit(handle, &json!({ "operations": [{ "type": "quote.append", "text": "quoted line" }] }))
        .expect("append quote");
    assert_eq!(quoted["results"][0]["type"], json!("quote.append"));
    let after_quote = stats(handle).expect("stats");
    assert_eq!(after_quote["blocks"].as_u64().unwrap(), after_numbered["blocks"].as_u64().unwrap() + 2);
    assert_eq!(after_quote["paragraphs"].as_u64().unwrap(), after_numbered["paragraphs"].as_u64().unwrap() + 1);
    assert_eq!(summary_text(&quoted["document"]), "quoted line");

    let markdown = workspace.path("structures.md");
    export(handle, "md", &markdown, false).expect("export markdown");
    let text = fs::read_to_string(&markdown).expect("read markdown");
    assert!(text.contains("- alpha"), "unordered list: {text}");
    assert!(text.contains("- beta"), "unordered list: {text}");
    assert!(text.contains("1. one"), "ordered list: {text}");
    assert!(text.contains("2. two"), "ordered list: {text}");
    assert!(text.contains("> quoted line"), "quote: {text}");

    let before_error = revision_of(&summary(handle).expect("summary"));
    let error = edit(handle, &json!({ "operations": [{ "type": "list.append", "items": [] }] }))
        .expect_err("empty items must fail");
    assert_eq!(error.code, "session/argument-invalid");
    let error = edit(handle, &json!({ "operations": [{ "type": "quote.append", "text": "" }] }))
        .expect_err("empty quote text must fail");
    assert_eq!(error.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before_error);

    close(handle).expect("close");
}

#[test]
fn task_list_append_records_check_state_and_exports_markers() {
    let workspace = Workspace::new("tasks");
    let created = create(&json!({ "title": "Tasks" })).expect("create");
    let handle = handle_of(&created);
    let before = stats(handle).expect("stats");

    let edited = edit(
        handle,
        &json!({
            "operations": [{ "type": "taskList.append", "items": [
                "write tests",
                { "text": "ship", "checked": true },
            ] }]
        }),
    )
    .expect("append tasks");
    assert_eq!(edited["results"][0]["type"], json!("taskList.append"));
    assert_eq!(edited["results"][0]["items"], json!(2));
    assert_eq!(edited["results"][0]["checked"], json!(1));
    let after = stats(handle).expect("stats");
    assert_eq!(after["blocks"].as_u64().unwrap(), before["blocks"].as_u64().unwrap() + 1);

    let markdown = workspace.path("tasks.md");
    export(handle, "md", &markdown, false).expect("export markdown");
    let text = fs::read_to_string(&markdown).expect("read markdown");
    assert!(text.contains("- [ ] write tests"), "unchecked task: {text}");
    assert!(text.contains("- [x] ship"), "checked task: {text}");

    let before_error = revision_of(&summary(handle).expect("summary"));
    for request in [
        json!({ "operations": [{ "type": "taskList.append", "items": [] }] }),
        json!({ "operations": [{ "type": "taskList.append", "items": [{ "checked": true }] }] }),
        json!({ "operations": [{ "type": "taskList.append", "items": [{ "text": "x", "checked": "yes" }] }] }),
    ] {
        let error = edit(handle, &request).expect_err("invalid task list");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before_error);

    close(handle).expect("close");
}

#[test]
fn code_append_adds_a_fenced_block_and_counts_it() {
    let workspace = Workspace::new("code");
    let created = create(&json!({ "title": "Code" })).expect("create");
    let handle = handle_of(&created);
    let before = stats(handle).expect("stats");

    let appended = edit(
        handle,
        &json!({
            "operations": [
                { "type": "code.append", "text": "fn main() {\n    println!(\"hi\");\n}", "language": "rust" },
            ],
        }),
    )
    .expect("append code");
    assert_eq!(appended["results"][0]["type"], json!("code.append"));
    assert_eq!(appended["results"][0]["language"], json!("rust"));
    assert_eq!(revision_of(&appended), 1);

    let after = stats(handle).expect("stats");
    assert_eq!(after["blocks"].as_u64().unwrap(), before["blocks"].as_u64().unwrap() + 1);
    assert_eq!(after["paragraphs"], before["paragraphs"]);
    assert!(after["words"].as_u64().unwrap() > before["words"].as_u64().unwrap(), "code words: {after}");

    let markdown = workspace.path("code.md");
    export(handle, "md", &markdown, false).expect("export markdown");
    let text = fs::read_to_string(&markdown).expect("read markdown");
    assert!(text.contains("```rust"), "language fence: {text}");
    assert!(text.contains("fn main()"), "code source: {text}");

    let untagged = edit(handle, &json!({ "operations": [{ "type": "code.append", "text": "plain text" }] }))
        .expect("append untagged code");
    assert_eq!(untagged["results"][0]["language"], Value::Null);
    let plain = workspace.path("code-plain.md");
    export(handle, "md", &plain, false).expect("export untagged markdown");
    let text = fs::read_to_string(&plain).expect("read untagged markdown");
    assert!(text.contains("plain text"), "code source: {text}");

    let before_error = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "code.append", "text": "" }),
        json!({ "type": "code.append" }),
        json!({ "type": "code.append", "text": "x", "language": 3 }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid code append must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before_error);

    close(handle).expect("close");
}

#[test]
fn ordered_list_start_numbers_items_from_the_requested_value() {
    let workspace = Workspace::new("liststart");
    let created = create(&json!({ "title": "Numbering" })).expect("create");
    let handle = handle_of(&created);

    let numbered = edit(
        handle,
        &json!({
            "operations": [{ "type": "list.append", "ordered": true, "items": ["one", "two"], "start": 3 }],
        }),
    )
    .expect("append list with start");
    assert_eq!(numbered["results"][0]["start"], json!(3));
    let started = workspace.path("start.md");
    export(handle, "md", &started, false).expect("export markdown");
    let text = fs::read_to_string(&started).expect("read markdown");
    assert!(text.contains("3. one"), "start number must be used: {text}");
    assert!(text.contains("4. two"), "numbers must increase: {text}");

    let defaulted =
        edit(handle, &json!({ "operations": [{ "type": "list.append", "ordered": true, "items": ["one", "two"] }] }))
            .expect("append list without start");
    assert_eq!(defaulted["results"][0]["start"], Value::Null);
    let default = workspace.path("default.md");
    export(handle, "md", &default, false).expect("export markdown");
    let text = fs::read_to_string(&default).expect("read markdown");
    assert!(text.contains("1. one"), "default numbering: {text}");
    assert!(text.contains("2. two"), "default numbering: {text}");

    let bullets =
        edit(handle, &json!({ "operations": [{ "type": "list.append", "items": ["one"] }] })).expect("append bullets");
    assert_eq!(bullets["results"][0]["start"], Value::Null);

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "list.append", "ordered": true, "items": ["one"], "start": 0 }),
        json!({ "type": "list.append", "items": ["one"], "start": 2 }),
        json!({ "type": "list.append", "ordered": true, "items": ["one"], "start": "2" }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid start must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn list_set_ordered_switches_markers_without_touching_items() {
    let workspace = Workspace::new("listordered");
    let created = create(&json!({ "title": "Markers" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "list.append", "items": ["alpha", "beta"] }] }))
        .expect("append bullets");

    let hits = find(handle, "alpha", None).expect("find list item");
    let id = hits.as_array().expect("hits")[0]["blockId"].as_str().expect("block id").to_string();

    let numbered = edit(handle, &json!({ "operations": [{ "type": "list.setOrdered", "id": id, "ordered": true }] }))
        .expect("switch to numbers");
    assert_eq!(numbered["results"][0]["type"], json!("list.setOrdered"));
    assert_eq!(numbered["results"][0]["id"], json!(id));
    assert_eq!(numbered["results"][0]["ordered"], json!(true));

    let exported = workspace.path("numbered.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("1. alpha"), "numbers replace bullets: {text}");
    assert!(text.contains("2. beta"), "item order is kept: {text}");
    assert!(!text.contains("- alpha"), "bullets are gone: {text}");

    let bullets = edit(handle, &json!({ "operations": [{ "type": "list.setOrdered", "id": id, "ordered": false }] }))
        .expect("switch back to bullets");
    assert_eq!(bullets["results"][0]["ordered"], json!(false));

    let before = revision_of(&summary(handle).expect("summary"));
    let wrong_block = edit(
        handle,
        &json!({ "operations": [{ "type": "list.setOrdered", "id": "node-999999", "ordered": true }] }),
    )
    .expect_err("unknown block");
    assert_eq!(wrong_block.code, "object/not-found");
    let missing_flag =
        edit(handle, &json!({ "operations": [{ "type": "list.setOrdered", "id": id }] })).expect_err("missing ordered");
    assert_eq!(missing_flag.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn list_set_start_renumbers_an_ordered_list() {
    let workspace = Workspace::new("liststart");
    let created = create(&json!({ "title": "Start" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "list.append", "ordered": true, "items": ["alpha", "beta"] }] }))
        .expect("append numbers");

    let hits = find(handle, "alpha", None).expect("find list item");
    let id = hits.as_array().expect("hits")[0]["blockId"].as_str().expect("block id").to_string();

    let renumbered =
        edit(handle, &json!({ "operations": [{ "type": "list.setStart", "id": id, "start": 4 }] })).expect("renumber");
    assert_eq!(renumbered["results"][0]["type"], json!("list.setStart"));
    assert_eq!(renumbered["results"][0]["start"], json!(4));

    let exported = workspace.path("renumbered.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("4. alpha"), "the first number moves: {text}");
    assert!(text.contains("5. beta"), "later numbers follow it: {text}");

    edit(handle, &json!({ "operations": [{ "type": "list.setOrdered", "id": id, "ordered": false }] }))
        .expect("switch to bullets");
    let before = revision_of(&summary(handle).expect("summary"));
    let unordered =
        edit(handle, &json!({ "operations": [{ "type": "list.setStart", "id": id, "start": 2 }] })).expect_err("bullets");
    assert_eq!(unordered.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn table_set_header_replaces_and_clears_the_header_row() {
    let workspace = Workspace::new("tableheader");
    let created = create(&json!({ "title": "Table" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({ "operations": [{ "type": "table.append", "rows": [["a", "b"], ["1", "2"]] }] }),
    )
    .expect("append table");

    let changed =
        edit(handle, &json!({ "operations": [{ "type": "table.setHeader", "table": 0, "cells": ["Name", "Value"] }] }))
            .expect("set header");
    assert_eq!(changed["results"][0]["type"], json!("table.setHeader"));
    assert_eq!(changed["results"][0]["cells"], json!(["Name", "Value"]));

    let exported = workspace.path("header.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("| Name | Value |"), "new header is written: {text}");
    assert!(!text.contains("| a | b |"), "old header row is gone: {text}");

    edit(handle, &json!({ "operations": [{ "type": "table.setHeader", "table": 0, "cells": [] }] }))
        .expect("clear header");
    let cleared = workspace.path("cleared.md");
    export(handle, "md", &cleared, false).expect("export markdown");
    let text = fs::read_to_string(&cleared).expect("read markdown");
    assert!(text.contains("| 1 | 2 |"), "the remaining body row becomes the top: {text}");

    close(handle).expect("close");
}

#[test]
fn table_set_cell_span_merges_and_unmerges_one_cell() {
    let workspace = Workspace::new("cellspan");
    let created = create(&json!({ "title": "Table" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({ "operations": [{ "type": "table.append", "rows": [["a", "b"], ["1", "2"]] }] }),
    )
    .expect("append table");

    let merged = edit(
        handle,
        &json!({ "operations": [{ "type": "table.setCellSpan", "table": 0, "row": 1, "column": 0, "colspan": 2, "rowspan": 1 }] }),
    )
    .expect("merge body cells");
    assert_eq!(merged["results"][0]["type"], json!("table.setCellSpan"));
    assert_eq!(revision_of(&merged), 2);

    let project = workspace.path("span.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let table = envelope["document"]["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .find(|node| node["block"].get("Table").is_some())
        .expect("table block")["block"]["Table"]
        .clone();
    assert_eq!(table["body"][0]["cells"][0]["colspan"], json!(2));
    assert_eq!(table["body"][0]["cells"][0]["rowspan"], json!(1));

    let unmerged = edit(
        handle,
        &json!({ "operations": [{ "type": "table.setCellSpan", "table": 0, "row": 1, "column": 0, "colspan": 1, "rowspan": 1 }] }),
    )
    .expect("unmerge the cell");
    assert_eq!(revision_of(&unmerged), 3);

    let before = summary(handle).expect("summary");
    for operation in [
        json!({ "type": "table.setCellSpan", "table": 0, "row": 9, "column": 0, "colspan": 2, "rowspan": 1 }),
        json!({ "type": "table.setCellSpan", "table": 0, "row": 0, "column": 9, "colspan": 2, "rowspan": 1 }),
        json!({ "type": "table.setCellSpan", "table": 0, "row": 0, "column": 0, "colspan": 0, "rowspan": 1 }),
        json!({ "type": "table.setCellSpan", "table": 0, "row": 0, "column": 0, "colspan": 1, "rowspan": 0 }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid span must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(summary(handle).expect("summary"), before);

    close(handle).expect("close");
}

#[test]
fn code_set_language_retags_a_block_and_can_clear_it() {
    let workspace = Workspace::new("codelanguage");
    let created = create(&json!({ "title": "Code" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "code.append", "text": "let value = 1;", "language": "rust" }] }))
        .expect("append code");

    let id = "node-1".to_string();

    let tagged = edit(handle, &json!({ "operations": [{ "type": "code.setLanguage", "id": id, "language": "ts" }] }))
        .expect("retag code");
    assert_eq!(tagged["results"][0]["type"], json!("code.setLanguage"));
    assert_eq!(tagged["results"][0]["id"], json!(id));
    assert_eq!(tagged["results"][0]["language"], json!("ts"));

    let exported = workspace.path("tagged.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("```ts"), "new tag is written: {text}");
    assert!(!text.contains("```rust"), "old tag is gone: {text}");
    assert!(text.contains("let value = 1;"), "source text is kept: {text}");

    let cleared = edit(handle, &json!({ "operations": [{ "type": "code.setLanguage", "id": id }] })).expect("clear tag");
    assert_eq!(cleared["results"][0]["language"], Value::Null);
    let bare = workspace.path("bare.md");
    export(handle, "md", &bare, false).expect("export markdown");
    let text = fs::read_to_string(&bare).expect("read markdown");
    assert!(!text.contains("```ts"), "tag is cleared: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    let wrong_block =
        edit(handle, &json!({ "operations": [{ "type": "code.setLanguage", "id": "node-999999" }] }))
            .expect_err("unknown block");
    assert_eq!(wrong_block.code, "object/not-found");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn block_duplicate_copies_a_section_subtree_with_fresh_ids() {
    let workspace = Workspace::new("duplicate-section");
    let created = create(&json!({ "title": "Duplicate section" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Alpha" },
                { "type": "paragraph.append", "text": "one" },
                { "type": "paragraph.append", "text": "two" },
            ],
        }),
    )
    .expect("seed section");
    let seeded = summary(handle).expect("summary");
    assert_eq!(seeded["sections"].as_array().unwrap().len(), 1);
    assert_eq!(summary_text(&seeded), "one\ntwo");
    let source = seeded["sections"][0]["id"].as_str().expect("section id").to_string();
    let before = revision_of(&seeded);

    let duplicated =
        edit(handle, &json!({ "operations": [{ "type": "block.duplicate", "id": source }] })).expect("duplicate section");
    assert_eq!(duplicated["results"][0]["type"], json!("block.duplicate"));
    assert_eq!(duplicated["results"][0]["id"], json!(source));
    let new_id = duplicated["results"][0]["newId"].as_str().expect("new id").to_string();
    assert!(new_id.starts_with("node-"), "fresh addresses keep the declared shape: {new_id}");
    assert_eq!(duplicated["results"][0]["index"], json!(3), "the copy follows the source subtree");
    assert_eq!(revision_of(&duplicated), before + 1);

    let document = &duplicated["document"];
    let sections = document["sections"].as_array().expect("sections");
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0]["title"], json!("Alpha"));
    assert_eq!(sections[1]["title"], json!("Alpha"));
    assert_ne!(sections[1]["id"], json!(source), "the copy is a new node");
    assert_eq!(summary_text(document), "one\ntwo\none\ntwo");
    let paragraphs = document["paragraphs"].as_array().expect("paragraphs");
    assert_eq!(paragraphs[2]["sectionIndex"], json!(1));
    assert_eq!(paragraphs[3]["sectionIndex"], json!(1));

    let project = workspace.path("duplicate.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let blocks = envelope["document"]["blocks"].as_array().expect("blocks");
    let source_number = address_number(&source);
    let copy_number = address_number(&new_id);
    let source_children = section_children(blocks, source_number);
    let copy_children = section_children(blocks, copy_number);
    assert_eq!(source_children.len(), 2);
    assert_eq!(copy_children.len(), 2);
    for child in &copy_children {
        assert!(!source_children.contains(child), "copied child {child} must not alias a source id");
    }
    assert_eq!(paragraph_text(blocks, copy_children[0]), "one");
    assert_eq!(paragraph_text(blocks, copy_children[1]), "two");

    close(handle).expect("close");
}

#[test]
fn block_duplicate_copies_a_table_block() {
    let created = create(&json!({ "title": "Duplicate table" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "table.append", "rows": [["Name", "Value"], ["alpha", "1"]] }] }))
        .expect("append table");
    let seeded = summary(handle).expect("summary");
    assert_eq!(seeded["tables"].as_array().unwrap().len(), 1);
    let before = revision_of(&seeded);

    let project = Workspace::new("duplicate-table");
    let target = project.path("table.oword");
    save(handle, &target, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&target).expect("read project")).expect("parse project");
    let blocks = envelope["document"]["blocks"].as_array().expect("blocks");
    let table = blocks.iter().find(|node| node["block"].get("Table").is_some()).expect("table block");
    let table_id = table["id"].as_u64().expect("table id");
    let table_address = format!("node-{table_id}");

    let duplicated =
        edit(handle, &json!({ "operations": [{ "type": "block.duplicate", "id": table_address }] })).expect("duplicate table");
    assert_eq!(duplicated["results"][0]["id"], json!(table_address));
    let new_id = duplicated["results"][0]["newId"].as_str().expect("new id").to_string();
    assert_ne!(new_id, table_address);
    assert_eq!(duplicated["results"][0]["index"], json!(5), "the copy follows the source subtree");
    assert_eq!(revision_of(&duplicated), before + 1);

    let tables = duplicated["document"]["tables"].as_array().expect("tables");
    assert_eq!(tables.len(), 2);
    for table in tables {
        assert_eq!(table["rows"], json!(2));
        assert_eq!(table["columns"], json!(2));
    }
    let hits = find(handle, "alpha", None).expect("find");
    assert_eq!(hits.as_array().expect("hits").len(), 2, "the copied cell keeps its text: {hits}");

    let after = project.path("table-copy.oword");
    save(handle, &after, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&after).expect("read project")).expect("parse project");
    let blocks = envelope["document"]["blocks"].as_array().expect("blocks");
    let tables = blocks.iter().filter(|node| node["block"].get("Table").is_some()).collect::<Vec<_>>();
    assert_eq!(tables.len(), 2);
    let source_cells = table_cell_children(tables[0]);
    let copy_cells = table_cell_children(tables[1]);
    assert_eq!(source_cells.len(), 4, "two rows of two cells");
    assert_eq!(copy_cells.len(), 4);
    for children in &copy_cells {
        for child in children {
            assert!(
                !source_cells.iter().flatten().any(|source| source == child),
                "copied cell {child} must not alias a source id"
            );
        }
    }

    close(handle).expect("close");
}

#[test]
fn block_duplicate_rejects_unknown_ids_and_bumps_one_revision() {
    let created = create(&json!({ "title": "Duplicate errors" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "top" }] })).expect("seed paragraph");
    let seeded = summary(handle).expect("summary");
    let before = revision_of(&seeded);

    let error = edit(handle, &json!({ "operations": [{ "type": "block.duplicate", "id": "node-999999" }] }))
        .expect_err("unknown block must fail");
    assert_eq!(error.code, "object/not-found");
    let error = edit(handle, &json!({ "operations": [{ "type": "block.duplicate" }] })).expect_err("missing id must fail");
    assert_eq!(error.code, "session/argument-invalid");
    let unchanged = summary(handle).expect("summary");
    assert_eq!(revision_of(&unchanged), before);
    assert_eq!(unchanged, seeded);

    let hits = find(handle, "top", None).expect("find");
    let id = hits.as_array().expect("hits")[0]["blockId"].as_str().expect("block id").to_string();
    let duplicated = edit(handle, &json!({ "operations": [{ "type": "block.duplicate", "id": id }] }))
        .expect("duplicate a top-level paragraph");
    assert_eq!(duplicated["results"][0]["type"], json!("block.duplicate"));
    assert_eq!(duplicated["results"][0]["id"], json!(id));
    assert!(duplicated["results"][0]["newId"].as_str().unwrap_or_default().starts_with("node-"));
    assert_eq!(duplicated["results"][0]["index"], json!(1), "the copy follows the source at the top level");
    assert_eq!(revision_of(&duplicated), before + 1);
    assert_eq!(summary_text(&duplicated["document"]), "top\ntop");
    let paragraphs = duplicated["document"]["paragraphs"].as_array().unwrap();
    assert_eq!(paragraphs[0]["sectionIndex"], json!(-1));
    assert_eq!(paragraphs[1]["sectionIndex"], json!(-1));

    close(handle).expect("close");
}

fn address_number(address: &str) -> u64 {
    address.rsplit('-').next().expect("address suffix").parse().expect("numeric address")
}

fn section_children(blocks: &[Value], id: u64) -> Vec<u64> {
    let node = blocks.iter().find(|node| node["id"] == json!(id)).expect("section block");
    node["block"]["Section"]["children"]
        .as_array()
        .expect("section children")
        .iter()
        .map(|child| child.as_u64().expect("child id"))
        .collect()
}

fn paragraph_text(blocks: &[Value], id: u64) -> String {
    let node = blocks.iter().find(|node| node["id"] == json!(id)).expect("paragraph block");
    node["block"]["Paragraph"]["content"][0]["Text"]["text"].as_str().expect("paragraph text").to_string()
}

fn table_cell_children(node: &Value) -> Vec<Vec<u64>> {
    let table = &node["block"]["Table"];
    let mut cells = Vec::new();
    for band in ["head", "body", "foot"] {
        let Some(rows) = table.get(band).and_then(Value::as_array)
        else {
            continue;
        };
        for row in rows {
            for cell in row["cells"].as_array().expect("cells") {
                cells.push(
                    cell["children"]
                        .as_array()
                        .expect("cell children")
                        .iter()
                        .map(|child| child.as_u64().expect("child id"))
                        .collect(),
                );
            }
        }
    }
    cells
}

#[test]
fn create_opens_a_template_and_stats_reports_the_document() {
    let workspace = Workspace::new("stats");
    let template = workspace.markdown(
        "template.md",
        "# Overview\n\nHello world.\n\n- alpha beta\n- gamma\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n",
    );
    let created =
        create(&json!({ "template": template.display().to_string(), "title": "Filled" })).expect("create from template");
    assert_eq!(created["document"]["title"], json!("Filled"));
    let handle = handle_of(&created);

    let report = stats(handle).expect("stats");
    assert_eq!(report["headings"]["1"], json!(1));
    assert_eq!(report["tables"], json!(1));
    assert_eq!(report["tableRows"], json!(2));
    assert!(report["paragraphs"].as_u64().unwrap() >= 3, "paragraphs: {report}");
    assert_eq!(report["words"], json!(10));
    assert!(report["characters"].as_u64().unwrap() > report["charactersWithoutWhitespace"].as_u64().unwrap());
    assert_eq!(report["readingMinutes"], json!(1));

    close(handle).expect("close");
}

#[test]
fn quote_set_text_replaces_the_quote_content_in_place() {
    let workspace = Workspace::new("quotetext");
    let created = create(&json!({ "title": "Quote" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "quote.append", "text": "First draft." }] })).expect("append quote");

    // The quote owns one paragraph child: node-1 is the child, node-2 the quote.
    let changed = edit(handle, &json!({ "operations": [{ "type": "quote.setText", "id": "node-2", "text": "Revised quote." }] }))
        .expect("rewrite quote");
    assert_eq!(changed["results"][0]["type"], json!("quote.setText"));
    assert_eq!(changed["results"][0]["id"], json!("node-2"));
    assert_eq!(revision_of(&changed), 2);

    let exported = workspace.path("revised.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("> Revised quote."), "new text is quoted: {text}");
    assert!(!text.contains("First draft"), "old text is gone: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    let missing = edit(handle, &json!({ "operations": [{ "type": "quote.setText", "id": "node-999999", "text": "x" }] }))
        .expect_err("unknown block");
    assert_eq!(missing.code, "object/not-found");
    let wrong = edit(handle, &json!({ "operations": [{ "type": "quote.setText", "id": "node-3", "text": "x" }] }))
        .expect_err("the replacement paragraph child is not a quote");
    assert_eq!(wrong.code, "session/argument-invalid");
    let empty = edit(handle, &json!({ "operations": [{ "type": "quote.setText", "id": "node-2", "text": "" }] }))
        .expect_err("empty text must fail");
    assert_eq!(empty.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn code_set_text_rewrites_the_source_and_keeps_the_tag() {
    let workspace = Workspace::new("codetext");
    let created = create(&json!({ "title": "Code" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "code.append", "text": "let value = 1;", "language": "rust" }] }))
        .expect("append code");

    let changed = edit(handle, &json!({ "operations": [{ "type": "code.setText", "id": "node-1", "text": "let value = 2;" }] }))
        .expect("rewrite code");
    assert_eq!(changed["results"][0]["type"], json!("code.setText"));
    assert_eq!(changed["results"][0]["id"], json!("node-1"));
    assert_eq!(revision_of(&changed), 2);

    let exported = workspace.path("rewritten.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("let value = 2;"), "new source is written: {text}");
    assert!(!text.contains("let value = 1;"), "old source is gone: {text}");
    assert!(text.contains("```rust"), "the tag survives: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    let missing = edit(handle, &json!({ "operations": [{ "type": "code.setText", "id": "node-999999", "text": "x" }] }))
        .expect_err("unknown block");
    assert_eq!(missing.code, "object/not-found");
    let empty = edit(handle, &json!({ "operations": [{ "type": "code.setText", "id": "node-1", "text": "" }] }))
        .expect_err("empty text must fail");
    assert_eq!(empty.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn heading_insert_lands_before_the_addressed_block() {
    let workspace = Workspace::new("headinginsert");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let inserted = edit(handle, &json!({ "operations": [{ "type": "heading.insert", "index": 2, "level": 2, "text": "Background" }] }))
        .expect("insert a heading before the Details section");
    assert_eq!(inserted["results"][0]["type"], json!("heading.insert"));
    let id = inserted["results"][0]["id"].as_str().expect("heading id").to_owned();
    assert!(id.starts_with("section-"), "the new block is a section: {id}");
    assert_eq!(revision_of(&inserted), 1);

    let seeded = summary(handle).expect("summary");
    let titles: Vec<&str> = seeded["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .map(|section| section["title"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(titles, vec!["Overview", "Background", "Details"], "the new heading lands before the addressed block");

    let exported = workspace.path("inserted.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("## Background"), "the heading exports at its level: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "heading.insert", "index": 99, "level": 2, "text": "Far" }),
        json!({ "type": "heading.insert", "index": 0, "level": 0, "text": "Bottom" }),
        json!({ "type": "heading.insert", "index": 0, "level": 7, "text": "Top" }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid heading insert must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn table_row_moves_and_duplicates_copy_across_the_bands() {
    let workspace = Workspace::new("rowmoves");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let moved = edit(handle, &json!({ "operations": [{ "type": "table.moveRow", "table": 0, "row": 1, "index": 0 }] }))
        .expect("move the body row to the front");
    assert_eq!(moved["results"][0]["type"], json!("table.moveRow"));
    assert_eq!(moved["results"][0]["index"], json!(0));
    assert_eq!(moved["document"]["tables"][0]["rows"], json!(2));
    assert_eq!(revision_of(&moved), 1);

    let exported = workspace.path("moved.txt");
    export(handle, "txt", &exported, false).expect("export txt");
    let text = fs::read_to_string(&exported).expect("read txt");
    let position = |marker: &str| text.find(marker).unwrap_or_else(|| panic!("missing {marker:?} in: {text}"));
    assert!(position("alpha") < position("Name"), "the moved row leads the grid: {text}");

    let duplicated = edit(handle, &json!({ "operations": [{ "type": "table.duplicateRow", "table": 0, "row": 0 }] }))
        .expect("duplicate the top row");
    assert_eq!(duplicated["results"][0]["type"], json!("table.duplicateRow"));
    assert_eq!(duplicated["results"][0]["newRow"], json!(1));
    assert_eq!(duplicated["document"]["tables"][0]["rows"], json!(3));
    assert_eq!(revision_of(&duplicated), 2);

    // The copy is a fresh subtree: rewriting it leaves the source row untouched.
    edit(handle, &json!({ "operations": [{ "type": "table.setCell", "table": 0, "row": 1, "column": 0, "text": "gamma" }] }))
        .expect("rewrite the copy");
    export(handle, "md", &workspace.path("copied.txt"), false).expect("export txt");
    let text = fs::read_to_string(workspace.path("copied.txt")).expect("read txt");
    assert!(text.contains("gamma"), "the copy carries the edit: {text}");
    assert!(text.contains("alpha"), "the source row keeps its text: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "table.moveRow", "table": 0, "row": 9, "index": 0 }),
        json!({ "type": "table.moveRow", "table": 0, "row": 0, "index": 9 }),
        json!({ "type": "table.duplicateRow", "table": 0, "row": 9 }),
        json!({ "type": "table.duplicateRow", "table": 9, "row": 0 }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid table address must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn table_column_moves_and_duplicates_copy_across_each_row() {
    let workspace = Workspace::new("columnmoves");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let moved = edit(handle, &json!({ "operations": [{ "type": "table.moveColumn", "table": 0, "column": 1, "index": 0 }] }))
        .expect("move the value column to the front");
    assert_eq!(moved["results"][0]["type"], json!("table.moveColumn"));
    assert_eq!(moved["results"][0]["index"], json!(0));
    assert_eq!(revision_of(&moved), 1);

    let exported = workspace.path("moved-columns.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(
        text.find("Value").expect("Value") < text.find("Name").expect("Name"),
        "the moved column leads every row: {text}"
    );

    let duplicated = edit(handle, &json!({ "operations": [{ "type": "table.duplicateColumn", "table": 0, "column": 0 }] }))
        .expect("duplicate the leading column");
    assert_eq!(duplicated["results"][0]["type"], json!("table.duplicateColumn"));
    assert_eq!(duplicated["results"][0]["newColumn"], json!(1));
    assert_eq!(revision_of(&duplicated), 2);

    edit(handle, &json!({ "operations": [{ "type": "table.setCell", "table": 0, "row": 1, "column": 1, "text": "copied" }] }))
        .expect("rewrite the copy");
    let exported = workspace.path("copied-columns.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("copied"), "the copy carries the edit: {text}");
    assert!(text.contains("Value") && text.contains("Name"), "both headers survive: {text}");
    assert!(text.contains("| 1 |"), "the source cell keeps its text: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "table.moveColumn", "table": 0, "column": 9, "index": 0 }),
        json!({ "type": "table.moveColumn", "table": 0, "column": 0, "index": 9 }),
        json!({ "type": "table.duplicateColumn", "table": 0, "column": 9 }),
        json!({ "type": "table.duplicateColumn", "table": 9, "column": 0 }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid column address must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    // A spanned table has no single column boundary, so whole-column moves are refused.
    edit(handle, &json!({ "operations": [{ "type": "table.setCellSpan", "table": 0, "row": 0, "column": 0, "colspan": 2, "rowspan": 1 }] }))
        .expect("span the header");
    let spanned = edit(handle, &json!({ "operations": [{ "type": "table.moveColumn", "table": 0, "column": 0, "index": 1 }] }))
        .expect_err("a spanned table is refused");
    assert_eq!(spanned.code, "session/argument-invalid");

    close(handle).expect("close");
}

#[test]
fn list_item_edits_replace_insert_and_remove_items_in_place() {
    let workspace = Workspace::new("listitems");
    let created = create(&json!({ "title": "Items" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "list.append", "items": ["Alpha", "Beta"] },
        { "type": "taskList.append", "items": [{ "text": "Write spec", "checked": true }, "Review"] },
    ] }))
    .expect("append a bullet list and a task list");
    // node-1 is the bullet list, node-2 the task list.

    let renamed = edit(handle, &json!({ "operations": [{ "type": "list.setItem", "id": "node-1", "item": 0, "text": "Alpha edited" }] }))
        .expect("rename the first item");
    assert_eq!(renamed["results"][0]["type"], json!("list.setItem"));
    assert_eq!(renamed["results"][0]["id"], json!("node-1"));
    assert_eq!(renamed["results"][0]["item"], json!(0));
    assert_eq!(revision_of(&renamed), 3);

    // The check state survives a text rewrite.
    edit(handle, &json!({ "operations": [{ "type": "list.setItem", "id": "node-2", "item": 0, "text": "Write the spec" }] }))
        .expect("rewrite the task");

    edit(handle, &json!({ "operations": [{ "type": "list.insertItem", "id": "node-1", "item": 1, "text": "Gamma" }] }))
        .expect("insert before Beta");
    let exported = workspace.path("items.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    let position = |marker: &str| text.find(marker).unwrap_or_else(|| panic!("missing {marker:?} in: {text}"));
    assert!(text.contains("- Alpha edited") && text.contains("- Gamma") && text.contains("- Beta"), "bullet run: {text}");
    assert!(position("- Alpha edited") < position("- Gamma"), "insertion honours the index: {text}");
    assert!(position("- Gamma") < position("- Beta"), "insertion honours the index: {text}");
    assert!(text.contains("- [x] Write the spec"), "the check state survives: {text}");
    assert!(text.contains("- [ ] Review"), "the untouched task keeps its marker: {text}");

    let removed = edit(handle, &json!({ "operations": [{ "type": "list.removeItem", "id": "node-1", "item": 1 }] }))
        .expect("remove Gamma");
    assert_eq!(removed["results"][0]["type"], json!("list.removeItem"));
    assert_eq!(removed["results"][0]["item"], json!(1));
    assert_eq!(revision_of(&removed), 6);
    export(handle, "md", &workspace.path("removed.md"), false).expect("export markdown");
    let text = fs::read_to_string(workspace.path("removed.md")).expect("read markdown");
    assert!(!text.contains("Gamma"), "the removed item is gone: {text}");
    assert!(text.contains("- Alpha edited") && text.contains("- Beta"), "the neighbours stay: {text}");

    // A heading and a one-item list exercise the refusal paths; the heading is
    // addressed through its `section-N` alias and node-4 is the fresh one-item list.
    edit(handle, &json!({ "operations": [{ "type": "heading.append", "level": 2, "text": "Note" }] })).expect("append a heading");
    let seeded = summary(handle).expect("summary");
    let heading_id = seeded["sections"][0]["id"].as_str().expect("heading id").to_owned();
    edit(handle, &json!({ "operations": [{ "type": "list.append", "items": ["Solo"] }] })).expect("append a one-item list");

    let before = revision_of(&summary(handle).expect("summary"));
    let missing = edit(handle, &json!({ "operations": [{ "type": "list.setItem", "id": "node-999999", "item": 0, "text": "x" }] }))
        .expect_err("unknown block");
    assert_eq!(missing.code, "object/not-found");
    let not_a_list = edit(handle, &json!({ "operations": [{ "type": "list.setItem", "id": heading_id, "item": 0, "text": "x" }] }))
        .expect_err("a paragraph is not a list");
    assert_eq!(not_a_list.code, "session/argument-invalid");
    let out_of_range = edit(handle, &json!({ "operations": [{ "type": "list.setItem", "id": "node-4", "item": 9, "text": "x" }] }))
        .expect_err("index outside the run");
    assert_eq!(out_of_range.code, "session/argument-invalid");
    let last_item = edit(handle, &json!({ "operations": [{ "type": "list.removeItem", "id": "node-4", "item": 0 }] }))
        .expect_err("the last item cannot be removed");
    assert_eq!(last_item.code, "session/argument-invalid");
    let blank = edit(handle, &json!({ "operations": [{ "type": "list.setItem", "id": "node-4", "item": 0, "text": "  " }] }))
        .expect_err("blank text must fail");
    assert_eq!(blank.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn task_items_toggle_and_list_items_move_in_place() {
    let workspace = Workspace::new("listmoving");
    let created = create(&json!({ "title": "Checklist" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "taskList.append", "items": [{ "text": "Draft", "checked": false }, "Review", "Ship"] },
        { "type": "list.append", "items": ["One", "Two", "Three"], "ordered": true, "start": 3 },
    ] }))
    .expect("append a task list and an ordered list");
    // node-1 is the task list, node-2 the ordered list.

    let toggled =
        edit(handle, &json!({ "operations": [{ "type": "list.setItemChecked", "id": "node-1", "item": 1, "checked": true }] }))
            .expect("check the second task");
    assert_eq!(toggled["results"][0]["type"], json!("list.setItemChecked"));
    assert_eq!(toggled["results"][0]["id"], json!("node-1"));
    assert_eq!(toggled["results"][0]["item"], json!(1));
    assert_eq!(toggled["results"][0]["checked"], json!(true));

    let moved = edit(handle, &json!({ "operations": [{ "type": "list.moveItem", "id": "node-1", "item": 2, "index": 0 }] }))
        .expect("move the last task to the front");
    assert_eq!(moved["results"][0]["type"], json!("list.moveItem"));
    assert_eq!(moved["results"][0]["index"], json!(0));

    // An ordered list renumbers from the number its first item carried.
    edit(handle, &json!({ "operations": [{ "type": "list.moveItem", "id": "node-2", "item": 0, "index": 2 }] }))
        .expect("move the first ordered item to the end");

    let exported = workspace.path("checklist.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    let position = |marker: &str| text.find(marker).unwrap_or_else(|| panic!("missing {marker:?} in: {text}"));
    assert!(position("- [ ] Ship") < position("- [ ] Draft"), "the move put Ship first: {text}");
    assert!(text.contains("- [x] Review"), "the toggled task is checked: {text}");
    assert!(!text.contains("- [x] Ship"), "the moved task keeps its own state: {text}");
    assert!(text.contains("3. Two") && text.contains("4. Three") && text.contains("5. One"), "renumbered run: {text}");

    // A non-task list has no check state, a missing boolean is refused and an
    // index outside the run is rejected before anything changes.
    let before = revision_of(&summary(handle).expect("summary"));
    let not_a_task =
        edit(handle, &json!({ "operations": [{ "type": "list.setItemChecked", "id": "node-2", "item": 0, "checked": true }] }))
            .expect_err("an ordered list has no check state");
    assert_eq!(not_a_task.code, "session/argument-invalid");
    let out_of_range = edit(handle, &json!({ "operations": [{ "type": "list.moveItem", "id": "node-1", "item": 0, "index": 9 }] }))
        .expect_err("index outside the run");
    assert_eq!(out_of_range.code, "session/argument-invalid");
    let missing_checked = edit(handle, &json!({ "operations": [{ "type": "list.setItemChecked", "id": "node-1", "item": 0 }] }))
        .expect_err("a missing boolean must fail");
    assert_eq!(missing_checked.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn list_items_indent_under_the_previous_sibling() {
    let workspace = Workspace::new("listindent");
    let created = create(&json!({ "title": "Outline" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "list.append", "items": ["Alpha", "Beta", "Gamma"] },
        { "type": "list.append", "items": ["One", "Two", "Three"], "ordered": true },
    ] }))
    .expect("append a bullet list and an ordered list");
    // node-1 is the bullet list, node-2 the ordered list.

    let nested = edit(handle, &json!({ "operations": [{ "type": "list.indentItem", "id": "node-1", "item": 1 }] }))
        .expect("nest Beta under Alpha");
    assert_eq!(nested["results"][0]["type"], json!("list.indentItem"));
    assert_eq!(nested["results"][0]["id"], json!("node-1"));
    assert_eq!(nested["results"][0]["item"], json!(1));

    // A second indent joins the nested run under the same parent instead of
    // opening a sibling list.
    edit(handle, &json!({ "operations": [{ "type": "list.indentItem", "id": "node-1", "item": 1 }] }))
        .expect("nest Gamma under Alpha");

    // An ordered list renumbers the parent run and restarts the nested run at 1.
    edit(handle, &json!({ "operations": [{ "type": "list.indentItem", "id": "node-2", "item": 1 }] }))
        .expect("nest Two under One");

    let exported = workspace.path("outline.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("- Alpha\n  - Beta\n  - Gamma\n"), "one nested run under Alpha: {text}");
    assert!(text.contains("1. One\n  1. Two\n2. Three\n"), "ordered runs renumber on both levels: {text}");

    // The first item has no previous sibling and an index outside the run is
    // refused before anything changes.
    let before = revision_of(&summary(handle).expect("summary"));
    let first = edit(handle, &json!({ "operations": [{ "type": "list.indentItem", "id": "node-1", "item": 0 }] }))
        .expect_err("the first item cannot nest");
    assert_eq!(first.code, "session/argument-invalid");
    let out_of_range = edit(handle, &json!({ "operations": [{ "type": "list.indentItem", "id": "node-1", "item": 9 }] }))
        .expect_err("index outside the run");
    assert_eq!(out_of_range.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn list_items_unindent_back_into_the_parent_run() {
    let workspace = Workspace::new("listunindent");
    let created = create(&json!({ "title": "Outline" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "list.append", "items": ["Alpha", "Beta", "Gamma"] },
        { "type": "list.append", "items": ["One", "Two", "Three"], "ordered": true },
    ] }))
    .expect("append a bullet list and an ordered list");

    edit(handle, &json!({ "operations": [
        { "type": "list.indentItem", "id": "node-1", "item": 1 },
        { "type": "list.indentItem", "id": "node-2", "item": 1 },
    ] }))
    .expect("nest one item in each list");

    let lifted = edit(handle, &json!({ "operations": [
        { "type": "list.unindentItem", "id": "node-1", "item": 0 },
        { "type": "list.unindentItem", "id": "node-2", "item": 0 },
    ] }))
    .expect("lift the nested items back");
    assert_eq!(lifted["results"][0]["type"], json!("list.unindentItem"));
    assert_eq!(lifted["results"][1]["item"], json!(0));

    let exported = workspace.path("lifted.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("- Alpha\n- Beta\n- Gamma\n"), "the bullet run is flat again: {text}");
    assert!(text.contains("1. One\n2. Two\n3. Three\n"), "the ordered run is flat and renumbered: {text}");

    // Items with no nested list, and indexes outside the run, are refused
    // before anything changes.
    let before = revision_of(&summary(handle).expect("summary"));
    let plain = edit(handle, &json!({ "operations": [{ "type": "list.unindentItem", "id": "node-1", "item": 0 }] }))
        .expect_err("a flat item has nothing to lift");
    assert_eq!(plain.code, "session/argument-invalid");
    let out_of_range = edit(handle, &json!({ "operations": [{ "type": "list.unindentItem", "id": "node-1", "item": 7 }] }))
        .expect_err("index outside the run");
    assert_eq!(out_of_range.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn list_items_move_between_lists_and_renumber_both_runs() {
    let workspace = Workspace::new("moveacross");
    let created = create(&json!({ "title": "Cross list" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "list.append", "items": ["One", "Two", "Three"], "ordered": true },
        { "type": "list.append", "items": ["Four", "Five"], "ordered": true },
    ] }))
    .expect("append two ordered lists");
    edit(handle, &json!({ "operations": [{ "type": "list.indentItem", "id": "node-1", "item": 1 }] }))
        .expect("nest Two under One");

    let report = edit(handle, &json!({ "operations": [
        { "type": "list.moveItemTo", "id": "node-1", "item": 1, "target": "node-2", "index": 1 },
    ] }))
    .expect("move Three into the second list");
    assert_eq!(report["results"][0]["type"], json!("list.moveItemTo"));
    assert_eq!(report["results"][0]["id"], json!("node-1"));
    assert_eq!(report["results"][0]["target"], json!("node-2"));
    assert_eq!(report["results"][0]["item"], json!(1));
    assert_eq!(report["results"][0]["index"], json!(1));

    let exported = workspace.path("crossed.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("1. One\n  1. Two"), "the source keeps its nested run: {text}");
    assert!(text.contains("1. Four\n2. Three\n3. Five"), "the target renumbers around the moved item: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "list.moveItemTo", "id": "node-1", "item": 5, "target": "node-2", "index": 0 }),
        json!({ "type": "list.moveItemTo", "id": "node-1", "item": 0, "target": "node-2", "index": 9 }),
        json!({ "type": "list.moveItemTo", "id": "node-1", "item": 0, "target": "node-999", "index": 0 }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("bad cross-list move");
        assert!(
            error.code == "session/argument-invalid" || error.code == "object/not-found",
            "unexpected code: {}",
            error.code
        );
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before, "failed moves consume no revision");

    close(handle).expect("close");
}

#[test]
fn list_items_duplicate_with_their_nested_children() {
    let workspace = Workspace::new("listduplicate");
    let created = create(&json!({ "title": "Outline" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "list.append", "items": ["Alpha", "Beta", "Gamma"] },
        { "type": "list.append", "items": ["One", "Two"], "ordered": true },
    ] }))
    .expect("append a bullet list and an ordered list");
    edit(handle, &json!({ "operations": [{ "type": "list.indentItem", "id": "node-1", "item": 2 }] }))
        .expect("nest Gamma under Beta");

    let duplicated = edit(handle, &json!({ "operations": [
        { "type": "list.duplicateItem", "id": "node-1", "item": 1 },
        { "type": "list.duplicateItem", "id": "node-2", "item": 0 },
    ] }))
    .expect("duplicate one item in each list");
    assert_eq!(duplicated["results"][0]["type"], json!("list.duplicateItem"));
    assert_eq!(duplicated["results"][0]["id"], json!("node-1"));
    assert_eq!(duplicated["results"][0]["item"], json!(1));
    assert_eq!(duplicated["results"][1]["item"], json!(0));

    let exported = workspace.path("outline.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("- Alpha\n- Beta\n  - Gamma\n- Beta\n  - Gamma\n"), "the copy keeps its nested run: {text}");
    assert!(text.contains("1. One\n2. One\n3. Two\n"), "the ordered run renumbers around the copy: {text}");

    // An index outside the run is refused before anything changes.
    let before = revision_of(&summary(handle).expect("summary"));
    let out_of_range = edit(handle, &json!({ "operations": [{ "type": "list.duplicateItem", "id": "node-1", "item": 9 }] }))
        .expect_err("index outside the run");
    assert_eq!(out_of_range.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn list_items_sort_by_text_and_keep_their_nested_children() {
    let workspace = Workspace::new("listsort");
    let created = create(&json!({ "title": "Sort" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "list.append", "items": ["Gamma", "alpha", "2", "10"], "ordered": true },
    ] }))
    .expect("append an ordered list");
    edit(handle, &json!({ "operations": [{ "type": "list.indentItem", "id": "node-1", "item": 1 }] }))
        .expect("nest alpha under Gamma");

    let sorted = edit(handle, &json!({ "operations": [{ "type": "list.sortItems", "id": "node-1" }] }))
        .expect("sort ascending");
    assert_eq!(sorted["results"][0]["type"], json!("list.sortItems"));
    assert_eq!(sorted["results"][0]["id"], json!("node-1"));
    assert_eq!(sorted["results"][0]["items"], json!(3));
    assert_eq!(sorted["results"][0]["ascending"], json!(true));

    let exported = workspace.path("sorted.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(
        text.contains("1. 2\n2. 10\n3. Gamma\n  1. alpha\n"),
        "numbers sort numerically and the nested run travels with Gamma: {text}"
    );

    edit(handle, &json!({ "operations": [{ "type": "list.sortItems", "id": "node-1", "ascending": false }] }))
        .expect("sort descending");
    let exported = workspace.path("reversed.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(
        text.contains("1. Gamma\n  1. alpha\n2. 10\n3. 2\n"),
        "descending sort keeps the nested run attached: {text}"
    );

    let before = revision_of(&summary(handle).expect("summary"));
    let missing =
        edit(handle, &json!({ "operations": [{ "type": "list.sortItems", "id": "node-9" }] })).expect_err("unknown list");
    assert_eq!(missing.code, "object/not-found");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before, "failed sorts consume no revision");

    close(handle).expect("close");
}

#[test]
fn ordered_list_item_edits_renumber_from_the_list_start() {
    let workspace = Workspace::new("ordereditems");
    let created = create(&json!({ "title": "Steps" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "list.append", "items": ["One", "Two"], "ordered": true }] }))
        .expect("append an ordered list");

    // Inserting at the front keeps the start value, so the run stays consecutive.
    edit(handle, &json!({ "operations": [{ "type": "list.insertItem", "id": "node-1", "item": 0, "text": "Zero" }] }))
        .expect("insert at the front");
    edit(handle, &json!({ "operations": [{ "type": "list.setStart", "id": "node-1", "start": 7 }] })).expect("set start");
    edit(handle, &json!({ "operations": [{ "type": "list.insertItem", "id": "node-1", "item": 3, "text": "Ten" }] }))
        .expect("insert at the end");
    let removed = edit(handle, &json!({ "operations": [{ "type": "list.removeItem", "id": "node-1", "item": 0 }] }))
        .expect("remove the front item");
    assert_eq!(revision_of(&removed), 5);

    let exported = workspace.path("steps.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("7. One") && text.contains("8. Two") && text.contains("9. Ten"), "consecutive numbering from the start: {text}");
    assert!(!text.contains("Zero"), "the removed item is gone: {text}");

    close(handle).expect("close");
}

#[test]
fn block_text_case_rewrites_paragraph_and_heading_text() {
    let workspace = Workspace::new("textcase");
    let created = create(&json!({ "title": "Casing" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "heading.append", "level": 2, "text": "section TITLE" }] }))
        .expect("append heading");
    let inserted = edit(handle, &json!({ "operations": [{ "type": "paragraph.insert", "index": 0, "text": "hello WORLD" }] }))
        .expect("insert paragraph");
    let paragraph = inserted["results"][0]["id"].as_str().expect("paragraph id").to_owned();
    let outline_report = outline(handle).expect("outline");
    let (heading, _) = find_heading(outline_report["items"].as_array().expect("items"), "section TITLE")
        .expect("heading lookup");

    let report = edit(handle, &json!({ "operations": [
        { "type": "block.setTextCase", "id": paragraph, "case": "title" },
        { "type": "block.setTextCase", "id": heading, "case": "upper" },
    ] }))
    .expect("apply casing");
    assert_eq!(report["results"][0]["case"], json!("title"));
    assert_eq!(report["results"][0]["changed"], json!(1));
    assert_eq!(report["results"][1]["case"], json!("upper"));

    let exported = workspace.path("cased.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("Hello World"), "title case rewrites the paragraph: {text}");
    assert!(text.contains("## SECTION TITLE"), "upper case rewrites the heading: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    let unknown_case = edit(handle, &json!({ "operations": [{ "type": "block.setTextCase", "id": paragraph, "case": "wide" }] }))
        .expect_err("unknown case");
    assert_eq!(unknown_case.code, "session/argument-invalid");
    let unknown_block =
        edit(handle, &json!({ "operations": [{ "type": "block.setTextCase", "id": "node-999", "case": "upper" }] }))
            .expect_err("unknown block");
    assert_eq!(unknown_block.code, "object/not-found");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before, "failed casing consumes no revision");

    close(handle).expect("close");
}

#[test]
fn table_set_footer_writes_and_clears_the_footer_band() {
    let workspace = Workspace::new("footer");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let changed = edit(
        handle,
        &json!({ "operations": [{ "type": "table.setFooter", "table": 0, "cells": ["Total", "42"] }] }),
    )
    .expect("set footer");
    assert_eq!(changed["results"][0]["type"], json!("table.setFooter"));
    assert_eq!(changed["results"][0]["table"], json!(0));
    assert_eq!(changed["results"][0]["cells"], json!(["Total", "42"]));
    assert_eq!(changed["document"]["tables"][0]["rows"], json!(3), "the footer adds a row");
    assert_eq!(revision_of(&changed), 1);

    let exported = workspace.path("footer.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("| Total | 42 |"), "the footer exports as the last row: {text}");

    let project = workspace.path("footer.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let table = envelope["document"]["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .find(|node| node["block"].get("Table").is_some())
        .expect("table block");
    assert_eq!(table["block"]["Table"]["foot"].as_array().expect("foot").len(), 1);

    let cleared =
        edit(handle, &json!({ "operations": [{ "type": "table.setFooter", "table": 0, "cells": [] }] }))
            .expect("clear footer");
    assert_eq!(cleared["document"]["tables"][0]["rows"], json!(2));
    export(handle, "md", &workspace.path("cleared.md"), false).expect("export markdown");
    let text = fs::read_to_string(workspace.path("cleared.md")).expect("read markdown");
    assert!(!text.contains("Total"), "the footer row is gone: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "table.setFooter", "table": 9, "cells": ["x"] }),
        json!({ "type": "table.setFooter", "table": 0 }),
        json!({ "type": "table.setFooter", "table": 0, "cells": [7] }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid footer must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn list_items_become_tasks_and_revert_to_plain_items() {
    let workspace = Workspace::new("itemtask");
    let created = create(&json!({ "title": "Tasks" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "list.append", "items": ["Write", "Ship"] }] }))
        .expect("append a bullet list");

    let converted =
        edit(handle, &json!({ "operations": [{ "type": "list.setItemTask", "id": "node-1", "item": 1, "task": true, "checked": true }] }))
            .expect("turn the second item into a checked task");
    assert_eq!(converted["results"][0]["type"], json!("list.setItemTask"));
    assert_eq!(converted["results"][0]["id"], json!("node-1"));
    assert_eq!(converted["results"][0]["task"], json!(true));
    assert_eq!(converted["results"][0]["checked"], json!(true));

    let exported = workspace.path("tasks.md");
    export(handle, "md", &exported, false).expect("export markdown");
    let text = fs::read_to_string(&exported).expect("read markdown");
    assert!(text.contains("- Write") && !text.contains("- [ ] Write"), "plain items stay plain: {text}");
    assert!(text.contains("- [x] Ship"), "the converted item carries its marker: {text}");

    let reverted =
        edit(handle, &json!({ "operations": [{ "type": "list.setItemTask", "id": "node-1", "item": 1, "task": false }] }))
            .expect("revert the item");
    assert_eq!(reverted["results"][0]["checked"], Value::Null, "plain items carry no check state");
    export(handle, "md", &workspace.path("plain.md"), false).expect("export markdown");
    let text = fs::read_to_string(workspace.path("plain.md")).expect("read markdown");
    assert!(text.contains("- Ship") && !text.contains("- [x] Ship"), "the task marker is gone: {text}");

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "list.setItemTask", "id": "node-1", "item": 9, "task": true }),
        json!({ "type": "list.setItemTask", "id": "node-1", "item": 0 }),
        json!({ "type": "list.setItemTask", "id": "node-1", "item": 0, "task": false, "checked": true }),
        json!({ "type": "list.setItemTask", "id": "node-999", "item": 0, "task": true }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid item task must fail");
        assert_ne!(error.code, "");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn metadata_set_writes_language_and_tags() {
    let workspace = Workspace::new("metadata");
    let created = create(&json!({ "title": "Metadata" })).expect("create");
    let handle = handle_of(&created);

    let changed = edit(
        handle,
        &json!({ "operations": [{
            "type": "metadata.set",
            "title": "Field notes",
            "author": "Ada",
            "language": "  zh-CN  ",
            "tags": [" alpha ", "beta"],
        }] }),
    )
    .expect("set metadata");
    assert_eq!(changed["results"][0]["type"], json!("metadata.set"));
    assert_eq!(changed["results"][0]["language"], json!("zh-CN"));
    assert_eq!(changed["results"][0]["tags"], json!(["alpha", "beta"]));
    assert_eq!(revision_of(&changed), 1);

    let project = workspace.path("metadata.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let metadata = &envelope["document"]["metadata"];
    assert_eq!(metadata["title"], json!("Field notes"));
    assert_eq!(metadata["language"], json!("zh-CN"));
    assert_eq!(metadata["authors"], json!(["Ada"]));
    assert_eq!(metadata["tags"], json!(["alpha", "beta"]));

    let cleared = edit(handle, &json!({ "operations": [{ "type": "metadata.set", "tags": [] }] }))
        .expect("clear tags");
    assert_eq!(cleared["results"][0]["tags"], json!([]));
    save(handle, &project, true).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    assert_eq!(envelope["document"]["metadata"]["tags"], json!([]));

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "metadata.set" }),
        json!({ "type": "metadata.set", "language": "   " }),
        json!({ "type": "metadata.set", "language": 7 }),
        json!({ "type": "metadata.set", "tags": "alpha" }),
        json!({ "type": "metadata.set", "tags": [1] }),
        json!({ "type": "metadata.set", "tags": [" "] }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid metadata must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn table_rows_sort_by_numeric_and_text_keys() {
    let workspace = Workspace::new("tablesort");
    let created = create(&json!({ "title": "Scores" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "table.append", "rows": [
            ["Name", "Score"],
            ["Beta", "2"],
            ["Alpha", "10"],
            ["Gamma", ""],
        ] },
    ] }))
    .expect("seed the table");

    let sorted = edit(handle, &json!({ "operations": [
        { "type": "table.sortRows", "table": 0, "column": 1 },
    ] }))
    .expect("sort by score");
    assert_eq!(sorted["results"][0]["type"], json!("table.sortRows"));

    let by_score = workspace.path("by-score.md");
    export(handle, "md", &by_score, false).expect("export markdown");
    let text = fs::read_to_string(&by_score).expect("read markdown");
    let beta = text.find("Beta").expect("Beta");
    let alpha = text.find("Alpha").expect("Alpha");
    let gamma = text.find("Gamma").expect("Gamma");
    assert!(text.contains("| Name | Score |"), "the header band stays in place: {text}");
    assert!(beta < alpha && alpha < gamma, "numeric order with blanks trailing: {text}");

    edit(handle, &json!({ "operations": [
        { "type": "table.sortRows", "table": 0, "column": 0, "ascending": false },
    ] }))
    .expect("sort by name descending");
    let by_name = workspace.path("by-name.md");
    export(handle, "md", &by_name, false).expect("export markdown");
    let text = fs::read_to_string(&by_name).expect("read markdown");
    let beta = text.find("Beta").expect("Beta");
    let alpha = text.find("Alpha").expect("Alpha");
    let gamma = text.find("Gamma").expect("Gamma");
    assert!(gamma < beta && beta < alpha, "text order descends case-insensitively: {text}");

    // A vertical merge would describe the wrong rows after a reorder, so it is
    // refused without consuming a revision.
    edit(handle, &json!({ "operations": [
        { "type": "table.setCellSpan", "table": 0, "row": 1, "column": 0, "colspan": 1, "rowspan": 2 },
    ] }))
    .expect("merge two rows");
    let before = revision_of(&summary(handle).expect("summary"));
    let merged = edit(handle, &json!({ "operations": [
        { "type": "table.sortRows", "table": 0, "column": 0 },
    ] }))
    .expect_err("vertical merges block sorting");
    assert_eq!(merged.code, "session/argument-invalid");
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    let missing = edit(handle, &json!({ "operations": [{ "type": "table.sortRows", "table": 9, "column": 0 }] }))
        .expect_err("unknown table");
    assert_eq!(missing.code, "session/argument-invalid");

    close(handle).expect("close");
}

#[test]
fn table_bands_accept_multiple_header_and_footer_rows() {
    let workspace = Workspace::new("bands");
    let opened = open(&workspace.markdown("report.md", FIXTURE)).expect("open fixture");
    let handle = handle_of(&opened);

    let changed = edit(
        handle,
        &json!({ "operations": [{
            "type": "table.setHeaderRows",
            "table": 0,
            "rows": [["Group", "Group"], ["Name", "Value"]],
        }] }),
    )
    .expect("set two header rows");
    assert_eq!(changed["results"][0], json!({ "type": "table.setHeaderRows", "table": 0, "rows": 2 }));
    assert_eq!(changed["document"]["tables"][0]["rows"], json!(3), "two header rows plus one body row");

    let footer = edit(
        handle,
        &json!({ "operations": [{
            "type": "table.setFooterRows",
            "table": 0,
            "rows": [["A", "1"], ["B", "2"]],
        }] }),
    )
    .expect("set two footer rows");
    assert_eq!(footer["results"][0]["rows"], json!(2));

    let project = workspace.path("bands.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let table = envelope["document"]["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .find(|node| node["block"].get("Table").is_some())
        .expect("table block");
    let head = table["block"]["Table"]["head"].as_array().expect("head");
    let foot = table["block"]["Table"]["foot"].as_array().expect("foot");
    assert_eq!(head.len(), 2);
    assert_eq!(foot.len(), 2);
    assert_eq!(head[1]["cells"][0]["children"].as_array().expect("children").len(), 1);
    assert_eq!(foot[1]["cells"][1]["children"].as_array().expect("children").len(), 1);

    let cleared = edit(handle, &json!({ "operations": [{ "type": "table.setFooterRows", "table": 0, "rows": [] }] }))
        .expect("clear footer rows");
    assert_eq!(cleared["results"][0]["rows"], json!(0));
    assert_eq!(cleared["document"]["tables"][0]["rows"], json!(3));

    let before = revision_of(&summary(handle).expect("summary"));
    for operation in [
        json!({ "type": "table.setHeaderRows", "table": 0, "rows": "Group" }),
        json!({ "type": "table.setHeaderRows", "table": 0, "rows": [[]] }),
        json!({ "type": "table.setHeaderRows", "table": 0, "rows": [["a"], ["b", "c"]] }),
        json!({ "type": "table.setHeaderRows", "table": 0, "rows": [[1]] }),
        json!({ "type": "table.setFooterRows", "table": 9, "rows": [["a"]] }),
    ] {
        let error = edit(handle, &json!({ "operations": [operation] })).expect_err("invalid band must fail");
        assert_eq!(error.code, "session/argument-invalid");
    }
    assert_eq!(revision_of(&summary(handle).expect("summary")), before);

    close(handle).expect("close");
}

#[test]
fn block_convert_switches_kinds_and_keeps_position() {
    let workspace = Workspace::new("convert");
    let created = create(&json!({ "title": "Convert" })).expect("create");
    let handle = handle_of(&created);
    let seeded = edit(handle, &json!({ "operations": [
        { "type": "paragraph.append", "text": "Lead" },
        { "type": "paragraph.append", "text": "Tail" },
    ] }))
    .expect("seed paragraphs");

    let heading = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "heading", "level": 2 }] }))
        .expect("convert to heading");
    assert_eq!(
        heading["results"][0],
        json!({ "type": "block.convert", "id": "node-1", "from": "paragraph", "to": "heading", "level": 2 })
    );
    assert_eq!(heading["document"]["sections"][0]["title"], json!("Lead"));
    assert_eq!(revision_of(&heading), revision_of(&seeded) + 1);

    let paragraph = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "section-1", "to": "paragraph" }] }))
        .expect("convert back to paragraph");
    assert_eq!(paragraph["results"][0], json!({ "type": "block.convert", "id": "section-1", "from": "heading", "to": "paragraph" }));
    assert!(paragraph["document"]["sections"].as_array().expect("sections").is_empty());
    assert_eq!(summary_text(&paragraph["document"]), "Lead\nTail");

    let quote = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "quote" }] }))
        .expect("convert to quote");
    assert_eq!(quote["results"][0], json!({ "type": "block.convert", "id": "node-1", "from": "paragraph", "to": "quote" }));
    assert_eq!(summary_text(&quote["document"]), "Lead\nTail", "the quote child keeps the text");
    let back = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "paragraph" }] }))
        .expect("convert the quote back");
    assert_eq!(summary_text(&back["document"]), "Lead\nTail");

    let code = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "code", "language": "rust" }] }))
        .expect("convert to code");
    assert_eq!(
        code["results"][0],
        json!({ "type": "block.convert", "id": "node-1", "from": "paragraph", "to": "code", "language": "rust" })
    );
    let fenced = workspace.path("converted.md");
    export(handle, "md", &fenced, false).expect("export markdown");
    let text = fs::read_to_string(&fenced).expect("read markdown");
    assert!(text.contains("```rust") && text.contains("Lead"), "the paragraph text became fenced code: {text}");
    let restored = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "paragraph" }] }))
        .expect("convert the code back");
    assert_eq!(summary_text(&restored["document"]), "Lead\nTail");

    let list = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "list", "ordered": true }] }))
        .expect("convert to a list");
    assert_eq!(
        list["results"][0],
        json!({ "type": "block.convert", "id": "node-1", "from": "paragraph", "to": "list", "ordered": true })
    );
    edit(handle, &json!({ "operations": [{ "type": "list.insertItem", "id": "node-1", "item": 1, "text": "Second" }] }))
        .expect("add a second item");
    let split = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "paragraph" }] }))
        .expect("split the list into paragraphs");
    assert_eq!(split["results"][0], json!({ "type": "block.convert", "id": "node-1", "from": "list", "to": "paragraph" }));
    assert_eq!(summary_text(&split["document"]), "Lead\nSecond\nTail", "items become paragraphs in order");

    close(handle).expect("close");
}

#[test]
fn block_convert_rejects_unknown_ids_same_kinds_and_bad_arguments() {
    let created = create(&json!({ "title": "Convert errors" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "paragraph.append", "text": "Body" },
        { "type": "table.append", "rows": [["a", "b"]] },
        { "type": "heading.append", "level": 1, "text": "Head" },
    ] }))
    .expect("seed blocks");
    let before = revision_of(&summary(handle).expect("summary"));

    let missing =
        edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-999", "to": "heading", "level": 1 }] }))
            .expect_err("unknown id");
    assert_eq!(missing.code, "object/not-found");

    let same = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "paragraph" }] }))
        .expect_err("same kind");
    assert_eq!(same.code, "session/argument-invalid");

    let table = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-4", "to": "paragraph" }] }))
        .expect_err("a table is not convertible");
    assert_eq!(table.code, "session/argument-invalid");

    let no_level = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "heading" }] }))
        .expect_err("heading conversion needs a level");
    assert_eq!(no_level.code, "session/argument-invalid");

    let bad_level = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "heading", "level": 7 }] }))
        .expect_err("level out of range");
    assert_eq!(bad_level.code, "session/argument-invalid");

    let bad_kind = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "table" }] }))
        .expect_err("unsupported target kind");
    assert_eq!(bad_kind.code, "session/argument-invalid");

    let stray = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "node-1", "to": "quote", "level": 2 }] }))
        .expect_err("level is only valid for heading targets");
    assert_eq!(stray.code, "session/argument-invalid");

    let bad_pair = edit(handle, &json!({ "operations": [{ "type": "block.convert", "id": "section-5", "to": "code" }] }))
        .expect_err("heading to code is not a supported pair");
    assert_eq!(bad_pair.code, "session/argument-invalid");

    assert_eq!(revision_of(&summary(handle).expect("summary")), before, "failed conversions consume no revision");

    close(handle).expect("close");
}

#[test]
fn block_swap_exchanges_top_level_blocks_with_their_subtrees() {
    let created = create(&json!({ "title": "Swap" })).expect("create");
    let handle = handle_of(&created);
    let seeded = edit(handle, &json!({ "operations": [
        { "type": "heading.append", "level": 1, "text": "Alpha" },
        { "type": "paragraph.append", "text": "Tail" },
        { "type": "heading.insert", "index": 0, "level": 1, "text": "Zeta" },
    ] }))
    .expect("seed two top-level sections");
    let sections = seeded["document"]["sections"].as_array().expect("sections");
    assert_eq!(sections[0]["title"], json!("Zeta"));
    assert_eq!(sections[1]["title"], json!("Alpha"));

    let swapped = edit(handle, &json!({ "operations": [{ "type": "block.swap", "id": "section-3", "target": "section-1" }] }))
        .expect("swap the two sections");
    assert_eq!(swapped["results"][0], json!({ "type": "block.swap", "id": "section-3", "target": "section-1" }));
    assert_eq!(revision_of(&swapped), revision_of(&seeded) + 1);
    let sections = swapped["document"]["sections"].as_array().expect("sections");
    assert_eq!(sections[0]["title"], json!("Alpha"));
    assert_eq!(sections[1]["title"], json!("Zeta"));
    assert_eq!(swapped["document"]["paragraphs"][0]["sectionIndex"], json!(0), "the child travels with its section");

    let back = edit(handle, &json!({ "operations": [{ "type": "block.swap", "id": "section-1", "target": "section-3" }] }))
        .expect("swap back");
    let sections = back["document"]["sections"].as_array().expect("sections");
    assert_eq!(sections[0]["title"], json!("Zeta"));

    close(handle).expect("close");
}

#[test]
fn block_swap_rejects_same_nested_and_unknown_blocks() {
    let created = create(&json!({ "title": "Swap errors" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [
        { "type": "heading.append", "level": 1, "text": "Alpha" },
        { "type": "heading.append", "level": 1, "text": "Beta" },
        { "type": "paragraph.append", "text": "Child" },
    ] }))
    .expect("seed blocks");
    let before = revision_of(&summary(handle).expect("summary"));

    let same = edit(handle, &json!({ "operations": [{ "type": "block.swap", "id": "section-1", "target": "section-1" }] }))
        .expect_err("a block cannot swap with itself");
    assert_eq!(same.code, "session/argument-invalid");

    let missing = edit(handle, &json!({ "operations": [{ "type": "block.swap", "id": "section-1", "target": "section-999" }] }))
        .expect_err("unknown target");
    assert_eq!(missing.code, "object/not-found");

    let nested = edit(handle, &json!({ "operations": [{ "type": "block.swap", "id": "node-3", "target": "section-1" }] }))
        .expect_err("a nested block is not top-level");
    assert_eq!(nested.code, "session/argument-invalid");

    let bad_address = edit(handle, &json!({ "operations": [{ "type": "block.swap", "id": "section-1", "target": "nonsense" }] }))
        .expect_err("the target must be a block address");
    assert_eq!(bad_address.code, "session/argument-invalid");

    assert_eq!(revision_of(&summary(handle).expect("summary")), before, "failed swaps consume no revision");

    close(handle).expect("close");
}
