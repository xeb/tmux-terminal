//! Parser for Claude Code's interactive selection prompts as they appear in a
//! captured tmux pane.
//!
//! The web UI polls `capture-pane -p`, which strips ANSI attributes, so the
//! colour Claude Code uses to mark the highlighted row never reaches us. Detection
//! therefore runs on glyphs and structure alone:
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
        fingerprint,
        header,
        question,
        cursor,
        layout,
        options,
        preview,
    })
}

/// The arrow keys that move the terminal's cursor from `from` to `to`. Never
/// wraps: the direct path is always at most `len - 1` keys.
///
/// Enter is deliberately NOT included. Batched into the same `send-keys` call as
/// the movement, Claude Code's TUI applies it against pre-move state and commits
/// the wrong option. The caller sends it separately, after confirming the cursor
/// landed.
pub fn move_keys(from: usize, to: usize) -> Vec<String> {
    let (key, n) = if to >= from {
        ("Down", to - from)
    } else {
        ("Up", from - to)
    };
    std::iter::repeat(key.to_string()).take(n).collect()
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
    fn move_keys_go_the_right_way() {
        let empty: Vec<String> = vec![];
        assert_eq!(move_keys(0, 0), empty);
        assert_eq!(move_keys(0, 2), vec!["Down", "Down"]);
        assert_eq!(move_keys(3, 1), vec!["Up", "Up"]);
    }

    #[test]
    fn move_keys_never_carry_enter() {
        // Enter batched with movement commits the pre-move option. It is sent
        // separately, after the cursor is confirmed to have landed.
        for (a, b) in [(0, 5), (5, 0), (2, 3)] {
            assert!(!move_keys(a, b).iter().any(|k| k == "Enter"));
        }
    }
}
