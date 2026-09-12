//! Parser for Claude Code and Codex interactive selection prompts as they
//! appear in a captured tmux pane.
//!
//! The server derives a plain-text copy from `capture-pane -e` for this parser,
//! so the colour Claude Code and Codex use to mark highlighted rows does not
//! participate in detection. Detection runs on glyphs and structure alone:
//!
//! ```text
//! ──────────────────────────────────────────   <- top rule
//!  ☐ Allowlist                                 <- header chip
//!
//! Which addresses should I add ...?            <- question
//!
//! ❯ 1. Both real ones, drop the bogus          <- cursor row
//!      Add spencer... (wrapped continuation)
//!   2. Both real ones, keep ...
//! ──────────────────────────────────────────   <- meta rule
//!   6. Chat about this                         <- meta option
//!
//! Enter to select · ↑/↓ to navigate · Esc to cancel
//! ```
//!
//! Two layouts exist. When options carry previews, Claude Code splits the view
//! into a narrow option list and a bordered preview pane at a fixed column; the
//! parser finds that gutter and reads only the left side as options.
//!
//! Governing rule: **fail closed.** Anything not recognised with confidence
//! returns `None`, and the UI renders the pane exactly as it does today.

use serde::Serialize;

/// U+276F, the glyph Claude Code uses for the highlighted row.
const CURSOR: char = '❯';
/// U+203A, the glyph Codex uses for the highlighted row.
const CODEX_CURSOR: char = '›';
/// U+2500, the glyph the rules are drawn with.
const RULE: char = '─';
/// U+2610, the glyph in the header chip.
const CHIP: char = '☐';

/// Box-drawing characters that can begin a preview pane's left border.
const BOX_CHARS: [char; 7] = ['┌', '│', '└', '┐', '┘', '├', '┤'];

/// A rule must be at least this wide to count. Short runs of `─` show up inside
/// Claude Code's banner and inside preview panes.
const MIN_RULE_WIDTH: usize = 20;

/// How far above the footer to look for the prompt block. Bounds the damage if a
/// stray rule sits in scrollback.
const MAX_BLOCK_LINES: usize = 120;

/// Option rows sit at indent 0 (cursor) or 2 (unselected). Wrapped description
/// lines sit at 5. This threshold is what separates them.
const MAX_OPTION_INDENT: usize = 2;

/// At least this many lines must agree on a column before it is treated as a
/// preview gutter rather than incidental box-drawing in prose.
const MIN_GUTTER_VOTES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Options carry prose descriptions. The client can move the highlight
    /// locally because every description is already on screen.
    List,
    /// Options carry a preview pane. Only the *focused* option's preview is
    /// rendered by the terminal, so the client must steer the real cursor to see
    /// another one.
    Preview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Opt {
    /// Display only. Absent for rows Claude Code renders unnumbered, such as
    /// "Chat about this" in the preview layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    pub label: String,
    /// Wrapped continuation lines, re-joined into one paragraph. `List` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// True for rows below the trailing rule ("Type something.", "Chat about this").
    pub is_meta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Picker {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub codex_async: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub text_only: bool,
    /// Visible text already entered in Codex's answer editor. Never part of
    /// the fingerprint; the website can submit it without typing it twice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_draft: Option<String>,
    /// Stable across cursor movement, changes when the question or options do.
    /// The commit endpoint refuses a mismatch, so a prompt can never be answered
    /// on the strength of a stale render.
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub question: String,
    /// Index into `options` of the terminal's own `❯`.
    pub cursor: usize,
    pub layout: Layout,
    pub options: Vec<Opt>,
    /// The focused option's preview pane, column alignment preserved. `Preview` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// One scanned line, before it is classified as an option or a continuation.
struct Row {
    is_cursor: bool,
    indent: usize,
    number: Option<u32>,
    text: String,
}

fn chars_of(line: &str) -> Vec<char> {
    line.chars().collect()
}

fn take_cols(line: &str, n: usize) -> String {
    line.chars().take(n).collect()
}

fn skip_cols(line: &str, n: usize) -> String {
    line.chars().skip(n).collect()
}

/// True for a line that is nothing but a long run of `─`.
fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.chars().count() >= MIN_RULE_WIDTH && t.chars().all(|c| c == RULE)
}

/// A footer varies between prompt kinds — `gbc` ends with
/// `Enter to select · ↑/↓ to navigate · Esc to cancel`, while a prompt offering
/// notes adds `· n to add notes`. Match loosely on the two invariant phrases.
fn is_footer(line: &str) -> bool {
    let t = line.trim();
    t.contains("to navigate") && (t.contains("Enter to select") || t.contains("Enter to confirm"))
}

/// Split a line into indent, cursor marker, optional `N.`, and the remaining text.
/// Returns `None` for blank lines.
fn scan_row(line: &str) -> Option<Row> {
    let chars = chars_of(line);
    let mut i = 0;
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    let indent = i;

    let mut is_cursor = false;
    if chars[i] == CURSOR {
        is_cursor = true;
        i += 1;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
    }

    // Optional leading "N."
    let mut number = None;
    let num_start = i;
    let mut j = i;
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
    }
    if j > num_start && j < chars.len() && chars[j] == '.' {
        let digits: String = chars[num_start..j].iter().collect();
        if let Ok(n) = digits.parse::<u32>() {
            number = Some(n);
            i = j + 1;
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
        }
    }

    let text: String = chars[i..].iter().collect();
    let text = text.trim_end().to_string();
    if text.is_empty() {
        return None;
    }

    Some(Row {
        is_cursor,
        indent,
        number,
        text,
    })
}

