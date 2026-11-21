//! Structural inspection, including the font declaration check.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{close, inspect, open};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("open-word-inspect-{name}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary workspace");
        Self { root }
    }

    fn project(&self, name: &str, blocks: Value) -> PathBuf {
        let path = self.root.join(name);
        let envelope = json!({
            "schema_version": "document.ir/v0",
            "document": {
                "id": 1,
                "revision": 0,
                "metadata": { "title": "Fonts", "language": null, "authors": [], "tags": [] },
                "blocks": blocks,
                "relations": [],
                "assets": [],
                "sources": [],
                "coverage": { "complete": true, "loss": [] },
            },
        });
        fs::write(&path, envelope.to_string()).expect("write project");
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

fn paragraph(id: u64, styles: &[(&str, &str)]) -> Value {
    let content = styles
        .iter()
        .map(|(style, text)| {
            json!({
                "Styled": {
                    "style": style,
                    "children": [{ "Text": { "text": text } }],
                },
            })
        })
        .collect::<Vec<_>>();
    json!({ "id": id, "block": { "Paragraph": { "content": content } } })
}

fn issue_codes(report: &Value) -> Vec<String> {
    report["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .map(|issue| issue["code"].as_str().expect("code").to_string())
        .collect()
}

#[test]
fn fonts_check_reports_empty_family_and_invalid_size() {
    let workspace = Workspace::new("invalid");
    let path = workspace.project(
        "fonts.oword",
        json!([
            paragraph(1, &[("font-family:", "no family"), ("font-size:wide", "bad size")]),
            paragraph(2, &[("font-family:Calibri", "ok"), ("font-size:11", "ok")]),
        ]),
    );
    let opened = open(&path).expect("open fixture");
    let handle = handle_of(&opened);

    let report = inspect(handle, Some(vec!["fonts".to_string()])).expect("inspect fonts");
    assert_eq!(report["passed"], json!(false));
    assert_eq!(issue_codes(&report), vec!["font/empty-family", "font/invalid-size"]);
    assert_eq!(report["issues"][0]["objectId"], json!("node-1"));
    assert_eq!(report["issues"][1]["objectId"], json!("node-1"));
    assert_eq!(report["issues"][0]["severity"], json!("error"));

    let defaulted = inspect(handle, None).expect("inspect all checks");
    assert_eq!(defaulted["passed"], json!(false));
    let defaulted_codes = issue_codes(&defaulted);
    assert!(defaulted_codes.contains(&"font/empty-family".to_string()), "{defaulted_codes:?}");
    assert!(defaulted_codes.contains(&"font/invalid-size".to_string()), "{defaulted_codes:?}");

    close(handle).expect("close");
}

#[test]
fn fonts_check_accepts_valid_declarations_and_declares_support() {
    let workspace = Workspace::new("valid");
    let path = workspace.project(
        "fonts.oword",
        json!([paragraph(1, &[("font-family:Calibri", "ok"), ("font-size:11.5", "ok"), ("bold", "ok")])]),
    );
    let opened = open(&path).expect("open fixture");
    let handle = handle_of(&opened);

    let report = inspect(handle, Some(vec!["fonts".to_string()])).expect("inspect fonts");
    assert_eq!(report["passed"], json!(true));
    assert_eq!(report["issues"], json!([]));

    let capabilities = open_word_document::capabilities();
    assert_eq!(capabilities["checks"]["fonts"], json!("supported"));

    close(handle).expect("close");
}

#[test]
fn fonts_check_scopes_to_the_requested_check() {
    let workspace = Workspace::new("scope");
    let path = workspace.project("fonts.oword", json!([paragraph(1, &[("font-family:", "no family")])]));
    let opened = open(&path).expect("open fixture");
    let handle = handle_of(&opened);

    let sections = inspect(handle, Some(vec!["sections".to_string()])).expect("inspect sections");
    assert_eq!(sections["issues"], json!([]));

    let unknown = inspect(handle, Some(vec!["palette".to_string()])).expect("inspect unknown");
    assert_eq!(issue_codes(&unknown), vec!["check/unsupported"]);

    close(handle).expect("close");
}
