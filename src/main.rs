mod picker;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Json, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, services::ServeDir, set_header::SetResponseHeaderLayer};

#[derive(Clone)]
struct AppConfig {
    gemini_api_key: String,
    gemini_model: String,
    tts_voice: String,
}

#[derive(Deserialize)]
struct SendCommand {
    command: String,
    session: String,
}

#[derive(Serialize)]
struct ApiResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct TmuxWindow {
    target: String,
    name: String,
}

#[derive(Deserialize)]
struct CaptureRequest {
    target: String,
}

#[derive(Serialize)]
struct CaptureResponse {
    content: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    window_closed: bool,
    /// A live Claude selection prompt at the tail of the pane, if there is one.
    /// Optional and additive, so existing clients (including `mobile/`) ignore it.
    #[serde(skip_serializing_if = "Option::is_none")]
    picker: Option<picker::Picker>,
}

async fn capture_pane(Json(payload): Json<CaptureRequest>) -> impl IntoResponse {
    let target = if payload.target.is_empty() {
        "0".to_string()
    } else {
        payload.target
    };

    // Capture the pane content with history
    let result = Command::new("tmux")
        .args(["capture-pane", "-p", "-t", &target, "-S", "-1000"])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let content = String::from_utf8_lossy(&output.stdout).to_string();
                // Parsed server-side and only here. If the client parsed too, the
                // renderer and the committer would drift, and the failure mode is
                // a card that shows one option and sends another.
                let picker = picker::parse(&content);
                (StatusCode::OK, Json(CaptureResponse { content, window_closed: false, picker }))
            } else {
                (StatusCode::OK, Json(CaptureResponse { content: String::new(), window_closed: true, picker: None }))
            }
        }
        Err(_) => (StatusCode::OK, Json(CaptureResponse { content: String::new(), window_closed: true, picker: None })),
    }
}

/// Capture the pane with a bounded slice of scrollback. Visible-only capture
/// is not enough: a prompt taller than the window (narrow panes, long option
/// descriptions) pushes its own top rule into scrollback, and a capture that
/// cannot see the top rule cannot parse the prompt — every select then fails
/// with a false "no prompt is waiting".
fn capture_visible(target: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["capture-pane", "-p", "-t", target, "-S", "-200"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

#[derive(Deserialize)]
struct PickerSelectRequest {
    target: String,
    /// Index into the option list. Addressed by index, never by printed number —
    /// Claude Code renders some rows ("Chat about this") without one.
    index: usize,
    /// The prompt the client was looking at when the user chose.
    fingerprint: String,
}

#[derive(Deserialize)]
struct PickerStepRequest {
    target: String,
    /// -1 or +1. Used by the preview layout, where only the focused option's
    /// preview is rendered, so the real cursor has to move to reveal another.
    delta: i32,
    fingerprint: String,
}

#[derive(Serialize)]
struct PickerActionResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// On refusal: what is actually on screen now, so the client can re-render.
    /// On outcome "changed": the prompt the keystroke produced (next question
    /// of a set, toggled checkbox, review screen), for the same reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    picker: Option<picker::Picker>,
    /// "committed" — the prompt is answered and gone.
    /// "awaiting_text" — the dialog's free-text buffer is open; it will not
    /// move until text is typed into it.
    /// "changed" — a different prompt is on screen now; `picker` carries it.
    /// "pending" — the keystroke was sent but no effect was observed yet; the
    /// client should keep the card and let polling reconcile.
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
}

fn picker_conflict(reason: &str, current: Option<picker::Picker>) -> (StatusCode, Json<PickerActionResponse>) {
    (
        StatusCode::CONFLICT,
        Json(PickerActionResponse {
            success: false,
            error: Some(reason.to_string()),
            picker: current,
            outcome: None,
        }),
    )
}

/// Re-capture and re-parse, refusing if the prompt is not the one the client saw.
fn verify_picker(
    target: &str,
    fingerprint: &str,
) -> Result<picker::Picker, (StatusCode, Json<PickerActionResponse>)> {
    let Some(pane) = capture_visible(target) else {
        return Err(picker_conflict("window is gone", None));
    };
    let Some(current) = picker::parse(&pane) else {
        return Err(picker_conflict("no prompt is waiting", None));
    };
    if current.fingerprint != fingerprint {
        return Err(picker_conflict("the question changed", Some(current)));
    }
    Ok(current)
}

fn send_keys(target: &str, keys: &[String]) -> Result<(), String> {
    let mut args: Vec<&str> = vec!["send-keys", "-t", target];
    args.extend(keys.iter().map(|k| k.as_str()));
    let output = Command::new("tmux")
        .args(&args)
        .output()
        .map_err(|e| format!("failed to send keys: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// Commit a choice. The client sends intent, not keystrokes: the cursor delta is
/// computed here from a fresh capture, so there is no polling window in which the
/// terminal could move out from under it.
async fn picker_select(Json(payload): Json<PickerSelectRequest>) -> impl IntoResponse {
    let current = match verify_picker(&payload.target, &payload.fingerprint) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if payload.index >= current.options.len() {
        return picker_conflict("no such option", Some(current));
    }

    // Prefer the option's own digit: one keystroke, no traversal, no race.
    // Never for the rows Claude Code adds around the tool's options — their
    // printed digits are display-only, and pressing one types the digit into
    // the dialog's free-text buffer (verified live on 2.1.220: the buffer then
    // eats even arrow keys as literal escape bytes). Those rows, and anything
    // unnumbered, are reached by walking the cursor and pressing Enter.
    let chosen = &current.options[payload.index];
    let digit = if picker::is_input_row(chosen) {
        None
    } else {
        picker::select_key(chosen.number)
    };
    if let Some(key) = digit {
        if let Err(e) = send_keys(&payload.target, &[key]) {
            return (
                StatusCode::BAD_REQUEST,
                Json(PickerActionResponse { success: false, error: Some(e), picker: None, outcome: None }),
            );
        }
    } else {
        if let Err(e) = walk_cursor_to(&payload.target, &payload.fingerprint, current.cursor, payload.index).await {
            return picker_conflict(&e, capture_visible(&payload.target).and_then(|p| picker::parse(&p)));
        }
        // Enter must be its own invocation. Batched with movement, Claude Code's
        // TUI applies it against pre-move state and commits the wrong option.
        if let Err(e) = send_keys(&payload.target, &["Enter".to_string()]) {
            return (
                StatusCode::BAD_REQUEST,
                Json(PickerActionResponse { success: false, error: Some(e), picker: None, outcome: None }),
            );
        }
    }

    // Watch for evidence of what the keystroke did, and report only what was
    // seen. The previous version inferred "awaiting_text" from the prompt
    // still being on screen after a timeout — but the TUI's render can lag its
    // state by seconds on a loaded session, so a slow redraw of a committed
    // answer was reported as "Claude wants text". Timeouts are not evidence.
    //
    //   prompt gone, chrome gone   -> committed
    //   prompt gone, chrome still  -> awaiting_text (free-text buffer is open)
    //   different prompt parsed    -> changed (client re-renders from it)
    //   nothing observed in time   -> pending (client keeps the card, polls)
    let mut outcome = "pending";
    let mut fresh: Option<picker::Picker> = None;
    for _ in 0..34 {
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        let Some(pane) = capture_visible(&payload.target) else {
            outcome = "committed";
            break;
        };
        match picker::parse(&pane) {
            None => {
                outcome = if picker::awaiting_typed_reply(&pane) { "awaiting_text" } else { "committed" };
                break;
            }
            Some(p) if p.fingerprint != payload.fingerprint => {
                outcome = "changed";
                fresh = Some(p);
                break;
            }
            Some(_) => {}
        }
    }

    (
        StatusCode::OK,
        Json(PickerActionResponse {
            success: true,
            error: None,
            picker: fresh,
            outcome: Some(outcome.to_string()),
        }),
    )
}

/// Move the cursor one row at a time, waiting for each step to land before
/// sending the next.
///
/// Fixed sleeps are not enough — arrows spaced 250ms apart were still dropped,
/// while the same arrows spaced ~600ms apart all landed. Rather than tune a
/// delay, wait on the condition. If a step never lands, the caller aborts
/// without pressing Enter, so a failed move can never answer the wrong option.
async fn walk_cursor_to(
    target: &str,
    fingerprint: &str,
    from: usize,
    to: usize,
) -> Result<(), String> {
    let mut at = from;
    let mut guard = 0;
    while at != to {
        if guard > 64 {
            return Err("could not move the cursor — nothing was sent".to_string());
        }
        guard += 1;

        let key = picker::step_key(if to > at { 1 } else { -1 }).to_string();
        send_keys(target, &[key])?;

        let mut moved = false;
        for _ in 0..25 {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            match capture_visible(target).and_then(|p| picker::parse(&p)) {
                Some(p) if p.fingerprint == fingerprint && p.cursor != at => {
                    at = p.cursor;
                    moved = true;
                    break;
                }
                Some(p) if p.fingerprint != fingerprint => {
                    return Err("the question changed".to_string())
                }
                _ => {}
            }
        }
        if !moved {
            return Err("could not move the cursor — nothing was sent".to_string());
        }
    }
    Ok(())
}

/// Move the terminal's own cursor by one row without committing.
async fn picker_step(Json(payload): Json<PickerStepRequest>) -> impl IntoResponse {
    let current = match verify_picker(&payload.target, &payload.fingerprint) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let key = picker::step_key(payload.delta).to_string();
    match send_keys(&payload.target, &[key]) {
        Ok(()) => (
            StatusCode::OK,
            Json(PickerActionResponse { success: true, error: None, picker: Some(current), outcome: None }),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(PickerActionResponse { success: false, error: Some(e), picker: None, outcome: None }),
        ),
    }
}

#[derive(Deserialize)]
struct PickerTextRequest {
    target: String,
    text: String,
}

/// Send a typed reply for an option that asked for one.
///
/// Deliberately NOT `/api/send`, which presses Enter three times 500ms apart as
/// a delivery workaround for the fire-and-forget EXECUTE button. Those extra
/// presses land in a TUI that queues input while it is busy, and can resubmit
/// what was already sent. Here exactly one Enter is pressed.
async fn picker_text(Json(payload): Json<PickerTextRequest>) -> impl IntoResponse {
    let text = payload.text.trim().to_string();
    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse { success: false, error: Some("empty reply".to_string()) }),
        );
    }

    // Literal first, then Enter as its own invocation — `send-keys -l <s> Enter`
    // would type the word "Enter".
    if let Err(e) = send_keys_literal(&payload.target, &text) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse { success: false, error: Some(e) }),
        );
    }
    match send_keys(&payload.target, &["Enter".to_string()]) {
        Ok(()) => (StatusCode::OK, Json(ApiResponse { success: true, error: None })),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse { success: false, error: Some(e) }),
        ),
    }
}

