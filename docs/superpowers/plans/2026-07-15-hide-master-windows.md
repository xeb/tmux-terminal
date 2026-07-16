# Hide MASTER Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hide tmux windows whose name contains `MASTER` from the web UI by default, with a ☰ MENU toggle to reveal them.

**Architecture:** Pure frontend change in `static/index.html`. The `sessionSelect` dropdown is the single source of truth for every window surface (the `^B w` modal maps over its options; `^B n`/`^B p` move its `selectedIndex`), so one filter at dropdown-population time covers all three surfaces. `loadWindows()` is split: it fetches into a module-scope `allWindows` array, and a new `renderWindowOptions()` owns all dropdown population and filtering. Toggling re-renders from `allWindows` with no refetch.

**Tech Stack:** Vanilla JS inline in `static/index.html`. No build step. No backend changes. No new project dependencies.

## Global Constraints

- **Only `static/index.html` may be modified.** No changes to `src/main.rs`, `mobile/`, `Cargo.toml`, or `package.json`.
- **No new project dependencies and no build step.** Per CLAUDE.md: "Vanilla JS with inline CSS, no build step." The verification harness lives entirely in the scratchpad, outside the repo.
- **Do not touch the existing Logout code** in `static/index.html` (the `.action-menu-item.menu-logout` CSS rule, the `#menuLogout` button, and its click handler). It was committed separately in `5f4bbb0` before this work began. Do not revert, move, or reformat it.
- **Work happens on branch `feature/hide-master-windows`**, branched from `5f4bbb0` on `master`.
- **Detection string is the literal `MASTER`**, case-sensitive substring, matched against `win.name` only — never `win.target` or the composed option label.
- **Default is hidden.** `localStorage['tmux-show-master']` absent means hidden.
- **Existing behavior preserved:** the saved-target fallback (invalid saved target → select first, clear storage) and the `FAILED TO LOAD WINDOWS` error path must behave exactly as they do today.

## Testing Approach — Read This First

This project has **no frontend test framework** and no build step, and the JS is inline in an HTML file, so it cannot be imported by a unit test runner. Standing up Jest/Vitest plus extracting the JS into modules would be a larger and riskier change than the feature itself, and would violate the codebase's stated conventions. It is deliberately out of scope.

Instead, verification drives the **real app in a real browser** via Playwright, installed in the scratchpad only:

```
/tmp/claude-1000/-media-xeb-GreyArea-projects-tmux-terminal/fa047f04-3ef6-4032-b4b1-72c7b3c4e4b8/scratchpad
```

Playwright is already installed there and confirmed working. It must launch with `{ channel: 'chrome' }` — the cached Playwright browser build (1208) is older than what the installed Playwright expects (1228), so the system `google-chrome` is used instead. Do not run `npx playwright install`.

The app must be reachable at `http://localhost:5533/`. It is currently served and returns 200. If it is not running, start it with `cargo run` from the repo root (note: the systemd service is `inactive`; something else is serving port 5533 — do not restart the service).

**The live tmux session changes under you.** Between two listings during planning, a window named `mm DEV*` was renamed to `mm MASTER`. At plan time the session has 11 windows, 3 of them MASTER (`alexa MASTER`, `mm MASTER`, `sink MASTER`). **Never hardcode window counts or names in assertions** — always compute expected values from `/api/windows` at test time.

---

### Task 1: Filter engine — `allWindows`, visibility rules, and `renderWindowOptions()`

Splits fetching from rendering and makes MASTER windows hidden by default. After this task MASTER windows are always hidden (no toggle yet) — independently testable and independently rejectable.

**Files:**
- Modify: `static/index.html:1190-1195` (state declarations)
- Modify: `static/index.html:1280-1324` (`loadWindows`)
- Test: `<scratchpad>/verify-task1.js` (create)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces, all at module scope in the inline `<script>`, relied on by Task 2:
  - `const SHOW_MASTER_KEY = 'tmux-show-master'` — localStorage key.
  - `let showMaster: boolean` — mutable visibility flag.
  - `let allWindows: Array<{target: string, name: string}>` — unfiltered API response.
  - `function isMasterWindow(win: {name: string}): boolean`
  - `function visibleWindows(keepTarget: string|null): Array<{target, name}>`
  - `function renderWindowOptions(): void` — rebuilds `sessionSelect.options`, restores selection, calls `startCapture()`, emits the `N WINDOWS` status.

