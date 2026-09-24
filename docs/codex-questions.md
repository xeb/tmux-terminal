# Codex asynchronous questions

Codex 0.154 can keep questions under **Queued follow-up inputs**, while its main
composer and running turn remain available. This differs from the older
`request_user_input` dialog already handled by the picker.

When a live queue is detected, the website shows **Answer questions (N)**. Tapping
it opens Codex's question editor and displays native answer buttons above the
normal command box. A tap selects a choice; Select submits it, and another tap
on the armed choice also submits. Further questions appear in the same card.
The final Other option opens a separate answer field; free-text questions show
that field directly, with their question number. **Back to typing** returns
to the main composer without answering, skipping, or interrupting the turn.
The normal command draft stays intact throughout.

`POST /api/picker/open` verifies the current queue fingerprint before sending
its displayed entry key. `POST /api/picker/close` follows the displayed
previous-question/main-prompt hints until the main composer is visible.
Supported hints are Shift+Left/Right and Alt+Up/Down (including macOS symbols).
Both spaced (`shift + ←`) and compact (`shift+←`) key hints are accepted, as
are lowercase model IDs and capitalized display names such as `GPT-6-Astra`.
Unknown hints and clipped or unrecognized layouts are refused. No Escape key
is used for this transition because Codex can interpret it as an interruption.

The existing picker selection and text endpoints verify live question
fingerprints. Async custom answers include the fingerprint, so a late response
cannot type into another question or the main composer. Picker transitions and
normal `/api/send` calls share a lock for the resolved tmux pane id; normal text
submission first exits an active asynchronous question editor and confirms the
main prompt. Window reorders therefore cannot redirect an in-flight action.

Typed answers wait for a stable native draft before sending a single Enter,
outside Codex's [paste Enter suppression window](https://github.com/openai/codex/blob/main/codex-rs/tui/src/bottom_pane/paste_burst.rs).
The server reports `changed` or `committed` only after observing the next question
or main composer. A timeout reports `pending`: the website keeps the draft and
allows another submission without typing it twice. Visible native drafts are
also recoverable after a reload using **Submit answer**. An empty `text` with a
valid async fingerprint submits that existing draft; it cannot submit an empty
editor. A different nonempty answer is refused while a native draft exists.

Queue detection requires the live Codex composer and footer below a queue count
and a recognized `to answer` hint. Active-question parsing requires its submit,
skip, and navigation footer, consecutive option numbers, and a live cursor.
Fingerprints include the question's position and labels, excluding cursor
movement and the Other field's draft. Older Claude and Codex prompt layouts
retain their existing parser paths.

Fixtures include the supplied queue screenshot and expanded examples based on
Codex's [question UI](https://github.com/openai/codex/tree/main/codex-rs/tui/src/bottom_pane/async_questions)
and [queue summary](https://github.com/openai/codex/blob/main/codex-rs/tui/src/bottom_pane/questions.rs).
Run the Rust, browser, and isolated HTTP checks described in
[mobile-performance.md](mobile-performance.md).
