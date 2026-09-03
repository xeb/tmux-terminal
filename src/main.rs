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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum AgentKind {
    Codex,
    Agy,
    Eunice,
}

#[derive(Serialize)]
struct CaptureResponse {
    /// Plain text remains the source for prompt parsing, status detection, and
    /// older clients.
    content: String,
    /// The same pane with tmux's SGR attributes preserved. The web client uses
    /// this to reproduce terminal foregrounds and backgrounds (notably Codex's
    /// tinted composer) while mobile clients can safely ignore the new field.
    #[serde(skip_serializing_if = "Option::is_none")]
    styled_content: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    window_closed: bool,
    /// Which agent owns the active TUI, used for a small monochrome identity
    /// badge. Omitted for shells and other programs.
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<AgentKind>,
    /// A live Claude or Codex selection prompt at the tail of the pane.
    /// Optional and additive, so existing clients (including `mobile/`) ignore it.
    #[serde(skip_serializing_if = "Option::is_none")]
    picker: Option<picker::Picker>,
}

fn detect_agent(pane: &str) -> Option<AgentKind> {
    // Content detection avoids a second tmux process on every one-second pane
    // poll. That extra display-message call doubled tmux command traffic and
    // made unrelated operations such as creating/listing windows queue behind
    // capture requests.
    let tail: Vec<&str> = pane.lines().rev().take(80).collect();
    let has_composer = tail.iter().any(|line| line.contains("Ask Codex to do anything"));
    let has_question = tail.iter().any(|line| {
        line.contains("enter to submit answer") || line.trim().starts_with("Question ")
    });
    let has_model_footer = tail
        .iter()
        .any(|line| line.trim_start().starts_with("gpt-") && line.contains(" · /"));
    let has_active_input = tail.iter().take(15).any(|line| line.trim_start().starts_with('›'));
    if has_composer || has_question || (has_model_footer && has_active_input) {
        return Some(AgentKind::Codex);
    }
    if tail.iter().any(|line| agy_footer(line).is_some()) {
        return Some(AgentKind::Agy);
    }
    if tail.iter().any(|line| is_eunice_marker(line)) {
        return Some(AgentKind::Eunice);
    }
    None
}

/// AGY (the Antigravity CLI) keeps one status line at the bottom of its screen:
///
///     ? for shortcuts                      Gemini 3.8 Flash · high · 1 task(s) · /tasks
///
/// and swaps the left-hand hint for `esc to cancel` while the agent is busy.
/// Claude Code prints the same `? for shortcuts` hint, so the `model · effort`
/// tail is what identifies AGY. Returns the hint when the line is AGY's.
fn agy_footer(line: &str) -> Option<&str> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(\? for shortcuts|esc to cancel)\s{2,}\S.*·\s*(?:high|medium|low)\b")
            .expect("static regex")
    });
    re.captures(line).map(|caps| caps.get(1).map_or("", |m| m.as_str()))
}

/// AGY's spinner line while a turn runs: a braille glyph, then what it is doing.
///
///     ⣟  Generating...
///     ⣟  Running command...
fn agy_spinner_verb(line: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^\s*[\u2800-\u28FF]\s+(\S.*?)\.\.\.\s*$").expect("static regex")
    });
    re.captures(line).map(|caps| caps[1].trim().to_string())
}

/// Lines only EUNICE's TUI draws: its banner, the rule above its composer, the
/// composer footer, and its tool-call arrow.
fn is_eunice_marker(line: &str) -> bool {
    static TOOL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let tool = TOOL.get_or_init(|| regex::Regex::new(r"^\s*→ [A-Za-z_][\w-]*\s*$").expect("static regex"));
    let trimmed = line.trim();
    is_eunice_footer(line)
        || line.contains("/quit or Ctrl+D to exit")
        || (trimmed.starts_with('─') && trimmed.ends_with(" eunice"))
        || tool.is_match(line)
}

fn is_eunice_footer(line: &str) -> bool {
    line.contains("↵ send") && line.contains("esc clear")
}

/// EUNICE prints `✻ Thinking…` once when a turn starts and never redraws it,
/// so the line alone cannot say whether the turn is still running.
fn is_eunice_thinking(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() > 1
        && trimmed.starts_with(['✻', '✶', '✺', '✹', '✷'])
        && trimmed[trimmed.chars().next().map_or(0, char::len_utf8)..].trim() == "Thinking…"
}

/// Strip terminal control sequences while preserving every displayed byte.
/// `tmux capture-pane -e` emits SGR attributes so the browser can paint the
/// pane faithfully; the picker and status parsers still need the exact plain
/// text shape they consumed before styled capture was added.
fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            output.push(bytes[i]);
            i += 1;
            continue;
        }

        if i + 1 >= bytes.len() {
            break;
        }
        match bytes[i + 1] {
            b'[' => {
                // CSI: parameters/intermediates followed by one final byte.
                i += 2;
                while i < bytes.len() {
                    let byte = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            b']' => {
                // OSC: terminated by BEL or String Terminator (ESC backslash).
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/' => {
                // Charset selection: ESC, intermediate, final byte.
                i = (i + 3).min(bytes.len());
            }
            _ => {
                // Other two-byte ESC sequences are not displayed either.
                i += 2;
            }
        }
    }
    String::from_utf8(output).expect("removing ASCII escapes preserves UTF-8")
}

