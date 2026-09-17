// Fetch server config for hostname-specific styling
fetch('/api/config')
    .then(r => r.json())
    .then(config => {
        if (config.large_mode && matchMedia('(hover: hover) and (pointer: fine)').matches) {
            document.documentElement.style.setProperty('--output-font-size', '10pt');
            document.documentElement.style.setProperty('--input-font-size', '14pt');
            document.documentElement.style.setProperty('--input-height', '84px');
        }
    })
    .catch(() => {});

const commandInput = document.getElementById('commandInput');
const sessionSelect = document.getElementById('sessionSelect');
const menuBtn = document.getElementById('menuBtn');
const actionMenuModal = document.getElementById('actionMenuModal');
const sendBtn = document.getElementById('sendBtn');
const outputContent = document.getElementById('outputContent');
const agentIndicator = document.getElementById('agentIndicator');
const status = document.getElementById('status');
const prefixIndicator = document.getElementById('prefixIndicator');
const windowModal = document.getElementById('windowModal');
const windowList = document.getElementById('windowList');
const helpModal = document.getElementById('helpModal');
const renameModal = document.getElementById('renameModal');
const renameInput = document.getElementById('renameInput');
const newWindowModal = document.getElementById('newWindowModal');
const newWindowInput = document.getElementById('newWindowInput');
const nwSuggest = document.getElementById('nwSuggest');
const killModal = document.getElementById('killModal');
const killTargetEl = document.getElementById('killTarget');
const historyModal = document.getElementById('historyModal');
const historyList = document.getElementById('historyList');
const workingBar = document.getElementById('workingBar');
const workingVerb = document.getElementById('workingVerb');
const workingMeta = document.getElementById('workingMeta');

let isFirstCapture = true;
let captureTarget = '';
let lastObservedPane = null;
let historyLines = 200;
let hasMoreHistory = false;
let historyLoading = false;
const loadOlderBtn = document.getElementById('loadOlderBtn');
const terminalRenderer = new TerminalRenderer(outputContent);
let prefixMode = false;
let prefixTimeout = null;
let modalSelectedIndex = 0;
let modalWindows = [];

// MASTER window visibility. Windows whose tmux name contains "MASTER"
// are hidden by default; the ☰ MENU toggle reveals them.
const SHOW_MASTER_KEY = 'tmux-show-master';
let allWindows = [];
let pendingWindowSelection = '';
let showMaster = localStorage.getItem(SHOW_MASTER_KEY) === 'true';

// Command history
const HISTORY_KEY = 'tmux-command-history';
const HISTORY_MAX = 100;
let commandHistory = JSON.parse(localStorage.getItem(HISTORY_KEY) || '[]');
let historyIndex = -1;
let currentInput = '';

function addToHistory(command) {
    if (!command.trim()) return;
    // Don't add duplicates of the most recent command
    if (commandHistory.length > 0 && commandHistory[commandHistory.length - 1] === command) return;
    commandHistory.push(command);
    // Rotate out old commands
    if (commandHistory.length > HISTORY_MAX) {
        commandHistory = commandHistory.slice(-HISTORY_MAX);
    }
    localStorage.setItem(HISTORY_KEY, JSON.stringify(commandHistory));
}

function resetHistoryNavigation() {
    historyIndex = -1;
    currentInput = '';
}

/* ── History browser ───────────────────────────────────────────────
   ↑/↓ recall only exists on a hardware keyboard; the iOS keyboard has
   no arrow keys, which left history unreachable on the phone — where
   this UI is mostly used. The list is that missing affordance, not a
   replacement: the key bindings are untouched. */

let historySelectedIndex = 0;
let historyClearArmed = false;

// Newest first — the entry you want is nearly always the last one sent.
function historyEntries() {
    return commandHistory.slice().reverse();
}

function renderHistoryList() {
    const entries = historyEntries();

    if (entries.length === 0) {
        historyList.innerHTML = '<div class="history-empty">No commands yet</div>';
        return;
    }

    historyList.innerHTML = entries.map((cmd, i) => `
        <div class="window-item history-item ${i === historySelectedIndex ? 'selected' : ''}" data-index="${i}">
            <span class="history-num">${i + 1}</span>
            <span class="history-cmd"></span>
        </div>
    `).join('');

    // Commands are arbitrary user text — set as text, never as markup.
    historyList.querySelectorAll('.history-item').forEach((item, i) => {
        item.querySelector('.history-cmd').textContent = entries[i];
        item.addEventListener('click', () => useHistoryEntry(i));
    });

    const selected = historyList.querySelector('.selected');
    if (selected) selected.scrollIntoView({ block: 'nearest' });
}

// Loads into the input rather than sending. Recalling a command and
// running it are different decisions, and one tap must not make both.
function useHistoryEntry(index) {
    const command = historyEntries()[index];
    if (command === undefined) return;
    closeHistoryModal();
    commandInput.value = command;
    resetHistoryNavigation();
    focusCommandInput(true);
    commandInput.setSelectionRange(command.length, command.length);
}

function openHistoryModal() {
    historySelectedIndex = 0;
    disarmHistoryClear();
    renderHistoryList();
    historyModal.classList.add('show');
}

function closeHistoryModal() {
    historyModal.classList.remove('show');
    disarmHistoryClear();
    focusCommandInput();
}

function moveHistorySelection(delta) {
    const count = historyEntries().length;
    if (count === 0) return;
    historySelectedIndex = Math.min(count - 1, Math.max(0, historySelectedIndex + delta));
    renderHistoryList();
}

function disarmHistoryClear() {
    historyClearArmed = false;
    const btn = document.getElementById('historyClear');
    if (btn) btn.textContent = 'CLEAR';
}

// Two taps: wiping history is unrecoverable and the button sits next to
// CLOSE, which is the one people actually mean.
function clearHistoryEntries(btn) {
    if (!historyClearArmed) {
        historyClearArmed = true;
        btn.textContent = 'SURE?';
        setTimeout(disarmHistoryClear, 4000);
        return;
    }
    commandHistory = [];
    localStorage.removeItem(HISTORY_KEY);
    resetHistoryNavigation();
    historySelectedIndex = 0;
    disarmHistoryClear();
    renderHistoryList();
    showStatus('HISTORY CLEARED', 'success');
}

function showStatus(message, type = 'success') {
    status.textContent = message;
    status.className = `status show ${type}`;
    setTimeout(() => {
        status.classList.remove('show');
    }, 2000);
}

function hasSelectionInOutput() {
    const selection = window.getSelection();
    if (!selection || selection.isCollapsed) return false;
    // Check if selection is within the output element
    const range = selection.getRangeAt(0);
    return outputContent.contains(range.commonAncestorContainer);
}

/* ── Agent working indicator ───────────────────────────────────────
   Claude Code prints one status line while it is running:

       ✻ Misting… (1m 32s · ↓ 5.1k tokens · thought for 4s)

   and replaces it with a past-tense summary the moment it stops:

       ✻ Worked for 3m 16s

   Codex uses a different live line:

       • Working (4m 21s • esc to interrupt) · 1 background terminal running

   A live timer is the discriminator in both formats. Read from the
   pane we already poll, so no extra request and no second source of
   truth. */
const CLAUDE_WORKING_LINE = /(?:^|\s)([A-Za-z][A-Za-z ]{0,20})…\s*\(([^)]*\b\d+s\b[^)]*)\)/;
const CODEX_WORKING_LINE = /^\s*(?:•\s*)?Working\s+\(([^)]*\b\d+s\b[^)]*)\)(?:\s*·\s*(.*))?\s*$/;

/* AGY (the Antigravity CLI) has no live timer. Its status footer is
   redrawn in place, so the hint it shows right now is the truth:

       ⣟  Generating...
       esc to cancel                      Gemini 3.8 Flash · high · 1 task(s) · /tasks

   "esc to cancel" means a turn is running; "? for shortcuts" means idle
   even if a spinner line lingers above. Claude Code prints the same
   shortcuts hint, so the "model · effort" tail is what makes it AGY.
   Background tasks in the footer are not the agent working. */
const AGY_FOOTER = /^\s*(\? for shortcuts|esc to cancel)\s{2,}\S/;
const AGY_SPINNER = /^\s*[\u2800-\u28FF]\s+(\S.*?)\.\.\.\s*$/;

/* EUNICE prints "✻ Thinking…" once when a turn starts and never redraws
   it; its composer footer comes back only when the turn is done. So the
   rule is: a Thinking line with no footer below it, on a screen that
   something else already identifies as EUNICE. */
const EUNICE_THINKING = /^\s*[✻✶✺✹✷]\s*Thinking…\s*$/;
const EUNICE_TOOL = /^\s*→ [A-Za-z_][\w-]*\s*$/;
const HERMES_SPINNER = /⌐■-■\s+([A-Za-z_][\w -]*?)\.\.\./;
const HERMES_ELAPSED = /⏱\s*(\d+s)/;
const HERMES_TOOL = /calling tool:\s*([A-Za-z_][\w-]*)/;

function isEuniceFooter(line) {
    return line.includes('↵ send') && line.includes('esc clear');
}

function isEuniceMarker(line) {
    const t = line.trim();
    return isEuniceFooter(line)
        || line.includes('/quit or Ctrl+D to exit')
        || (t.startsWith('─') && t.endsWith(' eunice'))
        || EUNICE_TOOL.test(line);
}

function parseAgyWorking(tail) {
    for (let i = tail.length - 1; i >= 0; i--) {
        const footer = tail[i].match(AGY_FOOTER);
        if (!footer) continue;
        if (footer[1] !== 'esc to cancel') return null;
        for (let j = tail.length - 1; j >= 0; j--) {
            const spin = tail[j].match(AGY_SPINNER);
            if (spin) return { verb: spin[1].trim(), meta: '' };
        }
        return { verb: 'Working', meta: '' };
    }
    return null;
}

function parseEuniceWorking(lines, tail) {
    if (!lines.some(isEuniceMarker)) return null;
    let thinking = -1;
    for (let i = tail.length - 1; i >= 0; i--) {
        if (EUNICE_THINKING.test(tail[i])) { thinking = i; break; }
    }
    if (thinking < 0) return null;
    if (tail.slice(thinking + 1).some(isEuniceFooter)) return null;
    return { verb: 'Thinking', meta: '' };
}

function isHermesStatus(line) {
    const rest = line.trimStart().match(/^☤\s+(.+)$/)?.[1].trimStart();
    return !!rest && !rest.startsWith('❯');
}

function parseHermesWorking(tail) {
    let busy = -1;
    for (let i = tail.length - 1; i >= 0; i--) {
        if (tail[i].includes('msg=interrupt') && tail[i].includes('/queue') && tail[i].includes('Ctrl+C cancel')) { busy = i; break; }
    }
    if (busy < 0 || tail.slice(busy + 1).some(isHermesStatus)) return null;
    let verb = '', meta = '';
    for (let i = busy; i >= 0; i--) {
        const spin = tail[i].match(HERMES_SPINNER);
        const tool = tail[i].match(HERMES_TOOL);
        const elapsed = tail[i].match(HERMES_ELAPSED);
        if (!verb && spin) verb = spin[1].trim().replace(/^./, c => c.toUpperCase());
        else if (!verb && tool) verb = `Tool: ${tool[1]}`;
        if (!meta && elapsed) meta = elapsed[1];
        if (verb && meta) break;
    }
    return { verb: verb || 'Working', meta };
}

function parseCodexWorking(line) {
    const match = line.match(CODEX_WORKING_LINE);
    if (!match) return null;
    const meta = [match[1].split('•')[0].trim()];
    if (match[2]) {
        meta.push(...match[2].split(' · ')
            .map(part => part.trim())
            .filter(part => part && !part.startsWith('/')));
    }
    return { verb: 'Working', meta: meta.join(' · ') };
}

function parseWorking(content) {
    if (!content) return null;
    // Blank rows do not count: tmux prints every row of the pane, so a
    // status line near the top of a tall window sits above dozens of
    // empty ones.
    const lines = content.split('\n').filter(line => line.trim() !== '');
    // Only the tail: the same line from an earlier turn is still in
    // scrollback, and matching it would pin the bar on permanently.
    const tail = lines.slice(Math.max(0, lines.length - 30));
    const agy = parseAgyWorking(tail);
    if (agy) return agy;
    const eunice = parseEuniceWorking(lines, tail);
    if (eunice) return eunice;
    const hermes = parseHermesWorking(tail);
    if (hermes) return hermes;
    for (let i = tail.length - 1; i >= 0; i--) {
        const codex = parseCodexWorking(tail[i]);
        if (codex) return codex;
        const match = tail[i].match(CLAUDE_WORKING_LINE);
        if (match) return { verb: match[1].trim(), meta: match[2].trim() };
    }
    return null;
}

