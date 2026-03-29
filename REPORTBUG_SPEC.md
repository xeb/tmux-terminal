# Bug Reporting & Settings — Implementation Spec

## Overview

Add a **Settings tab** to the mobile app (alongside the existing terminal screen) and a **bug report flow** that saves reports to the Rust server, triggers Claude to fix them, and texts Mark when done.

---

## 1. Mobile App Changes

### 1a. Tab Navigation Restructure

The app currently has one screen. We add a two-tab layout at the bottom:
- **TERMINAL** (existing MainScreen)
- **SETTINGS** (new SettingsScreen)

Implementation: custom two-tab switcher component (`AppTabs.tsx`) wrapping both screens — no heavy nav library needed. The window tab bar stays inside the TERMINAL tab.

```
App.tsx
  └── AppTabs.tsx
        ├── tab: TERMINAL → MainScreen (existing)
        └── tab: SETTINGS → SettingsScreen (new)
```

### 1b. SettingsScreen

File: `mobile/src/screens/SettingsScreen.tsx`

Sections (B&W aesthetic, monospace throughout):

```
┌──────────────────────────────────────┐
│  SETTINGS                            │
├──────────────────────────────────────┤
│  BUILD INFO                          │
│  Version: 1.0.0                      │
│  Built:   2026-03-22 14:33 UTC       │
│  Server:  100.68.192.63:5533         │
├──────────────────────────────────────┤
│                                      │
│  [ REPORT A BUG → ]                  │
│                                      │
└──────────────────────────────────────┘
```

Build info comes from a generated file `mobile/src/buildInfo.ts` (see §3).

### 1c. BugReportScreen

File: `mobile/src/screens/BugReportScreen.tsx`

Modelled on MacroMunch's `bugreport.tsx` but in B&W terminal aesthetic.

```
┌──────────────────────────────────────┐
│ ← BACK        REPORT A BUG          │
├──────────────────────────────────────┤
│ SCREENSHOTS  (tap + to add, max 5)   │
│ ┌──┐ ┌──┐ ┌──┐ ┌──┐ ┌──┐           │
│ │  │ │  │ │  │ │  │ │ + │           │
│ └──┘ └──┘ └──┘ └──┘ └──┘           │
│                                      │
│ DESCRIPTION                          │
│ ┌────────────────────────────────┐   │
│ │ What went wrong?               │   │
│ │                                │   │
│ └────────────────────────────────┘   │
│                                      │
│ WILL ALSO INCLUDE:                   │
│ App version: 1.0.0                   │
│ Built: 2026-03-22 14:33 UTC          │
│ Device: iPhone 15 Pro                │
│ OS: iOS 18.3                         │
│                                      │
│ ┌──────────────────────────────────┐ │
│ │          SUBMIT REPORT           │ │
│ └──────────────────────────────────┘ │
└──────────────────────────────────────┘
```

Dependencies to add:
- `expo-image-picker` — screenshot selection
- `expo-constants` — device name
- `expo-file-system` — read image as base64

On submit: `POST /api/bug-report` (see §2b).

Success → show "REPORT SUBMITTED" status, navigate back to Settings.

---

## 2. Rust Backend Changes (`src/main.rs`)

### 2a. New `bugs/` directory

Created automatically on first report. Structure per report:

```
bugs/
  2026-03-22T14-33-00-abc123/
    report.json          # metadata + description
    screenshot_1.jpg
    screenshot_2.jpg
    ...
```

`report.json` schema:
```json
{
  "id": "2026-03-22T14-33-00-abc123",
  "timestamp": "2026-03-22T14:33:00Z",
  "description": "...",
  "app_version": "1.0.0",
  "build_date": "2026-03-22T14:33:00Z",
  "device": "iPhone 15 Pro",
  "os": "iOS 18.3",
  "screenshot_count": 2,
  "status": "pending"   // → "fixed" once Claude processes it
}
```

### 2b. New endpoint: `POST /api/bug-report`

Request body:
```json
{
  "description": "optional string",
  "app_version": "1.0.0",
  "build_date": "...",
  "device": "iPhone 15 Pro",
  "os": "iOS 18.3",
  "screenshots": [
    { "name": "screenshot_1.jpg", "data": "<base64>" },
    ...
  ]
}
```

Handler logic (in order):
1. Generate report ID (`{timestamp}-{6-char-random}`)
2. Create `bugs/{id}/` directory
3. Write `report.json`
4. Decode + write each screenshot as `.jpg`
5. Send tmux command to trigger Claude (§2c)
6. Send SMS to Mark (§2d)
7. Return `{ "success": true, "id": "..." }`

### 2c. Trigger Claude in tmux

After saving the report, shell out to tmux:

```rust
Command::new("tmux")
    .args([
        "send-keys", "-t", "tmux terminal BUGFIX",
        "fix bugs", "Enter"
    ])
    .output();
```

If the window doesn't exist, create it first:
```rust
Command::new("tmux")
    .args(["new-window", "-t", "main", "-n", "tmux terminal BUGFIX"])
    .output();
```

