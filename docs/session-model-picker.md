# Session model picker

Click the black CLI badge in the top bar to open **Model & effort** for that
window. Pick a model, adjust its effort, then Apply. Cancel leaves the CLI's
selection uncommitted. The working and waiting tags sit before the badge so the
right edge of Menu stays fixed as activity changes.

## CLI integration

Inspected in isolated tmux windows on 2026-09-08:

| CLI | Version | Live controls |
| --- | --- | --- |
| Claude | 2.1.263 | `/model`; up/down selects a model; left/right adjusts effort when supported; `s` applies to this session only. |
| Codex | 0.153.4 | `/model`; choose model, then reasoning level. “More reasoning…” opens Max/Ultra where available. |
| AGY | 1.1.28 | `/model`; up/down selects a model; left/right adjusts its supported effort levels. Some models have fixed effort. |
| Eunice | locally extended 1.0.14 | `/model <id> [effort]` and `/effort <level>` replace the session client while preserving messages, instructions and tool outputs. |

The first three integrations read their live menus, including account-specific
choices. No model names or reasoning menus are hardcoded in the web client.
Their native persistence behavior applies; Claude's session-only shortcut avoids
changing its default. Codex's [model command](https://learn.chatgpt.com/docs/developer-commands?surface=cli)
also controls reasoning effort.

Eunice supplies its available-provider catalog through a pane-local tmux option.
The web bridge uses `/model --tmux`; manual users can inspect `/model --json`.
Application results carry a nonce, so an earlier confirmation cannot acknowledge
a new request. New input invalidates previously opened settings. Gemini's
[thinking controls](https://ai.google.dev/gemini-api/docs/thinking) are sent as
`generationConfig.thinkingConfig.thinkingLevel`; supported OpenAI reasoning
models receive `reasoning_effort`. Other adapters retain their provider default.
Local models that require download/server startup are excluded from live
switching. A running local model can keep its existing configuration.

Eunice tool-history migration removes model-specific thought signatures and
keeps tool calls linked to their results when crossing providers. No messages or
tool output content are discarded. An already-running pre-extension Eunice
process must be relaunched once; it cannot gain commands from a binary update.

## Consistency and recovery

`POST /api/session-model` accepts open, select, effort, back, apply, and cancel
actions. Opening requires an idle CLI with an empty terminal composer, or its
existing model menu. The resolved pane ID pins the operation across window
renames/reordering. Actions verify the foreground program and a fingerprint of
the currently displayed choices, cursor and effort. Cursor movement is observed
before any commit; each command sends exactly one Enter.

If another terminal user changes the menu or the CLI doesn't acknowledge a
change, the dialog offers Reload and reports the error. Closing a failed dialog
does not send speculative keys. The native terminal remains available for
recovery. Menus too narrow or otherwise unrecognized fail without choosing an
option.

## Validation

Fixtures cover Claude models with/without effort, AGY three-level/two-level/fixed
effort, and Codex model/basic/advanced reasoning screens. Unit tests cover stale
menus and nonempty/busy composers. Live tmux checks exercise model selection,
effort, apply and cancel; browser checks cover the dialog and fixed Menu position
at desktop and phone widths.
