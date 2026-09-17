# Multiple tmux hosts

Investigation: 2026-09-13. Recorded before implementation. The user chose
independent project directories. See [multi-host.md](multi-host.md) for the
implemented API, configuration, and verification instructions.

## Verified environment

- The current machine is `not-invented-here`, user `xeb`, and its user service
  `tmux-terminal.service` is active.
- The SSH alias `vade` resolves to `xeb@vade.tailfb20.ts.net:22`.
- Read-only SSH commands succeeded with `BatchMode=yes`, strict host-key
  checking, and an eight-second connection timeout. They also succeeded with
  `SSH_AUTH_SOCK` removed. The service's actual execution environment still needs
  an integration check during implementation.
- `vade` reports tmux 3.7c, session `0`, the default tmux socket, Bash as tmux's
  default shell, `/home/xeb/p` as its project root, and Python 3 available.
- A noninteractive SSH shell did not find any of the four agent commands. A
  `bash -ic` lookup found Codex and Eunice in `/home/xeb/.local/bin`; Claude and
  AGY were not found in that environment. Command discovery does not establish
  that agent credentials or model access work.
- No windows were created, no pane input was sent, and no remote files were
  uploaded during this investigation.

## User experience

Keep the existing website URL. Add a compact HOST selector above the agent and
session controls in New Window:

```text
HOST     [not-invented-here]  vade
AGENT    CLAUDE  [CODEX]  AGY  EUNICE
SESSION  [0]  ...
```

Every opening resets the creation host to `not-invented-here`, even while viewing
a window on `vade`. The override applies only to this creation. Continue to
prefer session `0` and require an explicit selection of `MASTER`.

Changing the creation host refreshes sessions, project suggestions, available
agents, and Eunice models for that host. Cache these by host, discard responses
from an earlier selection, and show unavailable agents as disabled with a short
explanation. Do not install missing agents as part of this feature.

The main window dropdown combines both hosts, keeping configured host order:

```text
not-invented-here › 0 › tmux-terminal (3)
vade › 0 › tmux-terminal (2)
```

Host, session, and window name are always visible in that order; the optional
index distinguishes duplicate names. Preserve waiting/working indicators and
the right-aligned Menu. Use the same labels in the keyboard window switcher,
rename/kill context, and upload dialog. Store identity separately from labels.

Remember the selected existing window across reloads. Migrate old local
`tmux-selected-target` values to the primary host. The creation-host default is
independent of the remembered viewing selection.

## Server architecture

Use one daemon on `not-invented-here` and an ordered, validated host registry:

| Host ID | Transport | SSH alias | Project root |
| --- | --- | --- | --- |
| `not-invented-here` | local | — | `~/p` |
| `vade` | SSH | `vade` | `~/p` |

Resolve `~` on the destination host. Allow additional hosts through configuration
without adding host-specific branches to handlers. Browser requests supply a
registered host ID, never an arbitrary SSH destination or executable.

Introduce a shared host operations layer for tmux commands and filesystem work.
Use local process/filesystem operations for the primary host and OpenSSH for
`vade`; no second HTTP daemon or exposed port is needed. Keep tmux and picker
parsing in Rust. Remote filesystem operations can use small fixed Python 3
helpers sent over SSH, with structured arguments rather than interpolated
filename or command input. This also supports canonical paths, instruction
symlinks, and exclusive file creation consistently.

Use asynchronous subprocesses with deadlines, cancellation and bounded
concurrency. Reuse SSH connections with `ControlMaster=auto`, `ControlPersist`
and a private control socket directory. Connect timeouts alone do not bound a
hung remote command. Avoid new SSH handshakes per polling request; batch status
captures per host and keep a short per-host cache so multiple browsers do not
multiply remote polling work.

