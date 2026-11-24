//! Thin Node-API surface for OpenWord.
//!
//! Every export performs one orthogonal operation and returns a JSON string, so
//! the TypeScript layer owns argument parsing, help text and exit codes.
//! Document semantics stay in the shared engine; this layer only forwards the
//! operations and hosts the WebView window. Both surfaces share the same live
//! handle table in `open-word-document`, so a document opened from the CLI and
//! one opened from the studio observe the same revisions.

#![deny(missing_docs)]

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use serde_json::{Value, json};
use wae::{
    CallContext, Error, JsValue, Result, function, optional_string_prop, register_module,
    webview::{IpcHandler, IpcOutcome, IpcSink, OpenOptions, WebViewError, WebViewHost},
};

fn failure_of(error: open_word_document::ServiceError) -> Error {
    let mut mapped = Error::new(error.message().to_owned()).code(error.code().to_owned());
    if !error.params().is_null() {
        mapped = mapped.detail(json!({ "params": error.params() }));
    }
    mapped
}

fn argument(ctx: &CallContext, index: usize) -> Result<JsValue> {
    ctx.args().get(index).copied().ok_or_else(|| Error::new("a required argument is missing").code("session/argument-invalid"))
}

fn handle(ctx: &CallContext, index: usize) -> Result<u64> {
    argument(ctx, index)?
        .as_i64()?
        .try_into()
        .map_err(|_| Error::new("`handle` must be a non-negative integer").code("session/argument-invalid"))
}

fn text(ctx: &CallContext, index: usize) -> Result<String> {
    argument(ctx, index)?.as_string()
}

fn optional_steps(ctx: &CallContext, index: usize) -> Result<Option<u32>> {
    match ctx.args().get(index).copied() {
        Some(value) if !value.is_undefined()? => {
            let raw = value.as_i64()?;
            u32::try_from(raw).map(Some).map_err(|_| Error::new("`steps` must be a positive integer").code("session/argument-invalid"))
        }
        _ => Ok(None),
    }
}

fn respond(ctx: &CallContext, value: &Value) -> Result<JsValue> {
    let text = value.to_string();
    ctx.env().string(&text)
}

/// Library version of the native binding.
#[function]
fn version(ctx: CallContext) -> Result<JsValue> {
    ctx.env().string(env!("CARGO_PKG_VERSION"))
}

/// Reports the declared format, check, renderer and export matrix.
#[function]
fn capabilities(ctx: CallContext) -> Result<JsValue> {
    let matrix = open_word_document::capabilities().to_string();
    ctx.env().string(&matrix)
}

/// Creates a document from a JSON request.
#[function]
fn document_create(ctx: CallContext) -> Result<JsValue> {
    let raw = text(&ctx, 0)?;
    let request: Value =
        serde_json::from_str(&raw).map_err(|error| Error::new(error.to_string()).code("session/argument-invalid"))?;
    respond(&ctx, &open_word_document::create(&request).map_err(failure_of)?)
}

/// Opens a document at a path.
#[function]
fn document_open(ctx: CallContext) -> Result<JsValue> {
    let path = text(&ctx, 0)?;
    respond(&ctx, &open_word_document::open(Path::new(&path)).map_err(failure_of)?)
}

/// Writes one document identified by a handle to a path.
#[function]
fn document_save(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let path = text(&ctx, 1)?;
    let overwrite = ctx.args().get(2).map(|value| value.as_bool()).transpose()?.unwrap_or(false);
    respond(&ctx, &open_word_document::save(handle, Path::new(&path), overwrite).map_err(failure_of)?)
}

/// Runs the declared rule checks against one document handle.
#[function]
fn document_inspect(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let checks = match ctx.args().get(1) {
        Some(value) if !value.is_undefined()? => {
            let raw = value.as_string()?;
            let names: Vec<String> =
                serde_json::from_str(&raw).map_err(|error| Error::new(error.to_string()).code("session/argument-invalid"))?;
            Some(names)
        }
        _ => None,
    };
    respond(&ctx, &open_word_document::inspect(handle, checks).map_err(failure_of)?)
}

/// Returns the committed summary of one document handle.
#[function]
fn document_summary(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    respond(&ctx, &open_word_document::summary(handle).map_err(failure_of)?)
}

/// Returns the nested heading outline of one document handle.
#[function]
fn document_outline(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    respond(&ctx, &open_word_document::outline(handle).map_err(failure_of)?)
}

