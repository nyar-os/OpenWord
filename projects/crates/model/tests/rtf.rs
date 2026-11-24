//! RTF import: text subset, inline styles and declared loss.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{capabilities, close, create, edit, export, open, save};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("open-word-rtf-{name}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary workspace");
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn rtf(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, text).expect("write rtf");
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

fn paragraph_texts(document: &Value) -> Vec<String> {
    document["paragraphs"]
        .as_array()
        .expect("paragraphs")
        .iter()
        .filter_map(|paragraph| paragraph["text"].as_str().map(str::to_string))
        .collect()
}

fn loss_codes(value: &Value) -> Vec<String> {
    value["losses"].as_array().expect("losses").iter().filter_map(|loss| loss["code"].as_str().map(str::to_string)).collect()
}

fn collect_styles(value: &Value, styles: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                if key == "Styled"
                    && let Some(style) = child["style"].as_str()
                {
                    styles.push(style.to_string());
                }
                collect_styles(child, styles);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_styles(item, styles);
            }
        }
        _ => {}
    }
}

fn assert_exists(path: &Path) {
    assert!(path.is_file(), "{} should exist", path.display());
}

#[test]
fn capabilities_declare_partial_rtf_import_with_loss_codes() {
    let matrix = capabilities();
    assert_eq!(matrix["formats"]["rtf"]["import"], json!("partial"));
    assert_eq!(matrix["formats"]["rtf"]["edit"], json!("supported"));
    assert_eq!(matrix["formats"]["rtf"]["export"], json!("partial"));
    assert_eq!(matrix["exportTargets"]["rtf"], json!("partial"));
    let codes = matrix["formats"]["rtf"]["lossCodes"].as_array().expect("lossCodes");
    for expected in [
        "import.rtf.table",
        "import.rtf.list",
        "import.rtf.image",
        "import.rtf.field",
        "import.rtf.color",
        "export.rtf.table",
        "export.rtf.image",
        "export.rtf.link",
    ] {
        assert!(codes.contains(&json!(expected)), "missing {expected}: {codes:?}");
    }
}

#[test]
fn imports_paragraphs_breaks_and_character_styles() {
    let workspace = Workspace::new("styles");
    let path = workspace.rtf(
        "styled.rtf",
        r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fnil Arial;}}\pard\plain\b\i Bold\i0\b0  plain\par one\line two\par}",
    );
    let opened = open(&path).expect("open rtf");
    let handle = handle_of(&opened);
    assert_eq!(opened["complete"], json!(false));
    assert_eq!(paragraph_texts(&opened["document"]), vec!["Bold plain".to_string(), "one\ntwo".to_string()]);

    let project = workspace.path("styled.oword");
    save(handle, &project, false).expect("save project");
    assert_exists(&project);
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let mut styles = Vec::new();
    collect_styles(&envelope["document"]["blocks"], &mut styles);
    assert!(styles.contains(&"bold".to_string()), "bold run: {styles:?}");
    assert!(styles.contains(&"italic".to_string()), "italic run: {styles:?}");

    let codes = loss_codes(&opened);
    assert!(codes.contains(&"import.rtf.font".to_string()), "font table loss: {codes:?}");

    close(handle).expect("close");
}

#[test]
fn decodes_escapes_cp1252_and_unicode() {
    let workspace = Workspace::new("escapes");
    let path = workspace.rtf("escapes.rtf", r"{\rtf1\ansi\pard \{braces\} back\\slash \'e9 \u20320?\par}");
    let opened = open(&path).expect("open rtf");
    let handle = handle_of(&opened);
    assert_eq!(paragraph_texts(&opened["document"]), vec!["{braces} back\\slash é 你".to_string()]);
    close(handle).expect("close");
}

#[test]
fn degrades_tables_lists_pictures_fields_and_colors_with_losses() {
    let workspace = Workspace::new("losses");
    let path = workspace.rtf(
        "mixed.rtf",
        r#"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\colortbl;\red0\green0\blue0;}\pard\trowd\cellx1000\cellx2000 Alpha\cell Beta\cell\row{\listtext\bullet }Item\par{\pict\pngblip 00ff}{\field{\*\fldinst HYPERLINK "x"}{\fldrslt link text}}\par}"#,
    );
    let opened = open(&path).expect("open rtf");
    let handle = handle_of(&opened);
    assert_eq!(
        paragraph_texts(&opened["document"]),
        vec!["Alpha\tBeta".to_string(), "Item".to_string(), "link text".to_string()]
    );

    let codes = loss_codes(&opened);
    for expected in ["import.rtf.table", "import.rtf.list", "import.rtf.image", "import.rtf.field", "import.rtf.color"] {
        assert!(codes.contains(&expected.to_string()), "missing {expected}: {codes:?}");
    }
    close(handle).expect("close");
}

#[test]
fn exports_rtf_that_the_reader_round_trips() {
    let workspace = Workspace::new("export");
    let markdown = workspace.path("report.md");
    fs::write(&markdown, "# Title\n\n**Bold** and *italic* text.\n\n| Name | Value |\n| --- | --- |\n| alpha | 1 |\n")
        .expect("write markdown");
    let opened = open(&markdown).expect("open markdown");
    let handle = handle_of(&opened);

    let target = workspace.path("report.rtf");
    let report = export(handle, "rtf", &target, false).expect("export rtf");
    assert_eq!(report["format"], json!("rtf"));
    assert_eq!(report["editable"], json!(true));
    let codes = loss_codes(&report);
    assert!(codes.contains(&"export.rtf.table".to_string()), "table degradation must be reported: {codes:?}");
    assert_exists(&target);
    assert!(fs::read(&target).expect("read rtf").starts_with(b"{\\rtf1"));

    let reopened = open(&target).expect("read the exported rtf back");
    assert_eq!(
        paragraph_texts(&reopened["document"]),
        vec!["Title".to_string(), "Bold and italic text.".to_string(), "Name\tValue".to_string(), "alpha\t1".to_string()]
    );

    let project = workspace.path("roundtrip.oword");
    save(handle_of(&reopened), &project, false).expect("save reopened");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let mut styles = Vec::new();
    collect_styles(&envelope["document"]["blocks"], &mut styles);
    assert!(styles.contains(&"bold".to_string()), "bold must survive: {styles:?}");
    assert!(styles.contains(&"italic".to_string()), "italic must survive: {styles:?}");

    close(handle).expect("close markdown");
    close(handle_of(&reopened)).expect("close rtf");
}

#[test]
fn exports_non_latin_text_as_unicode_escapes() {
    let workspace = Workspace::new("unicode");
    let created = create(&json!({ "title": "Unicode" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "你好 world" }] })).expect("append");

    let target = workspace.path("unicode.rtf");
    export(handle, "rtf", &target, false).expect("export rtf");
    let bytes = fs::read(&target).expect("read rtf");
    let text = String::from_utf8(bytes).expect("rtf is ASCII");
    assert!(text.contains("\\u20320?"), "你 must be a unicode escape: {text}");
    assert!(text.contains("\\u22909?"), "好 must be a unicode escape: {text}");

    let reopened = open(&target).expect("reopen rtf");
    assert_eq!(paragraph_texts(&reopened["document"]), vec!["你好 world".to_string()]);

    close(handle).expect("close");
    close(handle_of(&reopened)).expect("close rtf");
}

#[test]
fn rejects_a_non_rtf_file() {
    let workspace = Workspace::new("invalid");
    let path = workspace.path("notes.rtf");
    fs::write(&path, "not an rtf document").expect("write text");
    let error = open(&path).expect_err("non-rtf input must fail");
    assert_eq!(error.code(), "format/invalid");
}
