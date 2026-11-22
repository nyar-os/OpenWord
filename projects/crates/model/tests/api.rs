//! API smoke tests for the `open-word-document` facade.

use std::path::Path;

use open_word_document::*;
use serde_json::json;

#[test]
fn capability_matrix_reflects_wired_routes() {
    let matrix = capabilities();
    assert_eq!(matrix["engine"]["package"], json!("open-word-document"));
    assert_eq!(matrix["project"], json!("supported"));
    assert_eq!(matrix["formats"]["docx"]["import"], json!("supported"));
    assert_eq!(matrix["formats"]["docx"]["export"], json!("supported"));
    assert_eq!(matrix["formats"]["pdf"]["import"], json!("partial"));
    assert_eq!(matrix["formats"]["doc"]["import"], json!("unsupported"));
    assert_eq!(matrix["formats"]["txt"]["export"], json!("supported"));
    assert_eq!(matrix["checks"]["sections"], json!("supported"));
    assert_eq!(matrix["checks"]["fonts"], json!("supported"));
    assert_eq!(matrix["exportTargets"]["pdf"], json!("supported"));
    assert_eq!(matrix["exportTargets"]["txt"], json!("supported"));
    assert_eq!(matrix["exportTargets"]["dotx"], json!("unsupported"));
    assert_eq!(matrix["renderers"]["svg"], json!("unsupported"));
}

#[test]
fn unknown_handle_is_a_stable_failure() {
    for error in [summary(999_999), inspect(999_999, None), edit(999_999, &json!({ "operations": [] }))] {
        assert_eq!(error.unwrap_err().code, "document/not-found");
    }
    let error = find(999_999, "text", None).unwrap_err();
    assert_eq!(error.code, "document/not-found");
    let error = find(999_999, "", None).unwrap_err();
    assert_eq!(error.code, "session/argument-invalid");
    let error = save(999_999, Path::new("report.oword"), true).unwrap_err();
    assert_eq!(error.code, "document/not-found");
    export(999_999, "docx", Path::new("report.docx"), true).unwrap_err();
}

#[test]
fn close_is_idempotent() {
    let created = create(&json!({ "title": "Idempotent close" })).unwrap();
    let handle = created["handle"].as_u64().unwrap();
    close(handle).unwrap();
    close(handle).unwrap();
}
