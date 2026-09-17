//! Adapt the running CLI's model menu. Choices come from the actual session,
//! including account restrictions and model-specific reasoning controls.
use axum::{extract::Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use tokio::time::{sleep, Duration};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Choice {
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Menu {
    pub agent: String,
    pub stage: String,
    pub title: String,
    pub options: Vec<Choice>,
    pub cursor: usize,
    pub effort: Option<String>,
    pub adjustable: bool,
    pub session_only: bool,
    pub fingerprint: String,
}

fn fingerprint(menu: &Menu) -> String {
    let mut copy = menu.clone();
    copy.fingerprint.clear();
    let mut hash = DefaultHasher::new();
    serde_json::to_string(&copy).unwrap().hash(&mut hash);
    format!("{:016x}", hash.finish())
}

pub fn parse(pane: &str) -> Option<Menu> {
    let lines: Vec<&str> = pane.lines().collect();
    let last = lines.iter().rposition(|l| !l.trim().is_empty())?;
    if let Some(menu) = parse_hermes_menu(&lines, last) {
        return Some(menu);
    }
    // Native menus must still own the bottom of the pane. Never act on an old
    // menu in scrollback after the CLI has returned to a composer or shell.
    let tail = lines[last.saturating_sub(3)..=last].join("\n");
    let bottom = lines[last].trim();
    let (agent, stage, start, end) = if bottom.ends_with("Esc to cancel")
        && (tail.contains("s to use this session only") || tail.contains("Enter to confirm"))
    {
        (
            "claude",
            "model",
            lines.iter().rposition(|l| l.trim() == "Select model")?,
            last,
        )
    } else if bottom.ends_with("esc to go back") && tail.contains("Press enter to confirm") {
        let start = lines.iter().rposition(|l| {
            l.trim() == "Select Model and Effort"
                || l.trim().starts_with("Select Reasoning Level for ")
                || l.trim() == "Advanced Reasoning"
        })?;
        (
            "codex",
            if lines[start].trim() == "Select Model and Effort" {
                "model"
            } else {
                "effort"
            },
            start,
            last,
        )
    } else if tail.contains("Keyboard:") {
        let start = lines.iter().rposition(|l| l.trim() == "Switch Model")?;
        let end = lines.iter().rposition(|l| l.starts_with("Keyboard:"))?;
        if start < end
            && last <= end + 2
            && (lines[last].starts_with("  ")
                || bottom.starts_with("Keyboard:")
                || bottom.contains('·'))
            && !bottom.starts_with('$')
            && !bottom.starts_with('%')
            && !bottom.starts_with('>')
            && !bottom.starts_with('❯')
            && !bottom.starts_with('›')
        {
            ("agy", "model", start, end)
        } else {
            return None;
        }
    } else if tail.contains("↵ send") && bottom.ends_with("/model") {
        let start = lines
            .iter()
            .rposition(|l| l.starts_with("EUNICE_MODEL_SETTINGS "))?;
        let mut menu: Menu =
            serde_json::from_str(lines[start].strip_prefix("EUNICE_MODEL_SETTINGS ")?).ok()?;
        if menu.options.is_empty() || menu.cursor >= menu.options.len() {
            return None;
        }
        menu.fingerprint = fingerprint(&menu);
        return Some(menu);
    } else {
        return None;
    };
    let numbered = regex::Regex::new(r"^\s*([❯›]?)\s*\d+\.\s+(.+)$").unwrap();
    let columns = regex::Regex::new(r"\s{2,}").unwrap();
    let mut options = Vec::new();
    let mut cursor = None;
    let mut effort = None;
    let mut adjustable = false;
    let mut agy_rows = false;
    for (offset, line) in lines[start + 1..end].iter().enumerate() {
        if agent == "agy" {
            let trimmed = line.trim();
            if trimmed.starts_with("Search:")
                || (!trimmed.is_empty() && trimmed.chars().all(|c| c == '─' || c == '-'))
            {
                continue;
            }
            if line.trim_start().starts_with("Effort") {
                adjustable = true;
                if let (Some(dot), Some(labels)) = (
                    line.chars().position(|c| c == '◉'),
                    lines.get(start + offset + 2),
                ) {
                    let positions: Vec<(usize, &str)> = labels
                        .match_indices(|c: char| !c.is_whitespace())
                        .filter(|(i, _)| *i == 0 || labels.as_bytes()[i - 1].is_ascii_whitespace())
                        .collect();
                    effort = positions
                        .iter()
                        .min_by_key(|(i, _)| i.abs_diff(dot))
                        .map(|(i, _)| labels[*i..].split_whitespace().next().unwrap().to_string());
                }
                agy_rows = false;
                continue;
            }
            if line.is_empty() {
                if agy_rows {
                    agy_rows = false;
                }
                continue;
            }
            if !adjustable
                && (line.starts_with("  ") || line.starts_with("> "))
                && !line.contains("responses")
            {
                // The option block ends at its first blank row.
                if !options.is_empty() && !agy_rows {
                    continue;
                }
                agy_rows = true;
                if line.starts_with('>') {
                    cursor = Some(options.len());
                }
                options.push(Choice {
                    label: line.trim_start_matches('>').trim().to_string(),
                    description: String::new(),
                    id: String::new(),
                    efforts: vec![],
                });
            }
        } else if let Some(c) = numbered.captures(line) {
            if !c[1].is_empty() {
                cursor = Some(options.len());
            }
            let mut parts = columns.splitn(&c[2], 2);
            options.push(Choice {
                label: parts.next()?.to_string(),
                description: parts.next().unwrap_or("").to_string(),
                id: String::new(),
                efforts: vec![],
            });
        } else if agent == "claude" && line.contains("effort") && line.contains("←/→") {
            adjustable = true;
            effort = line
                .split_whitespace()
                .find(|word| {
                    ["low", "medium", "high", "xhigh", "max"]
                        .contains(&word.to_ascii_lowercase().as_str())
                })
                .map(|s| s.to_ascii_lowercase());
        }
    }
    let mut menu = Menu {
        agent: agent.into(),
        stage: stage.into(),
        title: lines[start].trim().into(),
        options,
        cursor: cursor?,
        effort,
        adjustable,
        session_only: agent == "agy" || tail.contains("s to use this session only"),
        fingerprint: String::new(),
    };
    if menu.options.is_empty() {
        return None;
    }
    menu.fingerprint = fingerprint(&menu);
    Some(menu)
}

fn hermes_prompt_column(line: &str) -> Option<usize> {
    let (profile, _) = line.split_once('❯')?;
    if !profile
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " _-.".contains(c))
    {
        return None;
    }
    Some(profile.chars().count() + 2)
}

fn parse_hermes_menu(lines: &[&str], last: usize) -> Option<Menu> {
    let start = lines.iter().rposition(|line| {
        let line = line.trim();
        line.starts_with("╭─ ⚙ Model Picker — ") && line.ends_with('╮')
    })?;
    // Blank padding varies with pane height and filtering. Require the actual
    // composer below the dialog, not a fixed number of screen rows.
    let end = (start + 1..=last)
        .rev()
        .find(|&i| lines[i].trim().starts_with('╰'))?;
    let suffix: Vec<_> = lines[end + 1..=last]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let rule = |line: &str| !line.is_empty() && line.chars().all(|c| c == '─');
    if end <= start
        || suffix.len() != 3
        || !rule(suffix[0])
        || hermes_prompt_column(suffix[1]).is_none()
        || !rule(suffix[2])
    {
        return None;
    }
    let heading = lines[start].trim();
    let stage = if heading.contains("Select Provider") {
        "provider"
    } else if heading.contains("Reasoning effort for ") {
        "effort"
    } else {
        "model"
    };
    let mut options = Vec::new();
    let mut cursor = None;
    for line in &lines[start + 1..end] {
        let Some(inner) = line
            .trim()
            .strip_prefix('│')
            .and_then(|s| s.strip_suffix('│'))
        else {
            continue;
        };
        let selected = inner.trim_start().starts_with('❯');
        let text = inner.trim().trim_start_matches('❯').trim();
        if text.is_empty()
            || text.starts_with("Current:")
            || text.starts_with("Select a model")
            || text.starts_with("Filter:")
            || text.starts_with("Applies with the model switch")
            || text == "Cancel"
            || text == "← Back"
        {
            continue;
        }
        let current = text.ends_with("← current");
        let text = text.trim_end_matches("← current").trim_end();
        let eligible = if stage == "provider" {
            text.contains(" model") || text.contains(" models")
        } else if stage == "effort" {
            [
                "minimal",
                "low",
                "medium",
                "high",
                "xhigh",
                "max",
                "ultra",
                "none (disable reasoning)",
                "Keep current effort",
            ]
            .contains(&text)
        } else {
            !text.contains(char::is_whitespace)
        };
        if !eligible {
            continue;
        }
        if selected {
            cursor = Some(options.len());
        }
        options.push(Choice {
            label: text.to_string(),
            description: if current {
                "current".into()
            } else {
                String::new()
            },
            id: text.split_whitespace().next().unwrap_or(text).to_string(),
            efforts: vec![],
        });
    }
    let effort_re =
        regex::Regex::new(r"Reasoning effort:\s+(none|minimal|low|medium|high|xhigh|max|ultra)")
            .ok()?;
    let effort = lines[..start]
        .iter()
        .rev()
        .find_map(|line| effort_re.captures(line).map(|caps| caps[1].to_string()));
    let mut menu = Menu {
        agent: "hermes".into(),
        stage: stage.into(),
        title: if stage == "provider" {
            "Select provider".into()
        } else {
            heading.trim_matches(['╭', '─', '╮', ' ']).to_string()
        },
        options,
        cursor: cursor?,
        effort,
        adjustable: false,
        session_only: true,
        fingerprint: String::new(),
    };
    if menu.options.is_empty() {
        return None;
    }
    menu.fingerprint = fingerprint(&menu);
    Some(menu)
}

struct Pane {
    target: String,
    command: String,
    x: usize,
    text: String,
    input: String,
    result: Option<serde_json::Value>,
}
async fn snapshot(target: &str) -> Result<Pane, String> {
    let info = super::hosts::tmux().args(["display-message", "-p", "-t", target, "#{pane_id}\t#{pane_current_command}\t#{cursor_x}\t#{cursor_y}\t#{pane_in_mode}\t#{@eunice_model_settings}\t#{@eunice_model_result}"]).output().await.map_err(|e| e.to_string())?;
    if !info.status.success() {
        return Err("This window has closed.".into());
    }
    let raw = String::from_utf8_lossy(&info.stdout);
    let fields: Vec<_> = raw.trim_end_matches('\n').split('\t').collect();
    if fields.len() != 7 || fields[4] != "0" {
        return Err("Leave tmux copy mode before changing models.".into());
    }
    let out = super::hosts::tmux()
        .args(["capture-pane", "-p", "-t", fields[0]])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("Could not read this window.".into());
    }
    let screen = String::from_utf8_lossy(&out.stdout).to_string();
    let input = screen
        .lines()
        .nth(fields[3].parse().unwrap_or(999))
        .unwrap_or("")
        .trim()
        .to_string();
    let text = if fields[1] == "eunice" && !fields[5].is_empty() {
        format!("EUNICE_MODEL_SETTINGS {}\n{}", fields[5], screen)
    } else {
        screen
    };
    let result = serde_json::from_str(fields[6]).ok();
    Ok(Pane {
        target: fields[0].into(),
        command: fields[1].into(),
        x: fields[2].parse().unwrap_or(999),
        text,
        input,
        result,
    })
}