/// Returns block, word and reading-time statistics for one document handle.
#[function]
fn word_stats(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    respond(&ctx, &open_word_document::stats(handle).map_err(failure_of)?)
}

/// Searches one document handle for text and returns the matches.
#[function]
fn document_find(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let query = text(&ctx, 1)?;
    let limit = match ctx.args().get(2) {
        Some(value) if !value.is_undefined()? => {
            let raw = value.as_i64()?;
            Some(
                u32::try_from(raw)
                    .map_err(|_| Error::new("`limit` must be a non-negative integer").code("session/argument-invalid"))?,
            )
        }
        _ => None,
    };
    respond(&ctx, &open_word_document::find(handle, &query, limit).map_err(failure_of)?)
}

/// Replaces text in one document handle with an optional case or whole-word rule.
#[function]
fn document_replace(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let raw = text(&ctx, 1)?;
    let request: Value =
        serde_json::from_str(&raw).map_err(|error| Error::new(error.to_string()).code("session/argument-invalid"))?;
    respond(&ctx, &open_word_document::replace(handle, &request).map_err(failure_of)?)
}

/// Applies one JSON edit request to a document handle.
#[function]
fn document_edit(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let raw = text(&ctx, 1)?;
    let request: Value =
        serde_json::from_str(&raw).map_err(|error| Error::new(error.to_string()).code("session/argument-invalid"))?;
    respond(&ctx, &open_word_document::edit(handle, &request).map_err(failure_of)?)
}

/// Undoes committed edits on a document handle.
#[function]
fn document_undo(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let steps = optional_steps(&ctx, 1)?;
    respond(&ctx, &open_word_document::undo(handle, steps).map_err(failure_of)?)
}

/// Replays undone edits on a document handle.
#[function]
fn document_redo(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let steps = optional_steps(&ctx, 1)?;
    respond(&ctx, &open_word_document::redo(handle, steps).map_err(failure_of)?)
}

/// Appends another document's blocks to a document handle.
#[function]
fn document_merge(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let path = text(&ctx, 1)?;
    let mode = match ctx.args().get(2).copied() {
        Some(value) if !value.is_undefined()? => Some(value.as_string()?),
        _ => None,
    };
    respond(&ctx, &open_word_document::merge(handle, Path::new(&path), mode.as_deref()).map_err(failure_of)?)
}

/// Writes one export target from a document handle.
#[function]
fn document_export(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    let format = text(&ctx, 1)?;
    let target = text(&ctx, 2)?;
    let overwrite = ctx.args().get(3).map(|value| value.as_bool()).transpose()?.unwrap_or(true);
    respond(&ctx, &open_word_document::export(handle, &format, Path::new(&target), overwrite).map_err(failure_of)?)
}

/// Releases one document handle.
#[function]
fn document_close(ctx: CallContext) -> Result<JsValue> {
    let handle = handle(&ctx, 0)?;
    open_word_document::close(handle).map_err(failure_of)?;
    ctx.env().boolean(true)
}

// One WebView host per process, owned by the thread that opened it.
thread_local! {
    static GUI: RefCell<Option<GuiState>> = const { RefCell::new(None) };
}

const GUI_CLOSED: &str = "gui_closed";
const GUI_HOST_EXITED: &str = "gui_host_exited";
const GUI_NOT_OPEN: &str = "gui_not_open";

struct GuiState {
    host: WebViewHost,
    backend: &'static str,
    opened_at: Instant,
    /// Set when the page asks the host to close, so a dead host is
    /// distinguishable from one that never processed the request.
    close_requested: Arc<AtomicBool>,
}

struct WordIpcHandler {
    close_requested: Arc<AtomicBool>,
}