fn send_keys_literal(target: &str, text: &str) -> Result<(), String> {
    let output = Command::new("tmux")
        .args(["send-keys", "-t", target, "-l", text])
        .output()
        .map_err(|e| format!("failed to send text: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

#[derive(Serialize)]
struct WindowStatus {
    target: String,
    /// A prompt is on screen and nothing moves until it is answered.
    waiting: bool,
    /// Claude's live status verb ("Wrangling"), absent when it is not working.
    #[serde(skip_serializing_if = "Option::is_none")]
    verb: Option<String>,
    /// The parenthesised meta from the same line: "10m 40s · ↓ 25.6k tokens".
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<String>,
}

/// Claude Code prints one status line while it is running:
///
///     ✻ Wrangling… (10m 40s · ↓ 25.6k tokens)
///
/// and replaces it with a past-tense summary the moment it stops:
///
///     ✻ Worked for 3m 16s
///
/// The parenthesised live timer is the discriminator — the spinner glyph and
/// the verb appear in both, so keying on either would report a finished window
/// as busy forever. Mirrors WORKING_LINE in static/index.html; the two must
/// stay in step.
fn parse_working(pane: &str) -> Option<(String, String)> {
    let re = regex::Regex::new(r"(?:^|\s)([A-Za-z][A-Za-z ]{0,20})…\s*\(([^)]*\b\d+s\b[^)]*)\)")
        .ok()?;
    // Only the tail: the same line from an earlier turn is still in scrollback,
    // and matching it would pin every window on permanently.
    let lines: Vec<&str> = pane.lines().collect();
    let start = lines.len().saturating_sub(30);
    for line in lines[start..].iter().rev() {
        if let Some(caps) = re.captures(line) {
            return Some((
                caps[1].trim().to_string(),
                caps[2].trim().to_string(),
            ));
        }
    }
    None
}

/// Per-window liveness: which windows are waiting on a prompt, and which are
/// busy working. This is what makes switching away safe rather than merely
/// permitted — without it you switch away and forget. One capture per window
/// answers both questions, so the busy state costs no extra tmux calls.
async fn window_status() -> impl IntoResponse {
    let Ok(output) = Command::new("tmux")
        .args(["list-windows", "-a", "-F", "#{session_name}:#{window_index}"])
        .output()
    else {
        return (StatusCode::OK, Json(Vec::<WindowStatus>::new()));
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let statuses: Vec<WindowStatus> = stdout
        .lines()
        .filter(|t| !t.trim().is_empty())
        .filter_map(|target| {
            let pane = capture_visible(target)?;
            let waiting = picker::parse(&pane).is_some();
            // A window at a prompt is not working, whatever the last status
            // line said — the question is the thing you need to act on.
            let working = if waiting { None } else { parse_working(&pane) };
            if !waiting && working.is_none() {
                return None;
            }
            let (verb, meta) = match working {
                Some((v, m)) => (Some(v), Some(m)),
                None => (None, None),
            };
            Some(WindowStatus {
                target: target.to_string(),
                waiting,
                verb,
                meta,
            })
        })
        .collect();

    (StatusCode::OK, Json(statuses))
}

async fn list_windows() -> impl IntoResponse {
    let result = Command::new("tmux")
        .args(["list-windows", "-a", "-F", "#{session_name}:#{window_index}\t#{window_name}"])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let windows: Vec<TmuxWindow> = stdout
                    .lines()
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.split('\t').collect();
                        if parts.len() == 2 {
                            Some(TmuxWindow {
                                target: parts[0].to_string(),
                                name: parts[1].to_string(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                (StatusCode::OK, Json(windows))
            } else {
                (StatusCode::OK, Json(vec![]))
            }
        }
        Err(_) => (StatusCode::OK, Json(vec![])),
    }
}

async fn send_to_tmux(Json(payload): Json<SendCommand>) -> impl IntoResponse {
    let session = if payload.session.is_empty() {
        "0".to_string()
    } else {
        payload.session
    };

    let command = payload.command;

    // Send the command text literally
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", &session, "-l", &command])
        .output();

    // Send Enter key 3 times with 500ms delay
    for i in 0..3 {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", &session, "Enter"])
            .output();
    }

    (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            error: None,
        }),
    )
}

async fn health() -> &'static str {
    "OK"
}

#[derive(Deserialize)]
struct ServeImageQuery {
    path: String,
}

async fn serve_image(
    axum::extract::Query(query): axum::extract::Query<ServeImageQuery>,
) -> impl IntoResponse {
    let path = std::path::Path::new(&query.path);

    // Canonicalize to prevent directory traversal
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain")],
                axum::body::Body::from("File not found"),
            );
        }
    };

    // Verify it's a file with an image extension
    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let content_type = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tiff" | "tif" => "image/tiff",
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "text/plain")],
                axum::body::Body::from("Not a supported image type"),
            );
        }
    };

    match tokio::fs::read(&canonical).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, content_type)],
            axum::body::Body::from(bytes),
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain")],
            axum::body::Body::from("Failed to read file"),
        ),
    }
}

