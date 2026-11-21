//! Editing, search and plain-text export behaviour of the document model.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{close, create, edit, export, find, open, save, stats, summary};
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