impl IpcHandler for WordIpcHandler {
    fn on_ipc(&mut self, message: &str, _sink: &IpcSink) -> IpcOutcome {
        let Ok(value) = serde_json::from_str::<Value>(message)
        else {
            return IpcOutcome::Ignored;
        };
        let ty = value.get("type").and_then(Value::as_str).unwrap_or("");
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        match ty {
            "close" => {
                self.close_requested.store(true, Ordering::SeqCst);
                IpcOutcome::CloseWindow
            }
            "ready" => reply(id, "readyResult", json!({ "version": env!("CARGO_PKG_VERSION"), "platform": platform_name() })),
            "capabilities" => reply(id, "capabilitiesResult", json!({ "capabilities": open_word_document::capabilities() })),
            "document.create" => {
                let request = value.get("request").cloned().unwrap_or(Value::Null);
                document_outcome(id, "createResult", open_word_document::create(&request))
            }
            "document.open" => match string_field(&value, "path") {
                Some(path) => document_outcome(id, "openResult", open_word_document::open(Path::new(&path))),
                None => failure(id, "documentError", argument_failure("`path` is required")),
            },
            "document.save" => match (handle_field(&value), string_field(&value, "path")) {
                (Some(handle), Some(path)) => {
                    let overwrite = value.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
                    document_outcome(id, "saveResult", open_word_document::save(handle, Path::new(&path), overwrite))
                }
                _ => failure(id, "documentError", argument_failure("`handle` and `path` are required")),
            },
            "document.inspect" => match handle_field(&value) {
                Some(handle) => {
                    let checks = value
                        .get("checks")
                        .and_then(Value::as_array)
                        .map(|names| names.iter().filter_map(Value::as_str).map(str::to_string).collect());
                    document_outcome(id, "inspectResult", open_word_document::inspect(handle, checks))
                }
                None => failure(id, "documentError", argument_failure("`handle` is required")),
            },
            "document.summary" => match handle_field(&value) {
                Some(handle) => document_outcome(id, "summaryResult", open_word_document::summary(handle)),
                None => failure(id, "documentError", argument_failure("`handle` is required")),
            },
            "document.find" => match (handle_field(&value), string_field(&value, "query")) {
                (Some(handle), Some(query)) => {
                    let limit = value.get("limit").and_then(Value::as_u64).and_then(|value| u32::try_from(value).ok());
                    document_outcome(id, "findResult", open_word_document::find(handle, &query, limit))
                }
                _ => failure(id, "documentError", argument_failure("`handle` and `query` are required")),
            },
            "document.replace" => match handle_field(&value) {
                Some(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    document_outcome(id, "replaceResult", open_word_document::replace(handle, &request))
                }
                None => failure(id, "documentError", argument_failure("`handle` is required")),
            },
            "document.edit" => match handle_field(&value) {
                Some(handle) => {
                    let request = value.get("request").cloned().unwrap_or(Value::Null);
                    document_outcome(id, "editResult", open_word_document::edit(handle, &request))
                }
                None => failure(id, "documentError", argument_failure("`handle` is required")),
            },
            "document.undo" | "document.redo" => match handle_field(&value) {
                Some(handle) => {
                    let steps = match optional_steps_field(&value) {
                        Ok(steps) => steps,
                        Err(message) => return failure(id, "documentError", message),
                    };
                    let result = if ty == "document.undo" {
                        open_word_document::undo(handle, steps)
                    }
                    else {
                        open_word_document::redo(handle, steps)
                    };
                    let kind = if ty == "document.undo" { "undoResult" } else { "redoResult" };
                    document_outcome(id, kind, result)
                }
                None => failure(id, "documentError", argument_failure("`handle` is required")),
            },
            "document.merge" => match handle_field(&value) {
                Some(handle) => {
                    let Some(path) = string_field(&value, "path")
                    else {
                        return failure(id, "documentError", argument_failure("`path` is required"));
                    };
                    let mode = string_field(&value, "mode");
                    document_outcome(id, "mergeResult", open_word_document::merge(handle, Path::new(&path), mode.as_deref()))
                }
                None => failure(id, "documentError", argument_failure("`handle` and `path` are required")),
            },
            "document.export" => match (handle_field(&value), string_field(&value, "format"), string_field(&value, "target")) {
                (Some(handle), Some(format), Some(target)) => {
                    let overwrite = value.get("overwrite").and_then(Value::as_bool).unwrap_or(true);
                    document_outcome(
                        id,
                        "exportResult",
                        open_word_document::export(handle, &format, Path::new(&target), overwrite),
                    )
                }
                _ => failure(id, "documentError", argument_failure("`handle`, `format` and `target` are required")),
            },
            "document.close" => match handle_field(&value) {
                Some(handle) => match open_word_document::close(handle) {
                    Ok(()) => reply(id, "closeResult", json!({ "closed": true })),
                    Err(error) => failure(id, "closeError", service_failure(error)),
                },
                None => failure(id, "closeError", argument_failure("`handle` is required")),
            },
            _ => IpcOutcome::Ignored,
        }
    }
}

fn failure(id: Value, kind: &str, error: Value) -> IpcOutcome {
    // The reply carries the verifiable `code` and typed `params` beside the
    // display `message`, so the page can branch on the contract, not on prose.
    let mut body = json!({ "message": error["message"].clone(), "code": error["code"].clone() });
    if let Some(params) = error.get("params").filter(|params| !params.is_null()) {
        body["params"] = params.clone();
    }
    reply(id, kind, body)
}