async fn serve_file(
    axum::extract::Query(query): axum::extract::Query<ServeImageQuery>,
) -> impl IntoResponse {
    let path = std::path::Path::new(&query.path);

    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "File not found".to_string(),
            );
        }
    };

    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Only allow known text file types
    match ext.as_str() {
        "json" | "txt" | "log" | "csv" | "xml" | "yaml" | "yml" | "toml" | "md" | "ini" | "cfg" => {}
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "Not a supported text file type".to_string(),
            );
        }
    }

    // Cap at 5MB to avoid loading huge files
    match tokio::fs::metadata(&canonical).await {
        Ok(meta) if meta.len() > 5 * 1024 * 1024 => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "File too large (>5MB)".to_string(),
            );
        }
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "File not found".to_string(),
            );
        }
        _ => {}
    }

    match tokio::fs::read_to_string(&canonical).await {
        Ok(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            content,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "Failed to read file".to_string(),
        ),
    }
}

async fn serve_voice() -> impl IntoResponse {
    match tokio::fs::read_to_string("static/voice.html").await {
        Ok(html) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain")],
            "Voice page not found".to_string(),
        ),
    }
}

#[derive(Deserialize)]
struct SendKeyRequest {
    key: String,
    session: String,
}

async fn send_key(Json(payload): Json<SendKeyRequest>) -> impl IntoResponse {
    let session = if payload.session.is_empty() {
        "0".to_string()
    } else {
        payload.session
    };

    // Send the key as a tmux key sequence (not literal, no Enter)
    let result = Command::new("tmux")
        .args(["send-keys", "-t", &session, &payload.key])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                (
                    StatusCode::OK,
                    Json(ApiResponse {
                        success: true,
                        error: None,
                    }),
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        success: false,
                        error: Some(stderr.to_string()),
                    }),
                )
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                error: Some(format!("Failed to send key: {}", e)),
            }),
        ),
    }
}

/// Validate a project/window name that will become a `~/p/<name>` directory and
/// a tmux `-n <name>`. Allowlist only: letters, digits, '.', '_', '-'. This keeps
/// the name out of every hazardous interpretation — shell metacharacters (it never
/// enters a shell here), tmux format chars like '#' (interpolated by `-n`), path
/// traversal, and tmux argv/flag injection (leading '-').
fn validate_window_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name cannot be empty".to_string());
    }
    if name == "." || name == ".." {
        return Err("name cannot be '.' or '..'".to_string());
    }
    if name.starts_with('-') {
        return Err("name cannot start with '-'".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err("name may contain only letters, digits, '.', '_', '-'".to_string());
    }
    Ok(())
}

async fn new_window() -> impl IntoResponse {
    // Create a new tmux window
    let result = Command::new("tmux")
        .args(["new-window", "-P", "-F", "#{session_name}:#{window_index}"])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let target = String::from_utf8_lossy(&output.stdout).trim().to_string();
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "success": true,
                        "target": target
                    })),
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "success": false,
                        "error": stderr.to_string()
                    })),
                )
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to create window: {}", e)
            })),
        ),
    }
}

#[derive(Deserialize)]
struct NewWindowNamedRequest {
    name: String,
}