function updateWorking(content) {
    const working = parseWorking(content);
    if (!working) {
        workingBar.hidden = true;
        return null;
    }
    setText(workingVerb, working.verb);
    setText(workingMeta, working.meta);
    workingBar.hidden = false;
    return working;
}

const AGENT_BADGES = { claude: 'CLAUDE', codex: 'CODEX', agy: 'AGY', eunice: 'EUNICE', hermes: 'HERMES' };
function updateAgentIndicator(agent) {
    const label = AGENT_BADGES[agent];
    agentIndicator.hidden = !label;
    if (label) {
        setText(agentIndicator, `${label} ▾`);
        agentIndicator.title = `Change model and effort for this ${label} session`;
        agentIndicator.setAttribute('aria-label', agentIndicator.title);
    }
}

function setText(element, text) {
    if (element.textContent !== text) element.textContent = text;
}

const capturePoller = new AdaptivePoller(async (signal, current) => {
    const target = sessionSelect.value;
    if (!target || !current()) return false;
    const response = await targetFetch('/api/capture', {
        method: 'POST', signal,
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ target, history_lines: historyLines }),
    });
    if (!response.ok) throw new Error(`Capture failed: ${response.status}`);
    const data = await response.json();
    if (!current() || sessionSelect.value !== target) return false;
    if (data.window_closed) {
        updateWorking('');
        updateAgentIndicator(null);
        loadWindows();
        return false;
    }
    updateAgentIndicator(data.agent || null);
    updateQuestionQueue(data.question_queue || null);
    updatePicker(target, data.picker || null);
    const working = updateWorking(data.content);
    const displayContent = data.styled_content ?? data.content ?? '';
    const changed = lastObservedPane !== displayContent;
    lastObservedPane = displayContent;
    hasMoreHistory = !!data.has_more && historyLines < 1000;
    historyLoading = false;
    updateHistoryButton();
    if (!hasSelectionInOutput()) {
        terminalRenderer.render(displayContent, isFirstCapture);
        outputContent.dataset.host = windowInfo(target).host;
        isFirstCapture = false;
    }
    return changed || !!working || !!data.picker;
});

function captureOutput() {
    return capturePoller.request();
}

function updateHistoryButton() {
    loadOlderBtn.hidden = !hasMoreHistory || outputContent.scrollTop > 80;
    loadOlderBtn.disabled = historyLoading;
    setText(loadOlderBtn, historyLoading ? 'Loading older output…' : 'Load older output');
}

function loadOlderOutput() {
    if (!hasMoreHistory || historyLoading || hasSelectionInOutput()) return;
    historyLines = Math.min(1000, historyLines + 200);
    historyLoading = true;
    updateHistoryButton();
    captureOutput();
}
loadOlderBtn.addEventListener('click', loadOlderOutput);
outputContent.addEventListener('scroll', () => {
    terminalRenderer.following = outputContent.scrollHeight - outputContent.scrollTop <= outputContent.clientHeight + 50;
    updateHistoryButton();
    if (outputContent.scrollTop < 40 && outputContent.scrollHeight > outputContent.clientHeight) loadOlderOutput();
}, { passive: true });

/* ── Claude picker ──────────────────────────────────────────────────
   Renders a live selection prompt as a keyboard-driven control.

   State is per tmux target, so switching windows never commits, cancels,
   or loses your place. Come back and your highlight is still there, as
   long as the prompt itself has not changed underneath you. */
const pickerCard = document.getElementById('pickerCard');
const pickerBar = document.getElementById('pickerBar');
const questionsBtn = document.getElementById('questionsBtn');
let questionQueue = null;
let openingQuestions = false;
const answerModeTargets = new Set();
const waitingPill = document.getElementById('waitingPill');
const busyPill = document.getElementById('busyPill');

const pickerByTarget = new Map();   // target -> {cursor, dirty, fingerprint, collapsed}
const focusedPrompts = new Set();   // `${target}|${fingerprint}` already focused once
let pickerData = null;              // the picker for the viewed target
let pickerRenderedFp = null;        // fingerprint the DOM was built for
let cancelArmed = false;
let textMode = false;               // the card is asking for typed text
// Guards against a second trigger landing before the first request
// returns. On a phone one gesture can raise two events -- the keyboard's
// return plus a tap on Send -- and without these the same reply is sent
// twice, because the field is only cleared after the await.
let sendingText = false;
let committing = false;
let armedIndex = -1;                // touch: row awaiting a confirming second tap
let pendingTargets = [];
let busyTargets = new Map();        // target -> short elapsed, e.g. "10m"

function updateQuestionQueue(queue) {
    questionQueue = queue;
    questionsBtn.hidden = !queue;
    questionsBtn.classList.toggle('show', !!queue);
    if (queue) setText(questionsBtn, `Answer questions (${queue.count})`);
}

async function openQuestions() {
    if (!questionQueue || openingQuestions) return;
    const target = sessionSelect.value;
    openingQuestions = true;
    questionsBtn.disabled = true;
    try {
        const response = await targetFetch('/api/picker/open', {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ target, fingerprint: questionQueue.fingerprint }),
        });
        const data = await response.json();
        if (!response.ok) throw new Error(data.error || 'Could not open questions');
        if (sessionSelect.value !== target) return;
        answerModeTargets.add(target);
        pickerStateFor(target).collapsed = false;
        updateQuestionQueue(null);
        updatePicker(target, data.picker);
        pickerCard.focus({ preventScroll: true });
    } catch (error) {
        showStatus(error.message, 'error');
    } finally {
        openingQuestions = false;
        questionsBtn.disabled = false;
        captureOutput();
    }
}
questionsBtn.addEventListener('click', openQuestions);

async function closeQuestions() {
    const target = sessionSelect.value;
    try {
        const response = await targetFetch('/api/picker/close', {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ target }),
        });
        const data = await response.json();
        if (!response.ok) throw new Error(data.error || 'Could not return to typing');
        answerModeTargets.delete(target);
        if (sessionSelect.value === target) {
            pickerByTarget.delete(target);
            clearPicker();
            focusCommandInput(true);
        }
    } catch (error) {
        showStatus(error.message, 'error');
    }
    captureOutput();
}

function pickerStateFor(target) {
    if (!pickerByTarget.has(target)) {
        pickerByTarget.set(target, {
            cursor: 0, dirty: false, fingerprint: null, collapsed: false,
            // Text mode is per-target too: switching away from a card that
            // is asking for text must not silently drop the request, or
            // the draft already typed into it.
            textMode: false, textPrompt: '', draft: '', codexAsync: false, staged: false, submitting: false,
        });
    }
    return pickerByTarget.get(target);
}

function clearPicker() {
    exitTextMode();
    pickerData = null;
    pickerRenderedFp = null;
    armedIndex = -1;
    cancelArmed = false;
    pickerCard.classList.remove('show', 'stale');
    pickerBar.classList.remove('show');
}

// Called from captureOutput with whatever the server parsed.
function updatePicker(target, data) {
    // A card asking for text must outlive the prompt that produced it —
    // "Chat about this" removes that prompt from the pane entirely, so
    // polling would otherwise dismiss the request a second later.
    const existing = pickerByTarget.get(target);
    if (existing?.textMode) {
        // Claude can leave its picker while asking for text. Codex keeps the
        // question visible: reconcile its fingerprint so polling can advance.
        if (!existing.codexAsync || (existing.submitting && (!data || data.fingerprint === existing.fingerprint))) {
            if (!textMode) showTextMode(target);
            return;
        }
        if (data?.fingerprint === existing.fingerprint) {
            pickerData = data;
            if (data.answer_draft && (!existing.draft || existing.staged)) {
                existing.draft = data.answer_draft;
                existing.staged = true;
                showTextMode(target, false);
            } else if (!textMode) showTextMode(target, false);
            return;
        }
        exitTextMode();
        existing.textMode = false;
    }

    if (!data) {
        if (pickerData) pickerByTarget.delete(target);
        clearPicker();
        return;
    }

    let st = pickerStateFor(target);
    const changed = st.fingerprint !== data.fingerprint;
    if (changed) {
        // A fresh object also keeps a late submission response from clearing
        // the next question's draft after polling has already advanced.
        pickerByTarget.delete(target);
        st = pickerStateFor(target);
        st.fingerprint = data.fingerprint;
        st.codexAsync = !!data.codex_async;
        st.textMode = false;
        st.draft = '';
        st.staged = false;
        st.cursor = data.cursor;
        st.dirty = false;
        st.collapsed = !!data.codex_async && !answerModeTargets.has(target);
    }

    // The preview layout renders only the focused option's preview, so
    // the client cannot move the highlight locally — it steers the real
    // cursor and follows. Mirroring is therefore mandatory here.
    if (data.layout === 'preview') st.dirty = false;

    // Mirror the terminal until the user takes over.
    if (!st.dirty) st.cursor = data.cursor;
    if (st.cursor >= data.options.length) st.cursor = data.cursor;

    pickerData = data;
    renderPicker(target);

    // Focus exactly once per prompt. Re-focusing on every poll would
    // fight the user the moment they clicked back into the textarea.
    const key = `${target}|${data.fingerprint}`;
    if (!focusedPrompts.has(key) && !st.collapsed && !textMode) {
        focusedPrompts.add(key);
        pickerCard.focus({ preventScroll: true });
    }
}

function renderPicker(target) {
    const data = pickerData;
    const st = pickerStateFor(target);
    if (!data) return;

    if (st.collapsed) {
        pickerCard.classList.remove('show');
        pickerBar.classList.add('show');
        pickerBar.textContent = data.codex_async ? 'Answer questions' : `▸ Question pending — ${data.header || 'choose'} · ${data.options.length} options`;
        return;
    }
    pickerBar.classList.remove('show');
    pickerCard.classList.add('show');

    if (pickerRenderedFp !== data.fingerprint) {
        buildPickerDom(data);
        pickerRenderedFp = data.fingerprint;
    }
    paintPicker(data, st);
    if (data.codex_async && (data.text_only || data.answer_draft)) enterTextMode(target, 'Your answer', false);
}

function buildPickerDom(data) {
    pickerCard.innerHTML = '';
    pickerCard.classList.toggle('codex-question', !!data.codex_async);

    const head = document.createElement('div');
    head.className = 'picker-head';
    const chip = document.createElement('span');
    chip.className = 'picker-chip';
    chip.textContent = data.header || 'Choose';
    const hint = document.createElement('span');
    hint.className = 'picker-hint';
    hint.textContent = data.codex_async ? 'Question answering mode' : '↑↓ move · ⏎ select · esc hide';
    const collapse = document.createElement('button');
    collapse.className = 'picker-collapse';
    collapse.textContent = '⌄';
    collapse.title = 'Hide (Esc)';
    collapse.addEventListener('click', () => collapsePicker());
    head.append(chip, hint, collapse);

    const q = document.createElement('div');
    q.className = 'picker-q';
    q.textContent = data.question;

    const body = document.createElement('div');
    body.className = 'picker-body' + (data.preview ? ' has-preview' : '');
    const opts = document.createElement('div');
    opts.className = 'picker-opts';
    body.appendChild(opts);

    data.options.forEach((o, i) => {
        if (o.is_meta && (i === 0 || !data.options[i - 1].is_meta)) {
            const rule = document.createElement('div');
            rule.className = 'picker-meta-rule';
            opts.appendChild(rule);
        }
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'picker-opt' + (o.is_meta ? ' meta' : '');
        b.setAttribute('role', 'option');
        b.dataset.idx = String(i);

        const bar = document.createElement('span');
        bar.className = 'bar';
        const num = document.createElement('span');
        num.className = 'num';
        num.textContent = o.number != null ? o.number + '.' : '·';
        const label = document.createElement('span');
        label.className = 'label';
        label.append(document.createTextNode(o.label));
        if (o.description) {
            const d = document.createElement('span');
            d.className = 'picker-desc';
            const inner = document.createElement('span');
            inner.textContent = o.description;
            d.appendChild(inner);
            label.appendChild(d);
        }
        b.append(bar, num, label);
        b.addEventListener('click', () => onOptionClick(i));
        opts.appendChild(b);
    });

    if (data.preview) {
        const pv = document.createElement('pre');
        pv.className = 'picker-preview';
        pv.id = 'pickerPreview';
        pv.textContent = data.preview;
        body.appendChild(pv);
    }

    const text = document.createElement('div');
    text.className = 'picker-text';
    const tlabel = document.createElement('div');
    tlabel.className = 'picker-text-label';
    tlabel.id = 'pickerTextLabel';
    const tinput = document.createElement('textarea');
    tinput.className = 'picker-text-input';
    tinput.id = 'pickerTextInput';
    tinput.setAttribute('aria-label', 'Your answer');
    tinput.rows = 2;
    tinput.placeholder = 'Type your reply…';
    tinput.addEventListener('keydown', (e) => {
        e.stopPropagation();
        if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); sendPickerText(); }
        // An escape hatch out of the escape hatch: abandon the reply and
        // hand back to the main command input.
        else if (e.key === 'Escape') { e.preventDefault(); abandonTextMode(); }
    });
    const trow = document.createElement('div');
    trow.className = 'picker-text-row';
    const tsend = document.createElement('button');
    tsend.className = 'picker-send';
    tsend.id = 'pickerTextSend';
    tsend.textContent = '⏎ Send';
    tsend.addEventListener('click', () => sendPickerText());
    const thint = document.createElement('span');
    thint.className = 'picker-text-hint';
    thint.textContent = 'shift+⏎ for a newline';
    trow.append(tsend, thint);
    text.append(tlabel, tinput);
    if (!data.codex_async) text.append(trow);

    const foot = document.createElement('div');
    foot.className = 'picker-foot';
    const send = document.createElement('button');
    send.className = 'picker-send';
    send.id = 'pickerSend';
    send.addEventListener('click', () => commitPicker());
    const sync = document.createElement('span');
    sync.className = 'picker-sync';
    sync.id = 'pickerSync';
    const cancel = document.createElement('button');
    cancel.className = 'picker-cancel';
    cancel.id = 'pickerCancel';
    cancel.textContent = data.codex_async ? 'Back to typing' : 'Cancel ⎋';
    cancel.addEventListener('click', () => cancelPicker());
    foot.append(send, sync, cancel);
    if (data.codex_async) foot.prepend(tsend);

    pickerCard.append(head, q, body, text, foot);
}

