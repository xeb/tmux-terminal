// Linkify terminal output: escape HTML, then wrap URLs and image paths in <a> tags
function linkifyTerminalOutput(text) {
    // Escape HTML special chars to prevent XSS
    const escaped = text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;');

    // Match http/https URLs - stop at whitespace or common trailing punctuation
    let result = escaped.replace(
        /(https?:\/\/[^\s&"<>)\]]+)/g,
        '<a class="terminal-link" href="$1" target="_blank" rel="noopener noreferrer">$1</a>'
    );

    // Match absolute file paths to images (not already inside an href)
    result = result.replace(
        /(?<![="\/\w])(\/[\w.\-\/]+\.(?:jpg|jpeg|png|gif|webp|bmp|svg|tiff|tif))(?=[\s&;,)\]"']|$)/gi,
        '<a class="terminal-link image-link" href="#" data-image-path="$1">$1</a>'
    );

    // Match absolute file paths to JSON and other text files
    result = result.replace(
        /(?<![="\/\w])(\/[\w.\-\/]+\.(?:json|txt|log|csv|xml|yaml|yml|toml|md|ini|cfg))(?=[\s&;,)\]"']|$)/gi,
        '<a class="terminal-link file-link" href="#" data-file-path="$1">$1</a>'
    );

    return result;
}

// tmux's `capture-pane -e` preserves the SGR attributes painted by a
// terminal app. Codex uses a true-colour background for its composer
// and user-message pills; the old plain capture discarded that entire
// layer. Preserve contrast and background shape but convert every ANSI
// colour to gray so the web view remains intentionally monochrome.
const ANSI_16 = [
    [17, 17, 17], [165, 29, 45], [35, 134, 54], [154, 103, 0],
    [36, 87, 197], [143, 42, 163], [8, 127, 140], [196, 196, 196],
    [110, 119, 129], [207, 34, 46], [45, 164, 78], [191, 135, 0],
    [9, 105, 218], [191, 57, 137], [5, 152, 168], [255, 255, 255],
];

function monochromeColor(r, g, b) {
    const shade = Math.round(0.2126 * r + 0.7152 * g + 0.0722 * b);
    return `rgb(${shade},${shade},${shade})`;
}

function ansiColor(index) {
    if (!Number.isInteger(index) || index < 0 || index > 255) return null;
    if (index < 16) return monochromeColor(...ANSI_16[index]);
    if (index < 232) {
        const n = index - 16;
        const values = [0, 95, 135, 175, 215, 255];
        const r = values[Math.floor(n / 36)];
        const g = values[Math.floor((n % 36) / 6)];
        const b = values[n % 6];
        return monochromeColor(r, g, b);
    }
    const grey = 8 + (index - 232) * 10;
    return `rgb(${grey},${grey},${grey})`;
}

function freshAnsiState() {
    return {
        fg: null, bg: null, bold: false, dim: false, italic: false,
        underline: false, strike: false, reverse: false,
    };
}

function applySgr(state, rawParams) {
    const values = (rawParams || '0')
        .replace(/:/g, ';')
        .split(';')
        .map(value => value === '' ? 0 : Number(value));
    for (let i = 0; i < values.length; i++) {
        const code = values[i];
        if (code === 0) Object.assign(state, freshAnsiState());
        else if (code === 1) state.bold = true;
        else if (code === 2) state.dim = true;
        else if (code === 3) state.italic = true;
        else if (code === 4 || code === 21) state.underline = true;
        else if (code === 7) state.reverse = true;
        else if (code === 9) state.strike = true;
        else if (code === 22) { state.bold = false; state.dim = false; }
        else if (code === 23) state.italic = false;
        else if (code === 24) state.underline = false;
        else if (code === 27) state.reverse = false;
        else if (code === 29) state.strike = false;
        else if (code >= 30 && code <= 37) state.fg = ansiColor(code - 30);
        else if (code >= 90 && code <= 97) state.fg = ansiColor(code - 90 + 8);
        else if (code === 39) state.fg = null;
        else if (code >= 40 && code <= 47) state.bg = ansiColor(code - 40);
        else if (code >= 100 && code <= 107) state.bg = ansiColor(code - 100 + 8);
        else if (code === 49) state.bg = null;
        else if ((code === 38 || code === 48) && values[i + 1] === 5) {
            const color = ansiColor(values[i + 2]);
            if (code === 38) state.fg = color;
            else state.bg = color;
            i += 2;
        } else if ((code === 38 || code === 48) && values[i + 1] === 2) {
            const rgb = values.slice(i + 2, i + 5);
            if (rgb.length === 3 && rgb.every(v => Number.isInteger(v) && v >= 0 && v <= 255)) {
                const color = monochromeColor(rgb[0], rgb[1], rgb[2]);
                if (code === 38) state.fg = color;
                else state.bg = color;
            }
            i += 4;
        }
    }
}

function ansiStyle(state) {
    let fg = state.fg;
    let bg = state.bg;
    if (state.reverse) {
        const oldFg = fg || 'var(--matrix-dim)';
        fg = bg || 'var(--terminal-bg)';
        bg = oldFg;
    }
    // Codex normally chooses a composer tint against the terminal's
    // own palette. The web view has a fixed light palette, so supply a
    // black/white foreground when a captured background would
    // otherwise leave default text without reliable contrast.
    if (bg && !fg) {
        const match = bg.match(/^rgb\((\d+),(\d+),(\d+)\)$/);
        if (match) fg = Number(match[1]) < 128 ? '#fff' : '#111';
    }
    // ANSI white includes normal white (palette 7), bright white, and the
    // cream/near-white colors and light-gray status text used by Hermes.
    // Resolve reverse video first so only the displayed foreground changes,
    // never the captured ANSI state.
    const foreground = fg?.match(/^rgb\((\d+),(\d+),(\d+)\)$/);
    if (fg === '#fff' || fg === 'var(--terminal-bg)'
            || (foreground && Number(foreground[1]) >= 188)) {
        fg = '#000';
        // Keep formerly white-on-dark highlights legible as black-on-light.
        const background = bg?.match(/^rgb\((\d+),(\d+),(\d+)\)$/);
        if (background && Number(background[1]) < 128) {
            const shade = 255 - Number(background[1]);
            bg = `rgb(${shade},${shade},${shade})`;
        } else if (bg === 'var(--matrix-dim)') {
            bg = 'var(--terminal-bg)';
        }
    }
    const css = [];
    if (fg) css.push(`color:${fg}`);
    if (bg) css.push(`background-color:${bg}`);
    if (state.bold) css.push('font-weight:700');
    if (state.dim) css.push('opacity:.65');
    if (state.italic) css.push('font-style:italic');
    const decorations = [];
    if (state.underline) decorations.push('underline');
    if (state.strike) decorations.push('line-through');
    if (decorations.length) css.push(`text-decoration:${decorations.join(' ')}`);
    return css.join(';');
}

const AGY_PROMPT_FG = ansiColor(4);
const AGY_THOUGHT_FG = ansiColor(7);
const AGY_TOOL_FGS = new Set([ansiColor(1), ansiColor(2), ansiColor(3)]);

function renderTerminalOutput(text, state = freshAnsiState(), meta = null) {
    let html = '';
    let plain = '';

    const flush = () => {
        if (!plain) return;
        const rendered = linkifyTerminalOutput(plain);
        const style = ansiStyle(state);
        html += style ? `<span class="ansi-run" style="${style}">${rendered}</span>` : rendered;
        plain = '';
    };

    for (let i = 0; i < text.length;) {
        if (text.charCodeAt(i) !== 0x1b) {
            // Carriage return and backspace are cursor operations, not
            // printable output. Newline and tab remain ordinary text.
            const ch = text[i];
            if (ch !== '\r' && ch !== '\b') {
                plain += ch;
                if (meta) {
                    meta.plain += ch;
                    if (ch !== ' ' && ch !== '\t' && ch !== '\n') {
                        if (!meta.seenChar) {
                            meta.seenChar = true;
                            meta.firstFg = state.fg;
                            meta.firstBold = state.bold;
                            meta.allSgr37 = state.fg === AGY_THOUGHT_FG;
                        } else if (meta.allSgr37 && state.fg !== AGY_THOUGHT_FG) {
                            meta.allSgr37 = false;
                        }
                    }
                }
            }
            i += 1;
            continue;
        }

        flush();
        if (text[i + 1] === '[') {
            let end = i + 2;
            while (end < text.length) {
                const code = text.charCodeAt(end);
                if (code >= 0x40 && code <= 0x7e) break;
                end += 1;
            }
            if (end >= text.length) break;
            if (text[end] === 'm') applySgr(state, text.slice(i + 2, end));
            i = end + 1;
            continue;
        }
        if (text[i + 1] === ']') {
            let end = i + 2;
            while (end < text.length && text.charCodeAt(end) !== 0x07
                    && !(text.charCodeAt(end) === 0x1b && text[end + 1] === '\\')) {
                end += 1;
            }
            i = end < text.length
                ? end + (text.charCodeAt(end) === 0x07 ? 1 : 2)
                : text.length;
            continue;
        }
        // Charset selectors have an intermediate plus a final byte;
        // other ESC controls are two bytes.
        i += ['(', ')', '*', '+', '-', '.', '/'].includes(text[i + 1]) ? 3 : 2;
    }
    flush();
    return html;
}

// AGY prints thoughts and tool metadata in SGR 37 (`\x1b[37m`) and immediately
// starts the assistant reply on the next 2-space-indented row in default
// foreground with no blank line. Track the turn phase across rows so the CSS
// layer can mute the thought/tool trace and visually anchor the reply start.
function classifyAgyRow(meta, phase) {
    const plain = meta.plain;
    const trimmed = plain.trim();
    if (!trimmed) {
        return {
            phase,
            rowClass: phase === 'prompt' || phase === 'trace' ? 'agy-trace-blank' : '',
        };
    }
    const atCol0 = !plain.startsWith(' ');
    if (atCol0 && /^─{20,}$/.test(trimmed)) {
        return { phase: 'idle', rowClass: '' };
    }
    if (atCol0 && /^─+\s*Conversation compacted\s*─+$/.test(trimmed)) {
        return { phase: 'trace', rowClass: 'agy-trace' };
    }
    if (/^\s*(\? for shortcuts|esc to cancel)\s{2,}\S/.test(plain)) {
        return { phase: 'idle', rowClass: '' };
    }
    if ((plain.startsWith('> ') && meta.firstFg === AGY_PROMPT_FG)
            || (phase === 'prompt' && plain.startsWith('  ') && meta.firstFg === AGY_PROMPT_FG && meta.firstBold)) {
        return { phase: 'prompt', rowClass: 'agy-prompt' };
    }
    const isThoughtHdr = atCol0 && /^[▸▾○]\s+Thought for\b/.test(trimmed);
    const isTool = atCol0 && /^[●○]\s+\S/.test(trimmed) && AGY_TOOL_FGS.has(meta.firstFg);
    const isMoreActions = atCol0 && /^…\s+\d+\s+more action/.test(trimmed);
    const isSpinner = atCol0 && /^[\u2800-\u28FF]\s+\S.+\.\.\.$/.test(trimmed);
    const isTip = atCol0 && trimmed.startsWith('└ Tip:');
    const isOrphanTrace = phase === 'idle' && meta.allSgr37
        && ((plain.startsWith('  ') && trimmed.endsWith('...')) || trimmed.endsWith('expand)'));
    const isSgr37Trace = (phase === 'prompt' || phase === 'trace') && meta.allSgr37;
    if (isThoughtHdr || isTool || isMoreActions || isSpinner || isTip || isOrphanTrace || isSgr37Trace) {
        const isThoughtBody = (isSgr37Trace || isOrphanTrace) && plain.startsWith('  ') && !isThoughtHdr;
        return {
            phase: 'trace',
            rowClass: isThoughtBody ? 'agy-trace agy-thought-body' : 'agy-trace',
        };
    }
    if ((phase === 'prompt' || phase === 'trace') && plain.startsWith('  ')) {
        return { phase: 'reply', rowClass: 'agy-reply agy-reply-start' };
    }
    if (phase === 'reply') {
        if (meta.allSgr37 && (trimmed === '(ctrl+o to expand)' || trimmed === 'Sources:')) {
            return { phase: 'reply', rowClass: 'agy-trace' };
        }
        return { phase: 'reply', rowClass: 'agy-reply' };
    }
    return { phase: 'idle', rowClass: '' };
}

// Reuse both parsed ANSI runs and DOM rows. A changing timer must not replace
// the transcript above it. The incoming ANSI state is part of the cache key:
// tmux can leave an attribute active across a newline.
class TerminalRenderer {
    constructor(element) {
        this.element = element;
        this.rows = [];
        this.cache = new Map();
        this.text = null;
        this.following = true;
    }

    reset() {
        this.element.replaceChildren();
        this.rows = [];
        this.cache.clear();
        this.text = null;
        this.following = true;
    }

    render(text, follow = false) {
        if (text === this.text) return false;
        const element = this.element;
        const atBottom = follow || element.scrollHeight - element.scrollTop <= element.clientHeight + 50;
        this.following = atBottom;
        const oldScrollTop = element.scrollTop;
        const available = new Map();
        for (const row of this.rows) {
            if (!available.has(row.key)) available.set(row.key, []);
            available.get(row.key).push(row);
        }
        const lines = text.split('\n');
        if (lines.at(-1) === '') lines.pop();
        const nextCache = new Map();
        let state = freshAnsiState();
        let agyPhase = 'idle';
        const rows = lines.map(line => {
            const key = agyPhase + '\0' + JSON.stringify(state) + '\0' + line;
            let parsed = this.cache.get(key) || nextCache.get(key);
            if (!parsed) {
                const meta = { plain: '', seenChar: false, firstFg: null, firstBold: false, allSgr37: false };
                const html = renderTerminalOutput(line, state, meta);
                const { phase: nextPhase, rowClass } = classifyAgyRow(meta, agyPhase);
                parsed = { html, endState: { ...state }, nextPhase, rowClass };
            }
            state = { ...parsed.endState };
            agyPhase = parsed.nextPhase;
            nextCache.set(key, parsed);
            const reused = available.get(key)?.shift();
            if (reused) return reused;
            const node = document.createElement('div');
            node.className = parsed.rowClass ? `terminal-line ${parsed.rowClass}` : 'terminal-line';
            node.innerHTML = parsed.html || '<br>';
            return { key, element: node };
        });

        // Remove expired rows before inserting, so scrollback shifting by one
        // line doesn't move every retained row through the DOM.
        const keep = new Set(rows.map(row => row.element));
        // A capture starting earlier can change the first visible row's
        // incoming ANSI state and replace its DOM node. Anchor to the first
        // surviving row, measuring its position before any nodes are removed.
        const anchor = atBottom ? null : this.rows.find(row =>
            keep.has(row.element)
            && row.element.offsetTop + row.element.offsetHeight > oldScrollTop);
        const anchorTop = anchor?.element.getBoundingClientRect().top;
        for (const child of [...element.childNodes]) {
            if (!keep.has(child)) child.remove();
        }
        let cursor = element.firstChild;
        for (const row of rows) {
            if (row.element !== cursor) element.insertBefore(row.element, cursor);
            else cursor = cursor.nextSibling;
        }
        this.rows = rows;
        this.cache = nextCache;
        this.text = text;

        if (atBottom) element.scrollTop = element.scrollHeight;
        else if (anchor?.element.isConnected) {
            element.scrollTop = oldScrollTop + anchor.element.getBoundingClientRect().top - anchorTop;
        }
        return true;
    }
}

// Completion-based scheduling prevents overlapping requests. Visibility and
// target changes abort the old generation, whose responses must be ignored.
class AdaptivePoller {
    constructor(task, { active = 1000, idle = 5000, maximum = 30000, retry = 3000 } = {}) {
        this.task = task;
        this.active = active;
        this.idle = idle;
        this.maximum = maximum;
        this.retry = retry;
        this.failures = 0;
        this.delay = active;
        this.generation = 0;
        this.running = false;
        this.pending = false;
        this.timer = null;
        this.controller = null;
        this.flight = null;
        this.waiters = [];
    }

    start() {
        this.stop();
        this.running = true;
        this.delay = this.active;
        this.failures = 0;
        return this.request();
    }

    stop() {
        this.running = false;
        this.pending = false;
        this.generation++;
        clearTimeout(this.timer);
        this.controller?.abort();
        this.waiters.splice(0).forEach(resolve => resolve());
    }

    request() {
        if (!this.running) return Promise.resolve();
        clearTimeout(this.timer);
        if (this.flight) {
            this.pending = true;
            return new Promise(resolve => this.waiters.push(resolve));
        }
        const generation = this.generation;
        const controller = this.controller = new AbortController();
        const current = () => this.running && generation === this.generation && !controller.signal.aborted;
        const timeout = setTimeout(() => controller.abort(), 10000);
        this.flight = Promise.resolve().then(() => this.task(controller.signal, current))
            .then(busy => {
                if (current()) {
                    this.failures = 0;
                    this.delay = busy ? this.active : this.idle;
                }
            })
            .catch(() => {
                if (this.running && generation === this.generation) {
                    this.delay = Math.min(this.maximum, this.retry * 2 ** Math.min(this.failures++, 5));
                }
            })
            .finally(() => {
                clearTimeout(timeout);
                this.flight = null;
                if (!this.running) return;
                const delay = this.pending ? 0 : this.delay;
                this.pending = false;
                this.timer = setTimeout(() => {
                    const waiters = this.waiters.splice(0);
                    this.request().then(() => waiters.forEach(resolve => resolve()));
                }, delay);
            });
        return this.flight;
    }
}
