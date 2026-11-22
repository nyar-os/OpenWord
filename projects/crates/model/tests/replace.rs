//! Text replacement across titles, paragraphs, list items and table cells.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{close, create, edit, open, replace, summary, undo};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("open-word-replace-{name}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary workspace");
        Self { root }
    }

    fn markdown(&self, name: &str, text: &str) -> PathBuf {
        let path = self.root.join(name);
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

fn revision_of(value: &Value) -> u64 {
    value["revision"].as_u64().expect("a revision")
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

fn seed(handle: u64) {
    edit(
        handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Alpha" },
                { "type": "paragraph.append", "text": "Alpha beta alpha." },
                { "type": "list.append", "items": ["alpha"] },
                { "type": "table.append", "rows": [["Alpha", "1"]] },
            ],
        }),
    )
    .expect("seed document");
}

#[test]
fn replace_is_case_sensitive_by_default_and_reports_affected_blocks() {
    let created = create(&json!({ "title": "Replace" })).expect("create");
    let handle = handle_of(&created);
    seed(handle);

    let report = replace(handle, &json!({ "find": "Alpha", "replace": "Omega" })).expect("replace");
    assert_eq!(report["replaced"], json!(3));
    assert_eq!(revision_of(&report), 5);
    assert_eq!(report["document"]["sections"][0]["title"], json!("Omega"));
    let text = summary_text(&report["document"]);
    assert!(text.contains("Omega beta alpha."), "case-sensitive pass: {text}");
    assert!(text.contains("Omega"), "table cell: {text}");

    let affected = report["affected"].as_array().expect("affected blocks");
    assert_eq!(affected.len(), 3);
    assert_eq!(affected[0]["blockId"], report["document"]["sections"][0]["id"]);
    assert_eq!(affected[0]["replaced"], json!(1));
    assert_eq!(affected[1]["blockIndex"], json!(1));
    assert_eq!(affected[2]["replaced"], json!(1));

    let undone = undo(handle, None).expect("undo the replacement");
    assert!(summary_text(&undone["document"]).starts_with("Alpha beta alpha."));
    assert_eq!(undone["document"]["sections"][0]["title"], json!("Alpha"));

    close(handle).expect("close");
}

#[test]
fn case_insensitive_and_whole_word_match_the_requested_spans() {
    let created = create(&json!({ "title": "Options" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "alpha alphabet alpha beta" }] }))
        .expect("append paragraph");

    let insensitive = replace(handle, &json!({ "find": "alpha", "replace": "x", "caseSensitive": false }))
        .expect("case-insensitive replace");
    assert_eq!(insensitive["replaced"], json!(3));
    assert_eq!(summary_text(&insensitive["document"]), "x xbet x beta");

    undo(handle, None).expect("restore");
    let whole = replace(
        handle,
        &json!({ "find": "alpha", "replace": "x", "caseSensitive": false, "wholeWord": true }),
    )
    .expect("whole-word replace");
    assert_eq!(whole["replaced"], json!(2));
    assert_eq!(summary_text(&whole["document"]), "x alphabet x beta");

    undo(handle, None).expect("restore again");
    let limited =
        replace(handle, &json!({ "find": "alpha", "replace": "x", "caseSensitive": false, "limit": 1 }))
            .expect("limited replace");
    assert_eq!(limited["replaced"], json!(1));
    assert_eq!(summary_text(&limited["document"]), "x alphabet alpha beta");

    close(handle).expect("close");
}

#[test]
fn case_insensitive_matching_handles_non_ascii_case() {
    let created = create(&json!({ "title": "Unicode" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "CAFÉ café" }] }))
        .expect("append paragraph");

    let report = replace(handle, &json!({ "find": "café", "replace": "tea", "caseSensitive": false }))
        .expect("case-insensitive unicode replace");
    assert_eq!(report["replaced"], json!(2));
    assert_eq!(summary_text(&report["document"]), "tea tea");

    close(handle).expect("close");
}

#[test]
fn regular_expressions_replace_capture_groups_and_follow_case_sensitivity() {
    let created = create(&json!({ "title": "Regex" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "Order 12, order 34." }] }))
        .expect("append paragraph");

    let report = replace(handle, &json!({ "find": "(?i)order\\s+(\\d+)", "replace": "item $1", "regex": true }))
        .expect("regex replace");
    assert_eq!(report["replaced"], json!(2));
    assert_eq!(report["affected"].as_array().expect("affected").len(), 1);
    assert_eq!(summary_text(&report["document"]), "item 12, item 34.");

    undo(handle, None).expect("restore");
    let insensitive =
        replace(handle, &json!({ "find": "ORDER", "replace": "line", "regex": true, "caseSensitive": false }))
            .expect("case-insensitive pattern");
    assert_eq!(insensitive["replaced"], json!(2));
    assert_eq!(summary_text(&insensitive["document"]), "line 12, line 34.");

    undo(handle, None).expect("restore again");
    let limited = replace(handle, &json!({ "find": "\\d+", "replace": "n", "regex": true, "limit": 1 }))
        .expect("limited pattern");
    assert_eq!(limited["replaced"], json!(1));
    assert_eq!(summary_text(&limited["document"]), "Order n, order 34.");

    close(handle).expect("close");
}

#[test]
fn invalid_requests_and_stale_revisions_leave_the_document_untouched() {
    let workspace = Workspace::new("bounds");
    let opened = open(&workspace.markdown("report.md", "Alpha alpha\n")).expect("open");
    let handle = handle_of(&opened);
    let before = summary(handle).expect("summary");
    assert_eq!(revision_of(&before), 0);

    let missing = replace(999_999, &json!({ "find": "a", "replace": "b" })).expect_err("unknown handle");
    assert_eq!(missing.code, "document/not-found");

    for request in [
        json!({ "find": "", "replace": "x" }),
        json!({ "find": 3, "replace": "x" }),
        json!({ "replace": "x" }),
        json!({ "find": "a" }),
        json!({ "find": "a", "replace": 3 }),
        json!({ "find": "a", "replace": "x", "caseSensitive": "yes" }),
        json!({ "find": "a", "replace": "x", "wholeWord": 1 }),
        json!({ "find": "a", "replace": "x", "regex": "yes" }),
        json!({ "find": "(", "replace": "x", "regex": true }),
        json!({ "find": "a", "replace": "x", "regex": true, "wholeWord": true }),
        json!({ "find": "a", "replace": "x", "limit": 0 }),
        json!({ "find": "a", "replace": "x", "limit": "2" }),
        json!({ "find": "a", "replace": "x", "expectedRevision": -1 }),
    ] {
        let error = replace(handle, &request).expect_err("invalid request must fail");
        assert_eq!(error.code, "session/argument-invalid", "{request}");
    }
    assert_eq!(summary(handle).expect("summary"), before);

    let stale = replace(handle, &json!({ "find": "alpha", "replace": "x", "expectedRevision": 7 }))
        .expect_err("stale revision must fail");
    assert_eq!(stale.code, "document/revision-conflict");
    assert_eq!(summary(handle).expect("summary"), before);

    let absent = replace(handle, &json!({ "find": "absent", "replace": "x" })).expect("no match");
    assert_eq!(absent["replaced"], json!(0));
    assert_eq!(absent["affected"], json!([]));
    assert_eq!(revision_of(&absent), 0);
    let error = undo(handle, None).expect_err("a no-op replacement records no history");
    assert_eq!(error.code, "session/argument-invalid");

    close(handle).expect("close");
}