/// An option row is shallow *and* either numbered or carrying the cursor.
///
/// The numbered-or-cursor test is not cosmetic. Claude Code echoes the user's own
/// input prefixed with the same `❯` glyph (`❯ Please consider 3 different
/// options...`), and shells like starship use `❯` as PS1. Requiring a number, or
/// membership in an already-established option block, excludes both.
fn is_option_row(row: &Row) -> bool {
    row.indent <= MAX_OPTION_INDENT && (row.number.is_some() || row.is_cursor)
}

/// Column of the first box-drawing character, if any.
fn first_box_col(line: &str) -> Option<usize> {
    line.chars().position(|c| BOX_CHARS.contains(&c))
}

/// Find the column at which a preview pane begins, by majority vote across the
/// block. Returns `None` for the list layout, where no such column exists.
fn detect_gutter(lines: &[&str]) -> Option<usize> {
    let mut votes: Vec<(usize, usize)> = Vec::new();
    for line in lines {
        if let Some(col) = first_box_col(line) {
            if col == 0 {
                continue; // full-width banners, not a gutter
            }
            match votes.iter_mut().find(|(c, _)| *c == col) {
                Some((_, n)) => *n += 1,
                None => votes.push((col, 1)),
            }
        }
    }
    votes
        .into_iter()
        .filter(|(_, n)| *n >= MIN_GUTTER_VOTES)
        .max_by_key(|(_, n)| *n)
        .map(|(col, _)| col)
}

/// FNV-1a. Not a security boundary — this only has to change when the prompt
/// changes, and only has to agree with itself within one running server.
fn fingerprint_of(question: &str, layout: Layout, options: &[Opt]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |s: &str| {
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(question);
    eat(match layout {
        Layout::List => "list",
        Layout::Preview => "preview",
    });
    for o in options {
        eat(&o.label);
        eat(if o.is_meta { "m" } else { "o" });
        if let Some(n) = o.number {
            eat(&n.to_string());
        }
    }
    format!("{:016x}", h)
}

/// Parse a captured pane. Returns `None` unless a live prompt sits at the tail.
///
/// Anchoring at the tail is also the liveness test: once a prompt is answered
/// Claude prints below it, so an answered prompt sitting in scrollback can never
/// re-arm the picker.
pub fn parse(pane: &str) -> Option<Picker> {
    parse_claude_dialog(pane)
        .or_else(|| parse_claude_review(pane))
        .or_else(|| parse_codex_dialog(pane))
        .or_else(|| parse_codex_async(pane))
}

fn parse_claude_dialog(pane: &str) -> Option<Picker> {
    let all: Vec<&str> = pane.lines().collect();

    // Trailing blank lines are tmux padding the pane to its height.
    let mut end = all.len();
    while end > 0 && all[end - 1].trim().is_empty() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }
    let footer_idx = end - 1;
    if !is_footer(all[footer_idx]) {
        return None;
    }

    // The block is delimited by rules. Two means there is a meta section below
    // the trailing rule; one means there is not.
    let search_start = footer_idx.saturating_sub(MAX_BLOCK_LINES);
    let rules: Vec<usize> = (search_start..footer_idx)
        .filter(|i| is_rule(all[*i]))
        .collect();
    let (top_rule, meta_rule) = match rules.len() {
        0 => return None,
        1 => (rules[0], None),
        _ => (rules[rules.len() - 2], Some(rules[rules.len() - 1])),
    };

    let opts_end = meta_rule.unwrap_or(footer_idx);
    if top_rule + 1 >= opts_end {
        return None;
    }

    let block: Vec<&str> = all[top_rule + 1..opts_end].to_vec();

    // Header chip and question sit between the top rule and the first option.
    // Option rows begin at column 0 or 2, so they are found without truncation.
    let first_opt = block
        .iter()
        .position(|l| scan_row(l).map(|r| is_option_row(&r)).unwrap_or(false))?;

    // A preview pane shifts the *option list* into a narrow left column — but
    // only from the first option down. The question above it is still rendered
    // full width and runs past the gutter, so it must not be truncated.
    let gutter = detect_gutter(&block[first_opt..]);
    let layout = if gutter.is_some() {
        Layout::Preview
    } else {
        Layout::List
    };

    // Left column only, from the first option down. Owned; truncation allocates.
    let left: Vec<String> = block
        .iter()
        .skip(first_opt)
        .map(|l| match gutter {
            Some(g) => take_cols(l, g),
            None => (*l).to_string(),
        })
        .collect();

    let mut header = None;
    let mut question_parts: Vec<String> = Vec::new();
    for line in block.iter().take(first_opt) {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix(CHIP) {
            header = Some(rest.trim().to_string());
        } else {
            question_parts.push(t.to_string());
        }
    }
    if question_parts.is_empty() {
        return None;
    }
    let question = question_parts.join(" ");

    // Answer options, with wrapped continuations folded into descriptions.
    let mut options: Vec<Opt> = Vec::new();
    let mut descriptions: Vec<Vec<String>> = Vec::new();
    let mut cursor: Option<usize> = None;

    for line in left.iter() {
        let Some(row) = scan_row(line) else { continue };
        if is_option_row(&row) {
            if row.is_cursor {
                cursor = Some(options.len());
            }
            options.push(Opt {
                number: row.number,
                label: row.text,
                description: None,
                is_meta: false,
            });
            descriptions.push(Vec::new());
        } else if let Some(last) = descriptions.last_mut() {
            // A wrapped description line. Claude Code breaks these mid-sentence
            // at pane width, so they are re-joined and left for CSS to re-wrap.
            last.push(row.text);
        }
    }
    if options.is_empty() {
        return None;
    }

    // Meta options: everything between the trailing rule and the footer. Every
    // non-blank row here is an option, numbered or not.
    if let Some(mr) = meta_rule {
        for line in all.iter().take(footer_idx).skip(mr + 1) {
            let left_part = match gutter {
                Some(g) => take_cols(line, g),
                None => (*line).to_string(),
            };
            let Some(row) = scan_row(&left_part) else {
                continue;
            };
            if row.indent > MAX_OPTION_INDENT {
                continue;
            }
            if row.is_cursor {
                cursor = Some(options.len());
            }
            options.push(Opt {
                number: row.number,
                label: row.text,
                description: None,
                is_meta: true,
            });
            descriptions.push(Vec::new());
        }
    }

    // No live cursor means nothing is awaiting input.
    let cursor = cursor?;

    for (opt, desc) in options.iter_mut().zip(descriptions.into_iter()) {
        if !desc.is_empty() {
            opt.description = Some(desc.join(" "));
        }
    }

    // The preview pane belongs to the focused option only — the terminal renders
    // no others, which is why the client must steer the real cursor in this layout.
    let preview = gutter.map(|g| {
        let raw: Vec<String> = block
            .iter()
            .skip(first_opt)
            .map(|l| skip_cols(l, g).trim_end().to_string())
            .collect();
        let start = raw.iter().position(|l| !l.is_empty()).unwrap_or(0);
        let stop = raw.iter().rposition(|l| !l.is_empty()).map_or(0, |i| i + 1);
        raw[start..stop].join("\n")
    });

    let fingerprint = fingerprint_of(&question, layout, &options);

    Some(Picker {
        codex_async: false,
        text_only: false,
        answer_draft: None,
        fingerprint,
        header,
        question,
        cursor,
        layout,
        options,
        preview,
    })
}