OpenSSH documents [connection reuse and batch options](https://man.openbsd.org/ssh_config#ControlMaster).
Its [command execution interface](https://man.openbsd.org/ssh#DESCRIPTION)
joins command arguments with spaces before remote execution: local argv handling
alone is insufficient. Quote every remote argument centrally or pass data over
stdin to a fixed helper. Keep binary upload content on stdin and diagnostics on
stderr. Use the remote interactive shell only where agent environment discovery
requires it, not for ordinary tmux capture or binary transfer.

## Identity and API compatibility

Use explicit fields such as `host`, `session`, `window_id`, `window_index`,
`name`, and native tmux `target`. Frontend maps use a collision-free tuple key,
not `session:index` alone or a parsed display label. Prefer stable window IDs
for new-client targeting and pin multistep actions to the resolved pane ID.
Server action locks and question-mode tracking must include the host, because
tmux pane IDs such as `%1` repeat on different machines.

Add `GET /api/hosts` for ordered host metadata and availability. Let
`GET /api/windows?host=<id>` and `/api/window-status?host=<id>` return one host's
data; the website polls hosts independently and merges their results. This keeps
local discovery responsive during remote outages. Existing calls without a host
continue to mean the primary host and retain compatible response shapes.

Add host selection to capture, send, send-key, all picker routes, session-model,
new-window, new-window-named, rename, kill, move, project-dirs, eunice-models,
upload, serve-image, and serve-file. Audit all shared helper calls, including
trust-prompt handling, path checks, and instruction symlink creation. Voice
generation from supplied text and the serving of website assets remain central.

Restrict existing-name lookup to the requested host and session. Today
`new_window_named` searches every local session, so it can ignore the chosen
session by returning an unrelated matching name. Suggestions must use that same
scope. Reordering is valid only within one host/session; reject other pairs in
both frontend and backend.

An offline host is different from an empty tmux server or a closed window.
Retain the selected remote identity and show an offline state; do not silently
select a local window, submit locally, or erase the remembered selection. Retry
reads with backoff. Do not automatically retry mutations after an ambiguous SSH
disconnect, since the remote action may already have completed.

## Uploads and previews

Snapshot the selected host and stable window identity when a file batch is
chosen. Resolve its active pane and working directory on that host. Local
uploads write locally; remote uploads send bytes over SSH to that remote
directory and return the actual remote path.

Reject missing targets or unavailable directories instead of falling back to
the daemon's home directory. The current `pane_cwd` helper has that fallback,
which is unsuitable when a failed lookup could put a file on the wrong host.

Keep the existing filename cleanup and 100 MB request limit. Reserve a unique
destination without overwriting existing files, using exclusive creation or
atomic no-replace publication rather than a separate exists/write check. Clean
up partial transfers and report success only after the remote write completes.
Once the browser has sent the body to the daemon, show a saving/transferring
state until the daemon confirms completion; browser upload progress does not
measure the SSH leg.

Show the batch destination in the upload dialog. If the user switches windows
while a file is transferring, keep that file associated with its original
destination and do not append its path to the newly selected window's composer.
Today the completion callback inserts into whichever input is currently shown.

File and image links produced from remote terminal output carry their source
host. Serve previews through the daemon over SSH, preserving supported-type and
text-size checks. Include the host in renderer/link caches so identical paths
on the two machines cannot display the wrong file.

## Implementation and validation

1. Add the host registry and shared command/filesystem transport with mockable
   boundaries. Move every host-dependent operation behind it.
2. Add host fields and stable identities throughout handlers, locks and caches;
   preserve requests from older clients as primary-host operations.
3. Add the host selector, independently polled window lists, scoped suggestions,
   and host-aware existing-window actions, previews, and uploads.
4. Extend the existing Rust, fake-tmux HTTP, Node and browser tests. Cover equal
   tmux IDs/names on different hosts, creation default reset, same-name windows
   in different sessions, remote outage/reconnect, stale responses after host
   switches, remote model/question actions, reordered windows, and old clients.
5. Exercise uploads with binary data, spaces/quotes in filenames, concurrent
   same-name uploads, interrupted SSH, and selection changes during transfer.
   Verify remote preview routing and remote-only filesystem operations.
6. Before deployment, check SSH from the service execution environment. Validate
   mutation behavior with isolated tmux sockets and temporary directories, then
   build and deploy through the documented local-build service workflow when
   implementation/deployment is in scope.

## Confirmed project behavior

The user selected independent `~/p/<project>` directories on each host.
Uploads follow the selected host; automatic project synchronization is excluded.