async fn capture_pane(Json(payload): Json<CaptureRequest>) -> impl IntoResponse {
    let target = if payload.target.is_empty() {
        "0".to_string()
    } else {
        payload.target
    };

    // Capture with terminal attributes. Plain text is derived from this one
    // snapshot so what the user sees and what the picker verifies cannot drift.
    let result = Command::new("tmux")
        .args(["capture-pane", "-p", "-e", "-t", &target, "-S", "-1000"])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let styled_content = String::from_utf8_lossy(&output.stdout).to_string();
                let content = strip_ansi(&styled_content);
                let agent = detect_agent(&content);
                // Parsed server-side and only here. If the client parsed too, the
                // renderer and the committer would drift, and the failure mode is
                // a card that shows one option and sends another.
                let picker = picker::parse(&content);
                (StatusCode::OK, Json(CaptureResponse {
                    content,
                    styled_content: Some(styled_content),
                    window_closed: false,
                    agent,
                    picker,
                }))
            } else {
                (StatusCode::OK, Json(CaptureResponse {
                    content: String::new(),
                    styled_content: None,
                    window_closed: true,
                    agent: None,
                    picker: None,
                }))
            }
        }
        Err(_) => (StatusCode::OK, Json(CaptureResponse {
            content: String::new(),
            styled_content: None,
            window_closed: true,
            agent: None,
            picker: None,
        })),
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
    /// The agent's live status verb ("Wrangling" or "Working"), absent when idle.
    #[serde(skip_serializing_if = "Option::is_none")]
    verb: Option<String>,
    /// Elapsed time and useful detail from the same live status line.
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
/// Codex renders its live state as:
///
///     • Working (4m 21s • esc to interrupt) · 1 background terminal running
///
/// Both formats have a live timer. That is the discriminator that keeps stale
/// transcript summaries from reporting a finished window as busy forever.
/// Mirrors the working-line parsers in static/index.html; the two must stay in
/// step.
fn parse_working(pane: &str) -> Option<(String, String)> {
    let claude_re = regex::Regex::new(
        r"(?:^|\s)([A-Za-z][A-Za-z ]{0,20})…\s*\(([^)]*\b\d+s\b[^)]*)\)",
    )
    .ok()?;
    let codex_re = regex::Regex::new(
        r"^\s*(?:•\s*)?Working\s+\(([^)]*\b\d+s\b[^)]*)\)(?:\s*·\s*(.*))?\s*$",
    )
    .ok()?;
    // Only the tail: the same line from an earlier turn is still in scrollback,
    // and matching it would pin every window on permanently. Blank rows do not
    // count towards it: tmux prints every row of the pane, so a status line
    // drawn near the top of a tall window sits above dozens of empty ones.
    let lines: Vec<&str> = pane.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(30);
    let tail = &lines[start..];
    if let Some(found) = parse_agy_working(tail) {
        return Some(found);
    }
    if let Some(found) = parse_eunice_working(pane, tail) {
        return Some(found);
    }
    for line in tail.iter().rev() {
        if let Some(caps) = claude_re.captures(line) {
            return Some((
                caps[1].trim().to_string(),
                caps[2].trim().to_string(),
            ));
        }
        if let Some(caps) = codex_re.captures(line) {
            // Keyboard and slash-command hints are useful in tmux but noisy in
            // the compact web indicator. Keep time and actual task state only.
            let elapsed = caps[1].split('•').next()?.trim();
            let mut meta = vec![elapsed.to_string()];
            if let Some(suffix) = caps.get(2) {
                meta.extend(
                    suffix
                        .as_str()
                        .split(" · ")
                        .map(str::trim)
                        .filter(|part| !part.is_empty() && !part.starts_with('/'))
                        .map(str::to_string),
                );
            }
            return Some(("Working".to_string(), meta.join(" · ")));
        }
    }
    None
}

/// AGY has no live timer. Its status footer is redrawn in place, so the hint it
/// shows right now is the truth: `esc to cancel` means a turn is running,
/// `? for shortcuts` means idle even if a spinner line lingers above. Background
/// tasks listed in the footer are not the agent working.
fn parse_agy_working(tail: &[&str]) -> Option<(String, String)> {
    let hint = tail.iter().rev().find_map(|line| agy_footer(line))?;
    if hint != "esc to cancel" {
        return None;
    }
    let verb = tail
        .iter()
        .rev()
        .find_map(|line| agy_spinner_verb(line))
        .unwrap_or_else(|| "Working".to_string());
    Some((verb, String::new()))
}

/// EUNICE hides its composer while a turn runs and draws it again when done, so
/// a `Thinking…` line with no composer footer below it means still working.
/// Something on screen must identify EUNICE first: other agents print a similar
/// glyph, and a bare `Thinking…` in their transcript would otherwise stick.
fn parse_eunice_working(pane: &str, tail: &[&str]) -> Option<(String, String)> {
    if !pane.lines().any(is_eunice_marker) {
        return None;
    }
    let thinking = tail.iter().rposition(|line| is_eunice_thinking(line))?;
    if tail[thinking + 1..].iter().any(|line| is_eunice_footer(line)) {
        return None;
    }
    Some(("Thinking".to_string(), String::new()))
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

/// The coding agents a new window can start. Every one launches with approvals
/// bypassed so the window never sits waiting on a permission prompt:
/// `claude`/`agy` take `--dangerously-skip-permissions`, `codex` takes its
/// `--yolo` alias, and `eunice` has no approval prompts at all, so it runs bare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Agent {
    Claude,
    #[default]
    Codex,
    Agy,
    Eunice,
}

impl Agent {
    fn parse(raw: &str) -> Option<Agent> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(Agent::Claude),
            "codex" => Some(Agent::Codex),
            "agy" => Some(Agent::Agy),
            "eunice" => Some(Agent::Eunice),
            _ => None,
        }
    }

    /// The lowercase name clients send and receive.
    fn name(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Agy => "agy",
            Agent::Eunice => "eunice",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Agent::Claude => "claude --dangerously-skip-permissions",
            Agent::Codex => "codex --yolo",
            Agent::Agy => "agy --dangerously-skip-permissions",
            Agent::Eunice => "eunice",
        }
    }
}

/// The exact text typed into the new window's shell. Only EUNICE takes a
/// model: the others pick theirs from their own configuration.
fn launch_command(agent: Agent, model: Option<&str>) -> String {
    match (agent, model) {
        (Agent::Eunice, Some(model)) => format!("eunice --model {}", model),
        _ => agent.command().to_string(),
    }
}

/// Model ids are typed into a shell, so only the characters that appear in
/// `eunice --list-models` output are allowed: `hf:gemma4:e4b`, `gpt-5.6-sol`.
fn validate_model(raw: &str) -> Result<String, String> {
    let model = raw.trim();
    if model.is_empty() {
        return Err("model must not be empty".to_string());
    }
    if model.len() > 120 {
        return Err("model id is too long".to_string());
    }
    if model.starts_with('-') {
        return Err("model must not start with '-'".to_string());
    }
    if !model
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
    {
        return Err("model may contain only letters, digits, '.', '_', ':', '-'".to_string());
    }
    Ok(model.to_string())
}

/// A blank model means "let the agent choose". A model for any agent other
/// than EUNICE is a client bug, not something to pass through silently.
fn requested_model(agent: Agent, raw: Option<&str>) -> Result<Option<String>, String> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(_) if agent != Agent::Eunice => Err("model applies to eunice only".to_string()),
        Some(model) => validate_model(model).map(Some),
    }
}