/// Multi-question prompts end on a "Review your answers" screen that carries no
/// footer line, so the structural parser cannot see it. It is still a live
/// two-option dialog (digits work on it), and missing it strands the whole
/// exchange: the card vanishes with every answer given but nothing submitted.
/// Anchored on Claude Code's literal strings — fail closed on anything else.
fn parse_claude_review(pane: &str) -> Option<Picker> {
    let all: Vec<&str> = pane.lines().collect();

    let mut end = all.len();
    while end > 0 && all[end - 1].trim().is_empty() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }

    // The tail must be nothing but option rows — the screen ends on them.
    let mut first_row = end;
    while first_row > 0 {
        let Some(row) = scan_row(all[first_row - 1]) else { break };
        if !is_option_row(&row) {
            break;
        }
        first_row -= 1;
    }
    if end - first_row < 2 {
        return None;
    }

    // Directly above: the confirm line, and further up the review header.
    let confirm = (first_row.saturating_sub(3)..first_row)
        .find(|i| all[*i].trim() == "Ready to submit your answers?")?;
    let review = (confirm.saturating_sub(MAX_BLOCK_LINES)..confirm)
        .find(|i| all[*i].trim() == "Review your answers")?;

    let mut options: Vec<Opt> = Vec::new();
    let mut cursor: Option<usize> = None;
    for line in &all[first_row..end] {
        let row = scan_row(line)?;
        if row.is_cursor {
            cursor = Some(options.len());
        }
        options.push(Opt {
            number: row.number,
            label: row.text,
            description: None,
            is_meta: false,
        });
    }
    let cursor = cursor?;

    // The card must show what is about to be submitted, so the answered pairs
    // become part of the question text.
    let question = all[review..=confirm]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    let fingerprint = fingerprint_of(&question, Layout::List, &options);
    Some(Picker {
        codex_async: false,
        text_only: false,
        answer_draft: None,
        fingerprint,
        header: Some("Review".to_string()),
        question,
        cursor,
        layout: Layout::List,
        options,
        preview: None,
    })
}

/// Codex's `request_user_input` view is deliberately parsed separately from
/// Claude's dialog. Its structure is compact and stable, but shares almost no
/// delimiters with Claude's: there are no rules, the cursor is U+203A, and the
/// footer talks about submitting an answer rather than navigating.
///
/// ```text
///   Question 1/1 (1 unanswered)
///   Which single option would you like to choose?
///
///   › 1. Explore Bright Forest Paths (Recommended)  A description that can
///                                                   wrap onto more lines.
///     2. Navigate Distant Mountain Trails Today     Another description.
///
///   tab to add notes | enter to submit answer | esc to interrupt
/// ```
///
/// As with the Claude parser, both the header and footer must be present at the
/// live tail of the pane. A numbered list in ordinary transcript output cannot
/// arm the picker by itself.
fn parse_codex_dialog(pane: &str) -> Option<Picker> {
    let all: Vec<&str> = pane.lines().collect();

    let mut end = all.len();
    while end > 0 && all[end - 1].trim().is_empty() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }

    // Narrow panes wrap this footer. It is separated from the options by a
    // blank line, so join the final contiguous run instead of assuming one row.
    let mut footer_idx = end;
    let mut footer_parts: Vec<&str> = Vec::new();
    while footer_idx > 0 && footer_parts.len() < 3 {
        let part = all[footer_idx - 1].trim();
        if part.is_empty() {
            break;
        }
        footer_idx -= 1;
        footer_parts.push(part);
    }
    footer_parts.reverse();
    let footer = footer_parts.join(" ");
    if !footer.contains("enter to submit answer") || !footer.contains("esc to interrupt") {
        return None;
    }

    let search_start = footer_idx.saturating_sub(MAX_BLOCK_LINES);
    let header_idx = (search_start..footer_idx)
        .rev()
        .find(|i| is_codex_question_header(all[*i].trim()))?;

    let first_opt = (header_idx + 1..footer_idx)
        .find(|i| scan_codex_option_row(all[*i]).is_some())?;
    if first_opt <= header_idx + 1 {
        return None;
    }

    let question = all[header_idx + 1..first_opt]
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if question.is_empty() {
        return None;
    }

    let mut options: Vec<Opt> = Vec::new();
    let mut cursor = None;
    for line in &all[first_opt..footer_idx] {
        if let Some(row) = scan_codex_option_row(line) {
            if row.is_cursor {
                cursor = Some(options.len());
            }
            let (label, description) = split_codex_option_text(&row.text);
            if label.is_empty() {
                return None;
            }
            options.push(Opt {
                number: row.number,
                label,
                description,
                // "None of the above" can carry notes, but it is still a
                // directly-submittable answer. Marking it as meta would make
                // the Claude-specific text-reply path type into the wrong TUI.
                is_meta: false,
            });
        } else if !line.trim().is_empty() {
            // Codex aligns wrapped descriptions under the description column.
            // Once an option exists, any nonblank non-option row before the
            // footer is one of those continuations.
            let last = options.last_mut()?;
            let continuation = line.trim();
            match &mut last.description {
                Some(description) => {
                    description.push(' ');
                    description.push_str(continuation);
                }
                None => last.description = Some(continuation.to_string()),
            }
        }
    }

    let cursor = cursor?;
    if options.len() < 2 || options.iter().any(|option| option.number.is_none()) {
        return None;
    }

    let header = all[header_idx]
        .trim()
        .split(" (")
        .next()
        .unwrap_or("Question")
        .to_string();
    let fingerprint = fingerprint_of(&question, Layout::List, &options);
    Some(Picker {
        codex_async: false,
        text_only: false,
        answer_draft: None,
        fingerprint,
        header: Some(header),
        question,
        cursor,
        layout: Layout::List,
        options,
        preview: None,
    })
}