- [ ] **Step 1: Write the failing test**

Create `<scratchpad>/verify-task1.js`:

```js
const { chromium } = require('playwright');

const APP = 'http://localhost:5533/';

function isMaster(name) { return name.includes('MASTER'); }

(async () => {
  // Ground truth straight from the API, computed at run time — the live
  // session's windows change, so nothing here may be hardcoded.
  const api = await (await fetch(APP + 'api/windows')).json();
  const masters = api.filter(w => isMaster(w.name));
  const nonMasters = api.filter(w => !isMaster(w.name));
  console.log(`API: ${api.length} windows, ${masters.length} MASTER`);
  if (masters.length === 0) throw new Error('No MASTER windows in session; cannot test hiding.');
  if (nonMasters.length === 0) throw new Error('All windows are MASTER; cannot test hiding.');

  const browser = await chromium.launch({ channel: 'chrome' });
  const page = await browser.newPage();

  // Fresh state: no stored preference, no stored target.
  await page.goto(APP);
  await page.evaluate(() => localStorage.clear());
  await page.goto(APP, { waitUntil: 'networkidle' });
  await page.waitForFunction(
    () => document.querySelectorAll('#sessionSelect option').length > 0
  );

  const shown = await page.$$eval('#sessionSelect option', els =>
    els.map(e => e.textContent)
  );
  const leaked = shown.filter(t => t.includes('MASTER'));
  const failures = [];

  if (leaked.length > 0) failures.push(`MASTER windows visible by default: ${JSON.stringify(leaked)}`);
  if (shown.length !== nonMasters.length) {
    failures.push(`Expected ${nonMasters.length} options, got ${shown.length}: ${JSON.stringify(shown)}`);
  }
  for (const w of nonMasters) {
    if (!shown.some(t => t.startsWith(w.target + ' '))) {
      failures.push(`Non-MASTER window missing from dropdown: ${w.target} - ${w.name}`);
    }
  }

  await browser.close();
  if (failures.length) { console.error('FAIL:\n' + failures.join('\n')); process.exit(1); }
  console.log(`PASS: ${shown.length} non-MASTER windows shown, ${masters.length} MASTER hidden`);
})().catch(e => { console.error('ERROR:', e.message); process.exit(1); });
```

- [ ] **Step 2: Run test to verify it fails**

Run from the scratchpad directory: `node verify-task1.js`

Expected: exit 1, `FAIL:` listing MASTER windows visible by default (e.g. `["0:2 - alexa MASTER","0:3 - mm MASTER","0:6 - sink MASTER"]`), because no filtering exists yet.

- [ ] **Step 3: Add state declarations**

In `static/index.html`, immediately after `let modalWindows = [];` (line 1195), add:

```js

        // MASTER window visibility. Windows whose tmux name contains "MASTER"
        // are hidden by default; the ☰ MENU toggle reveals them.
        const SHOW_MASTER_KEY = 'tmux-show-master';
        let allWindows = [];
        let showMaster = localStorage.getItem(SHOW_MASTER_KEY) === 'true';
```

- [ ] **Step 4: Add the filter helpers and `renderWindowOptions()`**

In `static/index.html`, immediately before `async function loadWindows() {` (line 1280), add:

```js
        function isMasterWindow(win) {
            return win.name.includes('MASTER');
        }

        // The currently selected target is always kept, even when it is a MASTER
        // window, so hiding never switches the terminal being viewed. If the filter
        // would empty the list entirely, fall open rather than leave a dead dropdown.
        function visibleWindows(keepTarget) {
            if (showMaster) return allWindows;
            const visible = allWindows.filter(
                win => !isMasterWindow(win) || win.target === keepTarget
            );
            return visible.length > 0 ? visible : allWindows;
        }

        function renderWindowOptions() {
            const savedTarget = localStorage.getItem('tmux-selected-target');
            const windows = visibleWindows(savedTarget);

            sessionSelect.innerHTML = '';

            if (windows.length === 0) {
                sessionSelect.innerHTML = '<option value="">No windows found</option>';
                return;
            }

            let foundSaved = false;

            windows.forEach((win, index) => {
                const option = document.createElement('option');
                option.value = win.target;
                option.textContent = `${win.target} - ${win.name}`;
                // Select saved target if it exists, otherwise default to first
                if (savedTarget && win.target === savedTarget) {
                    option.selected = true;
                    foundSaved = true;
                } else if (!savedTarget && index === 0) {
                    option.selected = true;
                }
                sessionSelect.appendChild(option);
            });

            // If saved target wasn't found, select first and clear invalid saved value
            if (savedTarget && !foundSaved) {
                sessionSelect.selectedIndex = 0;
                localStorage.removeItem('tmux-selected-target');
            }

            startCapture();
            showStatus(`${windows.length} WINDOWS`, 'success');
        }

```