fn ready(p: &Pane) -> Option<&'static str> {
    if super::parse_working(&p.text).is_some() {
        return None;
    }
    if ["hermes", "python", "python3"].contains(&p.command.as_str())
        && hermes_prompt_column(&p.input) == Some(p.x)
        && p.text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .rev()
            .take(15)
            .any(|line| super::is_hermes_status(line))
        && parse(&p.text).is_none()
    {
        return Some("hermes");
    }
    if p.x > 2 {
        return None;
    }
    let line = p.input.as_str();
    match p.command.as_str() {
        "claude" if line == "❯" => Some("claude"),
        "codex" | "node" if line == "›" || line == "› Ask Codex to do anything" => {
            Some("codex")
        }
        "agy" if line == ">" => Some("agy"),
        "eunice" if line == "›" && p.text.contains("↵ send") => Some("eunice"),
        _ => None,
    }
}

async fn key(target: &str, key: &str) -> Result<(), String> {
    super::send_keys(target, &[key.into()]).await
}
fn owns_menu(p: &Pane, menu: &Menu) -> bool {
    match menu.agent.as_str() {
        "claude" => p.command == "claude",
        "codex" => ["codex", "node"].contains(&p.command.as_str()),
        "agy" => p.command == "agy",
        "eunice" => p.command == "eunice",
        "hermes" => {
            ["hermes", "python", "python3"].contains(&p.command.as_str())
                && parse(&p.text).is_some_and(|m| m.agent == "hermes")
        }
        _ => false,
    }
}
async fn command(target: &str, text: &str) -> Result<(), String> {
    super::send_keys_literal(target, text).await?;
    sleep(Duration::from_millis(100)).await;
    key(target, "Enter").await
}
async fn changed(target: &str, before: &Menu) -> Result<Option<Menu>, String> {
    for _ in 0..40 {
        sleep(Duration::from_millis(50)).await;
        let p = snapshot(target).await?;
        match parse(&p.text) {
            Some(m) if m.fingerprint != before.fingerprint => return Ok(Some(m)),
            None if ready(&p).is_some() => return Ok(None),
            _ => {}
        }
    }
    Err("The CLI did not confirm the change. Check the terminal and reopen the picker.".into())
}