fn is_codex_question_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("Question ") else {
        return false;
    };
    let Some((position, state)) = rest.split_once(" (") else {
        return false;
    };
    let Some((current, total)) = position.split_once('/') else {
        return false;
    };
    current.parse::<usize>().is_ok()
        && total.parse::<usize>().is_ok()
        && state.ends_with("unanswered)")
}

fn scan_codex_option_row(line: &str) -> Option<Row> {
    let chars = chars_of(line);
    let mut i = 0;
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    let indent = i;
    if indent > 6 || i >= chars.len() {
        return None;
    }

    let mut is_cursor = false;
    if chars[i] == CODEX_CURSOR {
        is_cursor = true;
        i += 1;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
    }

    let number_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == number_start || i >= chars.len() || chars[i] != '.' {
        return None;
    }
    let digits: String = chars[number_start..i].iter().collect();
    let number = digits.parse::<u32>().ok()?;
    i += 1;
    if i >= chars.len() || chars[i] != ' ' {
        return None;
    }
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    let text: String = chars[i..].iter().collect();
    let text = text.trim_end().to_string();
    if text.is_empty() {
        return None;
    }

    Some(Row {
        is_cursor,
        indent,
        number: Some(number),
        text,
    })
}

/// Codex pads the label column with two or more spaces before the description.
/// Description continuations are handled by `parse_codex_dialog`.
fn split_codex_option_text(text: &str) -> (String, Option<String>) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b' ' {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if i - start >= 2 {
            let label = text[..start].trim_end().to_string();
            let description = text[i..].trim();
            return (
                label,
                if description.is_empty() {
                    None
                } else {
                    Some(description.to_string())
                },
            );
        }
    }
    (text.trim().to_string(), None)
}

/// True when the dialog is in text-entry state: the chrome (rule, numbered
/// rows, footer) is still on screen but no row carries the selection cursor —
/// a printable key opened the free-text buffer, which replaces the `❯` row.
/// In this state Claude accepts only typed text or Backspace.
pub fn awaiting_typed_reply(pane: &str) -> bool {
    if parse(pane).is_some() {
        return false;
    }
    let all: Vec<&str> = pane.lines().collect();
    let mut end = all.len();
    while end > 0 && all[end - 1].trim().is_empty() {
        end -= 1;
    }
    if end == 0 || !is_footer(all[end - 1]) {
        return false;
    }
    let start = end.saturating_sub(MAX_BLOCK_LINES);
    let has_rule = (start..end - 1).any(|i| is_rule(all[i]));
    let has_numbered_row = (start..end - 1).any(|i| {
        scan_row(all[i]).map_or(false, |r| r.indent <= MAX_OPTION_INDENT && r.number.is_some())
    });
    has_rule && has_numbered_row
}

/// True for the rows Claude Code adds around the tool's own options ("Type
/// something." above the trailing rule, "Chat about this" below it). Their
/// printed digits are display-only: pressing one types the digit into the
/// free-text buffer instead of selecting. These rows must be activated by
/// walking the cursor and pressing Enter, never by digit.
pub fn is_input_row(opt: &Opt) -> bool {
    if opt.is_meta {
        return true;
    }
    opt.label.trim().trim_end_matches('.').eq_ignore_ascii_case("Type something")
}

/// The tmux key for a single step, used by the preview layout where the client
/// must steer the real cursor to reveal another option's preview.
pub fn step_key(delta: i32) -> &'static str {
    if delta >= 0 {
        "Down"
    } else {
        "Up"
    }
}