- [ ] **Step 5: Rewrite `loadWindows()` to fetch-then-render**

Replace the entire body of `loadWindows()` (lines 1280-1324, from `async function loadWindows() {` through its closing brace) with:

```js
        async function loadWindows() {
            try {
                const controller = new AbortController();
                const timeout = setTimeout(() => controller.abort(), 5000);
                const response = await fetch('/api/windows', { signal: controller.signal });
                clearTimeout(timeout);
                allWindows = await response.json();

                renderWindowOptions();
            } catch (err) {
                sessionSelect.innerHTML = '<option value="0">0 (default)</option>';
                showStatus('FAILED TO LOAD WINDOWS', 'error');
            }
        }
```

Note: the original's empty `finally {}` block is dropped as dead code. All selection, status, and `startCapture()` logic now lives in `renderWindowOptions()`.

- [ ] **Step 6: Run test to verify it passes**

Run from the scratchpad directory: `node verify-task1.js`

Expected: exit 0, `PASS: N non-MASTER windows shown, M MASTER hidden`.

- [ ] **Step 7: Commit**

```bash
git add static/index.html
git commit -m "feat: hide MASTER tmux windows from web UI window list"
```

---

### Task 2: MENU toggle with flipping label

Adds the ☰ MENU entry that reveals and re-hides MASTER windows, with a label stating the action it performs.

**Files:**
- Modify: `static/index.html` (insert menu button before `#menuRefresh`)
- Modify: `static/index.html` (`openActionMenu`)
- Modify: `static/index.html` (add click handler near the other menu handlers)
- Test: `<scratchpad>/verify-task2.js` (create)

> **Line numbers below are stale by design.** Task 1 inserts ~45 lines into the same file, shifting
> everything in the `<script>` block down. Locate each edit site by **searching for the quoted code**,
> not by line number. The HTML edit (Step 3) sits above Task 1's insertions and is unaffected.

**Interfaces:**
- Consumes from Task 1: `SHOW_MASTER_KEY`, `showMaster`, `renderWindowOptions()`.
- Produces: `#menuToggleMaster` button and `#menuToggleMasterLabel` span.

- [ ] **Step 1: Write the failing test**

Create `<scratchpad>/verify-task2.js`:

