# Next feature: New Window → name prompt → claude project (notes, not a spec yet)

Parked while the MASTER-hiding feature ships. These are decisions already made plus
findings that must not be rediscovered. Brainstorm properly before implementing.

## Requested behavior

Clicking "New Window" in the web UI prompts for a name. On submit:

1. Open a new tmux window
2. `cd ~/p/<name>` and press Enter
3. Once in that directory, run `claude --yolo`
4. Rename the window to `<name>`

## Decisions made

- **Replaces** today's New Window behavior (both the ☰ MENU item and `^B c`).
  The one-click plain blank window goes away.
- **`mkdir -p ~/p/<name>`** if the directory doesn't exist — typing a new name
  scaffolds a fresh project rather than erroring.

## Findings that change the implementation

- **`claude --yolo` is NOT a real flag.** The `claude` binary rejects it
  (`error: unknown option '--yolo'`). It works only because `~/.bash_aliases`
  defines a `claude()` shell function that maps `--yolo` →
  `--dangerously-skip-permissions` and unsets `CLAUDECODE` / `ANTHROPIC_API_KEY`.

  **Consequence:** the command MUST be typed into the window via `tmux send-keys`,
  where an interactive bash sources the function. The Rust backend must never
  `Command::new("claude").arg("--yolo")` — that would fail outright.

- **Prior art to follow:** `trigger_bugfix_window()` at `src/main.rs:1057` already does
  exactly this shape — `tmux new-window -n <name>`, then
  `send-keys "cd <dir> && claude" Enter`, with a sleep to let claude boot.

- **Existing pieces to reuse:** `createNewWindow()` (`static/index.html:1654`) and the
  `POST /api/new-window` handler (`src/main.rs:338`). A rename modal already exists
  (`openRenameModal`) — reuse its prompt UI pattern rather than inventing one.

- **This needs backend changes**, unlike the MASTER feature. `/api/new-window` takes no
  body today; it will need a name parameter.

## Open questions for brainstorming

- **Name handling is the main risk.** The name flows into `cd ~/p/<name>` and
  `mkdir -p`. What is a legal name? Spaces, `/`, `..`, quotes, `$`, backticks?
  Sending unsanitized text via `send-keys` into a shell means a name like
  `foo; rm -rf ~` executes. Needs an explicit allowlist (e.g. `[A-Za-z0-9._-]+`)
  and rejection of `..` and leading `-`, decided deliberately.
- Auto-rename: `new-window -n` sets the name, but does `claude` starting up trigger
  tmux's automatic-rename and clobber it? The requested step 4 (rename last) suggests
  the user has seen this. Verify empirically.
- What if a window with that name already exists — reuse it (like
  `trigger_bugfix_window` does) or create a second?
- How long to wait between `cd` and `claude --yolo`? The prior art sleeps 4s after
  launching claude.

## Testing note

Test against a dev server on port **5534** serving the repo's `static/`, never 5533 —
5533 is the live installed app at `~/bin/tmux-terminal/` serving its own copy.
