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
- **Large mode**: Automatically enabled when the server's hostname ends in `.roam.internal` (Cloudflare WARP private DNS). Edit `src/main.rs` to match your own hostname if desired.

## Remote Access via Cloudflare Tunnel

tmux-terminal runs on localhost and is not exposed to the public internet. To access it securely from anywhere — including the mobile app — use a **Cloudflare Tunnel** combined with **Cloudflare WARP** for private DNS resolution.

### Overview

```
Your device (WARP client)
    │
    │  resolves myhost.roam.internal → Cloudflare network
    ▼
Cloudflare Edge
    │
    │  routes via named tunnel
    ▼
cloudflared (running on the server)
    │
    │  proxies to localhost:5533
    ▼
tmux-terminal
```

### Step 1 — Install cloudflared

```bash
# Debian / Ubuntu
curl -L https://pkg.cloudflare.com/cloudflare-main.gpg | sudo tee /usr/share/keyrings/cloudflare-main.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/cloudflare-main.gpg] https://pkg.cloudflare.com/cloudflared jammy main" \
  | sudo tee /etc/apt/sources.list.d/cloudflared.list
sudo apt update && sudo apt install cloudflared

# macOS
brew install cloudflare/cloudflare/cloudflared

# Verify
cloudflared --version
```

### Step 2 — Authenticate with your Cloudflare account

```bash
cloudflared tunnel login
```

This opens a browser window. Select the Cloudflare zone (domain) you want to use. A certificate is saved to `~/.cloudflared/cert.pem`.

### Step 3 — Create a named tunnel

```bash
cloudflared tunnel create tmux-terminal
```

Example output:
```
Created tunnel tmux-terminal with id a1b2c3d4-e5f6-7890-abcd-ef1234567890
Tunnel credentials written to /home/user/.cloudflared/a1b2c3d4-...json
```

Note the tunnel UUID — you'll need it in the next step.

### Step 4 — Write the tunnel config

Create `~/.cloudflared/config.yml`:

```yaml
tunnel: a1b2c3d4-e5f6-7890-abcd-ef1234567890   # your tunnel UUID
credentials-file: /home/user/.cloudflared/a1b2c3d4-e5f6-7890-abcd-ef1234567890.json

ingress:
  - hostname: myhost.roam.internal
    service: http://localhost:5533
  - service: http_status:404
```

Replace `myhost.roam.internal` with whatever private hostname you want. The `.roam.internal` suffix is resolved only by WARP clients (see Step 6); you can use any suffix your Zero Trust org is configured for.

### Step 5 — Route private DNS through Cloudflare Zero Trust

In the [Cloudflare Zero Trust dashboard](https://one.dash.cloudflare.com):

1. Go to **Networks → Tunnels** and confirm your tunnel appears.
2. Go to **Networks → Private Networks** → **Add a private network** and add your tunnel's virtual network.
3. Go to **Settings → WARP Client → Device settings** and ensure "Include all private networks" is enabled for your device profile.
4. Go to **Networks → Routes** and add a route:
   - **CIDR**: the private IP range your tunnel uses (or leave broad for split-tunnel)
   - **Tunnel**: select `tmux-terminal`

For hostname-based access (e.g. `myhost.roam.internal`), add a **Split Tunnel** DNS entry:

- Go to **Settings → WARP Client → Device settings → \<your profile\> → Split Tunnels**
- Add `roam.internal` (or your chosen suffix) so WARP resolves it internally

### Step 6 — Install cloudflared as a system service

```bash
# Install and start as a system service
sudo cloudflared service install

# Or as a systemd user service (no sudo needed)
cloudflared service install --user
systemctl --user enable cloudflared
systemctl --user start cloudflared

# Check status
systemctl --user status cloudflared
```

### Step 7 — Connect client devices via Cloudflare WARP

On each device that needs access:

1. Install the **Cloudflare WARP** app (iOS, Android, macOS, Windows, Linux).
2. Log in to your Zero Trust organization:
   - Open WARP → Preferences / Settings → Account → Login with Cloudflare Zero Trust
   - Enter your organization name (the slug from `<org>.cloudflareaccess.com`)
3. Enable WARP (the toggle should turn blue/on).
4. Verify DNS resolution:
   ```bash
   # macOS / Linux
   dig myhost.roam.internal
   # Should return a Cloudflare-assigned private IP

   curl http://myhost.roam.internal:5533/health
   # Should return {"status":"ok"}
   ```

### Updating the hostname detection in the server

tmux-terminal auto-enables "large mode" when it detects its own hostname ends in `.roam.internal`. To change the hostname or condition, edit `src/main.rs`:

```rust
// Before
let large_mode = hostname == "myhost.roam.internal";

// After — match any .roam.internal host
let large_mode = hostname.ends_with(".roam.internal");
```

Then rebuild and restart:

```bash
make update
```

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