fn reply(id: Value, kind: &str, payload: Value) -> IpcOutcome {
    let mut body = payload;
    body["type"] = Value::String(kind.to_string());
    body["id"] = id;
    IpcOutcome::ReplyToPage(body.to_string())
}

fn service_failure(error: open_word_document::ServiceError) -> Value {
    let mut payload = json!({ "code": error.code(), "message": error.message() });
    if !error.params().is_null() {
        payload["params"] = error.params().clone();
    }
    payload
}

fn argument_failure(message: &str) -> Value {
    json!({ "code": "session/argument-invalid", "message": message })
}

fn document_outcome(id: Value, kind: &str, result: std::result::Result<Value, open_word_document::ServiceError>) -> IpcOutcome {
    match result {
        Ok(value) => {
            let payload = if matches!(kind, "createResult" | "openResult") { value } else { json!({ "report": value }) };
            reply(id, kind, payload)
        }
        Err(error) => failure(id, "documentError", service_failure(error)),
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn handle_field(value: &Value) -> Option<u64> {
    value.get("handle").and_then(Value::as_u64)
}

fn optional_steps_field(value: &Value) -> std::result::Result<Option<u32>, Value> {
    match value.get("steps") {
        None | Some(Value::Null) => Ok(None),
        Some(steps) => steps
            .as_u64()
            .and_then(|raw| u32::try_from(raw).ok())
            .map(Some)
            .ok_or_else(|| argument_failure("`steps` must be a positive integer")),
    }
}

fn platform_name() -> &'static str {
    if cfg!(windows) {
        "win32"
    }
    else if cfg!(target_os = "macos") {
        "darwin"
    }
    else {
        "linux"
    }
}

fn gui_status(
    state: &'static str,
    code: Option<&'static str>,
    backend: Option<&'static str>,
    uptime_ms: u64,
    message: Option<String>,
) -> Value {
    json!({
        "state": state,
        "code": code,
        "backend": backend,
        "platform": platform_name(),
        "uptimeMs": uptime_ms,
        "message": message,
    })
}

fn gui_open_result(ok: bool, status: Value, error: Option<&WebViewError>) -> Value {
    let (stage, hresult, attempted) = match error {
        Some(error) => {
            let attempted = match error {
                WebViewError::Host(host) => host.attempted.clone(),
                _ => Vec::new(),
            };
            (error.stage(), error.hresult(), attempted)
        }
        None => (None, None, Vec::new()),
    };
    json!({ "ok": ok, "status": status, "stage": stage, "hresult": hresult, "attempted": attempted })
}

/// Resolves the GUI content directory: explicit override, then the built tree.
fn resolve_gui_dir() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("OPEN_WORD_GUI") {
        let path = PathBuf::from(value);
        if path.is_dir() {
            return Some(path);
        }
        if path.is_file() {
            return path.parent().map(Path::to_path_buf);
        }
    }

    let manifest_relative = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("projects")
        .join("packages")
        .join("open-word")
        .join("dist")
        .join("gui");
    if manifest_relative.join("index.html").is_file() {
        return Some(manifest_relative);
    }

    if let Ok(cwd) = std::env::current_dir() {
        for candidate in [cwd.join("projects/packages/open-word/dist/gui"), cwd.join("dist/gui"), cwd.join("gui")] {
            if candidate.join("index.html").is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Opens the OpenWord WebView window, or focuses the live one.
#[function(js_name = "openGui")]
fn open_gui(ctx: CallContext) -> Result<JsValue> {
    let mut title = "OpenWord".to_string();
    let mut content_dir: Option<String> = None;
    let mut width = 1280i32;
    let mut height = 820i32;

    if let Some(arg) = ctx.args().first().copied()
        && !arg.is_undefined()?
    {
        if let Some(value) = optional_string_prop(arg, "title")? {
            title = value;
        }
        if let Some(value) = optional_string_prop(arg, "contentDir")? {
            content_dir = Some(value);
        }
        if let Ok(value) = arg.get_named("width")
            && let Ok(number) = value.as_i64()
        {
            width = number as i32;
        }
        if let Ok(value) = arg.get_named("height")
            && let Ok(number) = value.as_i64()
        {
            height = number as i32;
        }
    }

    let live = GUI.with(|slot| {
        let guard = slot.borrow();
        guard
            .as_ref()
            .filter(|state| state.host.is_alive())
            .map(|state| (state.backend, state.opened_at.elapsed().as_millis() as u64))
    });
    if let Some((backend, uptime_ms)) = live {
        GUI.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                let _ = state.host.focus();
            }
        });
        let status = gui_status("open", None, Some(backend), uptime_ms, None);
        return respond(&ctx, &gui_open_result(true, status, None));
    }
    GUI.with(|slot| {
        *slot.borrow_mut() = None;
    });

    let dir = match content_dir {
        Some(dir) => PathBuf::from(dir),
        None => match resolve_gui_dir() {
            Some(dir) => dir,
            None => {
                let message = "the WebView content directory is missing; build the GUI first".to_string();
                let status = gui_status("failed", Some("gui_content_missing"), None, 0, Some(message));
                return respond(&ctx, &gui_open_result(false, status, None));
            }
        },
    };

    let close_requested = Arc::new(AtomicBool::new(false));
    let mut options = OpenOptions::from_content_dir(dir.display().to_string(), title);
    options.width = width;
    options.height = height;
    options.accept_file_drop = true;

    match WebViewHost::open(options, Box::new(WordIpcHandler { close_requested: Arc::clone(&close_requested) })) {
        Ok(host) => {
            let backend = host.backend_name();
            GUI.with(|slot| {
                *slot.borrow_mut() = Some(GuiState { host, backend, opened_at: Instant::now(), close_requested });
            });
            let status = gui_status("open", None, Some(backend), 0, None);
            respond(&ctx, &gui_open_result(true, status, None))
        }
        Err(error) => {
            let status = gui_status("failed", error.code(), None, 0, Some(error.to_string()));
            respond(&ctx, &gui_open_result(false, status, Some(&error)))
        }
    }
}