fn hermes_switch_confirmed(p: &Pane, menu: &Menu) -> bool {
    if ready(p) != Some("hermes") {
        return false;
    }
    let model = if menu.stage == "effort" {
        menu.title
            .split_once("Reasoning effort for ")
            .map(|(_, model)| model)
    } else {
        menu.options
            .get(menu.cursor)
            .map(|choice| choice.id.as_str())
    };
    let Some(model) = model else {
        return false;
    };
    let Some((_, confirmation)) = p.text.rsplit_once(&format!("✓ Model switched: {model}\n"))
    else {
        return false;
    };
    if confirmation.contains('✗') || confirmation.contains("cancelled") {
        return false;
    }
    if menu.stage == "effort" && menu.options[menu.cursor].label != "Keep current effort" {
        return confirmation.contains(&format!(
            "Reasoning effort: {}",
            menu.options[menu.cursor].id
        ));
    }
    true
}

async fn confirm_hermes_switch(target: &str, menu: &Menu) -> Result<(), String> {
    for _ in 0..200 {
        sleep(Duration::from_millis(50)).await;
        if hermes_switch_confirmed(&snapshot(target).await?, menu) {
            return Ok(());
        }
    }
    Err("Hermes has not confirmed the model and reasoning change. Check the terminal.".into())
}