/// The digit key that selects an option outright, skipping cursor traversal.
///
/// This is the whole reason traversal is avoided: Claude Code's TUI drops rapid
/// repeated arrow keys non-deterministically (sending 2, 3 and 4 batched `Down`s
/// from the same start landed on rows 0, 1 and 4 respectively — verified live).
/// A single digit has no such race, and selects regardless of where the cursor
/// currently sits. Only 1–9 are reachable; anything else must traverse.
pub fn select_key(number: Option<u32>) -> Option<String> {
    match number {
        Some(n) if (1..=9).contains(&n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/tests/fixtures/picker/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path, e))
    }

    #[test]
    fn parses_list_layout() {
        let p = parse(&fixture("list_gbc.txt")).expect("should parse");
        assert_eq!(p.layout, Layout::List);
        assert_eq!(p.header.as_deref(), Some("Allowlist"));
        assert!(p.question.starts_with("Which addresses should I add"));
        assert_eq!(p.cursor, 0);
        assert_eq!(p.options.len(), 6);
        assert_eq!(p.options[0].number, Some(1));
        assert_eq!(p.options[0].label, "Both real ones, drop the bogus");
        assert_eq!(p.options[4].label, "Type something.");
        assert!(!p.options[4].is_meta);
        assert_eq!(p.options[5].label, "Chat about this");
        assert!(p.options[5].is_meta);
        assert!(p.preview.is_none());
    }

    #[test]
    fn rejoins_wrapped_descriptions() {
        let p = parse(&fixture("list_gbc.txt")).unwrap();
        let d = p.options[0].description.as_deref().unwrap();
        // The terminal breaks this mid-sentence between "Gives" and "him".
        assert!(d.contains("Gives him Google SSO on personal"), "got: {}", d);
        assert!(!d.contains('\n'));
        assert!(p.options[4].description.is_none(), "meta rows carry none");
    }

    #[test]
    fn parses_preview_layout() {
        let p = parse(&fixture("preview_test.txt")).expect("should parse");
        assert_eq!(p.layout, Layout::Preview);
        assert_eq!(p.header.as_deref(), Some("Choice mech"));
        assert_eq!(p.question, "How should a single choice be made in Claude?");
        assert_eq!(p.cursor, 0);
        assert_eq!(p.options.len(), 4);
        assert_eq!(p.options[0].label, "A — Interactive prompt");
        assert_eq!(p.options[2].label, "C — Options in prose");
        // The escape hatch is rendered without a number in this layout.
        assert_eq!(p.options[3].label, "Chat about this");
        assert_eq!(p.options[3].number, None);
        assert!(p.options[3].is_meta);
    }

    #[test]
    fn parses_codex_request_user_input() {
        let p = parse(&fixture("codex.txt")).expect("should parse Codex picker");
        assert_eq!(p.layout, Layout::List);
        assert_eq!(p.header.as_deref(), Some("Question 1/1"));
        assert_eq!(p.question, "Which single option would you like to choose?");
        assert_eq!(p.cursor, 0);
        assert_eq!(p.options.len(), 4);
        assert_eq!(p.options[0].number, Some(1));
        assert_eq!(p.options[0].label, "Explore Bright Forest Paths (Recommended)");
        assert_eq!(
            p.options[0].description.as_deref(),
            Some(
                "Choose a guided route through a quiet forest filled with clear landmarks and gentle terrain throughout."
            )
        );
        assert_eq!(p.options[3].label, "None of the above");
        assert_eq!(
            p.options[3].description.as_deref(),
            Some("Optionally, add details in notes (tab).")
        );
        assert!(!p.options[3].is_meta);
        assert!(p.preview.is_none());
    }

    #[test]
    fn codex_fingerprint_ignores_cursor_position() {
        let base = fixture("codex.txt");
        let a = parse(&base).unwrap();
        let moved = base
            .replace(
                "  › 1. Explore Bright Forest Paths (Recommended)",
                "    1. Explore Bright Forest Paths (Recommended)",
            )
            .replace(
                "    2. Navigate Distant Mountain Trails Today",
                "  › 2. Navigate Distant Mountain Trails Today",
            );
        let b = parse(&moved).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(b.cursor, 1);
    }

    #[test]
    fn rejects_answered_codex_question() {
        let answered = fixture("codex.txt")
            + "• Questions 1/1 answered\n  • Which single option?\n    answer: Alpha\n";
        assert!(parse(&answered).is_none());
    }

    #[test]
    fn parses_codex_footer_wrapped_by_a_narrow_pane() {
        let narrow = fixture("codex.txt").replace(
            "tab to add notes | enter to submit answer | esc to interrupt",
            "tab to add notes | enter to submit answer | esc to\n  interrupt",
        );
        assert!(parse(&narrow).is_some());
    }

    #[test]
    fn preview_pane_keeps_its_columns() {
        let p = parse(&fixture("preview_test.txt")).unwrap();
        let pv = p.preview.as_deref().expect("preview layout has a pane");
        assert!(pv.starts_with('┌'), "starts at the border: {:?}", &pv[..12]);
        assert!(pv.contains("A) INTERACTIVE PROMPT"));
        assert!(pv.contains("who decides : user"));
        // The option labels live in the left column and must not leak in.
        assert!(!pv.contains("B — Default and proceed"));
    }

    #[test]
    fn rejects_answered_prompt() {
        // Claude has printed below the block, so it is no longer at the tail.
        assert!(parse(&fixture("answered.txt")).is_none());
    }

    #[test]
    fn rejects_shell_prompt() {
        // Contains a starship `❯ ` PS1 and Claude's own `❯ `-prefixed echo of
        // user input. Neither is an option row.
        assert!(parse(&fixture("shell_prompt.txt")).is_none());
    }

    #[test]
    fn rejects_plain_output() {
        assert!(parse(&fixture("plain.txt")).is_none());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse("").is_none());
        assert!(parse("\n\n\n").is_none());
    }

    #[test]
    fn fingerprint_ignores_cursor_position() {
        let base = fixture("preview_test.txt");
        let a = parse(&base).unwrap();
        // Move the terminal's cursor from option 1 to option 2.
        let moved = base
            .replace("❯ 1. A — Interactive prompt", "  1. A — Interactive prompt")
            .replace("  2. B — Default and proceed", "❯ 2. B — Default and proceed");
        let b = parse(&moved).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint, "cursor is not part of identity");
        assert_eq!(b.cursor, 1);
    }

    #[test]
    fn fingerprint_changes_with_the_question() {
        let base = fixture("list_gbc.txt");
        let a = parse(&base).unwrap();
        let b = parse(&base.replace("Which addresses", "Whose addresses")).unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn step_keys_go_the_right_way() {
        assert_eq!(step_key(1), "Down");
        assert_eq!(step_key(-1), "Up");
    }

    #[test]
    fn numbered_options_select_by_digit() {
        assert_eq!(select_key(Some(1)).as_deref(), Some("1"));
        assert_eq!(select_key(Some(9)).as_deref(), Some("9"));
    }

    #[test]
    fn unreachable_numbers_fall_back_to_traversal() {
        // No digit key exists for these, so the caller must walk the cursor.
        assert_eq!(select_key(None), None);
        assert_eq!(select_key(Some(0)), None);
        assert_eq!(select_key(Some(10)), None);
    }

    #[test]
    fn every_list_option_is_reachable_by_digit() {
        // The list layout numbers every row, including both escape hatches, so
        // no traversal is needed there at all.
        let p = parse(&fixture("list_gbc.txt")).unwrap();
        for o in &p.options {
            assert!(select_key(o.number).is_some(), "{:?} needs traversal", o.label);
        }
    }

    #[test]
    fn preview_escape_hatch_needs_traversal() {
        // The preview layout renders "Chat about this" without a number.
        let p = parse(&fixture("preview_test.txt")).unwrap();
        let meta = p.options.iter().find(|o| o.is_meta).unwrap();
        assert_eq!(select_key(meta.number), None);
    }

    #[test]
    fn parses_review_screen() {
        // Multi-question prompts end on a footer-less "Review your answers"
        // screen. It must parse, or the card vanishes with the answers never
        // submitted.
        let p = parse(&fixture("review.txt")).expect("should parse");
        assert_eq!(p.layout, Layout::List);
        assert_eq!(p.cursor, 0);
        assert_eq!(p.options.len(), 2);
        assert_eq!(p.options[0].label, "Submit answers");
        assert_eq!(p.options[0].number, Some(1));
        assert_eq!(p.options[1].label, "Cancel");
        assert_eq!(p.options[1].number, Some(2));
        assert!(!p.options[0].is_meta);
        assert!(p.question.contains("Ready to submit your answers?"));
        // The card must show what is being submitted.
        assert!(p.question.contains("Which fruit?"), "got: {}", p.question);
        assert!(p.question.contains("→ Apple"), "got: {}", p.question);
    }

    #[test]
    fn typed_buffer_state_is_not_a_picker() {
        // A stray printable key puts the dialog into text entry: the typed
        // buffer replaces the `❯` row, so there is no cursor to parse.
        assert!(parse(&fixture("typed_buffer.txt")).is_none());
    }

    #[test]
    fn typed_buffer_state_is_awaiting_typed_reply() {
        // The dialog chrome is still on screen but no row carries the cursor —
        // Claude is waiting for typed text, and the commit path must report
        // that instead of guessing from a timeout.
        assert!(awaiting_typed_reply(&fixture("typed_buffer.txt")));
    }

    #[test]
    fn live_dialog_is_not_awaiting_typed_reply() {
        assert!(!awaiting_typed_reply(&fixture("list_gbc.txt")));
    }

    #[test]
    fn plain_output_is_not_awaiting_typed_reply() {
        assert!(!awaiting_typed_reply(&fixture("plain.txt")));
        assert!(!awaiting_typed_reply(&fixture("answered.txt")));
    }

    #[test]
    fn tui_added_rows_are_input_rows() {
        // "Type something." and "Chat about this" are rendered by Claude Code,
        // not the tool call. Their printed digits are display-only — pressing
        // one types the digit into the free-text buffer instead of selecting.
        let type_something = Opt {
            number: Some(4),
            label: "Type something.".to_string(),
            description: None,
            is_meta: false,
        };
        let multi_variant = Opt {
            number: Some(4),
            label: "Type something".to_string(),
            description: None,
            is_meta: false,
        };
        let chat = Opt {
            number: Some(5),
            label: "Chat about this".to_string(),
            description: None,
            is_meta: true,
        };
        let real = Opt {
            number: Some(1),
            label: "Apple".to_string(),
            description: None,
            is_meta: false,
        };
        assert!(is_input_row(&type_something));
        assert!(is_input_row(&multi_variant));
        assert!(is_input_row(&chat));
        assert!(!is_input_row(&real));
    }
}

