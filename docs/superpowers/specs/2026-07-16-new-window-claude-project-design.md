# New Window → Named Claude Project — Design

**Date:** 2026-07-16
**Status:** Approved
**Scope:** `static/index.html` (frontend) + `src/main.rs` (backend). Requires a Rust rebuild
and redeploy. Mobile app unchanged.

## Problem

Today "New Window" (☰ MENU item and `^B c`) creates a blank tmux window with a default name.
The user wants it to instead prompt for a project name and, on submit, open a tmux window that
is already `cd`'d into `~/p/<name>` with `claude --yolo` running, named `<name>`.

## Behavior

Clicking **New Window** (or pressing `^B c`) opens a name-prompt modal. On submit with a valid
name `<name>`:

1. The window opens in `~/p/<name>` (created if it does not exist), named `<name>`, with
   `claude --yolo` launched in it, and the UI switches to it.
2. If a window named `<name>` already exists, the UI switches to it and nothing new is created.

This **replaces** the old blank-window behavior. There is no longer a one-click blank window from
the menu; both entry points now prompt.

## Key findings that shaped this design

These were established empirically (in a throwaway tmux session) and by reading the code. They
overturn several intuitions, so they are recorded here.

- **`new-window -n <name>` makes the name permanent.** It sets that window's `automatic-rename`
  to `0` at creation, so the name holds even while `claude` runs. **No follow-up
  `rename-window` is needed** — a rename-after step would be dead code. (Global `automatic-rename`
  is `on`, but `-n` makes that irrelevant for this window.)

- **The working directory is set via argv, not a typed `cd`.** `new-window -c <dir>` starts the
  window's shell in `<dir>`. This means the user's name never enters a shell command string, which
  is what removes the command-injection path — not the input validation. There is **no typed
  `cd ~/p/<name>`** anywhere in this design.

- **Shell metacharacters in a window name are harmless; `#` is the dangerous one.** A name is
  passed as a single argv element to `tmux`, never through a shell, so `;`, `$`, backticks, quotes,
  spaces all store verbatim. But `-n` runs tmux **format expansion** on the name: `#{window_index}`,
  `#H`, `#S` get interpolated and baked in. The chosen allowlist excludes `#`, so this is closed by
  construction.

- **Duplicate window names are a targeting hazard.** tmux allows two windows with the same name,
  and `-t session:name` then resolves to the **lowest-indexed** match — the *old* window. So the
  design never targets by name after creation. `new-window -P -F` returns the immutable
  `window_id` (`@N`), and all subsequent `send-keys` target that.

- **`-l` (literal) must cover only the payload, never `Enter`.** `send-keys -l 'claude --yolo' Enter`
  types the word "Enter" as text. The launch is therefore two calls: `send-keys -l <cmd>` then a
  separate `send-keys Enter`. This matches the existing `/api/send` pattern (`src/main.rs:121,130`).

- **`claude --yolo` is a shell function, not a binary flag.** `~/.bash_aliases` defines a `claude()`
  function mapping `--yolo` → `--dangerously-skip-permissions` (and unsetting `CLAUDECODE` /
  `ANTHROPIC_API_KEY`). The binary rejects `--yolo`. So the command MUST be **typed into the
  window's interactive bash** via `send-keys`, where the function is in scope. The backend must
  never `Command::new("claude")`.

- **Security context (informational, not addressed here).** The server has no authentication and
  binds `0.0.0.0`; `/api/send` already types arbitrary caller text into a shell. Unauthenticated
  callers who can reach the port already have RCE. In deployment this is mitigated externally by
  Google authentication via Cloudflare Access in front of the service. Name validation in this
  feature is therefore **correctness hygiene, not a security control.**

## Name validation

A valid name matches `^[A-Za-z0-9._-]+$` AND does not begin with `-` (tmux argv/flag-injection
guard) AND is not `.` or `..` (would make `mkdir -p ~/p/<name>` resolve to `~/p` or `~`). Validated
on **both** ends: the frontend for immediate feedback, the backend as the authority (a client is
not trusted to have validated). Invalid input returns an error and creates nothing.

This matches how `~/p` directories are already named (`adk-samples`, `3dmodels`, `A2A`). No spaces.

## Backend

New handler and route: `POST /api/new-window-named`.

- **Extractor:** `Option<Json<NewWindowNamedRequest>>` where `NewWindowNamedRequest { name: String }`.
  A bare `Json<T>` would return 415 for the existing no-body `/api/new-window` callers if they ever
  hit the wrong route; using `Option<Json<...>>` and rejecting `None` with a clear 400 is the safe
  axum-0.7 pattern. The existing `/api/new-window` handler and route are left unchanged.

- **Directory:** resolve `dir = <HOME>/p/<name>` using `std::env::var("HOME")`. Reject if `name`
  fails validation before touching the filesystem.

