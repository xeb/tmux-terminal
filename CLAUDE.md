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
- Captures last 100 lines of scrollback with `-S -100`
- Window selection persisted in browser localStorage
- Hostname-based configuration for display modes
- Prefix mode (Ctrl+B) implemented entirely in frontend JS