```js
const { chromium } = require('playwright');

const APP = 'http://localhost:5533/';
const LABEL = '#menuToggleMasterLabel';

(async () => {
  const api = await (await fetch(APP + 'api/windows')).json();
  const masters = api.filter(w => w.name.includes('MASTER'));
  const nonMasters = api.filter(w => !w.name.includes('MASTER'));
  if (masters.length === 0) throw new Error('No MASTER windows in session; cannot test.');
  if (nonMasters.length === 0) throw new Error('All windows are MASTER; cannot test.');

  const browser = await chromium.launch({ channel: 'chrome' });
  const page = await browser.newPage();
  const failures = [];
  const opts = () => page.$$eval('#sessionSelect option', els => els.map(e => e.textContent));

  await page.goto(APP);
  await page.evaluate(() => localStorage.clear());
  await page.goto(APP, { waitUntil: 'networkidle' });
  await page.waitForFunction(() => document.querySelectorAll('#sessionSelect option').length > 0);

  // Default hidden: label offers to unhide.
  await page.click('#menuBtn');
  let label = await page.textContent(LABEL);
  if (label.trim() !== 'Unhide MASTER') failures.push(`Default label: expected "Unhide MASTER", got "${label.trim()}"`);

  // Unhide: every window appears, label flips to the opposite action.
  await page.click('#menuToggleMaster');
  await page.waitForFunction(
    n => document.querySelectorAll('#sessionSelect option').length === n, api.length
  ).catch(() => {});
  let shown = await opts();
  if (shown.length !== api.length) failures.push(`After unhide: expected ${api.length} options, got ${shown.length}`);
  for (const w of masters) {
    if (!shown.some(t => t.startsWith(w.target + ' '))) failures.push(`After unhide, MASTER missing: ${w.target}`);
  }
  await page.click('#menuBtn');
  label = await page.textContent(LABEL);
  if (label.trim() !== 'Hide MASTER') failures.push(`Shown label: expected "Hide MASTER", got "${label.trim()}"`);

  // Persistence: reload keeps MASTER shown.
  await page.goto(APP, { waitUntil: 'networkidle' });
  await page.waitForFunction(() => document.querySelectorAll('#sessionSelect option').length > 0);
  shown = await opts();
  if (shown.filter(t => t.includes('MASTER')).length !== masters.length) {
    failures.push('Toggle state did not persist across reload');
  }

  // Re-hide.
  await page.click('#menuBtn');
  await page.click('#menuToggleMaster');
  await page.waitForFunction(
    n => document.querySelectorAll('#sessionSelect option').length === n, nonMasters.length
  ).catch(() => {});
  shown = await opts();
  if (shown.some(t => t.includes('MASTER'))) failures.push(`After re-hide, MASTER still visible: ${JSON.stringify(shown.filter(t => t.includes('MASTER')))}`);

  // Current-window exception: select a MASTER window while shown, then hide.
  await page.click('#menuBtn');
  await page.click('#menuToggleMaster'); // show again
  await page.waitForTimeout(200);
  const keep = masters[0];
  await page.selectOption('#sessionSelect', keep.target);
  await page.click('#menuBtn');
  await page.click('#menuToggleMaster'); // hide, while sitting on a MASTER window
  await page.waitForTimeout(200);
  shown = await opts();
  if (!shown.some(t => t.startsWith(keep.target + ' '))) {
    failures.push(`Current MASTER window ${keep.target} was removed while selected`);
  }
  const stillSelected = await page.$eval('#sessionSelect', el => el.value);
  if (stillSelected !== keep.target) failures.push(`Selection moved off ${keep.target} to ${stillSelected}`);
  const otherMasters = masters.filter(w => w.target !== keep.target);
  for (const w of otherMasters) {
    if (shown.some(t => t.startsWith(w.target + ' '))) failures.push(`Non-current MASTER ${w.target} should be hidden`);
  }

  await browser.close();
  if (failures.length) { console.error('FAIL:\n' + failures.join('\n')); process.exit(1); }
  console.log('PASS: toggle, label flip, persistence, and current-window exception all correct');
})().catch(e => { console.error('ERROR:', e.message); process.exit(1); });
```

- [ ] **Step 2: Run test to verify it fails**

Run from the scratchpad directory: `node verify-task2.js`

Expected: exit 1, `ERROR:` on a timeout waiting for `#menuBtn`'s menu to expose `#menuToggleMasterLabel` — the element does not exist yet.

- [ ] **Step 3: Add the menu button**

In `static/index.html`, find `<button class="action-menu-item" id="menuRefresh">` and insert immediately before that line:

```html
                <button class="action-menu-item" id="menuToggleMaster">
                    <span class="action-icon">M</span>
                    <span class="action-label" id="menuToggleMasterLabel">Unhide MASTER</span>
                    <span class="action-shortcut">Filter</span>
                </button>
```

- [ ] **Step 4: Sync the label on every menu open**

Find and replace the one-line `openActionMenu`:

```js
        function openActionMenu() { actionMenuModal.classList.add('show'); }
```

with:

```js
        function openActionMenu() {
            // Label states the action the click performs, recomputed on every open
            // so it cannot drift out of sync with stored state.
            document.getElementById('menuToggleMasterLabel').textContent =
                showMaster ? 'Hide MASTER' : 'Unhide MASTER';
            actionMenuModal.classList.add('show');
        }
```

- [ ] **Step 5: Add the click handler**

In `static/index.html`, find the `menuRefresh` click-handler line and add immediately after it:

```js
        document.getElementById('menuToggleMaster').addEventListener('click', () => {
            closeActionMenu();
            showMaster = !showMaster;
            localStorage.setItem(SHOW_MASTER_KEY, String(showMaster));
            renderWindowOptions();
            showStatus(showMaster ? 'MASTER SHOWN' : 'MASTER HIDDEN', 'success');
        });
```

The `showStatus` call runs after `renderWindowOptions()` so this message wins over the `N WINDOWS` status.