async fn new_window_named(Json(payload): Json<NewWindowNamedRequest>) -> impl IntoResponse {
    let name = payload.name.trim().to_string();

    if let Err(e) = validate_window_name(&name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"success": false, "error": e})),
        );
    }

    // 1. Existing window with this exact name? Read-only — no select-window probe
    //    (that would switch the user's active window as a side effect).
    if let Ok(out) = Command::new("tmux")
        .args([
            "list-windows", "-a", "-F",
            "#{window_id}\t#{window_name}\t#{session_name}:#{window_index}",
        ])
        .output()
    {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() == 3 && parts[1] == name {
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "success": true, "existing": true,
                            "window_id": parts[0], "target": parts[2],
                        })),
                    );
                }
            }
        }
    }

    // 2. Resolve ~/p/<name> and create it.
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let dir = format!("{}/p/{}", home, name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": format!("mkdir failed: {}", e)})),
        );
    }

    // 3. Create the window IN that dir. -n makes the name permanent; -P -F returns
    //    the stable window_id (targeting by name would hit the oldest duplicate).
    let create = Command::new("tmux")
        .args([
            "new-window", "-c", &dir, "-n", &name, "-P", "-F",
            "#{window_id}\t#{session_name}:#{window_index}",
        ])
        .output();
    let (window_id, target) = match create {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let parts: Vec<&str> = s.split('\t').collect();
            if parts.len() != 2 {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"success": false, "error": format!("unexpected new-window output: {}", s)})),
                );
            }
            (parts[0].to_string(), parts[1].to_string())
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "error": stderr.to_string()})),
            );
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"success": false, "error": format!("new-window failed: {}", e)})),
            );
        }
    };

    // 4. Verify the pane actually started in <dir> BEFORE launching claude --yolo
    //    (which skips permission prompts). Never fire it in an unintended dir.
    let expected = std::fs::canonicalize(&dir).unwrap_or_else(|_| std::path::PathBuf::from(&dir));
    let actual = Command::new("tmux")
        .args(["display-message", "-p", "-t", &window_id, "#{pane_current_path}"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let actual_canon =
        std::fs::canonicalize(&actual).unwrap_or_else(|_| std::path::PathBuf::from(&actual));
    if actual_canon != expected {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("window did not start in {} (got {})", dir, actual),
                "window_id": window_id, "target": target,
            })),
        );
    }

    // 5. Launch. -l on the payload only; Enter is a separate send-keys.
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", &window_id, "-l", "claude --yolo"])
        .output();
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", &window_id, "Enter"])
        .output();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true, "existing": false,
            "window_id": window_id, "target": target,
        })),
    )
}

#[derive(Deserialize)]
struct RenameWindowRequest {
    target: String,
    name: String,
}

#[derive(Deserialize)]
struct MoveWindowRequest {
    session: String,
    from_index: i32,
    to_index: i32,
}

async fn move_window(Json(payload): Json<MoveWindowRequest>) -> impl IntoResponse {
    let session = if payload.session.is_empty() {
        "0".to_string()
    } else {
        payload.session.clone()
    };

    // Use swap-window to swap the two positions
    let result = Command::new("tmux")
        .args([
            "swap-window",
            "-s",
            &format!("{}:{}", session, payload.from_index),
            "-t",
            &format!("{}:{}", session, payload.to_index),
        ])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                (
                    StatusCode::OK,
                    Json(ApiResponse {
                        success: true,
                        error: None,
                    }),
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        success: false,
                        error: Some(stderr.to_string()),
                    }),
                )
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                error: Some(format!("Failed to move window: {}", e)),
            }),
        ),
    }
}

async fn rename_window(Json(payload): Json<RenameWindowRequest>) -> impl IntoResponse {
    let target = if payload.target.is_empty() {
        "0".to_string()
    } else {
        payload.target
    };

    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                error: Some("Name cannot be empty".to_string()),
            }),
        );
    }

    let result = Command::new("tmux")
        .args(["rename-window", "-t", &target, &name])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                (
                    StatusCode::OK,
                    Json(ApiResponse {
                        success: true,
                        error: None,
                    }),
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        success: false,
                        error: Some(stderr.to_string()),
                    }),
                )
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                error: Some(format!("Failed to rename window: {}", e)),
            }),
        ),
    }
}

#[derive(Deserialize)]
struct KillWindowRequest {
    target: String,
}

/// Unlike rename-window, an empty target must NOT default to "0". Rename is
/// recoverable; killing the wrong window is not.
fn validate_kill_target(raw: &str) -> Result<&str, String> {
    let target = raw.trim();
    if target.is_empty() {
        return Err("target cannot be empty".to_string());
    }
    Ok(target)
}

/// Forcibly close a window. tmux SIGHUPs every process in it, which is the
/// point: whatever was running there dies with the window.
async fn kill_window(Json(payload): Json<KillWindowRequest>) -> impl IntoResponse {
    let target = match validate_kill_target(&payload.target) {
        Ok(t) => t.to_string(),
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    success: false,
                    error: Some(e),
                }),
            );
        }
    };

    let result = Command::new("tmux")
        .args(["kill-window", "-t", &target])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                (
                    StatusCode::OK,
                    Json(ApiResponse {
                        success: true,
                        error: None,
                    }),
                )
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        success: false,
                        error: Some(stderr.trim().to_string()),
                    }),
                )
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                error: Some(format!("Failed to kill window: {}", e)),
            }),
        ),
    }
}

#[derive(Serialize)]
struct ProjectDir {
    name: String,
    mtime: u64,
}

#[derive(Serialize)]
struct ProjectDirsResponse {
    dirs: Vec<ProjectDir>,
}

/// Directories under `base` that could serve as a window name, newest first.
///
/// mtime is the "last accessed" proxy: atime is unreliable under relatime and
/// noatime mounts, while a project directory's mtime moves whenever files are
/// added or removed in it. Names are held to `validate_window_name` because
/// the caller turns the pick straight into a tmux window name, and ~/p also
/// holds loose files and junk like `auth?code=...` that can never be one.
fn collect_project_dirs(base: &std::path::Path) -> Vec<ProjectDir> {
    let entries = match std::fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut dirs: Vec<ProjectDir> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if validate_window_name(&name).is_err() {
                return None;
            }
            // metadata() follows symlinks, so a symlinked project counts.
            let meta = entry.metadata().ok()?;
            if !meta.is_dir() {
                return None;
            }
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some(ProjectDir { name, mtime })
        })
        .collect();

    // Newest first; name as a tiebreaker so equal mtimes order predictably.
    dirs.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.name.cmp(&b.name)));
    dirs
}

/// The whole list ships in one response when the new-window modal opens, so
/// the client can filter keystroke-by-keystroke without a round trip.
async fn project_dirs() -> impl IntoResponse {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let base = std::path::PathBuf::from(format!("{}/p", home));
    (
        StatusCode::OK,
        Json(ProjectDirsResponse {
            dirs: collect_project_dirs(&base),
        }),
    )
}

#[derive(Serialize)]
struct ConfigResponse {
    large_mode: bool,
}

async fn get_config() -> impl IntoResponse {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default();

    let large_mode = hostname == "marcker-mac.roam.internal";

    (StatusCode::OK, Json(ConfigResponse { large_mode }))
}

#[derive(Deserialize)]
struct SpeakRequest {
    text: String,
}

#[derive(Deserialize)]
struct ClientErrorRequest {
    error: String,
    context: Option<String>,
    url: Option<String>,
}

