# New Window → Named Claude Project Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make "New Window" prompt for a name, then open a tmux window in `~/p/<name>` (created if absent) running `claude --yolo`, named `<name>`; switch to an existing window of that name instead of duplicating.

**Architecture:** New backend route `POST /api/new-window-named` (validator is a pure, unit-tested function). The window's directory is set via `new-window -c <dir>` argv — no `cd` is ever typed into a shell, which is what removes the injection path. Server verifies `pane_current_path` matches the intended dir before typing `claude --yolo`. Frontend clones the existing rename modal for the name prompt and reroutes both New Window entry points through it.

**Tech Stack:** Rust/Axum backend (`src/main.rs`), vanilla JS inline in `static/index.html`, no frontend build step. tmux 3.2a.

## Global Constraints

- **Files:** only `src/main.rs` and `static/index.html`. No mobile, no `Cargo.toml` (no new crates — uses `std::fs`, `std::env`, `std::process::Command`, already imported).
- **`claude --yolo` is a `~/.bash_aliases` shell function, not a binary flag.** It must be typed into the window's interactive bash via `send-keys`. The backend must never `Command::new("claude")`.
- **Never target a window by name after creation.** tmux allows duplicate names and `-t session:name` resolves to the lowest-indexed (oldest) match. Use the `window_id` (`@N`) from `new-window -P -F` for every follow-up command.
- **`send-keys -l` covers the payload only; `Enter` is a separate call.** `send-keys -l 'cmd' Enter` types the word "Enter". Two calls: `send-keys -t <id> -l 'claude --yolo'` then `send-keys -t <id> Enter`.
- **Name validation rule (identical on both ends):** matches `^[A-Za-z0-9._-]+$`, is not empty, does not start with `-`, and is not `.` or `..`. The `#` character is excluded by the allowlist (tmux runs format expansion on `-n`, so `#` would be interpolated).
- **Directory is set via `new-window -c <dir>` argv — no typed `cd`.** The user's name never enters a shell command string.
- **Do not target, rename, or kill any window in the user's live session during tests.** Tests create their own scratch windows and kill them by `window_id`.
- **Test against a dev server on port 5534 serving the repo, never the live 5533 install.** Backend changes require `cargo build --release` and a restart of the 5534 server for each backend iteration.
- **The MASTER-hiding feature is already merged.** `renderWindowOptions()` filters windows whose name contains `MASTER`; a new window whose name contains `MASTER` (uppercase letters are allowed by the rule) must still be selected and shown — see Task 3.

## Testing Approach

- **Backend name validation** is a pure function → real Rust unit tests via `cargo test`. This is the one genuinely unit-testable piece; use standard TDD.
- **Backend handler** and **frontend** are exercised end-to-end in a real browser (Playwright, `{ channel: 'chrome' }`, scratchpad install, port 5534) plus direct `curl`/`tmux` for the API, because they cross the tmux boundary. Any window a test creates, it kills by `window_id`, and any scratch `~/p/<name>` dir it makes, it removes.
- **Dev server (5534) is rebuilt per backend change.** Restart recipe (from repo root):
  `cargo build --release && (kill the old 5534 pid) && PORT=5534 nohup ./target/release/tmux-terminal > /tmp/devserver.log 2>&1 &`
  Never restart, stop, or `make update` the 5533 service — it is the user's live tool.

---

### Task 1: Backend name validator (pure function + unit tests)

**Files:**
- Modify: `src/main.rs` (add `validate_window_name` near the other free functions, e.g. just above `async fn new_window`)
- Test: `src/main.rs` (a `#[cfg(test)] mod tests` block at end of file)

**Interfaces:**
- Consumes: nothing.
- Produces: `fn validate_window_name(name: &str) -> Result<(), String>` — `Ok(())` iff the name is valid; `Err(reason)` otherwise. Task 2 calls this.

- [ ] **Step 1: Write the failing tests**

Add at the very end of `src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::validate_window_name;

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
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test validate_window_name 2>&1 | tail -20` — actually run `cargo test 2>&1 | tail -30`.
Expected: compile error — `cannot find function validate_window_name`.