/// One model as `eunice --list-models` describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct EuniceModel {
    provider: String,
    id: String,
    aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    /// Supports function calling. Per model where the list says so (Ollama),
    /// otherwise inherited from the provider heading.
    tools: bool,
}

/// Parse the human-readable model list:
///
/// ```text
/// 💎 Gemini (available) ✓  ...Yog0
///    - gemini-3.8-flash, flash (default)
/// 🦙 Ollama (available)  running
///    - deepseek-r1:14b ✓
///    - llava:34b
/// ```
///
/// Only models from available providers are offered, and template rows such as
/// `azure:<deployment-name>` are skipped because they are not real ids.
fn parse_eunice_models(output: &str) -> Vec<EuniceModel> {
    struct Provider {
        name: String,
        available: bool,
        tools: bool,
    }
    let mut models = Vec::new();
    let mut provider: Option<Provider> = None;
    for raw in output.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("- ").filter(|_| line.starts_with(' ')) {
            let Some(current) = &provider else { continue };
            if !current.available {
                continue;
            }
            let mut text = rest.trim();
            let mut tools = current.tools;
            if let Some(stripped) = text.strip_suffix('✓') {
                tools = true;
                text = stripped.trim_end();
            }
            let (ids, note) = match text.find(" (") {
                Some(i) if text.ends_with(')') => {
                    (text[..i].trim(), Some(text[i + 2..text.len() - 1].to_string()))
                }
                _ => (text, None),
            };
            let mut parts = ids.split(',').map(str::trim).filter(|s| !s.is_empty());
            let Some(id) = parts.next() else { continue };
            if id.contains('<') {
                continue;
            }
            models.push(EuniceModel {
                provider: current.name.clone(),
                id: id.to_string(),
                aliases: parts.map(str::to_string).collect(),
                note,
                tools,
            });
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(i) = line.find(" (available)").or_else(|| line.find(" (unavailable)")) else {
            continue;
        };
        provider = Some(Provider {
            name: line[..i]
                .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
                .trim()
                .to_string(),
            available: line[i..].starts_with(" (available)"),
            tools: line[i..].contains('✓'),
        });
    }
    models
}

/// The list the modal's model picker filters. Runs `eunice --list-models`
/// fresh each time: providers appear and disappear with keys and daemons.
///
/// Through an interactive shell on purpose. The API keys that make providers
/// available are exported from ~/.bashrc, which only runs for interactive
/// shells, and the new window is one — so this is the list EUNICE itself
/// will see there. The service's own environment has none of those keys.
async fn eunice_models() -> impl IntoResponse {
    let run = tokio::process::Command::new("bash")
        .args(["-ic", "eunice --list-models"])
        .stdin(std::process::Stdio::null())
        // An interactive bash with no terminal complains about job control
        // on stderr; that noise is not an error.
        .stderr(std::process::Stdio::null())
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(20), run).await {
        Ok(Ok(output)) if output.status.success() => (
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "models": parse_eunice_models(&String::from_utf8_lossy(&output.stdout)),
            })),
        ),
        Ok(Ok(output)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })),
        ),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": format!("could not run eunice: {}", error)})),
        ),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({"success": false, "error": "eunice --list-models timed out"})),
        ),
    }
}

/// An absent or blank `agent` field keeps the historical default, so older
/// clients (the mobile app) keep getting Codex windows.
fn requested_agent(raw: Option<&str>) -> Result<Agent, String> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(Agent::default()),
        Some(name) => Agent::parse(name).ok_or_else(|| format!("unknown agent: {}", name)),
    }
}

/// New windows go to session `0` unless the client says otherwise. The attached
/// `MASTER` session holds the backend control processes, and tmux would put an
/// untargeted `new-window` there simply because it is the attached one.
const DEFAULT_SESSION: &str = "0";

fn requested_session(raw: Option<&str>) -> Result<String, String> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(DEFAULT_SESSION.to_string()),
        Some(name) => validate_window_name(name)
            .map(|()| name.to_string())
            .map_err(|e| format!("invalid session: {}", e)),
    }
}

/// `session:` with no window part makes tmux append at the next free index.
fn session_target(session: &str) -> String {
    format!("{}:", session)
}

/// Instruction files the other agents read. Each becomes a symlink to the
/// project's CLAUDE.md so the rules live in exactly one place.
const INSTRUCTION_LINKS: [&str; 2] = ["AGENTS.md", "GEMINI.md"];

/// Guarantee every agent's instruction file exists when CLAUDE.md does, without
/// ever touching a file or symlink that is already there. Returns the names
/// created this time.
fn ensure_instruction_links(project_dir: &std::path::Path) -> Result<Vec<&'static str>, String> {
    let mut created = Vec::new();
    for name in INSTRUCTION_LINKS {
        if ensure_link(project_dir, name)? {
            created.push(name);
        }
    }
    Ok(created)
}

/// `symlink_metadata` deliberately does not follow the destination: a broken
/// symlink is still an existing entry and must not be replaced.
fn ensure_link(project_dir: &std::path::Path, name: &str) -> Result<bool, String> {
    let claude = project_dir.join("CLAUDE.md");
    let agents = project_dir.join(name);

    match std::fs::metadata(&claude) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!("could not inspect {}: {}", claude.display(), error));
        }
    }

    match std::fs::symlink_metadata(&agents) {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!("could not inspect {}: {}", agents.display(), error));
        }
    }

    match std::os::unix::fs::symlink(std::path::Path::new("./CLAUDE.md"), &agents) {
        Ok(()) => Ok(true),
        // Another creator may have won the check/create race. The required
        // postcondition is satisfied and, critically, nothing was overwritten.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(format!("could not create {}: {}", agents.display(), error)),
    }
}

fn read_pane_cwd(target: &str) -> Result<std::path::PathBuf, String> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", "-t", target, "#{pane_current_path}"])
        .output()
        .map_err(|error| format!("could not inspect new window: {}", error))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Err("new window reported an empty working directory".to_string());
    }
    Ok(std::path::PathBuf::from(path))
}

/// Start the chosen agent in a freshly-created interactive shell. Keep the
/// literal payload and Enter in separate tmux calls: with `-l`, putting Enter
/// in the same argv would type the word rather than press the key.
fn launch_agent(target: &str, agent: Agent, model: Option<&str>) -> Result<(), String> {
    send_keys_literal(target, &launch_command(agent, model))?;
    send_keys(target, &["Enter".to_string()])
}

