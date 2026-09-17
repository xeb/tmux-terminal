# Mobile rendering and polling

The website remains vanilla JavaScript/CSS with no frontend build step. Source
files are `static/index.html`, `static/app.js`, `static/hosts.js`, `static/terminal.js`,
`static/app.css`, and `static/session-model.js`.

At startup, `src/web_assets.rs` publishes content-hashed copies of scripts,
styles, and local IBM Plex Mono fonts in `static/assets/`. It rewrites the served
HTML and font URLs. Restart the server after source changes. Keep old generated
assets during deployment so already-open tabs can still fetch their versions.
HTML requires revalidation; versioned assets are immutable for one year; APIs,
previews, and mutable source URLs use `no-store`. Font licensing is in
`static/fonts/OFL.txt`. No external font requests are needed.

`AdaptivePoller` permits one in-flight request per endpoint, coalesces immediate
refreshes, aborts old generations, and rejects their late results. The terminal
polls one second after completion while changing/working and every five seconds
when idle. Status checks use 3/15 seconds; the window list uses 3/30 seconds
(empty/nonempty). Failures retry after 3 seconds with backoff capped at 30
seconds. Hidden, offline, and BFCache-suspended pages stop all three pollers and
resume immediately on return.

The web client requests 200 scrollback lines, loading 200 more when scrolling
near the top or pressing Load older output, up to the original 1000-line limit.
Clients omitting `history_lines` retain the original limit. Captures include
plain and styled text from one snapshot, and exact `has_more` metadata from the
same tmux invocation. That invocation also reads the foreground process for the
agent badge, so a Codex transcript quoting Hermes output cannot relabel the
window. Interpreter names such as Python/Node still require matching UI evidence.
Parsing/linkification is cached per line and incoming ANSI
state; unchanged DOM rows survive updates. Scroll anchoring retains the visible
row during history prepends, and text selection defers terminal changes.

The web renderer converts ANSI colors to grayscale. White, cream, and light-gray
foregrounds display as black for the light page theme; dark highlights behind
those runs are lightened to retain contrast. This happens after reverse-video
resolution and leaves tmux colors and captured ANSI state unchanged.

The layout follows `VisualViewport.height/offsetTop` for iOS keyboards, with
dynamic viewport units as a fallback and safe-area padding. Compact controls
also follow the visual height because iOS keyboard changes do not necessarily
trigger height media queries. Pinch zoom does not resize the app. Touch devices
only focus the composer for an explicit edit; dialogs and question cards scroll
within the available height.

## Verification

```sh
cargo test
node --test tests/web.test.cjs
cargo build --release
python3 tests/test_web_api.py
```

The HTTP test runs an isolated server with fake tmux commands, including checks
that ordinary messages never enter the question editor.

Optional browser tests require Playwright and Chromium/WebKit. Install them in
a temporary directory to keep the application free of npm dependencies:

```sh
npm install --prefix /tmp/tmux-terminal-browser-check playwright
/tmp/tmux-terminal-browser-check/node_modules/.bin/playwright install webkit
NODE_PATH=/tmp/tmux-terminal-browser-check/node_modules node --test tests/browser.test.cjs
```

The Chromium test uses `/usr/bin/google-chrome`; WebKit uses Playwright's browser.
Playwright's platform libraries must be available. Both tests mock all APIs and
check startup retry, narrow layouts, keyboard-sized viewports, history anchoring,
window switching, question controls, unchanged-row retention, and HTML escaping.
Device emulation cannot replace checking the actual keyboard in Chrome on iOS.
