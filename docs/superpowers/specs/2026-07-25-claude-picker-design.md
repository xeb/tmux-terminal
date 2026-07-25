# Claude Picker in the Web UI — Design

**Date:** 2026-07-25
**Status:** Implemented
**Scope:** `src/main.rs` (new parser module + endpoint) and `static/index.html` (frontend).
Requires a Rust rebuild and redeploy. The mobile app (`mobile/`) is unchanged in this work but
can adopt the same response field later without a backend change.

## Problem

When Claude Code blocks on a multiple-choice prompt, the web UI shows it as raw captured text —
19 lines of hard-wrapped prose for six choices in the `gbc` capture, and more as the pane narrows,
because the descriptions re-wrap. The only way to answer is to guess how many
`Down` presses are needed and fire them blind through `/api/send-key`, with a one-second polling
gap in which the cursor may have moved. There is no way to see that a *different* window is
waiting on you at all.

This replaces that with a parsed, keyboard-driven picker docked above the command input.

## Behavior

### Activation

When the captured pane for the viewed window ends in a live Claude prompt, a **picker card**
appears between the session bar and the command input. It is not a modal, does not overlay the
terminal, and does not scroll with it. Terminal output below renders exactly as it does today,
unaltered — the card is a control, the terminal remains the source of truth.

When the prompt is answered or cancelled — by the web UI, by the real terminal, or by anything
else — the card disappears on the next poll.

### The card

Two layouts, chosen by what the terminal itself is rendering (see Findings):

**List layout** — options carry prose descriptions. The card shows every label, and the
description of **only the highlighted option**, expanding under it. This is the core value: six
labels with one paragraph under the cursor is a menu you can scan; six labels with six paragraphs
is the wall we already have.

**Preview layout** — options carry a preview panel (ASCII diagrams, code, mockups). The card
shows the label list beside a monospace preview pane for the highlighted option, preserving column
alignment in a `<pre>`. Below 760px the pane stacks under the option list rather than shrinking.

Both layouts carry: the header chip (`☐ Choice mech`), the question, the option rows, a hairline
above the escape hatch Claude Code prints below its own rule (`Chat about this`), and a footer bar
whose primary button names the pending action (`⏎ Select 3`).

### Input

| Input | Effect |
|---|---|
| `↑` `↓` / `k` `j` | Move the highlight. Local only, no network, no latency. |
| `1`–`9` | Jump to that numbered option. Highlights; does not commit. |
| `Enter` | Commit the highlighted option. |
| `Esc` | Collapse the card to a one-line bar. **Not** forwarded to tmux. |
| Choosing an option that wants text | The card switches to a focused text box instead of closing. `Enter` sends, `Esc` abandons. An empty send does nothing. |
| Click a row | Highlight it. A second click on the already-highlighted row commits. |
| `Cancel ⎋` button | Sends a real `Escape` to tmux, after a click-again confirmation. |

Tap-to-highlight-then-tap-to-commit is the touch story: you can read an option's description or
preview on a phone without risking an accidental answer.

`Esc` collapsing rather than cancelling is deliberate. Dismissing a card and dismissing Claude's
question differ too much in consequence to share a key.

### Window switching

Switching the target window **never** commits, cancels, or discards anything. There is no modal
and no trap: the picker is a per-window control, not a global mode.

- Picker state (highlight index, local-vs-mirroring flag, fingerprint) is kept **per tmux target**
  in a client-side map. Selecting a different window in the dropdown leaves the previous window's
  state intact.
- Returning to a window restores your highlight, provided the prompt is still the same one
  (fingerprint match). If the prompt changed or was answered while you were away, the card resets
  to the terminal's own cursor with no warning — you never committed, so nothing was at stake.
- No `tmux select-window` is ever issued. Viewing and answering from the web UI does not change
  which window is active in the real tmux session.

### Finding the windows that are waiting