/* Text mode. Entered when the chosen option leaves Claude waiting to be
   typed at rather than answering anything:
     - "Type something." keeps the prompt up with an inline field focused
       (the server reports outcome "awaiting_text"),
     - "Chat about this" closes the prompt and Claude asks a follow-up.
   Both come down to "send literal text, then Enter", so both are handled
   the same way here. */
function enterTextMode(target, prompt, focus = true) {
    const st = pickerStateFor(target);
    st.textMode = true;
    st.textPrompt = prompt;
    st.codexAsync = !!pickerData?.codex_async;
    st.draft = pickerData?.answer_draft || st.draft || '';
    st.staged = !!pickerData?.answer_draft;
    showTextMode(target, focus);
}

// Paint text mode from stored state. Also the path back after a window
// switch, which is why the draft is restored rather than cleared.
function showTextMode(target, focus = true) {
    const st = pickerStateFor(target);
    textMode = true;
    if (pickerRenderedFp === null) {
        buildPickerDom(pickerData || TEXT_ONLY_SHELL);
        pickerRenderedFp = 'text-only';
    }
    pickerCard.classList.add('textmode', 'show');
    pickerBar.classList.remove('show');
    const label = document.getElementById('pickerTextLabel');
    const input = document.getElementById('pickerTextInput');
    if (label) label.textContent = st.staged ? 'Answer entered in Codex — submit to finish' : st.textPrompt;
    if (input) {
        input.value = st.draft || '';
        input.readOnly = st.staged || st.submitting;
        input.oninput = () => { st.draft = input.value; };
        if (focus && !input.readOnly) input.focus();
    }
    const send = document.getElementById('pickerTextSend');
    if (send) {
        send.disabled = st.submitting;
        send.textContent = st.submitting ? 'Sending…' : st.staged ? '⏎ Submit answer' : '⏎ Send';
    }
}

function exitTextMode() {
    textMode = false;
    pickerCard.classList.remove('textmode');
}

function abandonTextMode() {
    if (pickerData?.codex_async) { closeQuestions(); return; }
    pickerByTarget.delete(sessionSelect.value);
    clearPicker();
    focusCommandInput();
}

// Minimal picker shape so the card can still be built after the prompt
// it came from has left the pane.
const TEXT_ONLY_SHELL = {
    fingerprint: 'text-only', header: 'Reply', question: '',
    cursor: 0, layout: 'list', options: [],
};

async function sendPickerText() {
    if (sendingText) return;
    const input = document.getElementById('pickerTextInput');
    if (!input) return;
    const target = sessionSelect.value;
    const st = pickerStateFor(target);
    const text = input.value.trim();
    if (!text) { input.focus(); return; }
    const fingerprint = st.codexAsync ? st.fingerprint : undefined;
    st.draft = input.value;
    st.submitting = true;
    sendingText = true;
    showTextMode(target, false);
    let result;
    try {
        const response = await targetFetch('/api/picker/text', {
            method: 'POST', headers: { 'Content-Type': 'application/json' },
            // An already-entered native draft needs only a submit key. Never
            // type it again when retrying or recovering after a page reload.
            body: JSON.stringify({ target, text: st.staged ? '' : text, fingerprint }),
        });
        result = await response.json();
        if (!response.ok || !result.success) throw new Error(result.error || 'Reply was not accepted');
        if (fingerprint && !['changed', 'committed'].includes(result.outcome)) {
            if (result.picker?.answer_draft) st.staged = true;
            showStatus('Not submitted yet — your answer is saved. Try Submit answer again.', 'error');
            return;
        }
        showStatus('ANSWER SENT', 'success');
    } catch (err) {
        showStatus(err.message || 'SEND FAILED — answer kept', 'error');
        return;
    } finally {
        sendingText = false;
        st.submitting = false;
        if (sessionSelect.value === target && pickerByTarget.get(target) === st) showTextMode(target, false);
        captureOutput();
    }
    // A response for another window/question must not clear the current card.
    if (pickerByTarget.get(target) !== st) return;
    pickerByTarget.delete(target);
    if (sessionSelect.value !== target) return;
    clearPicker();
    if (result.picker) updatePicker(target, result.picker);
    captureOutput();
}

function paintPicker(data, st) {
    const rows = pickerCard.querySelectorAll('.picker-opt');
    rows.forEach((r, i) => {
        r.setAttribute('aria-selected', i === st.cursor ? 'true' : 'false');
        r.classList.toggle('armed', i === armedIndex && i !== st.cursor);
    });

    const send = document.getElementById('pickerSend');
    if (send) {
        const o = data.options[st.cursor];
        const n = o && o.number != null ? o.number : st.cursor + 1;
        send.textContent = `⏎ Select ${n}`;
    }

    const pv = document.getElementById('pickerPreview');
    if (pv && data.preview) pv.textContent = data.preview;

    const sync = document.getElementById('pickerSync');
    if (sync) {
        sync.classList.remove('warn');
        if (document.activeElement === commandInput) {
            sync.textContent = '↑↓ inactive — click card';
        } else if (data.layout === 'preview') {
            sync.textContent = 'steering terminal';
        } else if (st.dirty) {
            sync.innerHTML = '<b>local</b>';
        } else {
            sync.textContent = 'mirroring';
        }
    }
}

function pickerWarn(msg) {
    const sync = document.getElementById('pickerSync');
    if (sync) {
        sync.textContent = msg;
        sync.classList.add('warn');
    }
}

function movePicker(delta) {
    if (!pickerData) return;
    const target = sessionSelect.value;
    const st = pickerStateFor(target);
    armedIndex = -1;

    if (pickerData.layout === 'preview') {
        // Only the focused option's preview exists, so move the real
        // cursor and let the next capture bring the new pane back.
        stepPicker(delta);
        return;
    }

    const n = pickerData.options.length;
    st.cursor = (st.cursor + delta + n) % n;
    st.dirty = true;
    paintPicker(pickerData, st);
}

function jumpPicker(number) {
    if (!pickerData) return;
    const idx = pickerData.options.findIndex(o => o.number === number);
    if (idx < 0) return;
    const st = pickerStateFor(sessionSelect.value);
    if (pickerData.layout === 'preview') {
        stepPicker(idx - st.cursor, Math.abs(idx - st.cursor));
        return;
    }
    st.cursor = idx;
    st.dirty = true;
    armedIndex = -1;
    paintPicker(pickerData, st);
}

async function stepPicker(delta, repeat) {
    if (!pickerData || delta === 0) return;
    const target = sessionSelect.value;
    const fingerprint = pickerData.fingerprint;
    const times = repeat || 1;
    try {
        for (let i = 0; i < times; i++) {
            const res = await targetFetch('/api/picker/step', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ target, delta: delta > 0 ? 1 : -1, fingerprint }),
            });
            if (!res.ok) { pickerWarn('question changed'); break; }
        }
    } catch (err) {
        pickerWarn('connection error');
    }
    // Don't wait a full second to see the result of your own keypress.
    captureOutput();
    setTimeout(captureOutput, 250);
}

function onOptionClick(i) {
    if (!pickerData) return;
    pickerCard.focus({ preventScroll: true });
    const st = pickerStateFor(sessionSelect.value);

    // Touch: first tap reads, second tap commits. Prevents an accidental
    // answer from a stray thumb on a phone.
    if (st.cursor === i && armedIndex === i) { commitPicker(); return; }

    if (pickerData.layout === 'preview') {
        armedIndex = i;
        stepPicker(i - st.cursor, Math.abs(i - st.cursor));
        return;
    }
    st.cursor = i;
    st.dirty = true;
    armedIndex = i;
    paintPicker(pickerData, st);
}

// The rows Claude Code adds around the tool's own options. Selecting
// one dismisses the prompt and leaves Claude waiting at its ordinary
// input, so the card follows up by asking for the text.
function isInputRow(opt) {
    return opt.is_meta || /^type something\.?$/i.test(opt.label.trim());
}

async function commitPicker() {
    if (!pickerData || committing) return;
    committing = true;
    try { await commitPickerInner(); } finally { committing = false; }
}

async function commitPickerInner() {
    const target = sessionSelect.value;
    const st = pickerStateFor(target);
    const index = st.cursor;
    const fingerprint = pickerData.fingerprint;

    try {
        const res = await targetFetch('/api/picker/select', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ target, index, fingerprint }),
        });
        if (res.status === 409) {
            // The prompt moved between render and commit. Nothing was
            // sent; re-render against what is actually on screen.
            pickerCard.classList.add('stale');
            pickerWarn('question changed — re-choose');
            pickerByTarget.delete(target);
            setTimeout(() => pickerCard.classList.remove('stale'), 2000);
            captureOutput();
            return;
        }
        if (!res.ok) { pickerWarn('send failed'); return; }
        const body = await res.json().catch(() => ({}));

        // "Type something." and "Chat about this" don't answer anything
        // on their own — the server dismisses the prompt and Claude
        // waits at its ordinary input. Ask for the text here rather
        // than closing the card and leaving the user to discover that.
        if (sessionSelect.value !== target || pickerData?.fingerprint !== fingerprint) { captureOutput(); return; }
        const chosen = pickerData.options[index];
        if (chosen && isInputRow(chosen)) {
            enterTextMode(target, 'What do you want to say?');
            return;
        }
        if (body.outcome === 'awaiting_text') {
            enterTextMode(target, pickerData.codex_async ? 'Your answer' : 'The agent is waiting for your text');
            return;
        }
        if (body.outcome === 'changed') {
            // The keystroke produced a different prompt — the next
            // question of a set, a toggled checkbox, or the review
            // screen. Re-render it; nothing is finished yet.
            pickerByTarget.delete(target);
            if (body.picker) updatePicker(target, body.picker);
            else captureOutput();
            return;
        }
        if (body.outcome === 'pending') {
            // Keystroke sent, effect not yet observed — a busy session
            // can lag its redraw by seconds. Keep the card; polling
            // clears it if the answer landed, and if the keystroke was
            // dropped the card is still here to try again.
            pickerWarn('sent — waiting for the agent');
            setTimeout(captureOutput, 1500);
            return;
        }

        pickerByTarget.delete(target);
        showStatus('ANSWER SENT', 'success');
        captureOutput();
        setTimeout(captureOutput, 300);
    } catch (err) {
        pickerWarn('connection error');
    }
}

function collapsePicker() {
    if (!pickerData) return;
    if (pickerData.codex_async) { closeQuestions(); return; }
    pickerStateFor(sessionSelect.value).collapsed = true;
    renderPicker(sessionSelect.value);
    focusCommandInput();
}

function expandPicker() {
    if (!pickerData) return;
    if (pickerData.codex_async) answerModeTargets.add(sessionSelect.value);
    pickerStateFor(sessionSelect.value).collapsed = false;
    renderPicker(sessionSelect.value);
    pickerCard.focus({ preventScroll: true });
}