#[derive(Deserialize)]
pub struct Request {
    target: String,
    action: String,
    #[serde(default)]
    fingerprint: String,
    index: Option<usize>,
    delta: Option<i32>,
    model: Option<String>,
    effort: Option<String>,
}

async fn perform(req: Request) -> Result<serde_json::Value, String> {
    let p = snapshot(&req.target).await?;
    let target = p.target.clone();
    if req.action == "open" {
        if let Some(menu) = parse(&p.text) {
            if menu.agent != "eunice" {
                if !owns_menu(&p, &menu) {
                    return Err("The CLI in this window has changed.".into());
                }
                return Ok(serde_json::json!({"target": target, "menu": menu}));
            }
        }
        let agent = ready(&p).ok_or(
            "Wait for the CLI to finish and leave its input empty before changing models.",
        )?;
        if agent == "eunice" && !p.text.trim_end().ends_with("/model") {
            return Err("This Eunice process predates live model switching. Start a window with the updated CLI to use the picker.".into());
        }
        if agent == "eunice" {
            let cleared = super::hosts::tmux()
                .args([
                    "set-option",
                    "-p",
                    "-t",
                    &target,
                    "@eunice_model_settings",
                    "",
                ])
                .status()
                .await
                .map_err(|e| e.to_string())?;
            if !cleared.success() {
                return Err("Could not refresh Eunice settings.".into());
            }
        }
        command(
            &target,
            if agent == "eunice" {
                "/model --tmux"
            } else {
                "/model"
            },
        )
        .await?;
        for _ in 0..200 {
            sleep(Duration::from_millis(50)).await;
            if let Some(menu) = parse(&snapshot(&target).await?.text) {
                if menu.agent == agent {
                    return Ok(serde_json::json!({"target": target, "menu": menu}));
                }
            }
        }
        return Err("The model menu did not open. Check the terminal before trying again.".into());
    }
    let mut menu = parse(&p.text).ok_or("The model menu has closed. Reopen it to continue.")?;
    if menu.fingerprint != req.fingerprint {
        return Err("The model menu changed elsewhere. Reopen it to refresh the choices.".into());
    }
    // A shell printing a previous menu is not an agent. Pin every action to the
    // resolved pane id and check its foreground process again.
    if !owns_menu(&p, &menu) {
        return Err("The CLI in this window has changed.".into());
    }
    if menu.agent == "eunice" {
        if ready(&p) != Some("eunice") {
            return Err("Wait for Eunice and leave its input empty.".into());
        }
        if req.action == "cancel" {
            return Ok(serde_json::json!({"target": target, "closed": true}));
        }
        if req.action != "apply" {
            return Err("Unsupported action.".into());
        }
        let selected = menu
            .options
            .iter()
            .find(|m| Some(&m.id) == req.model.as_ref())
            .ok_or("Choose a listed model.")?;
        let effort = req.effort.as_deref().unwrap_or("default");
        if !selected.efforts.iter().any(|e| e == effort) {
            return Err("Unsupported effort for this model.".into());
        }
        super::validate_model(&selected.id)?;
        super::validate_model(effort)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string();
        command(
            &target,
            &format!("/model {} {} {}", selected.id, effort, nonce),
        )
        .await?;
        for _ in 0..200 {
            sleep(Duration::from_millis(50)).await;
            let fresh = snapshot(&target).await?;
            if ready(&fresh) == Some("eunice") {
                if let Some(result) = fresh.result.filter(|r| r["nonce"] == nonce) {
                    if result["success"] == true {
                        return Ok(
                            serde_json::json!({"target": target, "closed": true, "applied": true}),
                        );
                    }
                    return Err(result["error"]
                        .as_str()
                        .unwrap_or("Eunice could not apply these settings.")
                        .to_string());
                }
            }
        }
        return Err("Eunice has not confirmed the change. Check the terminal.".into());
    }
    match req.action.as_str() {
        "select" => {
            let index = req
                .index
                .filter(|i| *i < menu.options.len())
                .ok_or("Choose a listed option.")?;
            for _ in 0..64 {
                if menu.cursor == index {
                    break;
                }
                let labels = menu.options.clone();
                key(&target, if index > menu.cursor { "Down" } else { "Up" }).await?;
                menu = changed(&target, &menu)
                    .await?
                    .ok_or("The menu closed while selecting. Nothing was applied.")?;
                if menu.options != labels {
                    return Err("The choices changed. Reopen the picker.".into());
                }
            }
            if menu.cursor != index {
                return Err("Could not select that option.".into());
            }
            // Codex enters its per-model effort screen without committing yet.
            if (menu.agent == "codex"
                && (menu.stage == "model"
                    || menu.options[index].label.starts_with("More reasoning")))
                || (menu.agent == "hermes" && ["provider", "model"].contains(&menu.stage.as_str()))
            {
                key(&target, "Enter").await?;
                let next = changed(&target, &menu).await?;
                if menu.agent == "hermes" && next.is_none() {
                    confirm_hermes_switch(&target, &menu).await?;
                }
                return Ok(
                    serde_json::json!({"target": target, "menu": next, "closed": next.is_none(), "applied": next.is_none()}),
                );
            }
        }
        "effort" if menu.adjustable => {
            key(
                &target,
                if req.delta == Some(-1) {
                    "Left"
                } else if req.delta == Some(1) {
                    "Right"
                } else {
                    return Err("Invalid effort direction.".into());
                },
            )
            .await?;
            // At an endpoint the native slider may stay put.
            sleep(Duration::from_millis(250)).await;
            menu = parse(&snapshot(&target).await?.text).ok_or("The model menu closed.")?;
        }
        "back" if ["codex", "hermes"].contains(&menu.agent.as_str()) && menu.stage == "effort" => {
            key(&target, "Escape").await?;
            menu = changed(&target, &menu)
                .await?
                .ok_or("The model menu closed.")?;
        }
        "cancel" => {
            for _ in 0..4 {
                key(&target, "Escape").await?;
                match changed(&target, &menu).await? {
                    Some(next) => menu = next,
                    None => return Ok(serde_json::json!({"target": target, "closed": true})),
                }
            }
            return Err("Close the remaining menu in the terminal.".into());
        }
        "apply" => {
            if menu.agent == "hermes" && menu.stage != "effort" {
                return Err("Choose a Hermes model and reasoning level first.".into());
            }
            if menu.agent == "codex"
                && (menu.stage != "effort"
                    || menu.options[menu.cursor]
                        .label
                        .starts_with("More reasoning"))
            {
                return Err("Choose a model and effort first.".into());
            }
            key(
                &target,
                if menu.agent == "claude" && menu.session_only {
                    "s"
                } else {
                    "Enter"
                },
            )
            .await?;
            if menu.agent == "hermes" {
                confirm_hermes_switch(&target, &menu).await?;
                return Ok(serde_json::json!({"target": target, "closed": true, "applied": true}));
            }
            for _ in 0..40 {
                sleep(Duration::from_millis(50)).await;
                let p = snapshot(&target).await?;
                if parse(&p.text).is_none() && ready(&p).is_some() {
                    return Ok(
                        serde_json::json!({"target": target, "closed": true, "applied": true}),
                    );
                }
            }
            if let Some(next) = parse(&snapshot(&target).await?.text) {
                return Ok(serde_json::json!({"target": target, "menu": next}));
            }
            return Ok(serde_json::json!({"target": target, "closed": true, "applied": true}));
        }
        _ => return Err("Unsupported model action.".into()),
    }
    Ok(serde_json::json!({"target": target, "menu": menu}))
}