Switching away is only safe if you can find your way back. The session bar gains a **waiting
indicator**: a pill reading `● 2 WAITING` that jumps to the next window with a live prompt, and a
`●` marker appended to those windows' labels in the target dropdown.

This is polled separately from the main capture at 3s, using a visible-pane-only capture per
window (see Findings — a live prompt is always on the visible pane, so no scrollback is needed).

### Committing

The browser sends **intent, not keystrokes**. `Enter` posts the option index and the fingerprint
of the prompt you were looking at. The server re-captures, re-parses, refuses on mismatch, and then
presses the option's own digit — a single keystroke that selects it regardless of where the
terminal's cursor sits. Rows Claude Code renders without a number fall back to walking the cursor
one verified step at a time. No polling window sits between any of it.

If the fingerprint no longer matches, nothing is sent, the card reloads in a warning border, and
you re-choose against the current prompt.

Some options answer nothing and instead leave Claude waiting for typed text. The server reports
which happened, and the card switches to a focused text box rather than closing — see Findings.

## Key findings that shaped this design

Established from two live captures — `gbc` (list layout) and `test` (preview layout) — and by
reading the code. Several overturn the obvious approach, so they are recorded here.

- **The capture is plain text; colors are unavailable.** `capture_pane` shells out with `-p` and
  no `-e` (`src/main.rs:62`), so the ANSI attributes that mark the selected row in the terminal
  (xterm-256 color 153) never reach the browser. Detection must work on glyphs and structure
  alone. It does — and adding `-e` is rejected, because it would push escape sequences into
  `outputContent.innerHTML` and break the existing render for every window.

- **There are two distinct terminal layouts, not one.** When options carry previews, Claude Code
  renders a two-column view: a narrow option list on the left, a bordered preview pane on the
  right. A naive line regex captures the label as
  `A — Interactive prompt       ┌──────────────` — the box border becomes part of the label. The
  parser must first find the column gutter and split, then parse only the left column.

