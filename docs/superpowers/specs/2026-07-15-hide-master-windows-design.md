# Hide MASTER Windows — Design

**Date:** 2026-07-15
**Status:** Approved
**Scope:** Web UI only (`static/index.html`). No backend changes. No mobile app changes.

## Problem

tmux windows whose name contains `MASTER` are long-lived and rarely need attention. They
currently occupy slots in the window dropdown, the `^B w` window list, and the `^B n`/`^B p`
navigation cycle. Most of the time they should be out of the way, but they must remain
reachable on demand.

The set of MASTER windows changes at runtime — during planning, a window named `mm DEV*` was
renamed to `mm MASTER` between two listings minutes apart, and the non-MASTER count shifted from
7 to 6 between test runs the same afternoon. Tests must therefore compute expected windows from
`/api/windows` at run time and never hardcode names or counts.

## Behavior

MASTER windows are hidden by default. A new ☰ MENU entry toggles them into and out of view.
The toggle's label states the action it performs:

| State | Menu label | Effect of clicking |
|---|---|---|
| MASTER hidden (default) | `Unhide MASTER` | MASTER windows become visible |
| MASTER shown | `Hide MASTER` | MASTER windows become hidden |

Hiding removes MASTER windows from the dropdown, from the `^B w` window list modal, and from
the `^B n`/`^B p` navigation cycle — navigation skips them rather than landing on a window
absent from the list.

The preference persists across page reloads.

## Detection

A window is a MASTER window when its tmux window name contains the substring `MASTER`,
case-sensitively:

```js
win.name.includes('MASTER')
```

Matching is against `win.name` (the bare tmux window name from `/api/windows`), never against
`win.target` or the composed `"target - name"` option label. This prevents a session or window
index containing `MASTER` from triggering a match.

Two consequences follow from "all caps, anywhere in the name" and are intended:

- A window named `master` (lowercase) is **not** hidden.
- A window named `MASTERMIND` **is** hidden.

## Architecture

### Why filter in the frontend

`/api/windows` is shared with the mobile app, and this feature is scoped to the web UI. Filtering
server-side would require a Rust rebuild plus a service restart (`make update`) and would risk
altering mobile behavior. The frontend already receives window names, so the filter is a pure
client concern. `static/` is served with no-cache headers, so a browser refresh deploys it.

### The single choke point

The `sessionSelect` dropdown is the source of truth for every window surface in the web UI:

- `openWindowModal()` (`static/index.html:1380`) builds `modalWindows` by mapping over
  `sessionSelect.options`.
- `handlePrefixCommand('n')` / `('p')` (`static/index.html:1687`, `:1696`) move
  `sessionSelect.selectedIndex` within those same options.

Filtering what enters the dropdown therefore covers all three surfaces at once. No changes are
needed in the modal or navigation code.

### Components

**`allWindows`** — module-scope array holding the unfiltered `/api/windows` response.
`loadWindows()` fetches into it, then delegates rendering.

**`renderWindowOptions()`** — the sole owner of dropdown population. Reads `allWindows`, applies
the visibility filter, rebuilds `sessionSelect.options`, and restores selection. Owns the saved-target
resolution and the `N WINDOWS` status message currently inline in `loadWindows()`.

**`showMaster`** — boolean backed by `localStorage['tmux-show-master']`. Absent or any value other
than `'true'` means hidden, so the default is hidden without seeding storage.

**Menu toggle** — a new `.action-menu-item` with id `menuToggleMaster`, placed above
`Refresh Windows` in the action menu list. Its click handler flips `showMaster`, persists it, calls
`renderWindowOptions()`, and closes the menu. Its label is recomputed in `openActionMenu()` on
every open, so it cannot drift out of sync with stored state.

Toggling re-renders from `allWindows` with no refetch, so it is instant and works offline.

## Edge cases

**Currently-selected MASTER window.** `renderWindowOptions()` always emits the currently selected
target even when it is a MASTER window and MASTER windows are hidden. Hiding never changes which
terminal is being viewed. The window drops off the list once the user switches away and the next
render runs.

The "currently selected target" is `keepTarget = sessionSelect.value || localStorage['tmux-selected-target']`,
read **before** the options are wiped. Both sources are required, and they are deliberately kept
separate from `savedTarget`:

- `sessionSelect.value` is the live selection but is empty on the very first render.
- `localStorage['tmux-selected-target']` persists across reloads but is only written by the
  `change` handler and explicit flows (`selectWindow`, create-window) — so the default-selected
  window on a fresh load is **not** in it.

Deriving `keepTarget` from localStorage alone is a bug, found in review: on a fresh load the
default-selected window is unpersisted, so renaming the window you are currently viewing to include
`MASTER` (which happens — see Problem) would filter it out and silently jump you to a different
terminal, violating this very invariant. `savedTarget` must keep driving selection restore and the
invalid-target fallback on its own; overwriting it with `sessionSelect.value` would break the
clearing of stale localStorage entries.

**Filter would empty the list.** If every window is a MASTER window and none is selected, the
filter falls open and renders all windows rather than leaving a dead dropdown.

**Status count.** The `N WINDOWS` message counts rendered (visible) windows, not `allWindows.length`.

**Invalid saved target.** Existing behavior is preserved: if the saved target is absent from the
rendered list, selection falls back to the first option and the saved value is cleared.

## Testing

Automated verification drives the real app in a real browser via Playwright, installed in the
session scratchpad only (no project dependency, honoring the no-build-step convention). Launch with
`{ channel: 'chrome' }` — the cached Playwright browser build is older than the installed Playwright
expects, so system chrome is used.

**Test against port 5534, never 5533.** Port 5533 is the live installed app at `~/bin/tmux-terminal/`
(a systemd `--user` service) serving its *own* copy of `static/`; tests there would not exercise repo
edits at all. A dev server on 5534, run from the repo root, serves the repo's `static/`
(`ServeDir::new("static")` resolves relative to the working directory) and reflects edits with no
restart and no rebuild.

Tests must not create, rename, or kill tmux windows — that session is the user's live workspace.
Page internals are driven via `page.evaluate` instead.

Scripts: `verify-task1.js`, `verify-task2.js`, `verify-task3.js`, `verify-keeptarget.js`,
`verify-task1-extra.js`.

1. Default state on a fresh load hides every window whose name contains `MASTER`; all others appear.
   Expected counts are derived from `/api/windows` at test time.
2. Menu reads `Unhide MASTER`; clicking it reveals every MASTER window and the label becomes
   `Hide MASTER`.
3. Cycling `^B n` from the first window to the last never lands on a MASTER window while hidden.
4. `^B w` modal lists only visible windows.
5. Reloading the page preserves the toggle state.
6. Selecting a MASTER window while shown, then choosing `Hide MASTER`, keeps it listed and selected;
   switching to another window then removes it from the list.
7. A window named `master` (lowercase) stays visible while hiding is on; `MASTERMIND` is hidden.
8. Viewing an unpersisted default-selected window that becomes MASTER keeps it listed and selected
   (`verify-keeptarget.js` — the review finding above).
9. With every window MASTER and none selected, the filter falls open and shows them all rather than
   rendering an empty dropdown.

## Out of scope

- Mobile app (`mobile/`) — continues to show all windows.
- Backend (`src/main.rs`) — unchanged.
- Hiding by any pattern other than the literal `MASTER` substring; no user-configurable pattern.