async fn log_client_error(Json(payload): Json<ClientErrorRequest>) -> impl IntoResponse {
    let context = payload.context.unwrap_or_default();
    let url = payload.url.unwrap_or_default();

    log_to_file("client_errors", &format!(
        "URL: {}\nContext: {}\nError: {}",
        url, context, payload.error
    ));

    eprintln!("[CLIENT ERROR] {} | {} | {}", url, context, payload.error);

    (StatusCode::OK, Json(serde_json::json!({"logged": true})))
}

#[derive(Deserialize)]
struct SpeakDirectRequest {
    text: String,
    target: String,
}

fn hash_text(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

async fn summarize_with_gemini(config: &AppConfig, text: &str) -> Result<String, String> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        config.gemini_model, config.gemini_api_key
    );

    let prompt = format!(
        "Summarize the following terminal output in 1-2 concise sentences suitable for text-to-speech. \
         Focus on the key information or result. Be brief and natural-sounding:\n\n{}",
        text
    );

    let body = serde_json::json!({
        "contents": [{
            "parts": [{"text": prompt}]
        }]
    });

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Gemini request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Gemini API error {}: {}", status, body));
    }

    let resp_json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gemini response: {}", e))?;

    let summary = resp_json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("No summary generated")
        .to_string();

    Ok(summary)
}

async fn generate_tts_audio(config: &AppConfig, text: &str, output_path: &str) -> Result<(), String> {
    let result = tokio::process::Command::new("uv")
        .args([
            "run",
            "scripts/tts_generate.py",
            "--voice",
            &config.tts_voice,
            "--output",
            output_path,
            "--text",
            text,
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to run TTS script: {}", e))?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(format!("TTS generation failed: {}", stderr));
    }

    Ok(())
}

async fn speak_output(
    State(config): State<Arc<AppConfig>>,
    Json(payload): Json<SpeakRequest>,
) -> impl IntoResponse {
    let text = payload.text.trim();
    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "No text provided"})),
        );
    }

    let text_hash = hash_text(text);
    let cache_path = format!("/tmp/tts_{}.wav", text_hash);

    // Check cache
    if tokio::fs::metadata(&cache_path).await.is_ok() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "audio_url": format!("/api/tts-cache/{}.wav", text_hash),
                "cached": true
            })),
        );
    }

    // Summarize with Gemini
    let summary = match summarize_with_gemini(&config, text).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            );
        }
    };

    // Generate TTS audio to cache path
    if let Err(e) = generate_tts_audio(&config, &summary, &cache_path).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "audio_url": format!("/api/tts-cache/{}.wav", text_hash),
            "cached": false
        })),
    )
}

fn ensure_logs_dir() -> Result<std::path::PathBuf, String> {
    let logs_dir = std::path::PathBuf::from("logs");
    if !logs_dir.exists() {
        std::fs::create_dir_all(&logs_dir)
            .map_err(|e| format!("Failed to create logs dir: {}", e))?;
    }
    Ok(logs_dir)
}

fn log_to_file(filename: &str, content: &str) {
    if let Ok(logs_dir) = ensure_logs_dir() {
        let now = chrono::Local::now();
        let log_file = logs_dir.join(format!("{}_{}.log", filename, now.format("%Y-%m-%d")));
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
        {
            use std::io::Write;
            let _ = writeln!(file, "[{}] {}", now.format("%H:%M:%S"), content);
        }
    }
}

async fn log_voice_output(target: &str, raw_text: &str, sanitized_text: &str) -> Result<(), String> {
    use std::io::Write;

    let logs_dir = ensure_logs_dir()?;
    let now = chrono::Local::now();
    let log_file = logs_dir.join(format!("voice_{}.log", now.format("%Y-%m-%d")));

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map_err(|e| format!("Failed to open log file: {}", e))?;

    // Write log entry with both raw and sanitized
    writeln!(
        file,
        "---\nTimestamp: {}\nTarget: {}\n\n[RAW INPUT ({} chars)]:\n{}\n\n[SANITIZED ({} chars)]:\n{}\n",
        now.format("%Y-%m-%d %H:%M:%S"),
        target,
        raw_text.len(),
        raw_text,
        sanitized_text.len(),
        sanitized_text
    )
    .map_err(|e| format!("Failed to write log: {}", e))?;

    Ok(())
}

fn sanitize_for_tts(text: &str) -> String {
    // Remove ANSI escape codes
    let ansi_regex = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07|\x1b[()][0-9A-Za-z]").unwrap();
    let text = ansi_regex.replace_all(text, "");

    // Remove Claude tool call XML blocks (multiline)
    let tool_calls_regex = regex::Regex::new(r"(?s)<function_calls>.*?</function_calls>").unwrap();
    let text = tool_calls_regex.replace_all(&text, " ");

    let function_results_regex = regex::Regex::new(r"(?s)<function_results>.*?</function_results>").unwrap();
    let text = function_results_regex.replace_all(&text, " ");

    let thinking_regex = regex::Regex::new(r"(?s)<thinking>.*?</thinking>").unwrap();
    let text = thinking_regex.replace_all(&text, " ");

    let antml_regex = regex::Regex::new(r"(?s)<[^>]*>.*?</[^>]*>").unwrap();
    let text = antml_regex.replace_all(&text, " ");

    // Remove any remaining XML-like tags
    let tags_regex = regex::Regex::new(r"<[^>]+>").unwrap();
    let text = tags_regex.replace_all(&text, " ");

    // Remove box-drawing characters
    let box_chars_regex = regex::Regex::new(r"[\u2500-\u257F\u2580-\u259F]+").unwrap();
    let text = box_chars_regex.replace_all(&text, " ");

    // Remove special unicode symbols
    let symbols_regex = regex::Regex::new(r"[\u23F4-\u23F7\u25B6\u25C0\u25CF\u25CB\u25A0-\u25A1\u25AA-\u25AB\u25BA\u25C4\u276F\u276E\u2192\u2190\u2191\u2193\u21B5\u2713\u2717\u2718]+").unwrap();
    let text = symbols_regex.replace_all(&text, " ");

    // Remove control characters except newlines
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();

    // Collapse whitespace
    let space_regex = regex::Regex::new(r"[ \t]+").unwrap();
    let newline_regex = regex::Regex::new(r"\n{2,}").unwrap();
    let clean = space_regex.replace_all(&clean, " ");
    let clean = newline_regex.replace_all(&clean, "\n");

    // Filter lines
    let clean: String = clean
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .filter(|line| !line.contains("bypass permissions on"))
        .filter(|line| !line.contains("shift+tab to cycle"))
        .filter(|line| !(line.contains("files +") || line.contains("files -")))
        .collect::<Vec<_>>()
        .join("\n");

    clean
}

