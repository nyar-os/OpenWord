//! RTF import: text subset, inline styles and declared loss.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{capabilities, close, open, save};
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
    assert_eq!(matrix["formats"]["rtf"]["export"], json!("unsupported"));
    let codes = matrix["formats"]["rtf"]["lossCodes"].as_array().expect("lossCodes");
    for expected in ["import.rtf.table", "import.rtf.list", "import.rtf.image", "import.rtf.field", "import.rtf.color"] {
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
fn rejects_a_non_rtf_file() {
    let workspace = Workspace::new("invalid");
    let path = workspace.path("notes.rtf");
    fs::write(&path, "not an rtf document").expect("write text");
    let error = open(&path).expect_err("non-rtf input must fail");
    assert_eq!(error.code, "format/invalid");
}
