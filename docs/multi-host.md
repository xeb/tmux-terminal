# Multiple hosts

The website and Rust daemon run on `not-invented-here`. The daemon controls
`vade` through the existing SSH alias, using its user's SSH configuration and
credentials. The browser only connects to the original website URL.

On `nih`, the `vade` SSH alias connects directly to `192.168.88.27:22` as `xeb`
using the existing SSH key. The daemon uses that same alias; Tailscale SSH is
not involved. Restart the daemon after changing the SSH destination so pooled
connections are renewed.

## Using it

- The window dropdown shows `host › session › window name (index)`.
- `not-invented-here` is displayed as `nih` in window labels and New Window;
  its API identity and saved selections continue using the full hostname.
- New Window always starts on `not-invented-here`. Use HOST to override this
  for one creation, even while viewing a remote window. Named creation prefers
  session `0`, creating it in the selected project when absent; `MASTER` is
  only an explicit choice.
- Sessions, project suggestions, agent availability, and Eunice models come
  from the creation host. Agent/model choices are remembered by host.
- Both hosts use their own `~/p/<project>`. Creating a remote window never
  copies or synchronizes project files. Instruction symlinks are prepared on
  that host without replacing existing files.
- Uploads go into the selected pane's working directory. A saving state covers
  the final local write or SSH transfer. Existing files are never overwritten.
- Image/text previews use the host that produced the terminal output.
- Offline hosts retain their selected window and retry independently. Actions
  are never redirected. Reordering stays within one host and session.

## Configuration

The default registry requires no configuration:

```json
[{"id":"not-invented-here"},{"id":"vade","ssh":"vade"}]
```

Set `TMUX_HOSTS` to a JSON array in the daemon's environment or `.env` to change
the registry. The first entry must be local (omit `ssh`) and is the default for
creation and older clients. IDs must be unique and contain only letters, digits,
dots, underscores, and hyphens. `ssh` is a server-owned SSH alias or destination;
clients cannot supply arbitrary ones. Restart after configuration changes.

The remote host needs SSH access without prompts, trusted host keys, tmux,
Python 3, Bash, and the desired agents. No second daemon or HTTP port is needed.
The fixed `scripts/host-files.py` helper is embedded in the binary and sent over
SSH. Agent commands must be visible in interactive Bash. Availability detection
does not verify agent credentials or provider account access.

Connections use batch mode, strict host-key checking, a private multiplexing
socket, deadlines, and bounded concurrency. Remote status snapshots are batched
and cached for two seconds per host. Connection failures stop an action;
mutations are not automatically retried.

Remote command arguments travel as encoded JSON and are restored with Python's
`execvp`. This preserves tabs in tmux metadata, multiline input, and helper
scripts across the direct SSH connection. Upload bytes remain on standard input.

## API and identity

`GET /api/hosts` returns `{default_host, hosts}`. `GET /api/agents` returns the
selected host's agent names. Set `X-Tmux-Host: vade` on host-dependent API calls;
`?host=vade` is also supported, including uploads and image URLs. Conflicting
header/query selections and unknown IDs return HTTP 400.

Calls without a host select the first host, preserving older web and native
mobile clients. `/api/windows` still returns native `target` and `name`, adding
`window_id`. Each call lists one host; the website combines independent polls.

Browser identity includes host, session, and stable window ID. API calls carry a
native tmux target such as `@42` plus the host header. Multistep model/question
actions pin to a pane and lock by host plus pane ID. Old saved `session:index`
selections migrate to the primary host when its window list arrives.

SSH failures return HTTP 503 with `offline: true`, distinct from a closed window.
Complete uploads are published atomically without replacing existing filenames.
Missing upload targets are errors; there is no home-directory fallback.

## Verification and deployment

```sh
cargo test
cargo build --release
python3 tests/test_web_api.py
python3 tests/test_multi_host.py
node --test tests/web.test.cjs
NODE_PATH=/tmp/tmux-terminal-browser-check/node_modules node --test tests/multi-host.browser.test.cjs
```

The multi-host HTTP test uses separate fake local/SSH contexts and temporary
directories. It checks equal IDs across hosts, legacy routing, independent
projects, literal arguments, concurrent uploads, partial transfers, previews,
outages, and recovery without mutation retries. The Chromium test covers host
controls, creation defaults, upload races, independent polling, and small
viewports. The original browser suite also includes WebKit, which requires its
system dependencies.

Deploy through the local-build workflow in `AGENTS.md`: stop the service, copy
the binary, `static/`, and `scripts/`, and restart it.