- [ ] **Step 6: Run test to verify it passes**

Run from the scratchpad directory: `node verify-task2.js`

Expected: exit 0, `PASS: toggle, label flip, persistence, and current-window exception all correct`.

- [ ] **Step 7: Re-run Task 1's test to check for regression**

Run from the scratchpad directory: `node verify-task1.js`

Expected: exit 0, still `PASS`.

- [ ] **Step 8: Commit**

```bash
git add static/index.html
git commit -m "feat: add Unhide/Hide MASTER toggle to web UI menu"
```

---

### Task 3: Verify navigation surfaces and lowercase-master behavior

Confirms the choke-point claim that carries the whole design — that filtering the dropdown also filters the `^B w` modal and the `^B n`/`^B p` cycle — plus the intended detection consequences and the empty-list fallback. No production code is expected to change; this task exists to prove the design assumptions and to catch them if wrong.

**Files:**
- Test: `<scratchpad>/verify-task3.js` (create)
- Modify (only if the test fails): `static/index.html`

**Interfaces:**
- Consumes from Tasks 1-2: the rendered dropdown and the `#menuToggleMaster` toggle.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Write the test**

Create `<scratchpad>/verify-task3.js`:

```js
const { chromium } = require('playwright');

const APP = 'http://localhost:5533/';

(async () => {
  const api = await (await fetch(APP + 'api/windows')).json();
  const masters = api.filter(w => w.name.includes('MASTER'));
  if (masters.length === 0) throw new Error('No MASTER windows in session; cannot test.');

  const browser = await chromium.launch({ channel: 'chrome' });
  const page = await browser.newPage();
  const failures = [];

  await page.goto(APP);
  await page.evaluate(() => localStorage.clear());
  await page.goto(APP, { waitUntil: 'networkidle' });
  await page.waitForFunction(() => document.querySelectorAll('#sessionSelect option').length > 0);

  // ^B w modal must not list MASTER windows while hidden.
  await page.click('#menuBtn');
  await page.click('#menuWindowList');
  await page.waitForTimeout(300);
  const modalNames = await page.$$eval('.window-item', els => els.map(e => e.textContent));
  const modalLeak = modalNames.filter(t => t.includes('MASTER'));
  if (modalLeak.length) failures.push(`Window list modal shows MASTER: ${JSON.stringify(modalLeak)}`);
  await page.keyboard.press('Escape');
  await page.waitForTimeout(200);

  // Cycling next through every window must never land on a MASTER window.
  const count = await page.$$eval('#sessionSelect option', els => els.length);
  const visited = [];
  for (let i = 0; i < count + 2; i++) {
    visited.push(await page.$eval('#sessionSelect', el => el.options[el.selectedIndex].textContent));
    await page.click('#menuBtn');
    await page.click('#menuNextWindow');
    await page.waitForTimeout(120);
  }
  const navLeak = visited.filter(t => t.includes('MASTER'));
  if (navLeak.length) failures.push(`Next-window cycle landed on MASTER: ${JSON.stringify(navLeak)}`);

  // Detection consequences, asserted against the live filter in the page.
  const probes = await page.evaluate(() => ({
    lower: isMasterWindow({ name: 'mm master' }),
    mind: isMasterWindow({ name: 'MASTERMIND' }),
    mixed: isMasterWindow({ name: 'Master control' }),
    exact: isMasterWindow({ name: 'alexa MASTER' }),
  }));
  if (probes.lower !== false) failures.push('lowercase "master" should NOT be treated as MASTER');
  if (probes.mind !== true) failures.push('"MASTERMIND" SHOULD be treated as MASTER');
  if (probes.mixed !== false) failures.push('"Master control" should NOT be treated as MASTER');
  if (probes.exact !== true) failures.push('"alexa MASTER" SHOULD be treated as MASTER');

  // Empty-list fallback: when every window is MASTER and none is selected, the
  // filter must fall open rather than return an empty dropdown. Driven directly
  // against visibleWindows() by swapping allWindows, since the live session
  // cannot be forced into an all-MASTER state.
  const fallback = await page.evaluate(() => {
    const real = allWindows;
    const wasShowing = showMaster;
    try {
      showMaster = false;
      allWindows = [{ target: '9:0', name: 'a MASTER' }, { target: '9:1', name: 'b MASTER' }];
      const none = visibleWindows(null).length;          // no selection -> fall open
      const kept = visibleWindows('9:1').map(w => w.target); // selection -> keep just it
      allWindows = [];
      const empty = visibleWindows(null).length;          // genuinely no windows
      return { none, kept, empty };
    } finally {
      allWindows = real;
      showMaster = wasShowing;
    }
  });
  if (fallback.none !== 2) failures.push(`All-MASTER with no selection should fall open to 2, got ${fallback.none}`);
  if (JSON.stringify(fallback.kept) !== JSON.stringify(['9:1'])) {
    failures.push(`All-MASTER with 9:1 selected should keep only it, got ${JSON.stringify(fallback.kept)}`);
  }
  if (fallback.empty !== 0) failures.push(`No windows at all should stay empty, got ${fallback.empty}`);

  // The probe must leave the real dropdown untouched.
  await page.evaluate(() => renderWindowOptions());
  await page.waitForTimeout(200);
  const after = await page.$$eval('#sessionSelect option', els => els.map(e => e.textContent));
  if (after.some(t => t.includes('MASTER'))) failures.push('Probe leaked state: MASTER visible after restore');

  await browser.close();
  if (failures.length) { console.error('FAIL:\n' + failures.join('\n')); process.exit(1); }
  console.log('PASS: modal filtered, next-window skips MASTER, detection + fallback rules correct');
})().catch(e => { console.error('ERROR:', e.message); process.exit(1); });
```