/// Codex 0.154's asynchronous questions are collapsed under the composer until
/// the user explicitly enters them. Only a live queue plus a recognized hint
/// can arm the entry button; ordinary transcript questions cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestionQueue {
    pub count: usize,
    pub fingerprint: String,
    #[serde(skip)]
    pub open_key: String,
}

fn question_hint_key(hint: &str) -> Option<&'static str> {
    match hint.trim() {
        "shift + ←" => Some("S-Left"),
        "shift + →" => Some("S-Right"),
        "⌥ + ↑" | "alt + ↑" => Some("M-Up"),
        "⌥ + ↓" | "alt + ↓" => Some("M-Down"),
        _ => None,
    }
}

pub fn codex_main_prompt(pane: &str) -> bool {
    let tail: Vec<_> = pane.lines().rev().take(20).collect();
    let Some(composer) = tail.iter().position(|line| line.trim_start().starts_with('›')) else { return false };
    tail[..composer].iter().any(|line| line.trim_start().starts_with("gpt-"))
}

pub fn question_queue(pane: &str) -> Option<QuestionQueue> {
    let lines: Vec<_> = pane.lines().rev().take(80).collect::<Vec<_>>().into_iter().rev().collect();
    let composer = lines.iter().rposition(|line| line.trim_start().starts_with('›'))?;
    if !lines[composer..].iter().any(|line| line.trim_start().starts_with("gpt-")) {
        return None;
    }
    let queue = lines[..composer].iter().rposition(|line| line.trim() == "• Queued follow-up inputs")?;
    let count_re = regex::Regex::new(r"^\? (\d+) questions?(?: · .*)?$").ok()?;
    for pair in lines[queue + 1..composer].windows(2) {
        let Some(count) = count_re.captures(pair[0].trim()).and_then(|m| m[1].parse::<usize>().ok()) else { continue };
        if count == 0 { continue; }
        let hint = pair[1].trim().strip_suffix(" to answer")?;
        let open_key = question_hint_key(hint)?.to_string();
        return Some(QuestionQueue { count, fingerprint: format!("codex-questions:{count}:{open_key}"), open_key });
    }
    None
}

