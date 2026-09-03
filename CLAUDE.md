# tmux-terminal

Web-based tmux terminal interface written in Rust.

## Project Structure

```
src/main.rs         # Axum HTTP server with tmux command handlers
static/index.html   # Single-page web interface with all JS/CSS inline
Makefile            # Build, install, and service management
tmux-terminal.service  # systemd unit file
```

## Tech Stack

- **Backend**: Rust with Axum web framework
- **Frontend**: Vanilla JS with inline CSS, no build step
- **Process Control**: Direct `tmux` CLI invocation via `std::process::Command`

## Architecture

The server exposes REST endpoints that shell out to `tmux` commands:
- `tmux list-windows` for window enumeration
- `tmux capture-pane` for reading output
- `tmux send-keys` for command input
- `tmux new-window` for window creation

Static files served from `static/` directory with no-cache headers.

## Build Commands

```bash
cargo build --release    # Production build
cargo run               # Development
cargo test               # Unit tests (parsers, agent table, symlinks, trust prompts)
```

## Deploying

`make install` cannot overwrite the binary while the service is running
("Text file busy"), and `make update` runs `git pull` first. To deploy a local
build: `make stop`, copy `target/release/tmux-terminal`, `static/` and
`scripts/` into `~/bin/tmux-terminal/`, then `make start`. Verify with
`curl -s -o /dev/null -w '%{http_code}' http://localhost:5533/api/windows`.

## Key Implementation Details

- New windows pick an agent (`claude`, `codex`, `agy`, `eunice`) and a tmux
  session in the new-window modal. Each agent launches with approvals bypassed
  (`Agent::command` in `src/main.rs`); the modal shows the exact command.
  A missing `agent` field means Codex, so older clients keep working.
- With EUNICE selected, a MODEL button opens a type-to-filter list inside the
  modal, fed by `GET /api/eunice-models`, which runs `eunice --list-models`
  through an interactive shell so it sees the same API keys the window will.
  The choice is passed as `eunice --model <id>`; the server validates the id
  and rejects a model for any other agent.
- New windows go to session `0` by default. `MASTER` holds the backend tmux
  control processes and is only ever an explicit pick.
- When a project has a `CLAUDE.md`, creating a window guarantees `AGENTS.md`
  and `GEMINI.md` symlinks to it. Existing files or symlinks are never replaced.
- Claude, Codex and AGY ask "do you trust this folder?" on first launch in a
  directory; the server answers yes for windows it just created
  (`auto_accept_trust_prompt`). EUNICE never asks.
- The working pill and agent badge recognise Claude, Codex, AGY and EUNICE.
  The parsers live in both `src/main.rs` and `static/index.html` and must stay
  in step.
- Uses `-l` flag with `send-keys` for literal input (prevents escape sequence interpretation)
- Captures last 1000 lines of scrollback with `-S -1000`
- Window selection persisted in browser localStorage
- Hostname-based configuration for display modes
- Prefix mode (Ctrl+B) implemented entirely in frontend JS

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
   cd /media/xeb/GreyArea/projects/tmux-terminal/mobile && bash build.sh --ota "fix: <brief summary of fixes>"
   ```

6. **Notify Mark** via curl when the OTA is deployed:
   ```bash
   curl -X POST http://localhost:5533/api/notify \
     -H 'Content-Type: application/json' \
     -d '{"message": "Bug fixes deployed via OTA. Fixed: <summary>."}'
   ```

Do not run `build.sh` without `--ota` unless the fix requires native module
changes. OTA is sufficient for all JS/TS changes.