// Esc collapses the card; dismissing Claude's question is a separate,
// confirmed action. The two differ too much in consequence to share a key.
function cancelPicker() {
    if (pickerData?.codex_async) { closeQuestions(); return; }
    const btn = document.getElementById('pickerCancel');
    if (!cancelArmed) {
        cancelArmed = true;
        if (btn) { btn.textContent = 'Sure? ⎋'; btn.classList.add('armed'); }
        pickerWarn('sends Esc to Claude — click again');
        setTimeout(() => {
            cancelArmed = false;
            if (btn) { btn.textContent = 'Cancel ⎋'; btn.classList.remove('armed'); }
        }, 4000);
        return;
    }
    cancelArmed = false;
    if (btn) { btn.textContent = 'Cancel ⎋'; btn.classList.remove('armed'); }
    pickerByTarget.delete(sessionSelect.value);
    sendKeyToTmux('Escape');
    captureOutput();
}

pickerBar.addEventListener('click', expandPicker);

pickerCard.addEventListener('keydown', (e) => {
    if (!pickerData || textMode) return;
    if (e.key === 'ArrowDown' || e.key === 'j') { e.preventDefault(); e.stopPropagation(); movePicker(1); }
    else if (e.key === 'ArrowUp' || e.key === 'k') { e.preventDefault(); e.stopPropagation(); movePicker(-1); }
    else if (e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); commitPicker(); }
    else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); collapsePicker(); }
    else if (/^[1-9]$/.test(e.key)) { e.preventDefault(); e.stopPropagation(); jumpPicker(Number(e.key)); }
});

// Repaint the sync line when focus moves between the card and the input.
commandInput.addEventListener('focus', () => {
    if (pickerData) paintPicker(pickerData, pickerStateFor(sessionSelect.value));
});
pickerCard.addEventListener('focus', () => {
    if (pickerData) paintPicker(pickerData, pickerStateFor(sessionSelect.value));
});

/* The elapsed time Claude reports is "10m 40s · ↓ 25.6k tokens". In a
   dropdown row only the coarsest unit is worth the width — the point is
   "still going, a while now", not a stopwatch. */
function shortElapsed(meta) {
    if (!meta) return '';
    const m = meta.match(/(\d+)\s*h|\b(\d+)\s*m\b|\b(\d+)\s*s\b/);
    if (!m) return '';
    if (m[1]) return `${m[1]}h`;
    if (m[2]) return `${m[2]}m`;
    return `${m[3]}s`;
}

function mergeHostStatuses() {
    const rows = [...hostStatuses.entries()].flatMap(([host, rows]) => rows.map(row => {
        const win = allWindows.find(w => w.host === host && w.nativeTarget === row.target);
        return { ...row, target: win?.target || (host === defaultHost ? row.target : `${host}::${row.target}`) };
    }));
    pendingTargets = rows.filter(r => r.waiting).map(r => r.target);
    busyTargets = new Map(rows.filter(r => !r.waiting && r.verb).map(r => [r.target, shortElapsed(r.meta)]));
    paintWindowStatus();
}

const statusPoller = new HostPoller(async (host, signal, current) => {
    try {
        const res = await hostFetch('/api/window-status', { signal }, host);
        if (!res.ok) throw new Error(`Status failed: ${res.status}`);
        const rows = await res.json();
        if (!current()) return false;
        hostStatuses.set(host, rows);
        mergeHostStatuses();
        return rows.length > 0;
    } catch (error) {
        if (current()) { hostStatuses.delete(host); mergeHostStatuses(); }
        throw error;
    }
}, { active: 3000, idle: 15000 });

function refreshWindowStatus() {
    return statusPoller.request();
}

function paintWindowStatus() {
    const others = pendingTargets.filter(t => t !== sessionSelect.value);
    waitingPill.classList.toggle('show', others.length > 0);
    if (others.length > 0) {
        const html = `● ${others.length} <span class="waiting-word">waiting</span>`;
        if (waitingPill.innerHTML !== html) waitingPill.innerHTML = html;
        waitingPill.title = others.length === 1
            ? '1 window is waiting on a question'
            : `${others.length} windows are waiting on a question`;
    }

    // The viewed window already has the progress bar above the output,
    // so counting it here would say the same thing twice.
    const busyOthers = [...busyTargets.keys()].filter(t => t !== sessionSelect.value);
    busyPill.classList.toggle('show', busyOthers.length > 0);
    if (busyOthers.length > 0) {
        const html = `◆ ${busyOthers.length} <span class="waiting-word">working</span>`;
        if (busyPill.innerHTML !== html) busyPill.innerHTML = html;
        busyPill.title = busyOthers.length === 1
            ? '1 window is still working'
            : `${busyOthers.length} windows are still working`;
    }

    markPendingInDropdown();
}

/* Rebuild each label from its stored base name rather than trimming a
   suffix off what is already there: the busy marker is variable width
   ("◆ 10m" grows to "◆ 1h"), and suffix arithmetic on a label that has
   already been marked once eats the window name. */
function markPendingInDropdown() {
    for (const opt of sessionSelect.options) {
        const base = opt.dataset.baseLabel;
        if (base === undefined) continue;
        // Waiting wins: a window at a prompt needs an answer, and that
        // is the more urgent thing to say in one row of text.
        if (pendingTargets.includes(opt.value)) {
            setText(opt, `${base} ●`);
        } else if (busyTargets.has(opt.value)) {
            const elapsed = busyTargets.get(opt.value);
            setText(opt, elapsed ? `${base} ◆ ${elapsed}` : `${base} ◆`);
        } else {
            setText(opt, base);
        }
    }
}

// The target is often a MASTER window, which the filter keeps out of the
// dropdown — a plain `sessionSelect.value = target` then no-ops and the
// pill looks dead. Persist the target and blank the live selection first
// so renderWindowOptions()'s keepTarget resolves to it and the option
// actually exists before we select it.
async function jumpToWindow(target) {
    localStorage.setItem('tmux-selected-target', target);
    sessionSelect.value = '';
    await loadWindows();
    startCapture();
}

waitingPill.addEventListener('click', async () => {
    const others = pendingTargets.filter(t => t !== sessionSelect.value);
    if (others.length === 0) return;
    await jumpToWindow(others[0]);
});

busyPill.addEventListener('click', async () => {
    const others = [...busyTargets.keys()].filter(t => t !== sessionSelect.value);
    if (others.length === 0) return;
    await jumpToWindow(others[0]);
});

function startCapture() {
    const target = sessionSelect.value;
    if (target !== captureTarget) {
        capturePoller.stop();
        captureTarget = target;
        terminalRenderer.reset();
        isFirstCapture = true;
        lastObservedPane = null;
        historyLines = 200;
        hasMoreHistory = false;
        historyLoading = false;
        updateHistoryButton();
        updateQuestionQueue(null);
        clearPicker();
        updateWorking('');
        updateAgentIndicator(null);
    }
    if (!target || document.hidden) {
        capturePoller.stop();
        return;
    }
    if (!capturePoller.running) capturePoller.start();
    else captureOutput();
}

function isMasterWindow(win) {
    return win.name.includes('MASTER');
}

// The currently selected target is always kept, even when it is a MASTER
// window, so hiding never switches the terminal being viewed. If the filter
// would empty the list entirely, fall open rather than leave a dead dropdown.
function visibleWindows(keepTarget) {
    if (showMaster) return allWindows;
    const visible = allWindows.filter(
        win => !isMasterWindow(win) || win.target === keepTarget
    );
    return visible.length > 0 ? visible : allWindows;
}

function renderWindowOptions() {
    let savedTarget = localStorage.getItem('tmux-selected-target');
    const migrated = allWindows.find(w => w.host === defaultHost && w.nativeTarget === savedTarget);
    if (migrated) { savedTarget = migrated.target; localStorage.setItem('tmux-selected-target', savedTarget); }
    // Visibility must prefer the live DOM selection over localStorage:
    // the currently-viewed target may not be persisted yet (e.g. the
    // fresh-load default selection), so falling back to savedTarget
    // alone can let the actively-viewed window get hidden out from
    // under the user. Read sessionSelect.value BEFORE innerHTML is
    // wiped below, or it will already be empty.
    //
    // keepTarget (visibility) and savedTarget (selection restore) are
    // separate on purpose, but they must stay in lockstep: every writer
    // of the selection sets both synchronously. A writer that sets only
    // one would render a stray unselected MASTER window.
    const keepTarget = sessionSelect.value || savedTarget;
    const windows = visibleWindows(keepTarget);

    sessionSelect.innerHTML = '';

    if (windows.length === 0) {
        sessionSelect.innerHTML = '<option value="">No windows found</option>';
        if (!savedTarget) { startCapture(); return; }
    }

    let foundSaved = false;

    windows.forEach((win, index) => {
        const option = document.createElement('option');
        option.value = win.target;
        // Kept apart from textContent so markPendingInDropdown can
        // rebuild the label without ever parsing its own markers back.
        option.dataset.baseLabel = windowLabel(win) + (hostOffline.has(win.host) ? ' · OFFLINE' : '');
        option.dataset.windowName = win.name;
        option.textContent = option.dataset.baseLabel;
        // Select saved target if it exists, otherwise keep whatever is
        // currently being viewed selected (it may never have been
        // persisted — e.g. the fresh-load default), otherwise default
        // to first.
        if (savedTarget && win.target === savedTarget) {
            option.selected = true;
            foundSaved = true;
        } else if (!savedTarget && keepTarget && win.target === keepTarget) {
            option.selected = true;
        } else if (!savedTarget && !keepTarget && index === 0) {
            option.selected = true;
        }
        sessionSelect.appendChild(option);
    });

    // If saved target wasn't found, select first and clear invalid saved value
    if (savedTarget && !foundSaved) {
        const info = windowInfo(savedTarget);
        if (savedTarget === pendingWindowSelection || !hostWindows.has(info.host) || hostOffline.has(info.host)) {
            const option = document.createElement('option');
            option.value = savedTarget;
            option.textContent = `${hostLabel(info.host)} › ${info.session || ''} › ${hostOffline.has(info.host) ? 'OFFLINE' : 'Connecting…'}`;
            option.selected = true;
            sessionSelect.appendChild(option);
        } else {
            sessionSelect.selectedIndex = 0;
            localStorage.removeItem('tmux-selected-target');
        }
    }

    startCapture();
    markPendingInDropdown();
    showStatus(`${windows.length} WINDOWS`, 'success');
}

let windowsLoaded = false;
function mergeHostWindows() {
    allWindows = hostRegistry.flatMap(h => hostWindows.get(h.id) || []);
    renderWindowOptions();
    mergeHostStatuses();
}

const windowsPoller = new HostPoller(async (host, signal, current) => {
    try {
        const response = await hostFetch('/api/windows', { signal }, host);
        if (!response.ok) throw new Error(`Windows failed: ${response.status}`);
        const rows = await response.json();
        if (!Array.isArray(rows)) throw new Error('Invalid window list');
        if (!current()) return false;
        const windows = rows.map(w => normalizeWindow(host, w));
        const recovered = hostOffline.delete(host);
        if (recovered || !hostWindows.has(host) || JSON.stringify(windows) !== JSON.stringify(hostWindows.get(host))) {
            hostWindows.set(host, windows);
            mergeHostWindows();
        }
        windowsLoaded = true;
        return windows.length === 0;
    } catch (error) {
        if (current()) {
            hostOffline.add(host);
            mergeHostWindows();
        }
        throw error;
    }
}, { active: 3000, idle: 30000 });

async function loadWindows(host = windowInfo(sessionSelect.value).host) {
    await windowsPoller.request(host);
    // Callers may have changed the saved selection without changing the list.
    if (windowsLoaded && localStorage.getItem('tmux-selected-target') !== sessionSelect.value) {
        renderWindowOptions();
    }
}

async function sendCommand() {
    const command = commandInput.value.trim();
    const session = sessionSelect.value || '0';

    sendBtn.disabled = true;
    sendBtn.textContent = 'SENDING...';

    try {
        const response = await targetFetch('/api/send', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({ command, session }),
        });

        const data = await response.json();

        if (response.ok) {
            showStatus(command ? 'COMMAND SENT' : 'ENTER SENT', 'success');
            addToHistory(command);
            resetHistoryNavigation();
            commandInput.value = '';
            // Immediately capture to show result
            setTimeout(captureOutput, 100);
        } else {
            showStatus(data.error || 'TRANSMISSION FAILED', 'error');
        }
    } catch (err) {
        showStatus('CONNECTION ERROR', 'error');
    } finally {
        sendBtn.disabled = false;
        sendBtn.textContent = 'EXECUTE';
    }
}

// Prefix mode functions
function enterPrefixMode() {
    prefixMode = true;
    prefixIndicator.classList.add('active');
    if (prefixTimeout) clearTimeout(prefixTimeout);
    prefixTimeout = setTimeout(exitPrefixMode, 2000);
}

function exitPrefixMode() {
    prefixMode = false;
    prefixIndicator.classList.remove('active');
    if (prefixTimeout) {
        clearTimeout(prefixTimeout);
        prefixTimeout = null;
    }
}