/// Pumps the host message loop once and reports the observed lifecycle state.
#[function(js_name = "pumpEvents")]
fn pump_events(ctx: CallContext) -> Result<JsValue> {
    let status = GUI.with(|slot| {
        let mut guard = slot.borrow_mut();
        let Some(state) = guard.as_mut()
        else {
            return gui_status("closed", Some(GUI_NOT_OPEN), None, 0, Some("no OpenWord window is open".to_string()));
        };
        let backend = state.backend;
        let uptime_ms = state.opened_at.elapsed().as_millis() as u64;
        if state.host.pump() {
            return gui_status("open", None, Some(backend), uptime_ms, None);
        }
        let requested = state.close_requested.swap(false, Ordering::SeqCst);
        *guard = None;
        if requested {
            gui_status("closed", Some(GUI_CLOSED), Some(backend), uptime_ms, None)
        }
        else {
            let message = format!("the {backend} webview host stopped without a page close request");
            gui_status("closed", Some(GUI_HOST_EXITED), Some(backend), uptime_ms, Some(message))
        }
    });
    respond(&ctx, &status)
}

/// Closes the OpenWord window; the result distinguishes "already closed".
#[function(js_name = "closeGui")]
fn close_gui(ctx: CallContext) -> Result<JsValue> {
    let result = GUI.with(|slot| {
        let mut guard = slot.borrow_mut();
        let Some(state) = guard.as_mut()
        else {
            let message = "no OpenWord window is open".to_string();
            return json!({
                "closed": false,
                "status": gui_status("not_open", Some(GUI_NOT_OPEN), None, 0, Some(message)),
            });
        };
        let backend = state.backend;
        let uptime_ms = state.opened_at.elapsed().as_millis() as u64;
        match state.host.close() {
            Ok(()) => {
                *guard = None;
                json!({ "closed": true, "status": gui_status("closed", None, Some(backend), uptime_ms, None) })
            }
            Err(error) => json!({
                "closed": false,
                "status": gui_status("open", Some("gui_close_failed"), Some(backend), uptime_ms, Some(error.to_string())),
            }),
        }
    });
    respond(&ctx, &result)
}

/// Reports whether the OpenWord window is alive.
#[function(js_name = "isGuiOpen")]
fn is_gui_open(ctx: CallContext) -> Result<JsValue> {
    let open = GUI.with(|slot| slot.borrow().as_ref().is_some_and(|state| state.host.is_alive()));
    ctx.env().boolean(open)
}

register_module![
    version,
    capabilities,
    document_create,
    document_open,
    document_save,
    document_inspect,
    document_summary,
    document_outline,
    word_stats,
    document_find,
    document_replace,
    document_edit,
    document_undo,
    document_redo,
    document_merge,
    document_export,
    document_close,
    open_gui,
    pump_events,
    close_gui,
    is_gui_open
];