pub async fn handle(Json(req): Json<Request>) -> (StatusCode, Json<serde_json::Value>) {
    let (target, _guard) = match super::lock_pane_action(&req.target).await {
        Ok(locked) => locked,
        Err(error) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"success":false,"error":error})),
            )
        }
    };
    match perform(Request { target, ..req }).await {
        Ok(mut data) => {
            data["success"] = true.into();
            (StatusCode::OK, Json(data))
        }
        Err(error) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"success": false, "error": error})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_model_and_effort_screens() {
        for (text, agent, count, cursor, effort) in [
            (
                include_str!("../tests/fixtures/session_model/claude_model.txt"),
                "claude",
                5,
                1,
                Some("xhigh"),
            ),
            (
                include_str!("../tests/fixtures/session_model/claude_haiku.txt"),
                "claude",
                5,
                4,
                None,
            ),
            (
                include_str!("../tests/fixtures/session_model/agy_pro.txt"),
                "agy",
                7,
                3,
                Some("low"),
            ),
            (
                include_str!("../tests/fixtures/session_model/agy_fixed_effort.txt"),
                "agy",
                7,
                4,
                None,
            ),
            (
                include_str!("../tests/fixtures/session_model/agy_1_2_pro.txt"),
                "agy",
                7,
                0,
                Some("high"),
            ),
            (
                include_str!("../tests/fixtures/session_model/agy_1_2_fixed_effort.txt"),
                "agy",
                7,
                4,
                None,
            ),
            (
                include_str!("../tests/fixtures/session_model/codex_model.txt"),
                "codex",
                7,
                0,
                None,
            ),
            (
                include_str!("../tests/fixtures/session_model/codex_effort.txt"),
                "codex",
                5,
                3,
                None,
            ),
            (
                include_str!("../tests/fixtures/session_model/codex_more_effort.txt"),
                "codex",
                2,
                0,
                None,
            ),
        ] {
            let m = parse(text).expect(agent);
            assert_eq!(
                (
                    m.agent.as_str(),
                    m.options.len(),
                    m.cursor,
                    m.effort.as_deref()
                ),
                (agent, count, cursor, effort)
            );
            assert!(
                parse(&format!("{text}\n$ echo hello\n")).is_none(),
                "stale {agent} menu"
            );
            assert!(
                parse(&format!("{}\n$ ", text.trim_end())).is_none(),
                "adjacent shell after {agent}"
            );
        }
    }
    #[test]
    fn parses_live_hermes_provider_and_model_dialogs() {
        let provider = "\
  Reasoning effort:  medium
╭─ ⚙ Model Picker — Select Provider ───╮
│ Current: z-ai/glm-5.3-flash on OpenRouter │
│   Mixture of Agents (1 model)              │
│ ❯ OpenRouter (50 models)  ← current       │
│   Anthropic (13 models)                    │
│   Cancel                                   │
╰─────────────────────────────────────────────╯
────────
❯ Ask anything
────────
";
        let menu = parse(provider).expect("Hermes provider menu");
        assert_eq!(
            (
                menu.agent.as_str(),
                menu.stage.as_str(),
                menu.options.len(),
                menu.cursor
            ),
            ("hermes", "provider", 3, 1)
        );
        assert_eq!(menu.effort.as_deref(), Some("medium"));

        let models = provider.replace(
            "Select Provider",
            "OpenRouter",
        ).replace(
            "\u{2502} Current: z-ai/glm-5.3-flash on OpenRouter \u{2502}\n\u{2502}   Mixture of Agents (1 model)              \u{2502}\n\u{2502} ❯ OpenRouter (50 models)  ← current       \u{2502}\n\u{2502}   Anthropic (13 models)                    \u{2502}",
            "\u{2502} Select a model (2 available) — type to filter \u{2502}\n\u{2502} ❯ z-ai/glm-5.3-flash                         \u{2502}\n\u{2502}   anthropic/claude-sonnet-5                 \u{2502}\n\u{2502}   ← Back                                    \u{2502}",
        );
        let menu = parse(&models).expect("Hermes model menu");
        assert_eq!(
            (menu.stage.as_str(), menu.options.len(), menu.cursor),
            ("model", 2, 0)
        );
        assert!(!menu.adjustable);
    }

    #[test]
    fn rejects_a_stale_hermes_dialog() {
        let pane = "╭─ ⚙ Model Picker — Select Provider ─╮\n│ ❯ OpenRouter (50 models) │\n╰──╯\n❯ Ask anything\n───\nordinary output\n$ ";
        assert!(parse(pane).is_none());
    }
    #[test]
    fn hermes_requires_model_and_effort_confirmation() {
        let mut menu = parse(include_str!(
            "../tests/fixtures/session_model/hermes_effort.txt"
        ))
        .unwrap();
        menu.cursor = 3;
        let mut p = Pane { target: "%0".into(), command: "python".into(), x: 7,
            input: "test ❯ Ask anything".into(), result: None,
            text: "✓ Model switched: z-ai/glm-5.3-flash\nReasoning effort: high\n ☤ glm-5.3-flash │ ctx --\ntest ❯ Ask anything\n".into() };
        assert!(hermes_switch_confirmed(&p, &menu));
        p.text = p.text.replace("effort: high", "effort: medium");
        assert!(!hermes_switch_confirmed(&p, &menu));
        p.text = p
            .text
            .replace("effort: medium", "effort: high\nModel switch cancelled.");
        assert!(!hermes_switch_confirmed(&p, &menu));
        p.text = " ☤ glm-5.3-flash │ ctx --\ntest ❯ Ask anything\n".into();
        assert!(!hermes_switch_confirmed(&p, &menu));
    }
    #[test]
    fn parses_captured_hermes_stages_with_padding_and_profile_prompt() {
        for (text, stage, count, cursor) in [
            (
                include_str!("../tests/fixtures/session_model/hermes_provider.txt"),
                "provider",
                8,
                1,
            ),
            (
                include_str!("../tests/fixtures/session_model/hermes_model.txt"),
                "model",
                1,
                0,
            ),
            (
                include_str!("../tests/fixtures/session_model/hermes_effort.txt"),
                "effort",
                9,
                0,
            ),
        ] {
            let menu = parse(text).expect(stage);
            assert_eq!(
                (
                    menu.agent.as_str(),
                    menu.stage.as_str(),
                    menu.options.len(),
                    menu.cursor
                ),
                ("hermes", stage, count, cursor)
            );
            assert!(parse(&format!("{text}\n$ ")).is_none());
            assert!(parse(&text.replace(
                "test ❯ Plan a feature, then build it step by step",
                "unrelated output"
            ))
            .is_none());
            let mut p = Pane {
                target: "%0".into(),
                command: "python".into(),
                x: 7,
                text: text.into(),
                input: "test ❯ Plan a feature, then build it step by step".into(),
                result: None,
            };
            assert!(owns_menu(&p, &menu));
            assert_eq!(ready(&p), None);
            p.command = "bash".into();
            assert!(!owns_menu(&p, &menu));
            if stage == "effort" {
                assert_eq!(menu.options[2].label, "medium");
                assert_eq!(menu.options[2].description, "current");
                assert_eq!(menu.options[7].id, "none");
            }
        }
    }
    #[test]
    fn hermes_is_ready_after_context_usage_replaces_ctx_placeholder() {
        let idle = include_str!("../tests/fixtures/hermes/idle_after_turn.txt");
        assert!(!idle.contains("ctx "));
        let input = idle.lines().find(|line| line.starts_with('❯')).unwrap();
        let mut p = Pane {
            target: "%14".into(), command: "python".into(), x: 2,
            text: idle.into(), input: input.into(), result: None,
        };
        assert_eq!(ready(&p), Some("hermes"));
        let busy = "☤ ❯ msg=interrupt · /queue · /bg · /steer · Ctrl+C cancel\n";
        p.text = format!("{busy}{idle}");
        assert_eq!(ready(&p), Some("hermes"), "stale busy footer must not block an idle picker");
        p.text = format!("{idle}{busy}");
        assert_eq!(ready(&p), None, "live busy footer must still block the picker");
        p.text = idle.into();
        p.x = 3;
        assert_eq!(ready(&p), None, "typed input remains protected");
        p.x = 2;
        p.command = "bash".into();
        assert_eq!(ready(&p), None);
    }

    #[test]
    fn refuses_busy_or_nonempty_composers() {
        let mut p = Pane {
            target: "%1".into(),
            command: "claude".into(),
            x: 2,
            text: "❯\n".into(),
            input: "❯".into(),
            result: None,
        };
        assert_eq!(ready(&p), Some("claude"));
        p.text = "❯ draft\n".into();
        p.input = "❯ draft".into();
        assert_eq!(ready(&p), None);
        p.input = "❯".into();
        p.text = "❯\n✻ Working… (5s · esc to interrupt)\n".into();
        assert_eq!(ready(&p), None);
        p.text = "❯\n".into();
        p.command = "bash".into();
        assert_eq!(ready(&p), None);

        p.command = "python".into();
        p.x = 2;
        p.input = "❯ Ask anything".into();
        p.text = " ☤ glm-5.3-flash │ ctx --\n❯ Ask anything\n".into();
        assert_eq!(ready(&p), Some("hermes"));
        p.text.push_str(&"\n".repeat(50));
        assert_eq!(ready(&p), Some("hermes"));
        p.input = "test ❯ Plan a feature, then build it step by step".into();
        p.x = 7;
        assert_eq!(ready(&p), Some("hermes"));
        p.x = 8;
        assert_eq!(ready(&p), None);
        p.x = 7;
        p.text = "ordinary Python output".into();
        assert_eq!(ready(&p), None);
    }
}
