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
```

## Key Implementation Details

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