/// Back navigation preserves both answers-in-progress and the ordinary composer
/// draft. Esc can interrupt the running agent, so it is never our exit key.
pub fn codex_question_back_key(pane: &str) -> Option<String> {
    let lines: Vec<_> = pane.lines().collect();
    let (_, footer) = codex_async_footer(&lines)?;
    for tip in footer.split("   ").map(str::trim) {
        let hint = tip.strip_suffix(" main prompt").or_else(|| tip.strip_suffix(" prev question"));
        if let Some(key) = hint.and_then(question_hint_key) { return Some(key.to_string()); }
    }
    None
}

fn codex_async_footer<'a>(lines: &'a [&'a str]) -> Option<(usize, String)> {
    let end = lines.iter().rposition(|line| !line.trim().is_empty())? + 1;
    let start = (end.saturating_sub(5)..end).rev().find(|&i| lines[i].contains("enter submit"))?;
    let footer = lines[start..end].iter().map(|line| line.trim()).collect::<Vec<_>>().join("   ");
    if footer.split("   ").any(|tip| {
        let tip = tip.trim();
        tip.is_empty() || !(tip == "enter submit" || tip.ends_with(" skip")
            || tip.ends_with(" main prompt") || tip.ends_with(" prev question")
            || tip.ends_with(" next question") || tip.ends_with(" queued messages")
            || tip.starts_with("option "))
    }) { return None; }
    if !footer.contains(" skip") || !(footer.contains(" main prompt") || footer.contains(" prev question")) {
        return None;
    }
    Some((start, footer))
}

fn parse_codex_async(pane: &str) -> Option<Picker> {
    let all: Vec<_> = pane.lines().collect();
    let (footer, footer_text) = codex_async_footer(&all)?;
    let start = footer.saturating_sub(MAX_BLOCK_LINES);
    let progress = regex::Regex::new(r"^\d+ of \d+$").ok()?;
    let queue = (start..footer).rev().find(|&i| all[i].trim() == "• Queued follow-up inputs");
    // The free-text layout leaves a blank line after the progress counter.
    // Anchor above the question so blank lines or numbered lists in a draft
    // cannot become a new question (and change its submission fingerprint).
    let progress_line = (queue.map_or(start, |i| i + 1)..footer).find(|&i| progress.is_match(all[i].trim()));
    let anchor = progress_line.or(queue);
    let (question_start, question_end, first, text_only) = if let Some(anchor) = anchor {
        let mut question_start = anchor + 1;
        while question_start < footer && all[question_start].trim().is_empty() { question_start += 1; }
        let mut question_end = question_start;
        while question_end < footer && !all[question_end].trim().is_empty() { question_end += 1; }
        let mut first = question_end;
        while first < footer && all[first].trim().is_empty() { first += 1; }
        let free_text_gap = progress_line.is_some_and(|i| all.get(i + 1).is_some_and(|l| l.trim().is_empty()));
        let named = !free_text_gap && all.get(first).is_some_and(|l| scan_codex_option_row(l).is_some_and(|r| r.number == Some(1)));
        (question_start, question_end, first, !named)
    } else {
        // Older/single-question layouts may have neither a queue nor a counter.
        let named_first = (start..footer).rev().find(|&i| scan_codex_option_row(all[i]).is_some_and(|r| r.number == Some(1)));
        if named_first.is_none() && all[start..footer].iter().any(|l| scan_codex_option_row(l).is_some()) { return None; }
        let first = named_first.unwrap_or_else(|| {
            let mut end = footer;
            while end > start && all[end - 1].trim().is_empty() { end -= 1; }
            while end > start && !all[end - 1].trim().is_empty() { end -= 1; }
            end
        });
        let mut question_end = first;
        while question_end > start && all[question_end - 1].trim().is_empty() { question_end -= 1; }
        let mut question_start = question_end;
        while question_start > start && !all[question_start - 1].trim().is_empty() { question_start -= 1; }
        (question_start, question_end, first, named_first.is_none())
    };
    let header = progress_line.map(|i| format!("Question {}", all[i].trim()));
    let question = all[question_start..question_end].iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
    if question.is_empty() { return None; }
    let mut options: Vec<Opt> = Vec::new();
    let mut cursor = None;
    for line in if text_only { &all[0..0] } else { &all[first..footer] } {
        if let Some(row) = scan_codex_option_row(line) {
            if row.number != Some(options.len() as u32 + 1) { return None; }
            if row.is_cursor { cursor = Some(options.len()); }
            options.push(Opt { number: row.number, label: row.text, description: None, is_meta: false });
        } else if !line.trim().is_empty() {
            options.last_mut()?.label.push_str(&format!(" {}", line.trim()));
        }
    }
    if text_only {
        options.push(Opt { number: None, label: "Write an answer".to_string(), description: None, is_meta: true });
        cursor = Some(0);
    } else if options.len() < 2 { return None; }
    if let Some(total) = regex::Regex::new(r"option \d+/(\d+)").ok()?.captures(&footer_text) {
        if total[1].parse::<usize>().ok()? != options.len() { return None; }
    }
    let draft = if text_only {
        all[first..footer].iter().map(|l| l.trim()).collect::<Vec<_>>().join("\n")
    } else if cursor == Some(options.len() - 1) {
        options.last()?.label.clone()
    } else { String::new() };
    let draft = draft.trim();
    let answer_draft = (!draft.is_empty() && !matches!(draft, "Type your answer" | "Other" | "Other (write an answer)"))
        .then(|| draft.to_string());
    let last = options.last_mut()?;
    // Codex always adds a final free-text option. Normalize its inline draft
    // so typing does not change the question fingerprint.
    last.is_meta = true;
    last.label = if text_only { "Write an answer" } else { "Other (write an answer)" }.to_string();
    let fingerprint = fingerprint_of(&format!("codex-async:{}:{question}", header.as_deref().unwrap_or("")), Layout::List, &options);
    Some(Picker { codex_async: true, text_only, answer_draft, fingerprint, header: header.or(Some("Question".to_string())), question,
        cursor: cursor?, layout: Layout::List, options, preview: None })
}