// Window modal functions
function openWindowModal() {
    modalWindows = Array.from(sessionSelect.options).map(opt => ({
        target: opt.value,
        name: opt.textContent,
        current: opt.value === sessionSelect.value
    })).filter(w => w.target);

    if (modalWindows.length === 0) {
        showStatus('NO WINDOWS AVAILABLE', 'error');
        return;
    }

    modalSelectedIndex = modalWindows.findIndex(w => w.current);
    if (modalSelectedIndex === -1) modalSelectedIndex = 0;

    renderWindowList();
    windowModal.classList.add('show');
}

function closeWindowModal() {
    windowModal.classList.remove('show');
    focusCommandInput();
}

function openHelpModal() {
    helpModal.classList.add('show');
}

function closeHelpModal() {
    helpModal.classList.remove('show');
    focusCommandInput();
}

function openRenameModal() {
    const target = sessionSelect.value;
    if (!target) {
        showStatus('NO WINDOW SELECTED', 'error');
        return;
    }
    // Pre-fill with current window name
    const currentOption = sessionSelect.options[sessionSelect.selectedIndex];
    const currentName = currentOption?.dataset.windowName || '';
    renameInput.value = currentName;
    renameModal.classList.add('show');
    setTimeout(() => {
        renameInput.focus();
        renameInput.select();
    }, 50);
}

function closeRenameModal() {
    renameModal.classList.remove('show');
    focusCommandInput();
}

async function renameCurrentWindow(newName) {
    const target = sessionSelect.value;
    if (!target || !newName.trim()) return;

    try {
        const response = await targetFetch('/api/rename-window', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ target, name: newName.trim() }),
        });
        const data = await response.json();
        if (data.success) {
            showStatus('RENAMED: ' + newName.trim(), 'success');
            await loadWindows();
        } else {
            showStatus(data.error || 'RENAME FAILED', 'error');
        }
    } catch (err) {
        showStatus('CONNECTION ERROR', 'error');
    }
}


// --- Kill Window ---------------------------------------------------

// Pinned when the confirmation opens, so the kill can only ever land on
// the window the user was shown — never on whatever happens to be
// selected by the time they confirm.
let killPendingTarget = '';

function openKillModal() {
    const target = sessionSelect.value;
    if (!target) {
        showStatus('NO WINDOW SELECTED', 'error');
        return;
    }
    // tmux tears down the whole session with its last window, which
    // would take this server's own tmux down with it.
    if (allWindows.filter(w => w.host === windowInfo(target).host).length <= 1) {
        showStatus('CANNOT KILL LAST WINDOW', 'error');
        return;
    }
    const currentOption = sessionSelect.options[sessionSelect.selectedIndex];
    killPendingTarget = target;
    killTargetEl.textContent = currentOption
        ? (currentOption.dataset.baseLabel || currentOption.textContent)
        : target;
    killModal.classList.add('show');
}

function closeKillModal() {
    killModal.classList.remove('show');
    focusCommandInput();
}

async function killCurrentWindow() {
    const target = killPendingTarget;
    if (!target) return;

    try {
        const response = await targetFetch('/api/kill-window', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ target }),
        });
        const data = await response.json();
        if (data.success) {
            showStatus('KILLED: ' + target, 'success');
            // Drop the dead target from both selection stores before
            // reloading, so renderWindowOptions() falls through to a
            // live window instead of trying to keep a ghost visible.
            localStorage.removeItem('tmux-selected-target');
            sessionSelect.value = '';
            await loadWindows();
            startCapture();
        } else {
            showStatus(data.error || 'KILL FAILED', 'error');
        }
    } catch (err) {
        showStatus('CONNECTION ERROR', 'error');
    }
}

// --- New-window autocomplete ---------------------------------------

// Cached per modal open: ~200 short names is a few KB, so one fetch
// buys keystroke-by-keystroke filtering with no round trip.
let projectDirs = [];
let nwSuggestions = [];
let nwActiveIndex = -1;
// What the user actually typed, kept aside so arrowing off the end of
// the list can put their own text back in the input.
let nwTypedValue = '';

const NW_SUGGEST_COUNT = 3;

async function loadProjectDirs() {
    const host = nwHost;
    try {
        const response = await hostFetch('/api/project-dirs', {}, host);
        if (!response.ok) throw new Error('Projects unavailable');
        const data = await response.json();
        if (host !== nwHost) return;
        projectDirs = Array.isArray(data.dirs) ? data.dirs : [];
    } catch (err) {
        if (host !== nwHost) return;
        projectDirs = [];
    }
    renderNwSuggestions();
}

// Names arrive already restricted to [A-Za-z0-9._-] by the server's
// validate_window_name, so this is belt-and-braces for the one place
// that builds suggestion rows as an HTML string.
function escapeHtml(str) {
    return String(str).replace(/[&<>"']/g, c => ({
        '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
    }[c]));
}

function relativeAge(mtime) {
    if (!mtime) return '';
    const mins = Math.floor((Date.now() / 1000 - mtime) / 60);
    if (mins < 1) return 'now';
    if (mins < 60) return mins + 'm';
    const hours = Math.floor(mins / 60);
    if (hours < 24) return hours + 'h';
    return Math.floor(hours / 24) + 'd';
}

function computeNwSuggestions() {
    // Filter on what the user TYPED, never on the live input value:
    // arrowing onto a suggestion writes that name into the input, and
    // re-filtering against it would collapse the list out from under
    // the arrow keys.
    const query = nwTypedValue.trim().toLowerCase();

    if (!query) {
        // Nothing typed yet: the most recent projects that aren't
        // already open, since an open one is a switch, not a create.
        // Read window names from allWindows through the same filter
        // renderWindowOptions() uses, rather than parsing them back out
        // of option labels — those carry busy/waiting markers.
        const keepTarget = sessionSelect.value || localStorage.getItem('tmux-selected-target');
        const openNames = new Set(visibleWindows(keepTarget).filter(win => win.host === nwHost && win.session === nwSession).map(win => win.name));
        return projectDirs
            .filter(d => !openNames.has(d.name))
            .slice(0, NW_SUGGEST_COUNT);
    }

    // Prefix matches read as "what I'm typing", so they rank above a
    // mid-name hit. projectDirs is already mtime-desc, so each group
    // stays newest-first without re-sorting.
    const prefix = [];
    const substring = [];
    for (const d of projectDirs) {
        const name = d.name.toLowerCase();
        if (name.startsWith(query)) prefix.push(d);
        else if (name.includes(query)) substring.push(d);
    }
    return prefix.concat(substring).slice(0, NW_SUGGEST_COUNT);
}

function renderNwSuggestions() {
    nwSuggestions = computeNwSuggestions();
    if (nwActiveIndex >= nwSuggestions.length) nwActiveIndex = -1;

    const label = nwTypedValue.trim() ? 'MATCHES' : 'RECENT';
    let html = `<div class="nw-suggest-label">${label}</div>`;
    // Always render the full slot count: blank rows keep the list a
    // fixed height so it never jumps as matches come and go.
    for (let i = 0; i < NW_SUGGEST_COUNT; i++) {
        const dir = nwSuggestions[i];
        if (!dir) {
            // A non-breaking space, not an empty div: the blank row then
            // has the same line box as a filled one, so the list keeps
            // its height across any font metrics.
            html += '<div class="nw-suggest-item empty"><span class="nw-name">&nbsp;</span></div>';
            continue;
        }
        html += `<div class="nw-suggest-item${i === nwActiveIndex ? ' active' : ''}" data-index="${i}">`
             + `<span class="nw-name">${escapeHtml(dir.name)}</span>`
             + `<span class="nw-age">${relativeAge(dir.mtime)}</span>`
             + '</div>';
    }
    nwSuggest.innerHTML = html;

    nwSuggest.querySelectorAll('.nw-suggest-item[data-index]').forEach(item => {
        // mousedown, not click: the input must not lose focus first, or
        // the on-screen keyboard closes out from under the user.
        item.addEventListener('mousedown', (e) => {
            e.preventDefault();
            applyNwSuggestion(parseInt(item.dataset.index, 10));
        });
    });
}

// Selecting only fills the input — creating still takes an explicit
// Enter, so a stray tap can never spawn a window.
function applyNwSuggestion(index) {
    const dir = nwSuggestions[index];
    if (!dir) return;
    nwActiveIndex = index;
    newWindowInput.value = dir.name;
    renderNwSuggestions();
    newWindowInput.focus();
}

function moveNwActive(delta) {
    if (nwSuggestions.length === 0) return;
    const next = nwActiveIndex + delta;
    if (next < 0) {
        // Back above the list: restore whatever the user had typed.
        nwActiveIndex = -1;
        newWindowInput.value = nwTypedValue;
        renderNwSuggestions();
        return;
    }
    if (next >= nwSuggestions.length) return;
    applyNwSuggestion(next);
}

function renderWindowList() {
    windowList.innerHTML = modalWindows.map((win, i) => `
        <div class="window-item ${i === modalSelectedIndex ? 'selected' : ''} ${win.current ? 'current' : ''}"
             data-index="${i}">
            <span class="window-target">${escapeHtml(win.name)}</span>
            ${win.current ? '<span class="window-marker">(active)</span>' : ''}
        </div>
    `).join('');

    // Add click handlers
    windowList.querySelectorAll('.window-item').forEach(item => {
        item.addEventListener('click', () => {
            const index = parseInt(item.dataset.index);
            selectWindow(index);
        });
    });

    // Scroll selected into view
    const selected = windowList.querySelector('.selected');
    if (selected) selected.scrollIntoView({ block: 'nearest' });
}

function selectWindow(index) {
    const win = modalWindows[index];
    if (win) {
        sessionSelect.value = win.target;
        localStorage.setItem('tmux-selected-target', win.target);
        startCapture();
        showStatus(`SWITCHED TO ${win.target}`, 'success');
    }
    closeWindowModal();
}

async function reorderWindow(fromIdx, toIdx) {
    const fromWin = modalWindows[fromIdx];
    const toWin = modalWindows[toIdx];
    if (!fromWin || !toWin) return;

    const from = windowInfo(fromWin.target), to = windowInfo(toWin.target);
    if (from.host !== to.host || from.session !== to.session) {
        showStatus('REORDER WITHIN THE SAME HOST AND SESSION', 'error');
        return;
    }
    const session = from.session;
    const fromWindowIndex = from.index, toWindowIndex = to.index;

    try {
        const response = await hostFetch('/api/move-window', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ session, from_index: fromWindowIndex, to_index: toWindowIndex }),
        }, from.host);
        const data = await response.json();
        if (data.success) {
            await loadWindows(from.host);
            openWindowModal();
        } else {
            showStatus(data.error || 'MOVE FAILED', 'error');
        }
    } catch (err) {
        showStatus('CONNECTION ERROR', 'error');
    }
}

