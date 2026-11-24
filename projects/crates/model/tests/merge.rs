//! Document merge: appended structure, one revision bump and routed losses.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use open_word_document::{close, create, edit, merge, open, save, summary};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("open-word-merge-{name}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary workspace");
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, text).expect("write fixture");
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

fn texts(value: &Value) -> Vec<String> {
    value["paragraphs"]
        .as_array()
        .expect("paragraphs")
        .iter()
        .filter_map(|paragraph| paragraph["text"].as_str().map(str::to_string))
        .collect()
}

fn loss_codes(value: &Value) -> Vec<String> {
    value["losses"].as_array().expect("losses").iter().filter_map(|loss| loss["code"].as_str().map(str::to_string)).collect()
}

const DONOR: &str = "# Alpha

alpha paragraph

## Beta

beta paragraph
";

#[test]
fn merge_appends_sections_and_bumps_the_revision_once() {
    let workspace = Workspace::new("sections");
    let donor_path = workspace.path("donor.oword");
    let donor = create(&json!({ "title": "Donor" })).expect("create donor");
    let donor_handle = handle_of(&donor);
    edit(
        donor_handle,
        &json!({
            "operations": [
                { "type": "heading.append", "level": 1, "text": "Alpha" },
                { "type": "paragraph.append", "text": "alpha paragraph" },
                { "type": "heading.append", "level": 2, "text": "Beta" },
                { "type": "paragraph.append", "text": "beta paragraph" },
            ],
        }),
    )
    .expect("seed donor");
    save(donor_handle, &donor_path, false).expect("save donor");
    close(donor_handle).expect("close donor");

    let created = create(&json!({ "title": "Target" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "target paragraph" }] })).expect("seed target");

    let report = merge(handle, &donor_path, None).expect("merge project");
    assert_eq!(report["revision"], json!(2), "the merge bumps the revision once");
    assert!(report["blocksAdded"].as_u64().unwrap() >= 4, "sections and paragraphs are copied: {report}");
    assert_eq!(texts(&report["summary"]), vec!["target paragraph", "alpha paragraph", "beta paragraph"]);

    let sections = report["summary"]["sections"].as_array().expect("sections");
    assert_eq!(sections.len(), 2, "the donor keeps its own section structure");
    assert_eq!(sections[0]["title"], json!("Alpha"));
    assert_eq!(sections[1]["title"], json!("Beta"));
    let paragraphs = report["summary"]["paragraphs"].as_array().unwrap();
    assert_eq!(paragraphs[1]["sectionIndex"], json!(0));
    assert_eq!(paragraphs[2]["sectionIndex"], json!(1));

    let document = summary(handle).expect("summary");
    assert_eq!(texts(&document), vec!["target paragraph", "alpha paragraph", "beta paragraph"]);
    close(handle).expect("close");
}

#[test]
fn merge_reassigns_node_ids_and_carries_import_losses() {
    let workspace = Workspace::new("ids");
    let donor =
        workspace.write("donor.rtf", r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}\pard\trowd\cellx1000 Alpha\cell Beta\cell\row\par}");
    let created = create(&json!({ "title": "Target" })).expect("create");
    let handle = handle_of(&created);
    edit(handle, &json!({ "operations": [{ "type": "paragraph.append", "text": "target" }] })).expect("seed target");

    let report = merge(handle, &donor, Some("append")).expect("merge rtf");
    assert_eq!(report["revision"], json!(2));
    let codes = loss_codes(&report);
    assert!(codes.contains(&"import.rtf.table".to_string()), "donor losses must survive: {codes:?}");
    assert_eq!(texts(&report["summary"]), vec!["target", "Alpha\tBeta"]);

    // The copied ids are fresh, so removing the merged block cannot touch the target.
    let project = workspace.path("merged.oword");
    save(handle, &project, false).expect("save project");
    let envelope: Value = serde_json::from_slice(&fs::read(&project).expect("read project")).expect("parse project");
    let ids = envelope["document"]["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .map(|node| node["id"].as_u64().expect("node id"))
        .collect::<Vec<_>>();
    let unique = ids.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), unique.len(), "node ids must stay unique: {ids:?}");

    close(handle).expect("close");
}

#[test]
fn merge_accepts_its_own_source_path() {
    let workspace = Workspace::new("self");
    let path = workspace.write("notes.md", "# Note\n\nbody\n");
    let opened = open(&path).expect("open markdown");
    let handle = handle_of(&opened);
    let before = opened["document"]["paragraphs"].as_array().unwrap().len();

    let report = merge(handle, &path, None).expect("merge the same file");
    let paragraphs = report["summary"]["paragraphs"].as_array().unwrap().len();
    assert_eq!(paragraphs, before * 2);
    assert_eq!(report["summary"]["sections"].as_array().unwrap().len(), 2);

    close(handle).expect("close");
}

#[test]
fn merge_rejects_unknown_modes_formats_and_handles() {
    let workspace = Workspace::new("reject");
    let donor = workspace.write("donor.md", DONOR);
    let unknown = workspace.write("donor.xyz", "nothing");

    let created = create(&json!({ "title": "Target" })).expect("create");
    let handle = handle_of(&created);
    let before = summary(handle).expect("summary");

    let error = merge(handle, &donor, Some("prepend")).expect_err("only append is supported");
    assert_eq!(error.code(), "object/unsupported");
    let error = merge(handle, &unknown, None).expect_err("unknown formats are rejected");
    assert_eq!(error.code(), "format/unsupported");
    assert_eq!(summary(handle).expect("summary"), before);

    let error = merge(999_999, &donor, None).expect_err("unknown handles are rejected");
    assert_eq!(error.code(), "document/not-found");

    close(handle).expect("close");
}