#[cfg(test)]
mod async_question_tests {
    use super::*;
    const QUEUED: &str = include_str!("../tests/fixtures/picker/codex-queued.txt");
    const ACTIVE: &str = include_str!("../tests/fixtures/picker/codex-async.txt");
    const TEXT: &str = include_str!("../tests/fixtures/picker/codex-async-text.txt");

    #[test]
    fn recognizes_multiline_drafts_and_spaced_progress_from_phone_report() {
        let p = parse(TEXT).unwrap();
        assert!(p.codex_async && p.text_only);
        assert_eq!(p.header.as_deref(), Some("Question 1 of 6"));
        assert_eq!(p.question, "Which locations did you measure, and were these taken before food and training?");
        assert_eq!(p.answer_draft.as_deref(), Some("Narrowest point and high near the thickest part\nBefore food"));
        for draft in ["Type your answer", "First paragraph\n\nSecond paragraph", "1. First\n2. Second"] {
            let pane = TEXT.replace(p.answer_draft.as_ref().unwrap().replace('\n', "\n  ").as_str(), draft);
            assert_eq!(parse(&pane).unwrap().fingerprint, p.fingerprint);
        }
        assert_ne!(parse(&TEXT.replace("1 of 6", "2 of 6")).unwrap().fingerprint, p.fingerprint);
    }

    #[test]
    fn recognizes_the_screenshot_queue_without_opening_a_picker() {
        let queue = question_queue(QUEUED).unwrap();
        assert_eq!(queue.count, 2);
        assert_eq!(queue.open_key, "S-Left");
        assert!(parse(QUEUED).is_none());
        assert!(question_queue(&QUEUED.replace(" to answer", " edit last queued message")).is_none());
        assert!(question_queue(&QUEUED.replace("shift + ←", "ctrl + x")).is_none());
        assert!(question_queue(&format!("{QUEUED}\n› echo unrelated\nordinary shell output")).is_none());
    }

    #[test]
    fn parses_async_options_and_safe_navigation() {
        let picker = parse(ACTIVE).unwrap();
        assert!(picker.codex_async);
        assert_eq!(picker.header.as_deref(), Some("Question 1 of 2"));
        assert_eq!(picker.options.len(), 3);
        assert_eq!(picker.options[1].label, "Second option with a long label that wraps on a narrow terminal");
        assert!(picker.options[2].is_meta);
        assert_eq!(codex_question_back_key(ACTIVE).as_deref(), Some("S-Right"));
        assert_eq!(codex_question_back_key(&ACTIVE.replace("shift + → main prompt", "⌥ + ↓ prev question")).as_deref(), Some("M-Down"));
    }

    #[test]
    fn cursor_and_other_drafts_do_not_change_the_fingerprint() {
        let selected = ACTIVE.replace("› 1.", "  1.").replace("  3. Other", "› 3. My own answer");
        assert_eq!(parse(ACTIVE).unwrap().fingerprint, parse(&selected).unwrap().fingerprint);
        assert_eq!(parse(&selected).unwrap().cursor, 2);
        assert_ne!(parse(ACTIVE).unwrap().fingerprint, parse(&ACTIVE.replace("1 of 2", "2 of 2")).unwrap().fingerprint);
    }

    #[test]
    fn refuses_answered_or_clipped_async_questions() {
        assert!(parse(&format!("{ACTIVE}\n› Ask Codex to do anything\n  gpt-6-astra xhigh")).is_none());
        assert!(parse(&ACTIVE.replace("enter submit", "Expand terminal to read the entire option")).is_none());
        assert!(parse(&ACTIVE.replace("ctrl + ] skip", "ctrl + ] skip   option 1/8")).is_none());
    }

    #[test]
    fn parses_a_free_text_question_without_treating_it_as_a_command() {
        let pane = "\n  What should I investigate?\n\n  Type your answer\n\n  enter submit   ctrl + ] skip\n  shift + → main prompt\n";
        let p = parse(pane).unwrap();
        assert!(p.codex_async && p.text_only);
        assert_eq!(p.question, "What should I investigate?");
        assert_eq!(p.options[0].number, None);
        assert_eq!(p.fingerprint, parse(&pane.replace("Type your answer", "a draft")).unwrap().fingerprint);
        assert!(!codex_main_prompt(pane));
        assert!(codex_main_prompt(QUEUED));
    }
}