function handleModalKeydown(e) {
    // Handle upload modal
    if (uploadModal.classList.contains('show')) {
        if (e.key === 'Escape') {
            e.preventDefault();
            closeUploadModal();
            return true;
        }
        return true;
    }

    // Handle file modal
    if (fileModal.classList.contains('show')) {
        if (e.key === 'Escape') {
            e.preventDefault();
            closeFileModal();
            return true;
        }
        return true;
    }

    // Handle image modal
    if (imageModal.classList.contains('show')) {
        if (e.key === 'Escape') {
            e.preventDefault();
            closeImageModal();
            return true;
        }
        return true;
    }

    // Handle new-window prompt
    if (newWindowModal.classList.contains('show')) {
        if (nwModelsOpen()) {
            if (e.key === 'Escape') { e.preventDefault(); closeNwModels(); return true; }
            if (e.key === 'ArrowDown') { e.preventDefault(); moveNwModelActive(1); return true; }
            if (e.key === 'ArrowUp') { e.preventDefault(); moveNwModelActive(-1); return true; }
            if (e.key === 'Enter') {
                e.preventDefault();
                const items = filterNwModels(nwModelCatalog, nwModelFilter.value);
                if (nwModelCatalog && items.length) pickNwModel(items[nwModelActive].id);
                return true;
            }
            if (e.key === 'Tab') {
                e.preventDefault();
                closeNwModels(false);
                if (e.shiftKey) cycleNwSession(1); else cycleNwAgent(1);
                setTimeout(() => newWindowInput.focus(), 30);
                return true;
            }
            return true; // everything else types into the filter
        }
        if (e.key === 'Escape') { e.preventDefault(); closeNewWindowModal(); return true; }
        if (e.key === 'ArrowDown') { e.preventDefault(); moveNwActive(1); return true; }
        if (e.key === 'ArrowUp') { e.preventDefault(); moveNwActive(-1); return true; }
        if (e.key === 'Tab') {
            e.preventDefault();
            if (e.shiftKey) cycleNwSession(1); else cycleNwAgent(1);
            return true;
        }
        if (e.key === 'Enter') {
            e.preventDefault();
            // Whatever is in the input wins — arrowing onto a suggestion
            // has already written it there.
            const name = newWindowInput.value;
            closeNewWindowModal();
            submitNewWindow(name);
            return true;
        }
        return true; // consume all keys while the prompt is open
    }

    // Handle kill-window confirmation
    if (killModal.classList.contains('show')) {
        if (e.key === 'Escape') { e.preventDefault(); closeKillModal(); return true; }
        if (e.key === 'Enter') {
            e.preventDefault();
            closeKillModal();
            killCurrentWindow();
            return true;
        }
        return true; // consume all keys while the confirmation is open
    }

    // Handle rename modal
    if (renameModal.classList.contains('show')) {
        if (e.key === 'Escape') {
            e.preventDefault();
            closeRenameModal();
            return true;
        }
        if (e.key === 'Enter') {
            e.preventDefault();
            const newName = renameInput.value;
            closeRenameModal();
            renameCurrentWindow(newName);
            return true;
        }
        return true; // consume all keys while rename modal is open
    }

    // Handle action menu
    if (actionMenuModal.classList.contains('show')) {
        if (e.key === 'Escape') {
            e.preventDefault();
            closeActionMenu();
            return true;
        }
        return true;
    }

    // Handle history modal
    if (historyModal.classList.contains('show')) {
        if (e.key === 'Escape' || e.key === 'q') {
            e.preventDefault();
            closeHistoryModal();
        } else if (e.key === 'ArrowUp' || e.key === 'k') {
            e.preventDefault();
            moveHistorySelection(-1);
        } else if (e.key === 'ArrowDown' || e.key === 'j') {
            e.preventDefault();
            moveHistorySelection(1);
        } else if (e.key === 'Enter') {
            e.preventDefault();
            useHistoryEntry(historySelectedIndex);
        }
        return true; // consume all keys while the history modal is open
    }

    // Handle help modal
    if (helpModal.classList.contains('show')) {
        if (e.key === 'Escape' || e.key === 'q' || e.key === '?') {
            e.preventDefault();
            closeHelpModal();
            return true;
        }
        return false;
    }

    if (!windowModal.classList.contains('show')) return false;

    switch (e.key) {
        case 'ArrowUp':
            e.preventDefault();
            modalSelectedIndex = Math.max(0, modalSelectedIndex - 1);
            renderWindowList();
            return true;
        case 'ArrowDown':
            e.preventDefault();
            modalSelectedIndex = Math.min(modalWindows.length - 1, modalSelectedIndex + 1);
            renderWindowList();
            return true;
        case 'k':
            e.preventDefault();
            if (modalSelectedIndex > 0) {
                reorderWindow(modalSelectedIndex, modalSelectedIndex - 1);
            }
            return true;
        case 'j':
            e.preventDefault();
            if (modalSelectedIndex < modalWindows.length - 1) {
                reorderWindow(modalSelectedIndex, modalSelectedIndex + 1);
            }
            return true;
        case 'PageUp':
            e.preventDefault();
            modalSelectedIndex = Math.max(0, modalSelectedIndex - 10);
            renderWindowList();
            return true;
        case 'PageDown':
            e.preventDefault();
            modalSelectedIndex = Math.min(modalWindows.length - 1, modalSelectedIndex + 10);
            renderWindowList();
            return true;
        case 'Home':
            e.preventDefault();
            modalSelectedIndex = 0;
            renderWindowList();
            return true;
        case 'End':
            e.preventDefault();
            modalSelectedIndex = modalWindows.length - 1;
            renderWindowList();
            return true;
        case 'Enter':
            e.preventDefault();
            selectWindow(modalSelectedIndex);
            return true;
        case 'Escape':
        case 'q':
            e.preventDefault();
            closeWindowModal();
            return true;
    }
    return false;
}

// --- New-window agent + session pickers ------------------------------
// The command per agent is decided server-side (Agent::command in
// main.rs); this table is the preview shown under the buttons, so the
// user sees the real flag before pressing Enter. Keep the two in step.
const NW_AGENTS = [
    { id: 'claude', label: 'CLAUDE', command: 'claude --dangerously-skip-permissions' },
    { id: 'codex',  label: 'CODEX',  command: 'codex --yolo' },
    { id: 'agy',    label: 'AGY',    command: 'agy --dangerously-skip-permissions' },
    { id: 'eunice', label: 'EUNICE', command: 'eunice' },
    { id: 'hermes', label: 'HERMES', command: 'hermes --yolo' },
];
const NW_AGENT_KEY = 'tmux-new-window-agent';
const NW_DEFAULT_SESSION = '0';
const nwHostPick = document.getElementById('nwHostPick');
const nwAgentPick = document.getElementById('nwAgentPick');
let nwHost = defaultHost;
const nwCapabilities = new Map();
let nwSubmitting = false;
const nwSessionPick = document.getElementById('nwSessionPick');
const nwCommand = document.getElementById('nwCommand');
// The agent is remembered; the session is not. MASTER holds the backend
// control processes, so it must be a deliberate pick every single time.
let nwAgent = NW_AGENTS.some(a => a.id === localStorage.getItem(NW_AGENT_KEY))
    ? localStorage.getItem(NW_AGENT_KEY) : 'codex';
let nwSession = NW_DEFAULT_SESSION;

// Sessions come from the window list the page already polls: every
// tmux session has at least one window, so no extra endpoint is needed.
function nwSessions() {
    const names = [...new Set(allWindows.filter(w => w.host === nwHost).map(w => w.session))];
    if (!names.includes(NW_DEFAULT_SESSION)) names.push(NW_DEFAULT_SESSION);
    names.sort((a, b) => (b === NW_DEFAULT_SESSION) - (a === NW_DEFAULT_SESSION));
    return names;
}

function nwDefaultSession(sessions) {
    if (sessions.includes(NW_DEFAULT_SESSION)) return NW_DEFAULT_SESSION;
    return sessions.find(s => !s.includes('MASTER')) || sessions[0];
}

function renderNwPick(container, label, items, selected, onPick) {
    container.innerHTML = '';
    const tag = document.createElement('span');
    tag.className = 'nw-pick-label';
    tag.textContent = label;
    container.appendChild(tag);
    items.forEach(item => {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'menu-btn nw-pick-btn' + (item.id === selected ? ' selected' : '');
        btn.textContent = item.label;
        btn.setAttribute('role', 'radio');
        btn.setAttribute('aria-checked', String(item.id === selected));
        btn.disabled = !!item.disabled;
        if (item.disabled) btn.title = `Unavailable on ${hostLabel(nwHost)}`;
        // mousedown + preventDefault: a click would pull focus off the
        // name input, and the next keystrokes would go nowhere.
        btn.addEventListener('mousedown', e => e.preventDefault());
        btn.addEventListener('click', () => onPick(item.id));
        container.appendChild(btn);
    });
}

function renderNwPickers() {
    renderNwPick(nwHostPick, 'HOST', hostRegistry.map(h => ({ id: h.id, label: hostLabel(h.id) })), nwHost, setNwHost);
    const sessions = nwSessions();
    if (!sessions.includes(nwSession)) nwSession = nwDefaultSession(sessions);
    renderNwPick(nwAgentPick, 'AGENT', NW_AGENTS.map(a => ({ ...a, disabled: nwCapabilities.has(nwHost) && !nwCapabilities.get(nwHost).includes(a.id) })), nwAgent, setNwAgent);
    if (nwAgent === 'eunice') {
        nwAgentPick.appendChild(nwModelButton());
    } else if (nwModelsOpen()) {
        closeNwModels();
    }
    renderNwPick(nwSessionPick, 'SESSION',
        sessions.map(s => ({ id: s, label: s })), nwSession,
        (id) => { nwSession = id; renderNwPickers(); renderNwSuggestions(); });
    nwCommand.textContent = `${hostLabel(nwHost)} › ${nwSession}:  $ ${nwCommandFor(nwAgent)}`;
}

// --- EUNICE model picker ---------------------------------------------
// The server runs `eunice --list-models` (GET /api/eunice-models) and
// validates the id again before typing it into the shell; this side
// only filters and remembers the choice.
const NW_MODEL_KEY = 'tmux-new-window-eunice-model';
const NW_MODEL_DEFAULT = { id: '', provider: '', aliases: [], note: 'let eunice choose', tools: false, isDefault: true };
const nwModels = document.getElementById('nwModels');
const nwModelFilter = document.getElementById('nwModelFilter');
const nwModelList = document.getElementById('nwModelList');
let nwModel = localStorage.getItem(NW_MODEL_KEY) || '';
let nwModelCatalog = null;   // fetched on first open, kept for the page
let nwModelError = '';
let nwModelActive = 0;

function nwCommandFor(agentId) {
    if (agentId === 'eunice' && nwModel) return `eunice --model ${nwModel}`;
    return NW_AGENTS.find(a => a.id === agentId).command;
}

function nwModelsOpen() {
    return !nwModels.hidden;
}

function nwModelButton() {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'menu-btn nw-pick-btn nw-model-btn' + (nwModelsOpen() ? ' selected' : '');
    btn.textContent = `MODEL: ${nwModel || 'default'}`;
    btn.title = 'Pick the model EUNICE starts with';
    btn.addEventListener('mousedown', (e) => {
        e.preventDefault();
        if (nwModelsOpen()) closeNwModels(); else openNwModels();
    });
    return btn;
}

// Pure, so it can be checked outside the browser. Matches id, alias,
// note and provider; the "default" row stays whenever it could be meant.
function filterNwModels(catalog, query) {
    const q = (query || '').trim().toLowerCase();
    const hit = m => !q || [m.id, ...(m.aliases || []), m.note || '', m.provider]
        .some(s => s.toLowerCase().includes(q));
    const items = (catalog || []).filter(hit);
    if (!q || 'default'.includes(q)) items.unshift(NW_MODEL_DEFAULT);
    return items;
}

async function openNwModels() {
    const host = nwHost;
    nwModels.hidden = false;
    nwModelFilter.value = '';
    nwModelActive = 0;
    nwModelError = '';
    renderNwPickers();
    renderNwModelList();
    setTimeout(() => nwModelFilter.focus(), 30);
    if (nwModelCatalog) return;
    try {
        const response = await hostFetch('/api/eunice-models', {}, host);
        const data = await response.json();
        if (host !== nwHost) return;
        if (data.success) nwModelCatalog = data.models;
        else nwModelError = data.error || 'COULD NOT LIST MODELS';
    } catch (err) {
        if (host !== nwHost) return;
        nwModelError = 'CONNECTION ERROR';
    }
    if (nwModelsOpen()) renderNwModelList();
}

function closeNwModels(refocus = true) {
    if (nwModels.hidden) return;
    nwModels.hidden = true;
    renderNwPickers();
    if (refocus) setTimeout(() => newWindowInput.focus(), 30);
}

function renderNwModelList() {
    const items = filterNwModels(nwModelCatalog, nwModelFilter.value);
    nwModelActive = items.length ? Math.min(Math.max(0, nwModelActive), items.length - 1) : 0;
    nwModelList.innerHTML = '';
    const empty = (text) => {
        const row = document.createElement('div');
        row.className = 'nw-model-item empty';
        row.textContent = text;
        nwModelList.appendChild(row);
    };
    if (nwModelError) { empty(nwModelError); return; }
    if (!nwModelCatalog) { empty('LOADING…'); return; }
    if (items.length === 0) { empty('NO MATCHES'); return; }
    let lastProvider = null;
    items.forEach((m, i) => {
        if (!m.isDefault && m.provider !== lastProvider) {
            const group = document.createElement('div');
            group.className = 'nw-models-group';
            group.textContent = m.provider.toUpperCase();
            nwModelList.appendChild(group);
            lastProvider = m.provider;
        }
        const row = document.createElement('div');
        row.className = 'nw-model-item' + (i === nwModelActive ? ' active' : '');
        row.setAttribute('role', 'option');
        row.setAttribute('aria-selected', String(i === nwModelActive));
        const name = document.createElement('span');
        name.className = 'nw-model-name';
        name.textContent = m.isDefault ? 'default' : m.id;
        const meta = document.createElement('span');
        meta.className = 'nw-model-meta';
        meta.textContent = [
            (m.aliases || []).join(', '),
            m.note,
            m.tools ? '✓ tools' : '',
            m.id === nwModel ? 'current' : '',
        ].filter(Boolean).join(' · ');
        row.appendChild(name);
        row.appendChild(meta);
        row.addEventListener('mousedown', (e) => { e.preventDefault(); pickNwModel(m.id); });
        nwModelList.appendChild(row);
    });
    const active = nwModelList.querySelector('.nw-model-item.active');
    if (active) active.scrollIntoView({ block: 'nearest' });
}

