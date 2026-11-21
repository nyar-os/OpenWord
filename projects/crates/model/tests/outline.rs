//! Nested heading outline projection.

use open_word_document::{close, create, edit, outline};
use serde_json::{Value, json};

fn handle_of(value: &Value) -> u64 {
    value["handle"].as_u64().expect("a handle")
}

#[test]
fn outline_nests_headings_by_level_in_document_order() {
    let created = create(&json!({ "title": "Outline" })).expect("create");
    let handle = handle_of(&created);
    edit(
        handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Overview" },
                { "type": "paragraph.append", "text": "intro" },
                { "type": "heading.append", "level": 2, "text": "Details" },
                { "type": "heading.append", "level": 3, "text": "Deep" },
                { "type": "heading.append", "level": 1, "text": "Appendix" },
                { "type": "heading.append", "level": 2, "text": "" },
            ],
        }),
    )
    .expect("seed headings");

    let report = outline(handle).expect("outline");
    assert_eq!(report["revision"], json!(6));
    let items = report["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "two top-level headings: {report}");
    assert_eq!(items[0]["title"], json!("Overview"));
    assert_eq!(items[0]["level"], json!(1));
    assert_eq!(items[0]["blockIndex"], json!(0));
    assert!(items[0]["id"].as_str().unwrap_or_default().starts_with("section-"));

    let children = items[0]["children"].as_array().expect("children");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["title"], json!("Details"));
    assert_eq!(children[0]["blockIndex"], json!(2));
    let grandchildren = children[0]["children"].as_array().expect("grandchildren");
    assert_eq!(grandchildren.len(), 1);
    assert_eq!(grandchildren[0]["title"], json!("Deep"));
    assert_eq!(grandchildren[0]["blockIndex"], json!(3));
    assert_eq!(grandchildren[0]["children"], json!([]));

    assert_eq!(items[1]["title"], json!("Appendix"));
    assert_eq!(items[1]["blockIndex"], json!(4));
    let appendix = items[1]["children"].as_array().expect("appendix children");
    assert_eq!(appendix.len(), 1);
    assert_eq!(appendix[0]["title"], Value::Null, "an empty heading keeps a null title");

    close(handle).expect("close");
}

#[test]
fn outline_of_an_empty_document_is_empty_and_unknown_handles_fail() {
    let created = create(&json!({ "title": "Empty" })).expect("create");
    let handle = handle_of(&created);
    let report = outline(handle).expect("outline");
    assert_eq!(report["items"], json!([]));
    assert_eq!(report["revision"], json!(0));

    let error = outline(999_999).expect_err("unknown handle must fail");
    assert_eq!(error.code, "document/not-found");

    close(handle).expect("close");
}