fn split_into_sentences(text: &str) -> Vec<String> {
    // Split on sentence boundaries (. ! ?) followed by space or newline
    // Rust regex doesn't support look-behind, so we do it manually
    let mut sentences = Vec::new();
    let mut current = String::new();

    for c in text.chars() {
        current.push(c);
        if (c == '.' || c == '!' || c == '?') {
            // Check if next would be space/newline (but we can't look ahead easily)
            // Just push current sentence when we hit punctuation
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }

    // Don't forget remaining text
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    // If no sentences found, return the whole text as one chunk
    if sentences.is_empty() && !text.trim().is_empty() {
        return vec![text.trim().to_string()];
    }

    // Merge very short sentences with the next one
    let mut merged = Vec::new();
    let mut current = String::new();
    for sentence in sentences {
        if current.is_empty() {
            current = sentence;
        } else if current.len() < 50 {
            current.push(' ');
            current.push_str(&sentence);
        } else {
            merged.push(current);
            current = sentence;
        }
    }
    if !current.is_empty() {
        merged.push(current);
    }

    merged
}

async fn speak_direct(
    State(config): State<Arc<AppConfig>>,
    Json(payload): Json<SpeakDirectRequest>,
) -> impl IntoResponse {
    let raw_text = &payload.text;
    let text = sanitize_for_tts(raw_text);

    log_to_file("debug", &format!("speak_direct called for target={}, raw_len={}, sanitized_len={}",
        &payload.target, raw_text.len(), text.len()));

    if text.is_empty() {
        log_to_file("debug", "speak_direct: empty text after sanitization");
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "No text provided"})),
        );
    }

    // Log the output (raw for debugging, sanitized for reference)
    if let Err(e) = log_voice_output(&payload.target, raw_text, &text).await {
        log_to_file("error", &format!("Failed to log voice output: {}", e));
    }

    let text_hash = hash_text(&text);
    let cache_path = format!("/tmp/tts_direct_{}.wav", text_hash);

    // Check cache
    if tokio::fs::metadata(&cache_path).await.is_ok() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "audio_url": format!("/api/tts-cache/direct_{}.wav", text_hash),
                "cached": true
            })),
        );
    }

    // Generate TTS directly without summarization
    if let Err(e) = generate_tts_audio(&config, &text, &cache_path).await {
        log_to_file("error", &format!("TTS failed for target={}: {}\nText was: {}",
            &payload.target, e, &text));
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        );
    }

    log_to_file("debug", &format!("TTS success for target={}, cached at {}",
        &payload.target, &cache_path));

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "audio_url": format!("/api/tts-cache/direct_{}.wav", text_hash),
            "cached": false
        })),
    )
}

async fn speak_chunked(
    State(config): State<Arc<AppConfig>>,
    Json(payload): Json<SpeakDirectRequest>,
) -> impl IntoResponse {
    let raw_text = &payload.text;
    let text = sanitize_for_tts(raw_text);

    log_to_file("debug", &format!("speak_chunked called for target={}, sanitized_len={}",
        &payload.target, text.len()));

    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "No text provided", "chunks": []})),
        );
    }

    // Log the output
    if let Err(e) = log_voice_output(&payload.target, raw_text, &text).await {
        log_to_file("error", &format!("Failed to log voice output: {}", e));
    }

    // Split into sentences
    let sentences = split_into_sentences(&text);
    log_to_file("debug", &format!("Split into {} sentences", sentences.len()));

    // Generate TTS for each sentence in parallel (limit to 3 concurrent)
    let config = Arc::clone(&config);
    let mut audio_urls = Vec::new();
    let mut errors = Vec::new();

    // Process in batches of 3 for parallelism
    for chunk in sentences.chunks(3) {
        let futures: Vec<_> = chunk
            .iter()
            .map(|sentence| {
                let config = Arc::clone(&config);
                let sentence = sentence.clone();
                async move {
                    let hash = hash_text(&sentence);
                    let cache_path = format!("/tmp/tts_chunk_{}.wav", hash);

                    // Check cache first
                    if tokio::fs::metadata(&cache_path).await.is_ok() {
                        return Ok(format!("/api/tts-cache/chunk_{}.wav", hash));
                    }

                    // Generate TTS
                    match generate_tts_audio(&config, &sentence, &cache_path).await {
                        Ok(()) => Ok(format!("/api/tts-cache/chunk_{}.wav", hash)),
                        Err(e) => Err(e),
                    }
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        for result in results {
            match result {
                Ok(url) => audio_urls.push(url),
                Err(e) => errors.push(e),
            }
        }

        // If we have at least one URL and it's the first batch, return early
        // so frontend can start playing while we generate more
        if !audio_urls.is_empty() && audio_urls.len() <= 3 {
            break;
        }
    }

    if audio_urls.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": errors.first().unwrap_or(&"TTS failed".to_string()),
                "chunks": []
            })),
        );
    }

    let total = sentences.len();

    // Continue generating remaining sentences in background
    let remaining_sentences: Vec<_> = sentences.into_iter().skip(audio_urls.len()).collect();
    if !remaining_sentences.is_empty() {
        let config_bg = Arc::clone(&config);
        tokio::spawn(async move {
            for sentence in remaining_sentences {
                let hash = hash_text(&sentence);
                let cache_path = format!("/tmp/tts_chunk_{}.wav", hash);
                if tokio::fs::metadata(&cache_path).await.is_err() {
                    let _ = generate_tts_audio(&config_bg, &sentence, &cache_path).await;
                }
            }
        });
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "chunks": audio_urls,
            "total_sentences": total,
        })),
    )
}

async fn serve_tts_cache(
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> impl IntoResponse {
    // Handle all cache file types: tts_, tts_direct_, tts_chunk_
    let path = format!("/tmp/tts_{}", filename);

    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "audio/wav")],
            bytes,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain")],
            b"Not found".to_vec(),
        ),
    }
}

#[derive(Deserialize)]
struct ScreenshotItem {
    name: String,
    data: String,
}

#[derive(Deserialize)]
struct BugReportRequest {
    description: Option<String>,
    app_version: Option<String>,
    build_date: Option<String>,
    device: Option<String>,
    os: Option<String>,
    screenshots: Option<Vec<ScreenshotItem>>,
}

#[derive(Deserialize)]
struct NotifyRequest {
    message: String,
}

fn send_sms_to_mark(message: &str) {
    let payload = serde_json::json!({
        "chatGuid": "iMessage;-;+14802822064",
        "message": message,
        "method": "apple-script"
    });
    let payload_str = payload.to_string();
    // Escape single quotes for shell embedding
    let payload_escaped = payload_str.replace('\'', "'\\''");
    let curl_cmd = format!(
        "curl -s -X POST 'http://localhost:1235/api/v1/message/text?password=IhopeIgetajob1%21' \
         -H 'Content-Type: application/json' \
         -d '{}'",
        payload_escaped
    );
    let _ = Command::new("ssh")
        .args(["reasonable-excuse", &curl_cmd])
        .output();
}