function moveNwModelActive(delta) {
    const count = filterNwModels(nwModelCatalog, nwModelFilter.value).length;
    if (!count) return;
    nwModelActive = (nwModelActive + delta + count) % count;
    renderNwModelList();
}

function pickNwModel(id) {
    nwModel = id || '';
    localStorage.setItem(`${NW_MODEL_KEY}:${nwHost}`, nwModel);
    closeNwModels();
}

nwModelFilter.addEventListener('input', () => {
    nwModelActive = 0;
    renderNwModelList();
});

function setNwAgent(id) {
    if (nwCapabilities.has(nwHost) && !nwCapabilities.get(nwHost).includes(id)) return;
    nwAgent = id;
    localStorage.setItem(`${NW_AGENT_KEY}:${nwHost}`, id);
    if (nwHost === defaultHost) localStorage.setItem(NW_AGENT_KEY, id);
    renderNwPickers();
}

async function setNwHost(id) {
    nwHost = id;
    nwAgent = localStorage.getItem(`${NW_AGENT_KEY}:${id}`) || localStorage.getItem(NW_AGENT_KEY) || 'codex';
    nwSession = nwDefaultSession(nwSessions());
    projectDirs = [];
    nwModelCatalog = null;
    nwModel = localStorage.getItem(`${NW_MODEL_KEY}:${id}`) || (id === defaultHost ? localStorage.getItem(NW_MODEL_KEY) : '') || '';
    nwModels.hidden = true;
    renderNwPickers();
    renderNwSuggestions();
    loadProjectDirs();
    try {
        const response = await hostFetch('/api/agents', {}, id);
        const data = await response.json();
        if (response.ok && Array.isArray(data.agents)) nwCapabilities.set(id, data.agents);
        if (nwHost !== id) return;
        if (nwCapabilities.has(id) && !nwCapabilities.get(id).includes(nwAgent)) {
            nwAgent = nwCapabilities.get(id).includes('codex') ? 'codex' : nwCapabilities.get(id)[0] || 'codex';
        }
        renderNwPickers();
    } catch (_) { /* Creation reports the host's connection error. */ }
}

function cycleNwAgent(delta) {
    const agents = NW_AGENTS.filter(a => !nwCapabilities.has(nwHost) || nwCapabilities.get(nwHost).includes(a.id));
    if (!agents.length) return;
    const i = Math.max(0, agents.findIndex(a => a.id === nwAgent));
    setNwAgent(agents[(i + delta + agents.length) % agents.length].id);
}

function cycleNwSession(delta) {
    const sessions = nwSessions();
    const i = Math.max(0, sessions.indexOf(nwSession));
    nwSession = sessions[(i + delta + sessions.length) % sessions.length];
    renderNwPickers();
}

function createNewWindow() {
    setNwHost(defaultHost);
    newWindowInput.value = '';
    nwTypedValue = '';
    nwActiveIndex = -1;
    nwSession = nwDefaultSession(nwSessions());
    nwModels.hidden = true;
    renderNwPickers();
    newWindowModal.classList.add('show');
    // Render immediately from the previous fetch so the list is never
    // blank, then refresh — mtimes move while the page stays open.
    renderNwSuggestions();
    // Keep focus inside the tap/click handler so iOS can open the keyboard.
    newWindowInput.focus({ preventScroll: true });
}

function closeNewWindowModal() {
    newWindowModal.classList.remove('show');
    nwModels.hidden = true;
    focusCommandInput();
}

const NEW_WINDOW_NAME_RE = /^[A-Za-z0-9._-]+$/;
async function submitNewWindow(name) {
    if (nwSubmitting) return;
    const host = nwHost, agent = nwAgent, session = nwSession, model = nwModel;
    if (nwCapabilities.has(host) && !nwCapabilities.get(host).includes(agent)) { showStatus('AGENT UNAVAILABLE ON ' + host, 'error'); return; }
    name = (name || '').trim();
    if (!name || !NEW_WINDOW_NAME_RE.test(name) || name.startsWith('-') || name === '.' || name === '..') {
        showStatus('INVALID NAME', 'error');
        newWindowModal.classList.add('show');
        nwActiveIndex = -1;
        nwTypedValue = name;
        setTimeout(() => {
            newWindowInput.value = name;
            newWindowInput.focus();
            renderNwSuggestions();
        }, 50);
        return;
    }
    nwSubmitting = true;
    try {
        const response = await hostFetch('/api/new-window-named', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name, agent, session,
                model: agent === 'eunice' && model ? model : undefined,
            }),
        }, host);
        const data = await response.json();
        if (data.success && data.target) {
            // Select the new/switched window. Persist the target and blank the
            // live selection BEFORE re-rendering so renderWindowOptions()'s
            // keepTarget (= sessionSelect.value || savedTarget) falls through to
            // the new target. Otherwise a MASTER-named new window is filtered out
            // and the plain `sessionSelect.value = target` assignment no-ops.
            pendingWindowSelection = windowKey(host, data);
            localStorage.setItem('tmux-selected-target', pendingWindowSelection);
            sessionSelect.value = '';
            await loadWindows(host);
            pendingWindowSelection = '';
            const agentLabel = (data.agent || nwAgent).toUpperCase();
            showStatus((data.existing ? 'SWITCHED TO ' : `NEW ${agentLabel} WINDOW: `) + name, 'success');
            setTimeout(() => focusCommandInput(), 150);
        } else {
            showStatus(data.error || 'FAILED TO CREATE WINDOW', 'error');
        }
    } catch (err) {
        showStatus('CONNECTION ERROR', 'error');
    } finally { nwSubmitting = false; pendingWindowSelection = ''; }
}

function handlePrefixCommand(key) {
    exitPrefixMode();
    switch (key) {
        case 'c':
            // Create new window
            createNewWindow();
            return true;
        case ',':
            // Rename window
            openRenameModal();
            return true;
        case '&':
            // Kill window (force)
            openKillModal();
            return true;
        case 'w':
            openWindowModal();
            return true;
        case 'n':
            // Next window
            if (sessionSelect.selectedIndex < sessionSelect.options.length - 1) {
                sessionSelect.selectedIndex++;
                localStorage.setItem('tmux-selected-target', sessionSelect.value);
                startCapture();
                showStatus('NEXT WINDOW', 'success');
            }
            return true;
        case 'p':
            // Previous window
            if (sessionSelect.selectedIndex > 0) {
                sessionSelect.selectedIndex--;
                localStorage.setItem('tmux-selected-target', sessionSelect.value);
                startCapture();
                showStatus('PREV WINDOW', 'success');
            }
            return true;
        case 'h':
            openHistoryModal();
            return true;
        case 'r':
            // Refresh window list
            loadWindows();
            showStatus('REFRESHING...', 'success');
            return true;
        case '?':
            openHelpModal();
            return true;
    }
    return false;
}

async function sendKeyToTmux(key) {
    const session = sessionSelect.value || '0';
    try {
        const response = await targetFetch('/api/send-key', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ key, session }),
        });
        const data = await response.json();
        if (response.ok) {
            showStatus(`SENT ${key}`, 'success');
            setTimeout(captureOutput, 100);
        }
    } catch (err) {
        showStatus('CONNECTION ERROR', 'error');
    }
}

// Global keyboard handler
document.addEventListener('keydown', (e) => {
    // Handle modal keys first
    if (handleModalKeydown(e)) return;

    // Ctrl+R refreshes windows
    if (e.ctrlKey && e.key === 'r') {
        e.preventDefault();
        loadWindows();
        return;
    }

    // Ctrl+C sends interrupt to tmux (if no text selection)
    if (e.ctrlKey && e.key === 'c') {
        const selection = window.getSelection();
        if (!selection || selection.isCollapsed || selection.toString().length === 0) {
            e.preventDefault();
            sendKeyToTmux('C-c');
            return;
        }
        // Otherwise let browser handle copy
    }

    // Ctrl+B enters prefix mode
    if (e.ctrlKey && e.key === 'b') {
        e.preventDefault();
        enterPrefixMode();
        return;
    }

    // In prefix mode, handle command keys
    if (prefixMode && !e.ctrlKey && !e.altKey && !e.metaKey) {
        e.preventDefault();
        handlePrefixCommand(e.key);
        return;
    }
});

sendBtn.addEventListener('click', sendCommand);

// Action menu
function openActionMenu() {
    // Label states the action the click performs, recomputed on every open
    // so it cannot drift out of sync with stored state.
    document.getElementById('menuToggleMasterLabel').textContent =
        showMaster ? 'Hide MASTER' : 'Unhide MASTER';
    actionMenuModal.classList.add('show');
}
function closeActionMenu() { actionMenuModal.classList.remove('show'); focusCommandInput(); }

menuBtn.addEventListener('click', openActionMenu);
document.getElementById('newWindowBtn').addEventListener('click', createNewWindow);
document.getElementById('menuReconnect').addEventListener('click', () => {
    closeActionMenu();
    showStatus('RECONNECTING…', 'success');
    pausePolling();
    resumePolling();
});
actionMenuModal.addEventListener('click', (e) => {
    if (e.target === actionMenuModal) closeActionMenu();
});

document.getElementById('menuHistory').addEventListener('click', () => { closeActionMenu(); openHistoryModal(); });
document.getElementById('historyClose').addEventListener('click', closeHistoryModal);
document.getElementById('historyClear').addEventListener('click', (e) => clearHistoryEntries(e.currentTarget));
historyModal.addEventListener('click', (e) => {
    if (e.target === historyModal) closeHistoryModal();
});

document.getElementById('menuUpload').addEventListener('click', () => { closeActionMenu(); openUploadPicker(); });
document.getElementById('menuRenameWindow').addEventListener('click', () => { closeActionMenu(); openRenameModal(); });
document.getElementById('menuKillWindow').addEventListener('click', () => { closeActionMenu(); openKillModal(); });
document.getElementById('menuWindowList').addEventListener('click', () => { closeActionMenu(); openWindowModal(); });
document.getElementById('menuNextWindow').addEventListener('click', () => { closeActionMenu(); handlePrefixCommand('n'); });
document.getElementById('menuPrevWindow').addEventListener('click', () => { closeActionMenu(); handlePrefixCommand('p'); });
document.getElementById('menuRefresh').addEventListener('click', () => { closeActionMenu(); loadWindows(); });
document.getElementById('menuToggleMaster').addEventListener('click', () => {
    closeActionMenu();
    showMaster = !showMaster;
    localStorage.setItem(SHOW_MASTER_KEY, String(showMaster));
    renderWindowOptions();
    showStatus(showMaster ? 'MASTER SHOWN' : 'MASTER HIDDEN', 'success');
});
document.getElementById('menuSendCtrlC').addEventListener('click', () => { closeActionMenu(); sendKeyToTmux('C-c'); });
document.getElementById('menuHelp').addEventListener('click', () => { closeActionMenu(); openHelpModal(); });
document.getElementById('menuLogout').addEventListener('click', () => {
    closeActionMenu();
    if (confirm('Log out of Cloudflare Access?')) {
        // Cloudflare intercepts this same-origin path at the edge,
        // clears the CF_Authorization session cookie, and shows its logout page.
        window.location.href = '/cdn-cgi/access/logout';
    }
});
sessionSelect.addEventListener('change', () => {
    localStorage.setItem('tmux-selected-target', sessionSelect.value);
    // Switching windows must not commit, cancel, or discard anything —
    // per-window picker state is left exactly as it was. Clear only the
    // view, which the next capture repopulates for the new target.
    clearPicker();
    // Stale for up to a poll otherwise, and a bar that says the new
    // window is working when it is idle is worse than a beat of blank.
    updateWorking('');
    updateAgentIndicator(null);
    startCapture();
    refreshWindowStatus();
    // Don't yank focus back to the textarea if the new window turns out
    // to have a question waiting; updatePicker claims focus for that.
    setTimeout(() => { if (!pickerData) focusCommandInput(); }, 500);
});

commandInput.addEventListener('keydown', (e) => {
    // Don't handle if modal is open or in prefix mode.
    // killModal has to be listed explicitly: the other prompts own an
    // input that takes focus, so this handler never fires while they
    // are up. The kill confirmation has no input, commandInput keeps
    // focus, and this handler's stopPropagation() would otherwise eat
    // the Enter before handleModalKeydown ever sees it.
    if (windowModal.classList.contains('show') || historyModal.classList.contains('show')
        || killModal.classList.contains('show') || prefixMode) return;

    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        e.stopPropagation();
        sendCommand();
        return false;
    }

    // Command history navigation
    if (e.key === 'ArrowUp') {
        e.preventDefault();
        if (commandHistory.length === 0) return;
        // Save current input when starting to navigate
        if (historyIndex === -1) {
            currentInput = commandInput.value;
        }
        // Move back in history
        if (historyIndex < commandHistory.length - 1) {
            historyIndex++;
            commandInput.value = commandHistory[commandHistory.length - 1 - historyIndex];
        }
        return false;
    }

    if (e.key === 'ArrowDown') {
        e.preventDefault();
        if (historyIndex === -1) return;
        // Move forward in history
        historyIndex--;
        if (historyIndex === -1) {
            // Back to current input
            commandInput.value = currentInput;
        } else {
            commandInput.value = commandHistory[commandHistory.length - 1 - historyIndex];
        }
        return false;
    }
});