/// Claude, Codex and AGY each ask "do you trust this folder?" the first time
/// they start in a directory, and nothing happens until someone answers. The
/// pane shows the option rows with the terminal's own cursor glyph on the
/// highlighted one; the answer is the key presses that move that cursor onto
/// the yes row and confirm. Fails closed: any other question returns `None`.
fn trust_prompt_answer(pane: &str) -> Option<Vec<&'static str>> {
    const YES: [&str; 2] = ["Yes, I trust this folder", "Yes, continue"];
    const NO: [&str; 2] = ["No, exit", "No, quit"];
    // Blank rows are skipped for the same reason as in `parse_working`: the
    // dialog is drawn at the top of a fresh window, above a screen of them.
    let lines: Vec<&str> = pane.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(25);
    let tail = &lines[start..];
    let asks = tail.iter().any(|line| {
        line.contains("Do you trust the contents of this") || line.contains("Quick safety check:")
    });
    if !asks {
        return None;
    }
    let yes = tail.iter().rposition(|line| is_trust_option(line, &YES))?;
    let no = tail.iter().rposition(|line| is_trust_option(line, &NO))?;
    let has_cursor = |index: usize| tail[index].trim_start().starts_with(['❯', '›', '>']);
    if has_cursor(yes) {
        Some(vec!["Enter"])
    } else if has_cursor(no) {
        Some(vec![if yes > no { "Down" } else { "Up" }, "Enter"])
    } else {
        None
    }
}

fn is_trust_option(line: &str, labels: &[&str]) -> bool {
    let text = line.trim().trim_start_matches(['❯', '›', '>']).trim_start();
    // Codex numbers its rows ("1. Yes, continue"); Claude and AGY do not.
    let text = text
        .strip_prefix(|c: char| c.is_ascii_digit())
        .and_then(|rest| rest.strip_prefix(". "))
        .unwrap_or(text);
    labels.iter().any(|label| text.starts_with(label))
}

/// Watch a just-launched window briefly and answer its trust prompt if one
/// appears. The window was created for one of the user's own projects, so yes
/// is always the right answer. Gives up quietly once the window is gone or
/// the agent has started without asking.
///
/// One key per tick, re-read the screen, repeat. Claude Code drops arrow keys
/// that arrive while it is still probing the terminal, and Enter on its
/// default "No, exit" row quits the agent — so Enter is only ever sent after a
/// fresh capture shows the cursor already on the yes row.
fn auto_accept_trust_prompt(target: String) {
    tokio::spawn(async move {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let Some(pane) = capture_visible(&target) else {
                return;
            };
            let Some(keys) = trust_prompt_answer(&pane) else {
                continue;
            };
            let key = keys[0];
            if send_keys(&target, &[key.to_string()]).is_err() {
                return;
            }
            if key == "Enter" {
                return;
            }
        }
    });
}

#[derive(Deserialize, Default)]
struct NewWindowRequest {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    session: Option<String>,
    /// EUNICE only: passed as `--model`.
    #[serde(default)]
    model: Option<String>,
}

/// The body is optional: the mobile app posts with no body and expects the
/// default agent in the default session.
async fn new_window(body: Option<Json<NewWindowRequest>>) -> impl IntoResponse {
    let payload = body.map(|Json(p)| p).unwrap_or_default();
    let agent = match requested_agent(payload.agent.as_deref()) {
        Ok(agent) => agent,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"success": false, "error": error})),
            );
        }
    };
    let session = match requested_session(payload.session.as_deref()) {
        Ok(session) => session,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"success": false, "error": error})),
            );
        }
    };
    let model = match requested_model(agent, payload.model.as_deref()) {
        Ok(model) => model,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"success": false, "error": error})),
            );
        }
    };
    let result = Command::new("tmux")
        .args([
            "new-window", "-t", &session_target(&session), "-P", "-F",
            "#{session_name}:#{window_index}",
        ])
        .output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let target = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let project_dir = match read_pane_cwd(&target) {
                    Ok(path) => path,
                    Err(error) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "success": false,
                                "target": target,
                                "error": error,
                            })),
                        );
                    }
                };
                if let Err(error) = ensure_instruction_links(&project_dir) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "success": false,
                            "target": target,
                            "error": error,
                        })),
                    );
                }
                if let Err(error) = launch_agent(&target, agent, model.as_deref()) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "success": false,
                            "target": target,
                            "error": error,
                        })),
                    );
                }
                auto_accept_trust_prompt(target.clone());
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "success": true,
                        "target": target,
                        "agent": agent.name(),
                        "session": session,
                        "command": launch_command(agent, model.as_deref()),
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
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    session: Option<String>,
    /// EUNICE only: passed as `--model`.
    #[serde(default)]
    model: Option<String>,
}

async fn new_window_named(Json(payload): Json<NewWindowNamedRequest>) -> impl IntoResponse {
    let name = payload.name.trim().to_string();

    if let Err(e) = validate_window_name(&name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"success": false, "error": e})),
        );
    }
    let agent = match requested_agent(payload.agent.as_deref()) {
        Ok(agent) => agent,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"success": false, "error": error})),
            );
        }
    };
    let session = match requested_session(payload.session.as_deref()) {
        Ok(session) => session,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"success": false, "error": error})),
            );
        }
    };
    let model = match requested_model(agent, payload.model.as_deref()) {
        Ok(model) => model,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"success": false, "error": error})),
            );
        }
    };

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

    // 3. Create the window IN that dir, in the requested session. -n makes the
    //    name permanent; -P -F returns the stable window_id (targeting by name
    //    would hit the oldest duplicate).
    let create = Command::new("tmux")
        .args([
            "new-window", "-t", &session_target(&session), "-c", &dir, "-n", &name, "-P", "-F",
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

    // 4. Verify the pane actually started in <dir> BEFORE launching an agent
    //    with approvals bypassed. Never fire it in an unintended dir.
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

    // 5. Point the other agents' instruction files at CLAUDE.md when they do
    //    not already exist. Never replace an existing file or symlink.
    if let Err(error) = ensure_instruction_links(&expected) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": error,
                "window_id": window_id,
                "target": target,
            })),
        );
    }

    // 6. Launch the chosen agent only after the instruction links are ready,
    //    then answer its first-launch trust prompt if it shows one.
    if let Err(error) = launch_agent(&window_id, agent, model.as_deref()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": error,
                "window_id": window_id,
                "target": target,
            })),
        );
    }
    auto_accept_trust_prompt(window_id.clone());

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true, "existing": false,
            "window_id": window_id, "target": target,
            "agent": agent.name(), "session": session,
            "command": launch_command(agent, model.as_deref()),
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