fn trigger_bugfix_window() {
    let window_name = "tmux terminal BUGFIX";

    // Check if window exists by trying to select it
    let check = Command::new("tmux")
        .args(["select-window", "-t", window_name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !check {
        // Create window and start claude
        let _ = Command::new("tmux")
            .args(["new-window", "-n", window_name])
            .output();
        let project_dir = "/media/xeb/GreyArea/projects/tmux-terminal";
        let start_cmd = format!("cd {} && claude", project_dir);
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", window_name, &start_cmd, "Enter"])
            .output();
        // Give claude a moment to start
        std::thread::sleep(std::time::Duration::from_secs(4));
    }

    // Send a couple of enters to clear any existing prompt, then "fix bugs"
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", window_name, "", "Enter"])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", window_name, "fix bugs", "Enter"])
        .output();
}

async fn bug_report(Json(payload): Json<BugReportRequest>) -> impl IntoResponse {
    // 1. Generate report ID: {timestamp}-{6-char-hex}
    let now = chrono::Utc::now();
    let nanos = now.timestamp_subsec_nanos();
    let suffix = format!("{:06x}", nanos % 0xFF_FFFF);
    let id = format!("{}-{}", now.format("%Y-%m-%dT%H-%M-%S"), suffix);

    // 2. Create bugs/{id}/ directory
    let dir = format!("bugs/{}", id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": e.to_string()})),
        );
    }

    // 3. Write report.json
    let screenshot_count = payload.screenshots.as_ref().map(|s| s.len()).unwrap_or(0);
    let description = payload.description.clone().unwrap_or_default();
    let report = serde_json::json!({
        "id": id,
        "timestamp": now.to_rfc3339(),
        "description": description,
        "app_version": payload.app_version.unwrap_or_default(),
        "build_date": payload.build_date.unwrap_or_default(),
        "device": payload.device.unwrap_or_default(),
        "os": payload.os.unwrap_or_default(),
        "screenshot_count": screenshot_count,
        "status": "pending"
    });

    let report_path = format!("{}/report.json", dir);
    if let Err(e) = std::fs::write(&report_path, serde_json::to_string_pretty(&report).unwrap()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": e.to_string()})),
        );
    }

    // 4. Decode and write screenshots
    if let Some(screenshots) = &payload.screenshots {
        for (i, screenshot) in screenshots.iter().enumerate() {
            if let Ok(bytes) = BASE64.decode(&screenshot.data) {
                let filename = format!("{}/screenshot_{}.jpg", dir, i + 1);
                let _ = std::fs::write(filename, bytes);
            }
        }
    }

    // 5. Trigger Claude in tmux (run in background thread to avoid blocking response)
    std::thread::spawn(trigger_bugfix_window);

    // 6. Send SMS to Mark
    let desc_for_sms = if description.is_empty() { "(no description)" } else { &description };
    let sms_msg = format!("🐛 New bug report submitted: {}", desc_for_sms);
    send_sms_to_mark(&sms_msg);

    // 7. Return success
    (
        StatusCode::OK,
        Json(serde_json::json!({"success": true, "id": id})),
    )
}

async fn notify_handler(Json(payload): Json<NotifyRequest>) -> impl IntoResponse {
    send_sms_to_mark(&payload.message);
    (StatusCode::OK, Json(serde_json::json!({"success": true})))
}

#[derive(Deserialize)]
struct UploadQuery {
    target: String,
    name: String,
}

/// Resolve the working directory of the selected tmux pane, falling back to $HOME.
fn pane_cwd(target: &str) -> std::path::PathBuf {
    let target = if target.is_empty() { "0" } else { target };
    if let Ok(out) = Command::new("tmux")
        .args(["display-message", "-p", "-t", target, "#{pane_current_path}"])
        .output()
    {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                let pb = std::path::PathBuf::from(&path);
                if pb.is_dir() {
                    return pb;
                }
            }
        }
    }
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// Reduce an uploaded filename to a safe basename (strips directories and traversal).
fn safe_filename(name: &str) -> Option<String> {
    let base = std::path::Path::new(name)
        .file_name()?
        .to_str()?
        .trim()
        .to_string();
    if base.is_empty() || base == "." || base == ".." {
        return None;
    }
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Pick a destination path that does not overwrite an existing file.
fn unique_destination(dir: &std::path::Path, filename: &str) -> std::path::PathBuf {
    let first = dir.join(filename);
    if !first.exists() {
        return first;
    }
    let p = std::path::Path::new(filename);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(filename);
    let ext = p.extension().and_then(|s| s.to_str());
    for n in 1..10000 {
        let candidate_name = match ext {
            Some(e) => format!("{}-{}.{}", stem, n, e),
            None => format!("{}-{}", stem, n),
        };
        let candidate = dir.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

/// Receive a raw file body and write it into the selected pane's working directory.
async fn upload_file(
    axum::extract::Query(query): axum::extract::Query<UploadQuery>,
    body: Bytes,
) -> impl IntoResponse {
    let filename = match safe_filename(&query.name) {
        Some(f) => f,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"success": false, "error": "Invalid filename"})),
            );
        }
    };

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"success": false, "error": "Empty file"})),
        );
    }

    let dir = pane_cwd(&query.target);
    let dest = unique_destination(&dir, &filename);

    match tokio::fs::write(&dest, &body).await {
        Ok(()) => {
            let path = dest.to_string_lossy().to_string();
            let saved_name = dest
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(filename);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "path": path,
                    "name": saved_name,
                    "size": body.len(),
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": e.to_string()})),
        ),
    }
}