- [ ] **Step 2: Run the test**

Run from the scratchpad directory: `node verify-task3.js`

Expected: exit 0, `PASS: modal filtered, next-window skips MASTER, detection + fallback rules correct`.

If it fails, the choke-point assumption is wrong somewhere. Do not paper over it by filtering a second time in the modal — investigate why the surface does not derive from `sessionSelect`, and report before changing the design.

- [ ] **Step 3: Commit (only if code changed)**

If Step 2 passed with no production changes, there is nothing to commit; skip this step. If a fix was required:

```bash
git add static/index.html
git commit -m "fix: ensure MASTER filter applies to window list modal and navigation"
```

---

### Task 4: Update the spec's stale test section

The spec hardcodes "two such windows" and "the other seven windows appear." The live session now has 11 windows and 3 MASTER, and a window was renamed mid-planning. Correct the spec so it does not mislead a future reader.

**Files:**
- Modify: `docs/superpowers/specs/2026-07-15-hide-master-windows-design.md`

**Interfaces:**
- Consumes: nothing. Produces: nothing.

- [ ] **Step 1: Fix the window-count claims**

In the spec's **Problem** section, replace:

```
At time of writing the session has two such windows: `0:2 alexa MASTER` and `0:6 sink MASTER`.
```

with:

```
The set of MASTER windows changes at runtime — during planning, a window named `mm DEV*` was
renamed to `mm MASTER` between two listings minutes apart. Tests must therefore compute expected
windows from `/api/windows` at run time and never hardcode names or counts.
```

In the spec's **Testing** section, replace item 1:

```
1. Default state on a fresh load hides `alexa MASTER` and `sink MASTER`; the other seven windows appear.
```

with:

```
1. Default state on a fresh load hides every window whose name contains `MASTER`; all others appear.
   Expected counts are derived from `/api/windows` at test time.
```

In the spec's **Testing** section, replace the preamble:

```
Manual verification against the live session, which contains both MASTER and non-MASTER windows:
```

with:

```
Automated verification drives the real app at `http://localhost:5533/` via Playwright, installed in
the session scratchpad only (no project dependency). Launch with `{ channel: 'chrome' }`; the cached
Playwright browser build is older than the installed Playwright expects. Scripts: `verify-task1.js`,
`verify-task2.js`, `verify-task3.js`.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-07-15-hide-master-windows-design.md
git commit -m "docs: correct stale window counts in MASTER-hiding spec"
```

---

## Done When

- MASTER windows are absent from the dropdown, the `^B w` modal, and the `^B n`/`^B p` cycle on a fresh load.
- The ☰ MENU entry reads `Unhide MASTER` when hidden and `Hide MASTER` when shown, and toggles instantly with no refetch.
- The preference survives a reload.
- A MASTER window stays listed while it is the selected window, and drops off once you switch away.
- `verify-task1.js`, `verify-task2.js`, and `verify-task3.js` all exit 0.
- The Logout menu entry still works and its code is byte-identical to `5f4bbb0`.