#[derive(Clone, Serialize)]
struct ProjectDir {
    name: String,
    mtime: u64,
}

#[derive(Serialize)]
struct ProjectDirsResponse {
    dirs: Vec<ProjectDir>,
}

const PROJECT_DIR_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

struct ProjectDirCache {
    dirs: Vec<ProjectDir>,
    refreshed_at: std::time::Instant,
    refreshing: bool,
}

static PROJECT_DIR_CACHE: std::sync::OnceLock<std::sync::Mutex<ProjectDirCache>> =
    std::sync::OnceLock::new();

fn project_dirs_base() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    std::path::PathBuf::from(format!("{}/p", home))
}

fn project_dir_cache() -> &'static std::sync::Mutex<ProjectDirCache> {
    PROJECT_DIR_CACHE.get_or_init(|| {
        std::sync::Mutex::new(ProjectDirCache {
            dirs: Vec::new(),
            refreshed_at: std::time::Instant::now() - PROJECT_DIR_CACHE_TTL,
            refreshing: false,
        })
    })
}

fn lock_project_dir_cache() -> std::sync::MutexGuard<'static, ProjectDirCache> {
    project_dir_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Pay the slow filesystem scan once during service startup. `~/p` lives on a
/// filesystem where reading metadata for ~200 projects can take several
/// seconds; doing that in the modal request made RECENT appear broken.
fn prime_project_dir_cache() {
    let dirs = collect_project_dirs(&project_dirs_base());
    let mut cache = lock_project_dir_cache();
    cache.dirs = dirs;
    cache.refreshed_at = std::time::Instant::now();
    cache.refreshing = false;
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
            // fs::metadata, not entry.metadata(): the latter is an lstat and
            // would drop every symlinked project (~/p/body -> ~/p/health/body).
            // Following also skips dangling links, whose stat just fails.
            let meta = std::fs::metadata(entry.path()).ok()?;
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
    let (dirs, should_refresh) = {
        let mut cache = lock_project_dir_cache();
        let stale = cache.refreshed_at.elapsed() >= PROJECT_DIR_CACHE_TTL;
        let should_refresh = stale && !cache.refreshing;
        if should_refresh {
            cache.refreshing = true;
        }
        (cache.dirs.clone(), should_refresh)
    };

    // Stale-while-refresh: a modal always gets the last complete list
    // immediately. Only the background worker pays the slow metadata scan.
    if should_refresh {
        tokio::task::spawn_blocking(|| {
            let refreshed = collect_project_dirs(&project_dirs_base());
            let mut cache = lock_project_dir_cache();
            cache.dirs = refreshed;
            cache.refreshed_at = std::time::Instant::now();
            cache.refreshing = false;
        });
    }

    (
        StatusCode::OK,
        Json(ProjectDirsResponse { dirs }),
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
        // Create the internal bug-fix window with the same default agent and
        // session used by the interactive new-window flow.
        let _ = Command::new("tmux")
            .args(["new-window", "-t", &session_target(DEFAULT_SESSION), "-n", window_name])
            .output();
        let project_dir = "/media/xeb/GreyArea/projects/tmux-terminal";
        if let Err(error) = ensure_instruction_links(std::path::Path::new(project_dir)) {
            eprintln!("Could not prepare agent instruction links: {}", error);
            return;
        }
        let start_cmd = format!("cd {} && {}", project_dir, Agent::default().command());
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", window_name, &start_cmd, "Enter"])
            .output();
        // Give Codex a moment to start.
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

    // Build the expensive RECENT-project list before accepting requests. All
    // later refreshes are stale-while-refresh and never hold up the modal.
    prime_project_dir_cache();

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
        .route("/api/eunice-models", get(eunice_models))
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
    use super::{
        collect_project_dirs, detect_agent, ensure_instruction_links, launch_command,
        parse_eunice_models, parse_working, requested_agent, requested_model,
        requested_session, strip_ansi, trust_prompt_answer, validate_kill_target,
        validate_model, validate_window_name, Agent, AgentKind,
    };

    #[test]
    fn a_missing_or_blank_agent_field_means_codex() {
        assert_eq!(requested_agent(None).unwrap(), Agent::Codex);
        assert_eq!(requested_agent(Some("  ")).unwrap(), Agent::Codex);
        assert_eq!(requested_agent(Some("Claude")).unwrap(), Agent::Claude);
        assert!(requested_agent(Some("vim")).is_err());
    }

    // --- agent selection ---

    #[test]
    fn new_windows_default_to_codex() {
        assert_eq!(Agent::default(), Agent::Codex);
        assert_eq!(Agent::default().command(), "codex --yolo");
    }

    #[test]
    fn every_agent_launches_in_yolo_mode() {
        assert_eq!(Agent::Claude.command(), "claude --dangerously-skip-permissions");
        assert_eq!(Agent::Codex.command(), "codex --yolo");
        assert_eq!(Agent::Agy.command(), "agy --dangerously-skip-permissions");
        // EUNICE has no approval prompts, so there is nothing to bypass.
        assert_eq!(Agent::Eunice.command(), "eunice");
    }

    #[test]
    fn parses_agent_names_case_insensitively() {
        assert_eq!(Agent::parse("claude"), Some(Agent::Claude));
        assert_eq!(Agent::parse("Codex"), Some(Agent::Codex));
        assert_eq!(Agent::parse("AGY"), Some(Agent::Agy));
        assert_eq!(Agent::parse(" eunice "), Some(Agent::Eunice));
        assert_eq!(Agent::parse("vim"), None);
        assert_eq!(Agent::parse(""), None);
    }

    #[test]
    fn agent_names_round_trip_for_the_client() {
        for agent in [Agent::Claude, Agent::Codex, Agent::Agy, Agent::Eunice] {
            assert_eq!(Agent::parse(agent.name()), Some(agent));
        }
    }

    // --- EUNICE model list, from a real `eunice --list-models` ---

    const EUNICE_MODELS: &str = include_str!("../tests/fixtures/eunice/list_models.txt");

    #[test]
    fn lists_models_from_available_providers_only() {
        let models = parse_eunice_models(EUNICE_MODELS);
        assert!(models.iter().any(|m| m.id == "gemini-3.8-flash" && m.provider == "Gemini"));
        // Anthropic is listed but has no key, so none of its models are offered.
        assert!(models.iter().all(|m| m.provider != "Anthropic"));
        assert!(models.iter().all(|m| !m.id.contains('<')), "templates are not selectable");
        let providers: Vec<&str> = models.iter().map(|m| m.provider.as_str()).fold(Vec::new(), |mut acc, p| {
            if acc.last() != Some(&p) {
                acc.push(p);
            }
            acc
        });
        assert_eq!(providers, vec!["Abliteration AI", "Cerebras", "Gemini", "Local", "Ollama", "OpenAI"]);
    }

    #[test]
    fn splits_aliases_and_notes() {
        let models = parse_eunice_models(EUNICE_MODELS);
        let flash = models.iter().find(|m| m.id == "gemini-3.8-flash").unwrap();
        assert_eq!(flash.aliases, vec!["flash"]);
        assert_eq!(flash.note.as_deref(), Some("default"));
        assert!(flash.tools);
        let sol = models.iter().find(|m| m.id == "gpt-5.6").unwrap();
        assert_eq!(sol.aliases, vec!["gpt-5.6-sol"]);
        assert_eq!(sol.note.as_deref(), Some("default/flagship"));
        let local = models.iter().find(|m| m.id == "hf:gemma4:26b-q8").unwrap();
        assert_eq!(local.note.as_deref(), Some("Gemma 4 26B Q8_0, ~28 GB"));
        assert!(local.aliases.is_empty(), "commas inside the note are not aliases");
    }

    #[test]
    fn reads_per_model_tool_support() {
        let models = parse_eunice_models(EUNICE_MODELS);
        let with = models.iter().find(|m| m.id == "deepseek-r1:14b").unwrap();
        let without = models.iter().find(|m| m.id == "llava:34b").unwrap();
        assert!(with.tools);
        assert!(!without.tools);
        assert_eq!(with.provider, "Ollama");
        assert!(with.aliases.is_empty() && with.note.is_none());
    }

    #[test]
    fn model_ids_are_shell_safe() {
        assert_eq!(validate_model("gemini-3.8-flash").unwrap(), "gemini-3.8-flash");
        assert_eq!(validate_model(" hf:gemma4:e4b ").unwrap(), "hf:gemma4:e4b");
        for bad in ["", "  ", "-x", "a b", "a;b", "$(id)", "a/b", "azure:<name>"] {
            assert!(validate_model(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn eunice_launches_with_the_chosen_model() {
        assert_eq!(launch_command(Agent::Eunice, Some("hf:gemma4:e4b")), "eunice --model hf:gemma4:e4b");
        assert_eq!(launch_command(Agent::Eunice, None), "eunice");
        assert_eq!(launch_command(Agent::Codex, None), "codex --yolo");
    }

    #[test]
    fn a_model_only_applies_to_eunice() {
        assert_eq!(requested_model(Agent::Eunice, Some("flash")).unwrap(), Some("flash".to_string()));
        assert_eq!(requested_model(Agent::Eunice, Some("  ")).unwrap(), None);
        assert_eq!(requested_model(Agent::Claude, None).unwrap(), None);
        assert!(requested_model(Agent::Claude, Some("flash")).is_err());
        assert!(requested_model(Agent::Eunice, Some("a b")).is_err());
    }

    // --- session selection ---

    #[test]
    fn new_windows_default_to_session_zero() {
        assert_eq!(requested_session(None).unwrap(), "0");
        assert_eq!(requested_session(Some("")).unwrap(), "0");
        assert_eq!(requested_session(Some("  ")).unwrap(), "0");
    }

    #[test]
    fn accepts_a_named_session() {
        assert_eq!(requested_session(Some("MASTER")).unwrap(), "MASTER");
        assert_eq!(requested_session(Some(" 0 ")).unwrap(), "0");
    }

    #[test]
    fn rejects_session_names_tmux_would_misparse() {
        for s in ["a:b", "foo bar", "-x", "a/c", "$(id)"] {
            assert!(requested_session(Some(s)).is_err(), "should reject {s:?}");
        }
    }

    // --- instruction links ---

    #[test]
    fn creates_agents_and_gemini_links_to_existing_claude_instructions() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("CLAUDE.md"), "project rules\n").unwrap();

        let created = ensure_instruction_links(project.path()).unwrap();
        assert_eq!(created, vec!["AGENTS.md", "GEMINI.md"]);
        for name in ["AGENTS.md", "GEMINI.md"] {
            let link = project.path().join(name);
            assert_eq!(
                std::fs::read_link(&link).unwrap(),
                std::path::PathBuf::from("./CLAUDE.md"),
                "{name} should point at CLAUDE.md"
            );
            assert_eq!(std::fs::read_to_string(link).unwrap(), "project rules\n");
        }
    }

    #[test]
    fn does_not_create_links_without_claude_instructions() {
        let project = tempfile::tempdir().unwrap();
        assert!(ensure_instruction_links(project.path()).unwrap().is_empty());
        assert!(std::fs::symlink_metadata(project.path().join("AGENTS.md")).is_err());
        assert!(std::fs::symlink_metadata(project.path().join("GEMINI.md")).is_err());
    }

    #[test]
    fn never_replaces_an_existing_agents_file() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("CLAUDE.md"), "claude rules\n").unwrap();
        std::fs::write(project.path().join("AGENTS.md"), "codex rules\n").unwrap();

        let created = ensure_instruction_links(project.path()).unwrap();
        assert_eq!(created, vec!["GEMINI.md"]);
        assert_eq!(
            std::fs::read_to_string(project.path().join("AGENTS.md")).unwrap(),
            "codex rules\n"
        );
    }

    #[test]
    fn never_replaces_a_broken_gemini_symlink() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("CLAUDE.md"), "claude rules\n").unwrap();
        let gemini = project.path().join("GEMINI.md");
        std::os::unix::fs::symlink("./missing.md", &gemini).unwrap();

        let created = ensure_instruction_links(project.path()).unwrap();
        assert_eq!(created, vec!["AGENTS.md"]);
        assert_eq!(
            std::fs::read_link(gemini).unwrap(),
            std::path::PathBuf::from("./missing.md")
        );
    }

    #[test]
    fn creating_links_twice_is_a_no_op() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("CLAUDE.md"), "claude rules\n").unwrap();
        assert_eq!(ensure_instruction_links(project.path()).unwrap().len(), 2);
        assert!(ensure_instruction_links(project.path()).unwrap().is_empty());
    }

    // --- first-launch trust prompts, captured from real panes ---

    const CLAUDE_TRUST: &str = "\
 Accessing workspace:
 /media/xeb/GreyArea/projects/zz-trust-git-claude
 Quick safety check: Is this a project you created or one you trust? (Like your
 own code, a well-known open source project, or work from your team). If not,
 take a moment to review what's in this folder first.
 Claude Code'll be able to read, edit, and execute files here.
 Security guide
 ❯ No, exit
   Yes, I trust this folder
 Enter to confirm · Esc to cancel
