# tmux-terminal

A web-based terminal interface for interacting with tmux sessions. Features a retro CRT/Matrix aesthetic and tmux-style keyboard shortcuts.

## Features

- Web interface to send commands to tmux panes
- Live capture of pane output (polling every second)
- Window management (list, switch, create new)
- Tmux-style keyboard shortcuts with `Ctrl+B` prefix
- Retro CRT scanline visual effects
- Runs as a systemd user service

## Requirements

- Rust (for building)
- tmux (must be running with an active session)
- Linux with systemd (for service installation)

## Quick Start

```bash
# Build and run directly
./run.sh

# Or build manually
cargo build --release
./target/release/tmux-terminal
```

Access the interface at `http://localhost:5533`

## Installation

Install as a systemd user service:

```bash
make install
```

This will:
- Build the release binary
- Install to `~/bin/tmux-terminal/`
- Enable and start the systemd user service

### Other Make Commands

```bash
make build      # Build release binary
make uninstall  # Remove service and files
make update     # Pull latest changes and restart
make start      # Start service
make stop       # Stop service
make restart    # Restart service
make status     # Check service status
```

## Configuration

- **Port**: Set via `PORT` environment variable (default: `5533`)
- **Large mode**: Automatically enabled on hostname `marcker-mac.roam.internal` for larger fonts

## Keyboard Shortcuts

Press `Ctrl+B` to enter prefix mode, then:

| Key | Action |
|-----|--------|
| `c` | Create new window |
| `w` | Open window list |
| `n` | Next window |
| `p` | Previous window |
| `?` | Show help |

In window list modal:
- `j`/`k` or arrows to navigate
- `Enter` to select
- `Esc` or `q` to cancel

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/windows` | GET | List all tmux windows |
| `/api/capture` | POST | Capture pane content |
| `/api/send` | POST | Send command to tmux |
| `/api/new-window` | POST | Create new window |
| `/api/config` | GET | Get server configuration |
| `/health` | GET | Health check |

## License

MIT
