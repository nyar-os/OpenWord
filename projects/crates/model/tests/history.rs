//! Undo and redo: multi-step history, redo invalidation and rejected steps.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{close, create, edit, open, redo, summary, undo};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("open-word-history-{name}-{}-{unique}", std::process::id()));
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

fn append(handle: u64, text: &str) {
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": text }] })).expect("append");
}

#[test]
fn undo_and_redo_walk_multiple_steps() {
    let created = create(&json!({ "title": "History" })).expect("create");
    let handle = handle_of(&created);
    append(handle, "one");
    append(handle, "two");
    append(handle, "three");
    assert_eq!(summary_text(&summary(handle).expect("summary")), "one\ntwo\nthree");

    let undone = undo(handle, Some(2)).expect("undo two steps");
    assert_eq!(undone["undone"], json!(2));
    assert_eq!(undone["redoAvailable"], json!(true));
    assert_eq!(undone["revision"], json!(4));
    assert_eq!(summary_text(&undone["document"]), "one");

    let redone = redo(handle, None).expect("redo the default step");
    assert_eq!(redone["redone"], json!(1));
    assert_eq!(redone["undoAvailable"], json!(true));
    assert_eq!(redone["revision"], json!(5));
    assert_eq!(summary_text(&redone["document"]), "one\ntwo");

    let redone = redo(handle, Some(1)).expect("redo the last step");
    assert_eq!(summary_text(&redone["document"]), "one\ntwo\nthree");
    assert_eq!(redone["revision"], json!(6));

    // A new edit after an undo discards the redo path.
    undo(handle, None).expect("undo one step");
    append(handle, "four");
    let error = redo(handle, None).expect_err("redo must be gone after a new edit");
    assert_eq!(error.code(), "session/argument-invalid");
    assert_eq!(summary_text(&summary(handle).expect("summary")), "one\ntwo\nfour");

    close(handle).expect("close");
}

#[test]
fn out_of_range_steps_leave_the_document_untouched() {
    let created = create(&json!({ "title": "Bounds" })).expect("create");
    let handle = handle_of(&created);
    append(handle, "only");
    let before = summary(handle).expect("summary");

    for steps in [Some(2), Some(0)] {
        let error = undo(handle, steps).expect_err("an unavailable undo must fail");
        assert_eq!(error.code(), "session/argument-invalid");
        assert_eq!(summary(handle).expect("summary"), before);
    }
    let error = redo(handle, None).expect_err("nothing has been undone");
    assert_eq!(error.code(), "session/argument-invalid");
    assert_eq!(summary(handle).expect("summary"), before);

    let undone = undo(handle, None).expect("default step");
    assert_eq!(undone["undone"], json!(1));
    assert_eq!(summary(handle).expect("summary")["paragraphs"].as_array().unwrap().len(), 0);

    let error = undo(handle, Some(2)).expect_err("only one snapshot was left");
    assert_eq!(error.code(), "session/argument-invalid");
    assert_eq!(summary(handle).expect("summary")["paragraphs"].as_array().unwrap().len(), 0);

    close(handle).expect("close");
}

#[test]
fn opened_documents_start_with_an_empty_history() {
    let workspace = Workspace::new("opened");
    let path = workspace.path("notes.md");
    fs::write(&path, "first\n").expect("write markdown");

    let opened = open(&path).expect("open markdown");
    let handle = handle_of(&opened);
    let error = undo(handle, None).expect_err("an opened document has no undo history");
    assert_eq!(error.code(), "session/argument-invalid");
    let error = redo(handle, None).expect_err("an opened document has no redo history");
    assert_eq!(error.code(), "session/argument-invalid");

    close(handle).expect("close");
}

#[test]
fn history_keeps_only_the_fifty_most_recent_mutations() {
    let created = create(&json!({ "title": "Depth" })).expect("create");
    let handle = handle_of(&created);
    for index in 0..51 {
        append(handle, &format!("paragraph {index}"));
    }

    let error = undo(handle, Some(51)).expect_err("the oldest snapshot was dropped");
    assert_eq!(error.code(), "session/argument-invalid");

    let undone = undo(handle, Some(50)).expect("the whole retained history");
    assert_eq!(undone["undone"], json!(50));
    let paragraphs = undone["document"]["paragraphs"].as_array().unwrap();
    assert_eq!(paragraphs.len(), 1, "only the first append remains");
    assert_eq!(paragraphs[0]["text"], json!("paragraph 0"));

    close(handle).expect("close");
}