";

    const CODEX_TRUST: &str = "\
> You are in /media/xeb/GreyArea/projects/zz-trust-plain-codex
  Do you trust the contents of this directory? Working with untrusted contents
  comes with higher risk of prompt injection. Trusting the directory allows
  project-local config, hooks, and exec policies to load.
› 1. Yes, continue
  2. No, quit
  Press enter to continue
";

    const AGY_TRUST: &str = "\
Accessing workspace:
/home/xeb/p/zz-trust-plain-agy
Do you trust the contents of this project?
Antigravity CLI requires permission to read, edit, and execute files here.
> Yes, I trust this folder
  No, exit
  ↑/↓ Navigate · enter Confirm
                                                          Gemini 3.8 Flash · high
";

    #[test]
    fn accepts_claude_trust_prompt_by_moving_off_the_default_no() {
        assert_eq!(trust_prompt_answer(CLAUDE_TRUST), Some(vec!["Down", "Enter"]));
    }

    #[test]
    fn accepts_codex_trust_prompt_with_enter() {
        assert_eq!(trust_prompt_answer(CODEX_TRUST), Some(vec!["Enter"]));
    }

    #[test]
    fn accepts_agy_trust_prompt_with_enter() {
        assert_eq!(trust_prompt_answer(AGY_TRUST), Some(vec!["Enter"]));
    }

    #[test]
    fn moves_up_when_yes_is_above_the_cursor() {
        let pane = "Do you trust the contents of this project?\n  Yes, I trust this folder\n> No, exit\n";
        assert_eq!(trust_prompt_answer(pane), Some(vec!["Up", "Enter"]));
    }

    #[test]
    fn leaves_other_questions_alone() {
        let picker = "──────────────────────\n ☐ Allowlist\n\nWhich addresses should I add?\n\n❯ 1. Both real ones\n  2. Neither\n\nEnter to select · ↑/↓ to navigate · Esc to cancel\n";
        assert!(trust_prompt_answer(picker).is_none());
        assert!(trust_prompt_answer("$ ls\nsrc\n$ ").is_none());
        // The words alone, without a highlighted yes/no pair, are transcript text.
        assert!(trust_prompt_answer("I trust this folder is fine.\n❯ \n").is_none());
    }

    // tmux prints every row of the pane, and a dialog drawn at the top of a
    // tall window is followed by dozens of empty rows.
    #[test]
    fn sees_a_trust_prompt_above_a_screenful_of_blank_rows() {
        let pane = format!("{}{}", CLAUDE_TRUST, "\n".repeat(50));
        assert_eq!(trust_prompt_answer(&pane), Some(vec!["Down", "Enter"]));
    }

    #[test]
    fn sees_eunice_thinking_above_a_screenful_of_blank_rows() {
        let pane = format!(
            "  /help for commands · /quit or Ctrl+D to exit\n hello\n  ✻ Thinking…\n{}",
            "\n".repeat(50)
        );
        assert_eq!(parse_working(&pane), Some(("Thinking".to_string(), String::new())));
    }

    #[test]
    fn ignores_a_trust_prompt_that_scrolled_into_history() {
        let mut pane = String::from(CODEX_TRUST);
        for i in 0..40 {
            pane.push_str(&format!("line {i}\n"));
        }
        pane.push_str("› Ask Codex to do anything\n");
        assert!(trust_prompt_answer(&pane).is_none());
    }

    // --- AGY and EUNICE working status, captured from real panes ---

    #[test]
    fn reads_agy_generating_status() {
        let pane = "> Run the shell command\n⣟  Generating...\n└ Tip: Use /feedback to share your experience with the team.\n────\n>\n────\nesc to cancel                                             Gemini 3.8 Flash · high\n";
        assert_eq!(parse_working(pane), Some(("Generating".to_string(), String::new())));
    }

    #[test]
    fn reads_agy_tool_status_with_a_background_task() {
        let pane = "○ Bash(sleep 25 && echo done) (ctrl+o to expand)\n⣟  Running command...\n────\n>\n────\n  ● [08:57:07] sleep 25 && echo done running\n────\nesc to cancel                        Gemini 3.8 Flash · high · 1 task(s) · /tasks\n";
        assert_eq!(parse_working(pane), Some(("Running command".to_string(), String::new())));
    }

    #[test]
    fn agy_waiting_on_a_background_task_is_idle() {
        let pane = "○ Bash(sleep 25 && echo done) (ctrl+o to expand)\n  I have launched the command.\n────\n>\n────\n  ● [08:57:07] sleep 25 && echo done running\n────\n? for shortcuts                      Gemini 3.8 Flash · high · 1 task(s) · /tasks\n";
        assert!(parse_working(pane).is_none());
    }

    #[test]
    fn agy_spinner_left_in_scrollback_is_idle() {
        let pane = "⣟  Generating...\n  done\n────\n>\n────\n? for shortcuts                                           Gemini 3.8 Flash · high\n";
        assert!(parse_working(pane).is_none());
    }

    #[test]
    fn agy_cancel_footer_without_a_spinner_line_is_still_working() {
        let pane = "> hi\n────\n>\n────\nesc to cancel                                             Gemini 3.8 Flash · high\n";
        assert_eq!(parse_working(pane), Some(("Working".to_string(), String::new())));
    }

    #[test]
    fn reads_eunice_thinking() {
        let pane = "  model: gemini-3.8-flash  ·  tools: 4\n  /help for commands · /quit or Ctrl+D to exit\n Reply with the single word hi and nothing else.\n  ✻ Thinking…\n";
        assert_eq!(parse_working(pane), Some(("Thinking".to_string(), String::new())));
    }

    #[test]
    fn eunice_tool_call_after_thinking_is_still_working() {
        let pane = "  /help for commands · /quit or Ctrl+D to exit\n Run ls\n  ✻ Thinking…\n  → bash\n    {\"command\": \"ls\"}\n";
        assert_eq!(parse_working(pane), Some(("Thinking".to_string(), String::new())));
    }

    #[test]
    fn eunice_is_idle_once_its_composer_is_back() {
        let pane = " Reply with the single word hi and nothing else.\n  ✻ Thinking…\nhi\n───────────────────────── eunice\n›\n─────────────────────────\n▸▸ ↵ send · esc clear · /help · ctrl+d exit\n";
        assert!(parse_working(pane).is_none());
    }

    #[test]
    fn a_bare_thinking_line_outside_eunice_is_not_working() {
        // Nothing on screen says EUNICE, so the line is not its spinner.
        assert!(parse_working("some transcript\n  ✻ Thinking…\n").is_none());
    }

    // --- agent badge ---

    #[test]
    fn detects_agy_from_its_status_footer() {
        let idle = "> \n─────\n? for shortcuts                                           Gemini 3.8 Flash · high\n";
        assert_eq!(detect_agent(idle), Some(AgentKind::Agy));
        let busy = "⣟  Generating...\n>\nesc to cancel                        Gemini 3.8 Flash · high · 1 task(s) · /tasks\n";
        assert_eq!(detect_agent(busy), Some(AgentKind::Agy));
    }

    #[test]
    fn claude_shortcut_hint_is_not_agy() {
        // Claude Code prints the same hint, but without a model · effort tail.
        assert_eq!(detect_agent("❯ \n─────\n  ? for shortcuts\n"), None);
    }

    #[test]
    fn detects_eunice_from_its_composer_or_banner() {
        let idle = "hi\n──────────────── eunice\n›\n────────────────\n▸▸ ↵ send · esc clear · /help · ctrl+d exit\n";
        assert_eq!(detect_agent(idle), Some(AgentKind::Eunice));
        let busy = "  model: gemini-3.8-flash  ·  tools: 4\n  /help for commands · /quit or Ctrl+D to exit\n hello\n  ✻ Thinking…\n";
        assert_eq!(detect_agent(busy), Some(AgentKind::Eunice));
    }

    #[test]
    fn detects_codex_without_mistaking_an_ordinary_node_process() {
        let codex = "╭── OpenAI Codex (v0.152.0) ──╮\n\n› Ask Codex to do anything\n";
        assert_eq!(detect_agent(codex), Some(AgentKind::Codex));
        assert_eq!(detect_agent("$ node server.js\nlistening\n"), None);
    }

    #[test]
    fn detects_codex_question_after_the_banner_leaves_history() {
        let question = "  Question 1/1 (1 unanswered)\n  1. Alpha\n\n  enter to submit answer | esc to interrupt\n";
        assert_eq!(detect_agent(question), Some(AgentKind::Codex));
    }

    #[test]
    fn styled_capture_strips_to_the_original_pane_text() {
        let styled = "\x1b[1;38;5;6m› 1. Alpha\x1b[0m\n\x1b[48;2;31;31;31m  composer  \x1b[49m\n";
        assert_eq!(strip_ansi(styled), "› 1. Alpha\n  composer  \n");
    }

    #[test]
    fn styled_capture_strips_osc_sequences() {
        let styled = "before\x1b]8;;https://example.com\x07link\x1b]8;;\x1b\\ after";
        assert_eq!(strip_ansi(styled), "beforelink after");
    }

    #[test]
    fn styled_capture_strips_charset_selectors() {
        assert_eq!(strip_ansi("a\x1b(0b\x1b(Bc"), "abc");
    }

    #[test]
    fn reads_the_live_status_line() {
        let pane = "$ claude\n✻ Wrangling… (10m 40s · ↓ 25.6k tokens)\n";
        let (verb, meta) = parse_working(pane).expect("should see a working line");
        assert_eq!(verb, "Wrangling");
        assert_eq!(meta, "10m 40s · ↓ 25.6k tokens");
    }

    #[test]
    fn reads_codex_live_status_line() {
        let pane = "› Add a food indicator\n\n• Working (4m 21s • esc to interrupt) · 1 background terminal running · /ps to view · /stop to close\n\n› Ask Codex to do anything\n";
        let (verb, meta) = parse_working(pane).expect("should see Codex working");
        assert_eq!(verb, "Working");
        assert_eq!(meta, "4m 21s · 1 background terminal running");
    }

    #[test]
    fn reads_codex_live_status_without_background_tasks() {
        let pane = "• Working (9s • esc to interrupt)\n";
        let (verb, meta) = parse_working(pane).expect("should see Codex working");
        assert_eq!(verb, "Working");
        assert_eq!(meta, "9s");
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
    fn project_dirs_includes_symlinked_dirs_but_not_broken_or_file_links() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        // The link target lives outside `base`, the way ~/p/body points off to
        // ~/p/health/body, so the listing can only see it through the link.
        let elsewhere = tempfile::tempdir().unwrap();
        touch_dir(elsewhere.path(), "real-body", 1_700_000_000);
        std::fs::write(elsewhere.path().join("notes.txt"), b"x").unwrap();

        std::os::unix::fs::symlink(elsewhere.path().join("real-body"), base.join("body")).unwrap();
        // A link to a plain file is no more a project than the file itself.
        std::os::unix::fs::symlink(elsewhere.path().join("notes.txt"), base.join("notes")).unwrap();
        // A dangling link must be skipped, not counted as a directory.
        std::os::unix::fs::symlink(elsewhere.path().join("gone"), base.join("dangling")).unwrap();

        let dirs = collect_project_dirs(base);
        let names: Vec<&str> = dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["body"]);
        // The age shown is the target's, not the link's own mtime.
        assert_eq!(dirs[0].mtime, 1_700_000_000);
    }

    #[test]
    fn project_dirs_on_missing_base_is_empty_not_a_panic() {
        let dirs = collect_project_dirs(std::path::Path::new("/nope/does/not/exist"));
        assert!(dirs.is_empty());
    }
}