- **The gutter is detectable and stable.** In the `test` capture, the first box-drawing character
  sits at **column 34 on every one of lines 24–38**. Taking the modal first-box-column across the
  option block, requiring at least three lines to agree, and requiring it to be non-zero, cleanly
  separates the two columns. Column 0 box characters (Claude Code's welcome banner) are outside
  the option block and never interfere.

- **Options are not reliably numbered.** In `gbc`, the escape hatch is `6. Chat about this`. In
  `test`, the same option is rendered as bare `  Chat about this` with no number. Therefore
  **commits must address options by list index, never by printed number.** Number is display-only.

- **The number in the cursor regex is what prevents false positives.** Line 15 of the `test`
  capture is `❯ Please consider 3 different options…` — Claude Code echoing the user's own input,
  starting with the exact cursor glyph. Shell prompts using `❯` as PS1 (starship, pure) are the
  same hazard. Requiring `❯\s*\d+\.\s` — or, for unnumbered rows, membership in an already-matched
  numbered block — excludes both. A bare-`❯` detector would fire constantly.

- **A live prompt is always on the visible pane, and always at the tail.** In `test` the footer is
  at line 45 with only tmux's blank padding after it. This gives liveness for free: once a prompt
  is answered, Claude prints below it, so an old prompt sitting in scrollback can never re-arm the
  card. It also means the *waiting indicator* can use `capture-pane -p` with no `-S`, which is the
  cheapest possible probe.

- **The footer varies and must be matched loosely.** `gbc` ends with
  `Enter to select · ↑/↓ to navigate · Esc to cancel`; `test` ends with
  `Enter to select · ↑/↓ to navigate · n to add notes · Esc to cancel`. Match on
  `Enter to select` plus `to navigate`, not the whole string.

- **Descriptions are hard-wrapped mid-sentence at pane width.** `gbc` breaks
  `Gives\nhim Google SSO`. Continuation lines must be re-joined into one paragraph per option and
  re-wrapped by CSS. This de-raggedising is most of what makes the card look native rather than
  like pasted terminal text.

- **The output pane is destroyed and rebuilt every second.** `captureOutput` assigns
  `outputContent.innerHTML` wholesale (`static/index.html:1281`) and scrolls to the bottom. A card
  rendered inside that container would lose focus, hover, and highlight on every poll. The card
  must be a **sibling** of the output frame, in the fixed layout, not a child of it.

- **The page autofocuses the command textarea, where arrows move the caret.** Focus must be taken
  by the card, but only once per prompt — see Focus below.

- **Rapid repeated keys are dropped non-deterministically, so the commit does not use arrows at
  all.** From the same starting row, batched `send-keys Down` runs of 2, 3 and 4 landed on rows
  0, 1 and 4 respectively. Even *individually* sent arrows spaced 250ms apart were dropped, while
  the same arrows spaced ~600ms apart all landed. Batching Enter with movement is worse still: it
  commits the option focused *before* the move.

  **Pressing the option's own digit avoids all of it.** With the cursor on row 0, pressing `2`
  selected the option numbered 2 outright — one keystroke, no traversal, no race, and independent
  of where the cursor sits. The list layout numbers every row including both escape hatches, so
  traversal is never needed there. Only the preview layout's unnumbered "Chat about this" falls
  back to walking, and that walk sends **one arrow at a time and waits for each to land** before
  the next — condition-based, not a tuned sleep. If a step never lands, Enter is never pressed, so
  a failed move cannot answer the wrong option.

- **Two options don't answer anything — they ask for text.** `Type something.` does not commit;
  pressing its number turns the row into an inline text field and leaves the prompt up (the footer
  gains `ctrl+g to edit in VS Code`). Typed characters replace the label, and Enter submits.
  `Chat about this` does commit, but Claude then records *"User declined to answer questions"*,
  asks a follow-up, and returns to the ordinary prompt waiting to be typed at. Either way the user
  now owes Claude text, so the card must ask for it rather than closing and leaving them to work
  that out.

  The two are distinguished by **watching, not by matching the label**: after the keypress, the
  server polls briefly and reports `awaiting_text` if the prompt is still up with the cursor on the
  chosen row, `committed` otherwise.

- **One gesture on a phone can raise two events.** The keyboard's return and a tap on Send both
  reach the handler, and because the field was only cleared *after* the `await`, both read the same
  text and both sent it — reproduced as two identical requests and two submissions in the pane.
  Fixed with an in-flight guard on both the text send and the commit, plus a disabled button while
  the request is out. The same class of bug applies to any async handler wired to more than one
  event, which on touch is most of them.

- **`/api/send` presses Enter three times.** Pre-existing, and reasonable for the EXECUTE button.
  It is wrong for a reply typed into the card: the extra presses land in a TUI that queues input
  while it is busy. The picker's text path uses its own endpoint that presses Enter once.

- **The session bar overflows once the waiting pill joins it.** `.menu-btn` had neither
  `flex-shrink: 0` nor `white-space: nowrap`, so on a phone the label wrapped to two lines and the
  pill was clipped by the viewport edge. The dropdown also needed `min-width: 0` so it, rather than
  the buttons, absorbs the squeeze. Below 560px the pill drops the word "waiting" and the `TARGET:`
  label is hidden — a clipped pill reads as breakage, while a dot and a count do not.

- **The preview pane only ever renders the focused option.** Stepping the cursor from A to B on a
  live window replaced the pane's contents entirely. The client therefore cannot move the
  highlight locally in this layout — there would be nothing to show. Preview prompts steer the
  real cursor and follow it; only the list layout gets a local, zero-latency highlight.

## Architecture

### Parsing lives in Rust, exactly once

The client must never parse the pane itself. Two parsers — one to render, one to commit — will
drift, and the failure mode is a card that displays one thing and sends another.

New module `src/picker.rs`, pure and dependency-free:

```rust
pub struct Picker {
    pub fingerprint: String,      // sha256 of question + all labels + layout
    pub header: Option<String>,   // "Choice mech"
    pub question: String,
    pub cursor: usize,            // index of the terminal's own ❯ row
    pub layout: Layout,           // List | Preview
    pub options: Vec<Opt>,
}

pub struct Opt {
    pub number: Option<u32>,      // display only — may be absent
    pub label: String,
    pub description: Option<String>,  // List layout, continuation lines re-joined
    pub preview: Option<String>,      // Preview layout, column-preserved
    pub is_meta: bool,                // below the trailing rule
}

pub fn parse(pane: &str) -> Option<Picker>;
```

`parse` takes the whole pane text and returns `None` for anything it does not recognise with
confidence. **Fail closed** is the governing rule: an unrecognised prompt means no card and the
terminal renders as it does today, which is strictly no worse than the status quo.

### API surface

**`/api/capture`** — `CaptureResponse` gains one optional field. The picker rides the existing 1s
poll, so this costs no extra request and no extra tmux invocation:

```rust
struct CaptureResponse {
    content: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    window_closed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    picker: Option<Picker>,
}
```

Additive and optional, so `mobile/src/api.ts` keeps working untouched.

**`POST /api/picker/select`** — the guarded commit:

```
{ target: "0:2", index: 2, fingerprint: "4c9a…e1" }

1. capture-pane -p -t <target>
2. picker::parse — 409 Conflict if fingerprint differs or parse returns None
3. numbered row  -> send-keys <digit>            (one keystroke, done)
   unnumbered    -> walk one arrow at a time, waiting for each to land,
                    then send-keys Enter separately
4. poll ~2s: prompt gone -> "committed"
             prompt up, cursor on the row -> "awaiting_text"
```

Steps 1–4 run back-to-back with no polling window between them. The digit path exists because
repeated arrows are dropped non-deterministically; the walk exists only for rows Claude Code
renders without a number, and never presses Enter unless every step landed (see Findings). `target` is always explicit, never inferred from a notion of "current window" —
that is what keeps window switching safe, and it leaves answering-from-the-badge cheap to add
later without an API change.

**`POST /api/picker/step`** — moves the terminal's cursor one row without committing. Used by the
preview layout, where the client cannot move a highlight locally because only the focused option's
preview exists. Same fingerprint guard.

**`POST /api/picker/text`** — sends a typed reply as literal text plus **exactly one** Enter.
Deliberately not `/api/send`, which presses Enter three times 500ms apart as a delivery workaround
for the fire-and-forget EXECUTE button; those extra presses reach a TUI that queues input while
busy (see Findings).

**`GET /api/pending-questions`** — `["0:2", "0:8"]`. Runs `capture-pane -p` (visible pane only,
no `-S`) per window and reports which ones `picker::parse` accepts. Polled at 3s.

### Frontend

A new `#pickerCard` node between `.session-bar` and `.input-frame` in the fixed layout. All state
lives in one module-scoped object:

```js
pickerByTarget = Map<target, { cursor, dirty, fingerprint }>
focusedPrompts = Set<`${target}|${fingerprint}`>
```

**Mirror-until-touched (list layout).** Each poll carries the terminal's own cursor. While `dirty`
is false the card follows it, so arrowing on the real tmux moves the web highlight. The first local
move sets `dirty` and the card stops following, so a background redraw cannot yank your selection.
The footer states which mode it is in.

**Always-mirror (preview layout).** `dirty` is forced false. Arrowing calls `/api/picker/step` and
the card follows the terminal, because the preview for an unfocused option does not exist to show.
The footer reads `steering terminal` so the difference is visible rather than mysterious. A capture
is triggered immediately after each step rather than waiting out the 1s tick.

**Focus, once.** The card calls `.focus()` only when `` `${target}|${fingerprint}` `` is not in
`focusedPrompts`, then records it. Switching to a window whose prompt you have not yet seen takes
focus; every subsequent poll on that same prompt does not, so it cannot fight you after you click
back into the textarea. When the textarea holds focus, the card keeps its highlight, stops
handling arrows, and says so.

**Rendering.** The card is rebuilt only when the fingerprint changes; cursor moves mutate
`aria-selected` in place. This keeps the description expand/collapse animation smooth and avoids
the churn that killed the inline approach.

## Error handling

| Situation | Behavior |
|---|---|
| Prompt changed between render and commit | 409, nothing sent, card reloads with a warning border. |
| Prompt vanished (answered elsewhere) | 409, card disappears on the next poll. |
| Parse fails / unfamiliar layout | No card. Terminal renders as today. No error shown. |
| Cursor fails to reach the chosen row | Enter is never sent. 409 with the current prompt, card re-renders. |
| Text-mode reply sent while the prompt is gone | Expected for `Chat about this` — the text goes to the ordinary prompt via `/api/send`. |
| `send-keys` fails midway | Cursor may have moved, nothing committed. Next poll resyncs the card. Non-destructive. |
| Window closed while a prompt was pending | Existing `window_closed` path runs; per-window state is dropped. |
| `/api/pending-questions` fails | Waiting indicator hides. Never blocks the main capture. |

## Testing

`picker::parse` is a pure function over text, so it is tested directly against fixtures captured
from real windows — `tests/fixtures/picker/`:

- `list_gbc.txt` — list layout, numbered meta option, six choices.
- `preview_test.txt` — preview layout, unnumbered meta option, column gutter at 34.
- `answered.txt` — the `gbc` prompt with Claude's reply printed below it. Must return `None`.
- `shell_prompt.txt` — a `❯`-prefixed PS1 and the `❯ Please consider…` input echo. Must return `None`.
- `plain.txt` — ordinary shell output. Must return `None`.

Assertions cover: cursor index, option count, meta flags, number-absent handling, description
re-joining across wrapped lines, preview column fidelity, fingerprint stability across cursor
movement, and that `move_keys` never emits `Enter`.

The fixtures are masked. `list_gbc.txt` preserves the structure and line-wrap points of the real
`gbc` capture with the third party's addresses and the organisation name substituted, because this
repository is public.

End-to-end behaviour is verified against live tmux windows rather than only fixtures — that is how
the dropped-key and double-send bugs were found, and no unit test over captured text could have
caught either. The double-send regression is pinned by driving a phone-sized viewport, firing the
return key and a Send tap in the same tick, and asserting one request and one line in the pane. The
browser flow is driven headless: card renders, focus is taken once, only the selected description
expands, arrows and number keys move the highlight, `Esc` collapses without cancelling, switching
windows preserves the highlight across a round trip, the waiting pill appears for other windows,
and a committed choice is confirmed by reading back what Claude recorded.

## Out of scope

- **Multi-select prompts.** Detected only insofar as they fail to parse; the card does not arm and
  the terminal handles them. Guessing at a commit sequence for a checkbox list is not worth the
  risk.
- **`n to add notes`.** The `test` footer offers it; the card ignores it. Notes remain a
  terminal-only affordance.
- **Answering a background window from the badge.** The API supports it by construction; the v1 UI
  only commits for the window you are viewing, which keeps the mental model simple.
- **Free-text prompts.** The existing command input already handles those.
- **The mobile app.** Consumes the new field whenever it chooses to; no work here.

## Implementation phases

1. **`src/picker.rs` + fixtures.** Parser and tests, no wiring. Both layouts, all negative cases.
2. **`/api/capture` field + `/api/picker/select`.** Backend complete and curl-testable.
3. **The card.** Rendering, keyboard, touch, focus rule, per-window state, both layouts.
4. **Waiting indicator.** `/api/pending-questions`, the session-bar pill, dropdown markers.

Phases 1–3 deliver the feature. Phase 4 is what makes switching away genuinely safe rather than
merely permitted, and should not be dropped.