- [ ] **Step 3: Implement the validator**

Add to `src/main.rs` immediately above `async fn new_window() -> impl IntoResponse {`:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test 2>&1 | tail -30`
Expected: the five `tests::` cases pass (`test result: ok`). Warnings about `validate_window_name` being unused elsewhere are acceptable at this task (Task 2 wires it in); the test module uses it, so there should be no dead-code warning for it.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: add validate_window_name with unit tests"
```

---

### Task 2: Backend handler `POST /api/new-window-named`

**Files:**
- Modify: `src/main.rs` (add `NewWindowNamedRequest` struct + `new_window_named` handler near `new_window`; register route in `main()` next to `/api/new-window`)
- Test: `<scratchpad>/verify-backend.js` (Node, drives the API + tmux directly, cleans up)

**Interfaces:**
- Consumes: `validate_window_name` (Task 1).
- Produces: `POST /api/new-window-named`, body `{ name: string }`, response `{ success: bool, target?: string, window_id?: string, existing?: bool, error?: string }`. Task 3 consumes this. Extractor is a plain `Json<NewWindowNamedRequest>` (matches every other handler in the file; the only callers are the new frontend code, which always sends a valid JSON body — a bodyless request just gets axum's default rejection, which is fine).

- [ ] **Step 1: Write the failing test**

Create `<scratchpad>/verify-backend.js`:

```js
// Drives POST /api/new-window-named against the 5534 dev server and cleans up
// every window/dir it creates. Never touches a pre-existing window.
const { execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');

const APP = 'http://localhost:5534/';
const uniq = process.env.WNAME || ('claudetest' + Date.now().toString(36));
const home = os.homedir();
const dir = `${home}/p/${uniq}`;
const failures = [];
const created = []; // window_ids to kill

async function post(name) {
  const r = await fetch(APP + 'api/new-window-named', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name }),
  });
  return { status: r.status, body: await r.json() };
}
function tmux(args) { return execFileSync('tmux', args, { encoding: 'utf8' }).trim(); }

(async () => {
  try {
    // 1. invalid names are rejected, create nothing
    for (const bad of ['', 'foo bar', 'foo;rm', 'a#b', '../etc', '.', '-rf', 'a$(id)']) {
      const { body } = await post(bad);
      if (body.success) failures.push(`invalid name accepted: ${JSON.stringify(bad)}`);
    }

    // 2. create new: correct name, dir on disk, pane in dir, claude launched
    const { status, body } = await post(uniq);
    if (!body.success) { failures.push(`create failed: ${JSON.stringify(body)}`); }
    else {
      created.push(body.window_id);
      if (body.existing) failures.push('new window reported existing:true');
      if (!fs.existsSync(dir)) failures.push(`dir not created: ${dir}`);
      const nm = tmux(['display-message', '-p', '-t', body.window_id, '#{window_name}']);
      if (nm !== uniq) failures.push(`window name ${nm} !== ${uniq}`);
      const cwd = tmux(['display-message', '-p', '-t', body.window_id, '#{pane_current_path}']);
      if (fs.realpathSync(cwd) !== fs.realpathSync(dir)) failures.push(`pane cwd ${cwd} !== ${dir}`);
      // the pane should show a claude invocation was typed (best-effort)
      const pane = tmux(['capture-pane', '-p', '-t', body.window_id]);
      if (!/claude/.test(pane)) failures.push(`pane does not show a claude command:\n${pane}`);
    }

    // 3. existing: second call with same name switches, no new window
    const before = tmux(['list-windows', '-a']).split('\n').length;
    const second = await post(uniq);
    if (!second.body.existing) failures.push('second create did not report existing:true');
    if (created[0] && second.body.window_id !== created[0]) {
      failures.push(`existing returned different window_id ${second.body.window_id} !== ${created[0]}`);
    }
    const after = tmux(['list-windows', '-a']).split('\n').length;
    if (after !== before) failures.push(`window count changed on existing-switch: ${before} -> ${after}`);
  } finally {
    for (const id of created) { try { tmux(['kill-window', '-t', id]); } catch {} }
    try { fs.rmSync(dir, { recursive: true, force: true }); } catch {}
  }
  if (failures.length) { console.error('FAIL:\n' + failures.join('\n')); process.exit(1); }
  console.log('PASS: backend new-window-named — validation, create, dir, cwd, launch, existing-switch');
})().catch(e => { console.error('ERROR:', e.message); process.exit(1); });
```

- [ ] **Step 2: Run test to verify it fails**

Ensure the 5534 dev server is running the CURRENT binary (rebuild if you already changed `src/main.rs` this task):
`cargo build --release 2>&1 | tail -3` then restart 5534 (see Testing Approach).
Run from the scratchpad: `node verify-backend.js`
Expected: FAIL/ERROR — the route does not exist yet, so `post()` returns a 404 and `body.success` is undefined (invalid-name loop may pass vacuously, but the create step fails). Confirm the failure is "route missing," not a test bug.

- [ ] **Step 3: Add the request struct and handler**

In `src/main.rs`, immediately below the `new_window` handler (after its closing brace, before `#[derive(Deserialize)] struct RenameWindowRequest`), add:

```rust
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
```

- [ ] **Step 4: Register the route**

In `main()`, immediately after the `.route("/api/new-window", post(new_window))` line, add:

```rust
        .route("/api/new-window-named", post(new_window_named))
```

- [ ] **Step 5: Rebuild, restart 5534, run the test to verify it passes**

```bash
cargo build --release 2>&1 | tail -3
```
Restart the 5534 dev server (kill old pid, relaunch per Testing Approach). Then from the scratchpad:
`node verify-backend.js`
Expected: `PASS: backend new-window-named — validation, create, dir, cwd, launch, existing-switch`. The test kills its own window and removes its scratch dir. Confirm with `tmux list-windows -a` that no `claudetest*` window remains.

- [ ] **Step 6: Re-run the Rust unit tests**

Run: `cargo test 2>&1 | tail -15`
Expected: all `tests::` cases still pass.

- [ ] **Step 7: Commit**

```bash
git add src/main.rs
git commit -m "feat: add POST /api/new-window-named (create-in-dir, verify cwd, launch claude --yolo)"
```

---

### Task 3: Frontend name prompt + rerouted New Window

**Files:**
- Modify: `static/index.html` (modal markup; node caching; repurpose `createNewWindow`; new `closeNewWindowModal` + `submitNewWindow`; new branch in `handleModalKeydown`; backdrop-click handler)
- Test: `<scratchpad>/verify-frontend.js` (Playwright, cleans up any window it creates)

**Interfaces:**
- Consumes: `POST /api/new-window-named` (Task 2); the existing `loadWindows()`, `renderWindowOptions()`, `showStatus()`, `startCapture()`, `commandInput`, `sessionSelect`.
- Produces: nothing consumed by later tasks.

> Line numbers below were accurate at plan time; locate each site by searching for the quoted code.

- [ ] **Step 1: Write the failing test**

Create `<scratchpad>/verify-frontend.js`:

```js
const { chromium } = require('playwright');
const { execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');

const APP = 'http://localhost:5534/';
const uniq = 'MASTERtest' + Date.now().toString(36); // MASTER-named on purpose (filter regression)
const dir = `${os.homedir()}/p/${uniq}`;
const failures = [];
let createdId = null;
function tmux(a){ try { return execFileSync('tmux', a, {encoding:'utf8'}).trim(); } catch { return ''; } }

(async () => {
  const browser = await chromium.launch({ channel: 'chrome' });
  const page = await browser.newPage();
  try {
    await page.goto(APP);
    await page.evaluate(() => localStorage.clear());
    await page.goto(APP, { waitUntil: 'networkidle' });
    await page.waitForFunction(() => document.querySelectorAll('#sessionSelect option').length > 0);

    // Menu → New Window opens the prompt (not an immediate blank window)
    await page.click('#menuBtn');
    await page.click('#menuNewWindow');
    await page.waitForSelector('#newWindowModal.show', { timeout: 2000 });
    if (!(await page.$('#newWindowInput'))) failures.push('no #newWindowInput in prompt');

    // Invalid name → stays open, red status, creates nothing
    await page.fill('#newWindowInput', 'bad name;rm');
    await page.keyboard.press('Enter');
    await page.waitForTimeout(200);
    if (!(await page.$('#newWindowModal.show'))) failures.push('invalid name closed the modal');

    // Valid MASTER-named project → creates, selects, and SHOWS it despite the MASTER filter
    await page.fill('#newWindowInput', uniq);
    await page.keyboard.press('Enter');
    await page.waitForFunction(
      (n) => Array.from(document.querySelectorAll('#sessionSelect option')).some(o => o.textContent.includes(n)),
      uniq, { timeout: 8000 }
    ).catch(() => {});
    createdId = tmux(['list-windows','-a','-F','#{window_id}\t#{window_name}'])
      .split('\n').find(l => l.endsWith('\t'+uniq))?.split('\t')[0] || null;

    const selected = await page.$eval('#sessionSelect', el => el.options[el.selectedIndex]?.textContent || '');
    if (!selected.includes(uniq)) failures.push(`created window not selected: got "${selected}"`);
    const shown = await page.$$eval('#sessionSelect option', els => els.map(e => e.textContent));
    if (!shown.some(t => t.includes(uniq))) failures.push('MASTER-named new window filtered out of dropdown (the no-op bug)');
    if (!fs.existsSync(dir)) failures.push(`dir not created: ${dir}`);
  } finally {
    await browser.close();
    if (createdId) tmux(['kill-window','-t',createdId]);
    try { fs.rmSync(dir, {recursive:true, force:true}); } catch {}
  }
  if (failures.length) { console.error('FAIL:\n' + failures.join('\n')); process.exit(1); }
  console.log('PASS: prompt opens, rejects invalid, creates + selects + shows a MASTER-named project');
})().catch(e => { console.error('ERROR:', e.message); process.exit(1); });
```

- [ ] **Step 2: Run test to verify it fails**

From the scratchpad: `node verify-frontend.js`
Expected: FAIL/ERROR — `#newWindowModal.show` never appears (clicking New Window still creates a blank window via the old path). Confirm the failure is "no prompt," not a test bug. (The test cleans up if it created anything.)

- [ ] **Step 3: Add the modal markup**

In `static/index.html`, find the `#renameModal` block (`<div class="modal-overlay" id="renameModal">` … its closing `</div></div></div>`) and insert immediately AFTER it:

```html
    <div class="modal-overlay" id="newWindowModal">
        <div class="rename-modal">
            <div class="modal-header">New Claude Window <span style="font-weight:400;font-size:0.65rem;color:var(--matrix-dim)">^B c</span></div>
            <input type="text" class="rename-input" id="newWindowInput" placeholder="Project name (a-z, 0-9, . _ -)" autocomplete="off" spellcheck="false">
            <div class="rename-hint">Enter to create • Esc to cancel</div>
        </div>
    </div>
```

- [ ] **Step 4: Cache the nodes**

Find `const renameInput = document.getElementById('renameInput');` and add immediately after:

```js
        const newWindowModal = document.getElementById('newWindowModal');
        const newWindowInput = document.getElementById('newWindowInput');
```

- [ ] **Step 5: Repurpose `createNewWindow` to open the prompt**

Replace the entire existing `async function createNewWindow() { … }` (the one that does `fetch('/api/new-window', { method: 'POST' })`) with:

```js
        function createNewWindow() {
            newWindowInput.value = '';
            newWindowModal.classList.add('show');
            // 50ms: the element is display:none until .show lands, and a synchronous
            // focus would also be stolen by closeActionMenu()'s commandInput.focus().
            setTimeout(() => { newWindowInput.focus(); }, 50);
        }

        function closeNewWindowModal() {
            newWindowModal.classList.remove('show');
            commandInput.focus();
        }

        const NEW_WINDOW_NAME_RE = /^[A-Za-z0-9._-]+$/;
        async function submitNewWindow(name) {
            name = (name || '').trim();
            if (!name || !NEW_WINDOW_NAME_RE.test(name) || name.startsWith('-') || name === '.' || name === '..') {
                showStatus('INVALID NAME', 'error');
                newWindowModal.classList.add('show');
                setTimeout(() => { newWindowInput.value = name; newWindowInput.focus(); }, 50);
                return;
            }
            try {
                const response = await fetch('/api/new-window-named', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ name }),
                });
                const data = await response.json();
                if (data.success && data.target) {
                    // Select the new/switched window. Persist the target and blank the
                    // live selection BEFORE re-rendering so renderWindowOptions()'s
                    // keepTarget (= sessionSelect.value || savedTarget) falls through to
                    // the new target. Otherwise a MASTER-named new window is filtered out
                    // and the plain `sessionSelect.value = target` assignment no-ops.
                    localStorage.setItem('tmux-selected-target', data.target);
                    sessionSelect.value = '';
                    await loadWindows();
                    showStatus((data.existing ? 'SWITCHED TO ' : 'NEW WINDOW: ') + name, 'success');
                    setTimeout(() => commandInput.focus(), 150);
                } else {
                    showStatus(data.error || 'FAILED TO CREATE WINDOW', 'error');
                }
            } catch (err) {
                showStatus('CONNECTION ERROR', 'error');
            }
        }
```

- [ ] **Step 6: Add the key-routing branch**

In `handleModalKeydown`, find the rename-modal branch (`if (renameModal.classList.contains('show')) { … }`) and insert immediately BEFORE it (branch order is load-bearing; each branch `return true`s):

```js
            // Handle new-window prompt
            if (newWindowModal.classList.contains('show')) {
                if (e.key === 'Escape') { e.preventDefault(); closeNewWindowModal(); return true; }
                if (e.key === 'Enter') {
                    e.preventDefault();
                    const name = newWindowInput.value;
                    closeNewWindowModal();
                    submitNewWindow(name);
                    return true;
                }
                return true; // consume all keys while the prompt is open
            }
```

- [ ] **Step 7: Add the backdrop-click handler**

Find the rename backdrop handler (`renameModal.addEventListener('click', (e) => { if (e.target === renameModal) closeRenameModal(); });`) and add after it:

```js
        newWindowModal.addEventListener('click', (e) => {
            if (e.target === newWindowModal) closeNewWindowModal();
        });
```

- [ ] **Step 8: Run the frontend test to verify it passes**

Ensure 5534 is serving the repo (frontend edits are picked up with no rebuild). From the scratchpad:
`node verify-frontend.js`
Expected: `PASS: prompt opens, rejects invalid, creates + selects + shows a MASTER-named project`. Verify no `MASTERtest*` window remains: `tmux list-windows -a`.

- [ ] **Step 9: Regression — re-run the MASTER-feature tests**

The MASTER tests still live in the scratchpad. From the scratchpad:
`for t in verify-task1.js verify-keeptarget.js verify-task2.js verify-task3.js; do echo "== $t =="; node $t 2>&1 | tail -1; done`
Expected: all PASS. (These target 5534 and must be unaffected by this change.)

- [ ] **Step 10: Commit**

```bash
git add static/index.html
git commit -m "feat: New Window prompts for a name and opens a claude project window"
```

---

## Done When

- Clicking New Window (or `^B c`) opens a name prompt; submitting `<name>` opens a window in `~/p/<name>` (created if absent), named `<name>`, running `claude --yolo`, and selects it.
- Submitting a name that already exists switches to that window without creating a duplicate.
- Invalid names are rejected on both ends and create nothing.
- The server verifies `pane_current_path` before typing `claude --yolo`.
- A window whose name contains `MASTER` is still selected and shown after creation.
- `cargo test` passes; `verify-backend.js` and `verify-frontend.js` pass; the four MASTER `verify-*.js` scripts still pass.
- No `claudetest*`/`MASTERtest*` scratch window or `~/p/` scratch dir is left behind by tests.