- **Steps, in order:**
  1. Validate `name`. On failure → `400 { success:false, error:"invalid name" }`.
  2. Check for an existing window named `name`: run `list-windows -a -F '#{window_id}\t#{window_name}'`,
     parse, and if a row's name equals `name`, return `{ success:true, target, window_id, existing:true }`
     for that window without creating anything. (Read-only — does **not** use `select-window`, which
     would switch the user's active window as a side effect.)
  3. `mkdir -p <dir>` via `std::fs::create_dir_all`. On failure → `500 { error }`.
  4. `tmux new-window -c <dir> -n <name> -P -F '#{window_id}\t#{session_name}:#{window_index}'`.
     Parse stdout into `window_id` and `target`. On failure → `500 { error: stderr }`.
  5. **Validate context before launching (per user requirement):** query
     `tmux display-message -p -t <window_id> '#{pane_current_path}'`. Compare the trimmed result to
     the canonicalized `<dir>`. If they do **not** match, do **not** send the launch command; return
     `500 { success:false, error:"window did not start in <dir> (got <actual>)", target, window_id }`.
     The window is left at a bare shell prompt — never fire `claude --yolo` (which skips permission
     prompts) in an unintended directory.
  6. On match: `tmux send-keys -t <window_id> -l 'claude --yolo'`, then a **separate**
     `tmux send-keys -t <window_id> Enter`. Return `{ success:true, target, window_id, existing:false }`.

- **Response shape:** `{ success: bool, target?: string, window_id?: string, existing?: bool, error?: string }`.
  `target` is `session:index` for the frontend's existing selection logic; `window_id` is the stable
  handle. No sleeps are needed (unlike `trigger_bugfix_window`) — the launch does not depend on a
  prior process having booted.

## Frontend

- **Modal:** a new `#newWindowModal` cloning `#renameModal`'s markup, reusing the existing
  `.modal-overlay` / `.rename-modal` / `.rename-input` / `.rename-hint` CSS classes (zero new CSS).
  Header "New Claude Window"; input placeholder "Project name (a-z, 0-9, . _ -)"; hint
  "Enter to create • Esc to cancel". No `<form>` element (matches the codebase's Enter-in-JS
  convention).

- **Open:** `createNewWindow()` is repurposed to open this modal (clear the input, add `.show`,
  then focus via the established 50ms `setTimeout` idiom — a synchronous focus fails while the
  element is still `display:none`, and would also be stolen by `closeActionMenu()`'s synchronous
  `commandInput.focus()` on the menu path). Both existing call sites (`^B c` at the `case 'c'`
  branch, and the `#menuNewWindow` click handler) now reach the prompt unchanged.

- **Key routing:** add a `#newWindowModal` branch to `handleModalKeydown`, placed adjacent to the
  rename branch (before the action-menu branch). Branch order is load-bearing — each branch
  `return true`s unconditionally, so the first matching `.show` wins. Escape closes (focus returns
  to `commandInput`); Enter reads the input value, closes, and calls `submitNewWindow(name)`.

- **`submitNewWindow(name)`:**
  1. Trim. If empty or fails `^[A-Za-z0-9._-]+$` (or starts with `-`) → red status
     "INVALID NAME", reopen/keep the modal, create nothing.
  2. `POST /api/new-window-named` with `{ name }` and `Content-Type: application/json`.
  3. On `success`: `await loadWindows()`, then select the returned `target` by handing it to the
     render path as an explicit keep/select target (see MASTER-filter fix below), `startCapture()`,
     status "NEW WINDOW: <name>" (or "SWITCHED TO <name>" when `existing:true`), focus `commandInput`.
  4. On failure: red status with the server's `error`.

- **MASTER-filter fix (latent bug found during investigation):** today `createNewWindow` does
  `sessionSelect.value = data.target` *after* `renderWindowOptions()` has run. If the new window's
  name contains `MASTER`, the filter drops it from the options and that assignment silently no-ops
  to `''`, yet a green success toast still shows. Fix: pass the new target into the selection path
  so a just-created window is always kept visible and selected regardless of the MASTER filter —
  the same "keep the current target visible" mechanism the filter already implements. Do not add a
  second `startCapture()`; `renderWindowOptions()` already calls it.

## Testing

Verification drives the real app in a real browser via Playwright installed in the session
scratchpad only (no project dependency), launched with `{ channel: 'chrome' }`. **Against a dev
server on port 5534 serving the repo's `static/` and the freshly rebuilt binary — never 5533**,
the user's live install.

Backend changes mean the dev server must be rebuilt (`cargo build --release`) and restarted for
each backend iteration, unlike the frontend-only MASTER feature.

**The tmux session is the user's live workspace.** Tests must not create, rename, or kill windows
in it. Backend behavior is exercised through the API against windows the test itself creates *and
cleans up* — or, preferably, the risky tmux interactions are unit-tested at the validation layer
(pure name-validation function) plus one guarded end-to-end creation into a scratch project name
that is killed afterward via `window_id`.

Cases:

1. **Name validation (pure, exhaustive):** accepts `foo`, `my-proj`, `A2A`, `3dmodels`, `a.b_c-1`;
   rejects ``, ` `, `foo bar`, `foo;rm`, `../etc`, `.`, `..`, `-rf`, `a#b`, `a$(id)`, `foo/bar`,
   and a name with a leading `-`.
2. **Create new:** submitting `claudetest-<unique>` creates a window named exactly that, whose
   pane starts in `~/p/claudetest-<unique>`, with the directory created on disk. Clean up: kill the
   window by `window_id` and remove the scratch dir.
3. **Directory-context guard:** if `pane_current_path` does not match the intended dir, the launch
   command is not sent and the API returns an error (simulated by pointing at a dir that
   `create_dir_all` cannot produce, e.g. under a read-only parent, or by asserting the compare logic
   directly).
4. **Existing window:** creating a name that already exists returns `existing:true` with the
   original window's `window_id`, and creates no second window (`list-windows` count unchanged).
5. **`#` and other format chars** are rejected by validation (cannot reach `-n`).
6. **MASTER-named project:** creating a window named e.g. `MASTERtest` still selects and displays it
   (regression guard for the filter no-op fix). Clean up afterward.
7. The launch types `claude --yolo` followed by a separate Enter (assert two send-keys calls, `-l`
   on the payload only).

## Out of scope

- Configurable base directory (hardcoded `~/p/`).
- Any change to `/api/new-window` or the old blank-window path beyond rerouting its two callers.
- Mobile app (`mobile/src/api.ts` `createWindow()` is dead code; left as-is).
- Adding authentication to the server (mitigated externally by Cloudflare Access).