commandInput.addEventListener('keypress', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        return false;
    }
});

// Close modals on overlay click
windowModal.addEventListener('click', (e) => {
    if (e.target === windowModal) closeWindowModal();
});
helpModal.addEventListener('click', (e) => {
    if (e.target === helpModal) closeHelpModal();
});
renameModal.addEventListener('click', (e) => {
    if (e.target === renameModal) closeRenameModal();
});
newWindowInput.addEventListener('input', () => {
    // Typing invalidates any arrow selection — the list is about to be
    // a different list.
    nwTypedValue = newWindowInput.value;
    nwActiveIndex = -1;
    renderNwSuggestions();
});

killModal.addEventListener('click', (e) => {
    if (e.target === killModal) closeKillModal();
});
document.getElementById('killCancel').addEventListener('click', closeKillModal);
document.getElementById('killConfirm').addEventListener('click', () => {
    closeKillModal();
    killCurrentWindow();
});

newWindowModal.addEventListener('click', (e) => {
    if (e.target === newWindowModal) closeNewWindowModal();
});

// File viewer modal
const fileModal = document.getElementById('fileModal');
const fileModalContent = document.getElementById('fileModalContent');
const fileModalPath = document.getElementById('fileModalPath');

function syntaxHighlightJson(json) {
    // Pretty-print then syntax highlight
    const str = (typeof json === 'string') ? json : JSON.stringify(json, null, 2);
    // Escape HTML first
    const escaped = str
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
    // Highlight tokens
    return escaped.replace(
        /("(?:\\.|[^"\\])*")\s*:/g,
        '<span class="json-key">$1</span>:'
    ).replace(
        /:\s*("(?:\\.|[^"\\])*")/g,
        ': <span class="json-string">$1</span>'
    ).replace(
        /:\s*(\d+\.?\d*(?:[eE][+-]?\d+)?)/g,
        ': <span class="json-number">$1</span>'
    ).replace(
        /:\s*(true|false)/g,
        ': <span class="json-bool">$1</span>'
    ).replace(
        /:\s*(null)/g,
        ': <span class="json-null">$1</span>'
    ).replace(
        /(?<=[\[,\s])("(?:\\.|[^"\\])*")(?=\s*[,\]])/g,
        '<span class="json-string">$1</span>'
    ).replace(
        /(?<=[\[,\s])(\d+\.?\d*(?:[eE][+-]?\d+)?)(?=\s*[,\]])/g,
        '<span class="json-number">$1</span>'
    ).replace(
        /(?<=[\[,\s])(true|false)(?=\s*[,\]])/g,
        '<span class="json-bool">$1</span>'
    ).replace(
        /(?<=[\[,\s])(null)(?=\s*[,\]])/g,
        '<span class="json-null">$1</span>'
    );
}

async function openFileModal(filePath, host = outputContent.dataset.host || windowInfo(sessionSelect.value).host) {
    fileModalPath.textContent = filePath;
    fileModalContent.textContent = 'Loading...';
    fileModal.classList.add('show');

    try {
        const response = await hostFetch('/api/serve-file?path=' + encodeURIComponent(filePath), {}, host);
        if (!response.ok) {
            fileModalContent.textContent = 'Error: ' + (await response.text());
            return;
        }
        const text = await response.text();
        const ext = filePath.split('.').pop().toLowerCase();

        if (ext === 'json') {
            try {
                const parsed = JSON.parse(text);
                fileModalContent.innerHTML = syntaxHighlightJson(parsed);
            } catch {
                // Invalid JSON, show raw with basic escaping
                fileModalContent.textContent = text;
            }
        } else {
            fileModalContent.textContent = text;
        }
    } catch (err) {
        fileModalContent.textContent = 'Failed to load file: ' + err.message;
    }
}

function closeFileModal() {
    fileModal.classList.remove('show');
    fileModalContent.textContent = '';
    focusCommandInput();
}

fileModal.addEventListener('click', (e) => {
    if (e.target === fileModal) closeFileModal();
});

// Delegate click for file links in output
outputContent.addEventListener('click', (e) => {
    const fileLink = e.target.closest('a.file-link');
    if (fileLink) {
        e.preventDefault();
        const filePath = fileLink.dataset.filePath;
        if (filePath) openFileModal(filePath);
        return;
    }
});

// Image preview modal
const imageModal = document.getElementById('imageModal');
const imageModalImg = document.getElementById('imageModalImg');
const imageModalPath = document.getElementById('imageModalPath');

function openImageModal(imagePath, host = outputContent.dataset.host || windowInfo(sessionSelect.value).host) {
    imageModalPath.textContent = imagePath;
    imageModalImg.src = '/api/serve-image?host=' + encodeURIComponent(host) + '&path=' + encodeURIComponent(imagePath);
    imageModal.classList.add('show');
}

function closeImageModal() {
    imageModal.classList.remove('show');
    imageModalImg.src = '';
    focusCommandInput();
}

imageModal.addEventListener('click', (e) => {
    if (e.target === imageModal) closeImageModal();
});

// Delegate click for image links in output
outputContent.addEventListener('click', (e) => {
    const link = e.target.closest('a.image-link');
    if (link) {
        e.preventDefault();
        const imagePath = link.dataset.imagePath;
        if (imagePath) openImageModal(imagePath);
    }
});

// ---- File upload ----
const fileInput = document.getElementById('fileInput');
const uploadModal = document.getElementById('uploadModal');
const uploadList = document.getElementById('uploadList');
const uploadTargetEl = document.getElementById('uploadTarget');
const uploadCloseBtn = document.getElementById('uploadClose');
const uploadAddMoreBtn = document.getElementById('uploadAddMore');
let uploadSeq = 0;
let uploadPickerTarget = '';
let uploadBatchTarget = '';

function openUploadPicker() {
    uploadPickerTarget = sessionSelect.value;
    if (!uploadPickerTarget) { showStatus('CHOOSE A WINDOW', 'error'); return; }
    // Reset so picking the same file again still fires a change event
    fileInput.value = '';
    fileInput.click();
}

function openUploadModal() {
    uploadTargetEl.textContent = '→ ' + (allWindows.find(w => w.target === uploadBatchTarget) ? windowLabel(windowInfo(uploadBatchTarget)) : uploadBatchTarget);
    // Clear finished rows from earlier batches; keep any still uploading
    uploadList.querySelectorAll('.upload-item.done, .upload-item.error').forEach(el => el.remove());
    uploadModal.classList.add('show');
}

function closeUploadModal() {
    uploadModal.classList.remove('show');
    focusCommandInput();
}

function insertPathIntoInput(path) {
    const cur = commandInput.value;
    const sep = (cur && !cur.endsWith(' ')) ? ' ' : '';
    // Shell-quote the path if it contains whitespace
    const quoted = /[^A-Za-z0-9_./-]/.test(path) ? `'${path.replace(/'/g, `'\\''`)}'` : path;
    commandInput.value = cur + sep + quoted + ' ';
}

function uploadOneFile(file, target) {
    const destination = windowInfo(target);
    const item = document.createElement('div');
    item.className = 'upload-item uploading';
    item.id = 'upload-' + (++uploadSeq);
    item.innerHTML =
        '<div class="upload-item-top">' +
            '<span class="upload-name"></span>' +
            '<span class="upload-pct">0%</span>' +
        '</div>' +
        '<div class="upload-bar"><div class="upload-bar-fill"></div></div>' +
        '<div class="upload-status">UPLOADING…</div>';
    // Untrusted filename goes in via textContent (no HTML injection)
    item.querySelector('.upload-name').textContent = file.name;
    uploadList.appendChild(item);

    const pctEl = item.querySelector('.upload-pct');
    const fillEl = item.querySelector('.upload-bar-fill');
    const statusEl = item.querySelector('.upload-status');
    const xhr = new XMLHttpRequest();
    xhr.open('POST', '/api/upload?host=' + encodeURIComponent(destination.host) + '&target=' + encodeURIComponent(destination.window_id || destination.nativeTarget) +
                     '&name=' + encodeURIComponent(file.name));

    xhr.upload.addEventListener('progress', (e) => {
        if (e.lengthComputable) {
            const pct = Math.round((e.loaded / e.total) * 100);
            pctEl.textContent = pct + '%';
            if (pct === 100) statusEl.textContent = 'SAVING ON ' + destination.host + '…';
            fillEl.style.width = pct + '%';
        }
    });

    xhr.addEventListener('load', () => {
        let data = {};
        try { data = JSON.parse(xhr.responseText); } catch (_) {}
        if (xhr.status >= 200 && xhr.status < 300 && data.success) {
            item.classList.remove('uploading');
            item.classList.add('done');
            pctEl.textContent = '100%';
            fillEl.style.width = '100%';
            statusEl.textContent = destination.host + ' → ' + data.path;
            if (sessionSelect.value === target) insertPathIntoInput(data.path);
            showStatus('UPLOADED ' + (data.name || file.name), 'success');
        } else {
            item.classList.remove('uploading');
            item.classList.add('error');
            statusEl.textContent = data.error || ('FAILED (' + xhr.status + ')');
            showStatus('UPLOAD FAILED', 'error');
        }
    });

    xhr.addEventListener('error', () => {
        item.classList.remove('uploading');
        item.classList.add('error');
        statusEl.textContent = 'CONNECTION ERROR';
        showStatus('UPLOAD FAILED', 'error');
    });

    xhr.send(file);
}

uploadAddMoreBtn.addEventListener('click', openUploadPicker);
uploadCloseBtn.addEventListener('click', closeUploadModal);
uploadModal.addEventListener('click', (e) => {
    if (e.target === uploadModal) closeUploadModal();
});
fileInput.addEventListener('change', () => {
    const files = Array.from(fileInput.files || []);
    if (files.length === 0) return;
    uploadBatchTarget = uploadPickerTarget || sessionSelect.value;
    if (!uploadBatchTarget) return;
    openUploadModal();
    files.forEach(file => uploadOneFile(file, uploadBatchTarget));
});

// Pause all background work while hidden/offline, including BFCache pages.
function pausePolling() {
    capturePoller.stop();
    statusPoller.stop();
    windowsPoller.stop();
}
function resumePolling() {
    if (document.hidden || !navigator.onLine) return;
    windowsPoller.start();
    statusPoller.start();
    startCapture();
}
document.addEventListener('visibilitychange', () => {
    document.documentElement.classList.toggle('page-hidden', document.hidden);
    if (document.hidden) pausePolling();
    else resumePolling();
});
window.addEventListener('pagehide', pausePolling);
window.addEventListener('pageshow', event => { if (event.persisted) resumePolling(); });
window.addEventListener('offline', pausePolling);
window.addEventListener('online', resumePolling);
resumePolling();
focusCommandInput();

function focusCommandInput(editing = false) {
    if (editing || matchMedia('(hover: hover) and (pointer: fine)').matches) {
        commandInput.focus({ preventScroll: true });
    }
}

// Dynamic viewport units handle browser chrome; VisualViewport also handles
// keyboards on Safari. Don't resize the layout during pinch zoom.
let viewportFrame = 0;
function updateViewport() {
    if (viewportFrame) return;
    viewportFrame = requestAnimationFrame(() => {
        viewportFrame = 0;
        const viewport = window.visualViewport;
        if (viewport && Math.abs(viewport.scale - 1) > 0.01) return;
        const height = `${Math.round(viewport?.height || window.innerHeight)}px`;
        const top = `${Math.round(viewport?.offsetTop || 0)}px`;
        const style = document.documentElement.style;
        document.documentElement.classList.toggle('compact-viewport', (viewport?.height || window.innerHeight) < 500);
        if (style.getPropertyValue('--app-height') !== height) style.setProperty('--app-height', height);
        if (style.getPropertyValue('--app-top') !== top) style.setProperty('--app-top', top);
        if (terminalRenderer.following) outputContent.scrollTop = outputContent.scrollHeight;
    });
}
window.visualViewport?.addEventListener('resize', updateViewport);
window.visualViewport?.addEventListener('scroll', updateViewport);
window.addEventListener('resize', updateViewport);
window.addEventListener('pageshow', updateViewport);
updateViewport();
document.fonts?.addEventListener('loadingdone', () => {
    if (terminalRenderer.following) outputContent.scrollTop = outputContent.scrollHeight;
});