#[tokio::main]
async fn main() {
    // Load .env file (ignore if missing)
    let _ = dotenvy::dotenv();

    let config = Arc::new(AppConfig {
        gemini_api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
        gemini_model: std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-3-flash-preview".to_string()),
        tts_voice: std::env::var("TTS_VOICE").unwrap_or_else(|_| "alba".to_string()),
    });

    let port = std::env::var("PORT").unwrap_or_else(|_| "5533".to_string());
    let addr = format!("0.0.0.0:{}", port);

    // Serve static files from the static directory with no-cache headers
    let static_service = ServeDir::new("static").append_index_html_on_directories(true);

    let app = Router::new()
        .route("/api/send", post(send_to_tmux))
        .route("/api/send-key", post(send_key))
        .route("/api/windows", get(list_windows))
        .route("/api/capture", post(capture_pane))
        .route("/api/picker/select", post(picker_select))
        .route("/api/picker/step", post(picker_step))
        .route("/api/picker/text", post(picker_text))
        .route("/api/window-status", get(window_status))
        .route("/api/config", get(get_config))
        .route("/api/new-window", post(new_window))
        .route("/api/new-window-named", post(new_window_named))
        .route("/api/rename-window", post(rename_window))
        .route("/api/kill-window", post(kill_window))
        .route("/api/project-dirs", get(project_dirs))
        .route("/api/move-window", post(move_window))
        .route("/api/speak", post(speak_output))
        .route("/api/speak-direct", post(speak_direct))
        .route("/api/speak-chunked", post(speak_chunked))
        .route("/api/tts-cache/:filename", get(serve_tts_cache))
        .route("/api/client-error", post(log_client_error))
        .route("/api/bug-report", post(bug_report))
        .route("/api/notify", post(notify_handler))
        .route(
            "/api/upload",
            post(upload_file).layer(DefaultBodyLimit::max(100 * 1024 * 1024)),
        )
        .route("/api/serve-image", get(serve_image))
        .route("/api/serve-file", get(serve_file))
        .route("/health", get(health))
        .route("/voice", get(serve_voice))
        .with_state(config)
        .fallback_service(static_service)
        .layer(CorsLayer::permissive())
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-cache, no-store, must-revalidate"),
        ));

    println!("TMUX Terminal running on http://{}", addr);
    println!("Make sure tmux is running with an active session!");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::{collect_project_dirs, parse_working, validate_kill_target, validate_window_name};

    #[test]
    fn reads_the_live_status_line() {
        let pane = "$ claude\n✻ Wrangling… (10m 40s · ↓ 25.6k tokens)\n";
        let (verb, meta) = parse_working(pane).expect("should see a working line");
        assert_eq!(verb, "Wrangling");
        assert_eq!(meta, "10m 40s · ↓ 25.6k tokens");
    }

    // Claude redraws the status line in place, so when it stops, the live line
    // is gone and only this summary is left. Keying on the verb or the spinner
    // glyph would report the window as busy forever.
    #[test]
    fn ignores_the_past_tense_summary() {
        let pane = "$ claude\n✻ Worked for 3m 16s\n\n> \n";
        assert!(parse_working(pane).is_none());
    }

    #[test]
    fn ignores_a_working_line_left_in_scrollback() {
        let mut pane = String::from("✻ Misting… (1m 32s · ↓ 5.1k tokens)\n");
        for i in 0..40 {
            pane.push_str(&format!("output line {i}\n"));
        }
        assert!(parse_working(&pane).is_none());
    }

    #[test]
    fn reads_the_most_recent_of_several() {
        let pane = "✻ Misting… (1m 32s · ↓ 5.1k tokens)\n✻ Wrangling… (10m 40s · ↓ 25.6k tokens)\n";
        assert_eq!(parse_working(pane).unwrap().0, "Wrangling");
    }

    #[test]
    fn reads_a_multi_word_verb_and_thinking_meta() {
        let pane = "✻ Deep thinking… (2s · ↑ 41 tokens · thought for 4s)\n";
        let (verb, meta) = parse_working(pane).expect("should see a working line");
        assert_eq!(verb, "Deep thinking");
        assert_eq!(meta, "2s · ↑ 41 tokens · thought for 4s");
    }

    #[test]
    fn ignores_an_idle_pane() {
        assert!(parse_working("$ ls\nCargo.toml  src  static\n$ ").is_none());
    }

    #[test]
    fn accepts_valid_names() {
        for n in ["foo", "my-proj", "A2A", "3dmodels", "a.b_c-1", "MASTERtest", "x"] {
            assert!(validate_window_name(n).is_ok(), "should accept {n:?}");
        }
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_window_name("").is_err());
    }

    #[test]
    fn rejects_dot_and_dotdot() {
        assert!(validate_window_name(".").is_err());
        assert!(validate_window_name("..").is_err());
    }

    #[test]
    fn rejects_leading_dash() {
        assert!(validate_window_name("-rf").is_err());
        assert!(validate_window_name("-").is_err());
    }

    #[test]
    fn rejects_shell_and_format_metachars() {
        for n in ["foo bar", "foo;rm", "a#b", "a$(id)", "foo/bar", "back\\slash",
                  "a`b`", "a:b", "a\tb", "a\nb", "qu'ote", "quo\"te"] {
            assert!(validate_window_name(n).is_err(), "should reject {n:?}");
        }
    }

    // --- kill-window target validation ---

    #[test]
    fn kill_rejects_empty_target() {
        // Unlike rename-window, an empty target must NOT fall back to "0":
        // silently killing window 0 is unrecoverable.
        assert!(validate_kill_target("").is_err());
        assert!(validate_kill_target("   ").is_err());
    }

    #[test]
    fn kill_accepts_a_real_target() {
        assert_eq!(validate_kill_target("0:3").unwrap(), "0:3");
        assert_eq!(validate_kill_target(" 0:3 ").unwrap(), "0:3");
    }

    // --- ~/p project directory listing ---

    // No filetime crate in the tree; shell out to `touch -d @<secs>`, which is
    // enough to pin an mtime for an ordering assertion.
    fn touch_dir(base: &std::path::Path, name: &str, mtime_secs: u64) {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::process::Command::new("touch")
            .args(["-m", "-d", &format!("@{}", mtime_secs)])
            .arg(&dir)
            .status()
            .unwrap();
    }

    #[test]
    fn project_dirs_skips_files_and_invalid_names() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        touch_dir(base, "my-proj", 1000);
        touch_dir(base, "auth", 2000);
        // A directory whose name can never be a tmux window name.
        touch_dir(base, "auth?code=ANUh", 3000);
        // Plain files must not be offered as projects.
        std::fs::write(base.join("attack.py"), b"x").unwrap();
        std::fs::write(base.join("awscliv2.zip"), b"x").unwrap();

        let names: Vec<String> = collect_project_dirs(base)
            .into_iter()
            .map(|d| d.name)
            .collect();
        assert_eq!(names, vec!["auth".to_string(), "my-proj".to_string()]);
    }

    #[test]
    fn project_dirs_sorted_by_mtime_desc() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        touch_dir(base, "oldest", 1_700_000_000);
        touch_dir(base, "newest", 1_700_002_000);
        touch_dir(base, "middle", 1_700_001_000);

        let dirs = collect_project_dirs(base);
        let names: Vec<&str> = dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["newest", "middle", "oldest"]);
        assert!(dirs[0].mtime >= dirs[1].mtime);
    }

    #[test]
    fn project_dirs_on_missing_base_is_empty_not_a_panic() {
        let dirs = collect_project_dirs(std::path::Path::new("/nope/does/not/exist"));
        assert!(dirs.is_empty());
    }
}