Then send `cd /path/to/tmux-terminal && claude` to start Claude Code in it (if not already running). Then send "fix bugs".

### 2d. SMS to Mark via BlueBubbles

The tmux-terminal Rust server calls the BlueBubbles API directly (same method as Sink).

Config: Read from `.env`:
```
BLUEBUBBLES_URL=http://...
BLUEBUBBLES_PASSWORD=...
MARK_PHONE=iMessage;-;+14802822064
```

Sends two SMS moments:
1. **On bug report received**: "🐛 New bug report submitted: {description_or_'(no description)'}"
2. **Triggered by Claude when OTA done**: (see §4 — Claude calls `POST /api/notify` which sends the completion SMS)

### 2e. New endpoint: `POST /api/notify`

Simple endpoint Claude can call via curl when the OTA fix is done:
```json
{ "message": "Bug fixes deployed via OTA. Reports fixed: 2." }
```

Sends the message to Mark via BlueBubbles.

---

## 3. Build Info Generation

`build.sh` generates `mobile/src/buildInfo.ts` before triggering EAS:

```typescript
// AUTO-GENERATED — do not edit
export const BUILD_INFO = {
  version: '1.0.0',
  buildDate: '2026-03-22T14:33:00Z',
  commitHash: 'abc1234',
};
```

`build.sh` updated to:
```bash
# Generate build info
cat > src/buildInfo.ts << EOF
export const BUILD_INFO = {
  version: '$(node -p "require('./package.json').version")',
  buildDate: '$(date -u +%Y-%m-%dT%H:%M:%SZ)',
  commitHash: '$(git rev-parse --short HEAD)',
};
EOF
```

`buildInfo.ts` is in `.gitignore` (it changes every build).

---

## 4. CLAUDE.md Updates

Add a new section to `CLAUDE.md`:

```markdown
## Bug Fix Workflow

When you receive the command **"fix bugs"** in this session:

1. **Read all pending reports** in the `bugs/` directory.
   Each report has a `report.json` and optional `screenshot_N.jpg` files.
   Only process reports with `"status": "pending"`.

2. **Analyse each report** — read the description, view screenshots (use
   the Read tool on the image files), understand what broke.

3. **Fix the issues** in the codebase. Focus on the mobile app
   (`mobile/`) and Rust backend (`src/`). Make targeted, minimal fixes.

4. **Mark reports as fixed** — update `status` from `"pending"` to
   `"fixed"` in each `report.json` you handled.

5. **Push OTA update**:
   ```bash
   cd mobile && bash build.sh --ota "fix: <brief summary of fixes>"
   ```

6. **Notify Mark** via curl when the OTA is deployed:
   ```bash
   curl -X POST http://localhost:5533/api/notify \
     -H 'Content-Type: application/json' \
     -d '{"message": "Bug fixes deployed via OTA. Fixed: <summary>."}'
   ```

Do not run `build.sh` (full native rebuild) unless the fix requires
native module changes. OTA is sufficient for all JS/TS changes.
```

---

## 5. Implementation Order

1. `build.sh` — add buildInfo.ts generation
2. `mobile/src/buildInfo.ts` — add to `.gitignore`
3. Rust: `POST /api/bug-report` + `POST /api/notify` endpoints
4. Rust: BlueBubbles SMS helper (reuse `.env` pattern already in place)
5. `mobile/src/screens/SettingsScreen.tsx`
6. `mobile/src/screens/BugReportScreen.tsx`
7. `mobile/src/AppTabs.tsx` — two-tab wrapper
8. `App.tsx` — swap `<MainScreen>` for `<AppTabs>`
9. `CLAUDE.md` — add bug fix workflow section
10. OTA build (JS-only, no native changes needed except `expo-image-picker`)

> ⚠️ `expo-image-picker` is a native module — it needs a **full build**,
> not OTA. Roll it in with the Settings tab full build.

---

## Clarifying Questions

1. **BlueBubbles access** — Is the BlueBubbles server reachable from this machine (`100.68.192.63`)? What's the URL and password? (Can store in `.env`.) If not accessible, the SMS fallback could be a shell script using sink's existing config.

2. **"tmux terminal BUGFIX" window** — Should Claude Code already be running in this window waiting for input, or should the Rust server spawn a fresh `claude` process in a new window when a report arrives? (Starting a new `claude` process each time is simpler but loses context.)

3. **Settings as tab vs modal** — Confirmed: two-tab bottom nav (TERMINAL | SETTINGS), with the existing window tab bar staying inside TERMINAL. Or would you prefer a settings button/gear icon that opens a modal overlay instead?

4. **Bug report size limit** — Screenshots compressed to 0.8 quality via ImagePicker. Any maximum total payload size concern? (5 × ~300KB ≈ 1.5MB per report.)

5. **SMS on report received** — Should Mark be texted immediately when a report comes in, or only when the fix is deployed? Or both?
