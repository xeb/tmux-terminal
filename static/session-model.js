// Native CLI menus supply the choices and confirm every change. The pane id
// returned on open pins this dialog even if tmux window indexes are reordered.
(() => {
    const style = document.createElement('style');
    style.textContent = `
        .sm-dialog { width: min(580px, calc(100vw - 24px)); max-height: 100%; display: flex; flex-direction: column; background: var(--terminal-bg); border: 1px solid var(--matrix-green); box-shadow: 0 12px 60px #0008; }
        .sm-context, .sm-note { padding: 10px 16px; font-size: .7rem; color: var(--matrix-dim); overflow-wrap: anywhere; }
        .sm-context { border-bottom: 1px solid var(--matrix-dim); }
        .sm-body { overflow-y: auto; min-height: 0; padding: 12px 16px; }
        .sm-label { font-size: .65rem; letter-spacing: .1em; margin: 0 0 8px; }
        .sm-options { display: grid; gap: 6px; margin-bottom: 16px; }
        .sm-option { display: block; text-align: left; white-space: normal; padding: 10px 12px; width: 100%; }
        .sm-dialog [hidden] { display: none; }
        .sm-option[aria-pressed="true"] { background: var(--matrix-green); color: var(--void); border-color: var(--matrix-green); }
        .sm-description { display: block; font-size: .65rem; font-weight: normal; opacity: .8; margin-top: 4px; }
        .sm-effort { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
        .sm-effort-value { min-width: 80px; text-align: center; font-size: .8rem; }
        .sm-error { color: var(--matrix-green); padding: 0 16px; font-size: .75rem; }
        .sm-actions { display: flex; justify-content: flex-end; gap: 8px; padding: 12px 16px; border-top: 1px solid var(--matrix-dim); }
        .sm-actions .menu-btn { min-height: 36px; }
        .sm-dialog button:disabled { opacity: .45; cursor: wait; }
        .sm-filter { width: 100%; margin: 0 0 12px; box-sizing: border-box; }
    `;
    document.head.append(style);
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.id = 'sessionModelModal';
    overlay.innerHTML = `<section class="sm-dialog" role="dialog" aria-modal="true" aria-labelledby="smTitle" tabindex="-1">
        <div class="modal-header" id="smTitle">MODEL & EFFORT</div>
        <div class="sm-context"></div><div class="sm-body"></div>
        <p class="sm-error" role="alert"></p><div class="sm-note" aria-live="polite"></div>
        <div class="sm-actions"><button class="menu-btn sm-refresh" hidden>RELOAD</button><button class="menu-btn sm-cancel">CANCEL</button><button class="menu-btn sm-apply" hidden>APPLY</button></div>
    </section>`;
    document.body.append(overlay);
    const body = overlay.querySelector('.sm-body');
    const note = overlay.querySelector('.sm-note');
    const error = overlay.querySelector('.sm-error');
    const apply = overlay.querySelector('.sm-apply');
    const cancel = overlay.querySelector('.sm-cancel');
    const reload = overlay.querySelector('.sm-refresh');
    let target = '', menu = null, busy = false, failed = false, selectedModel = '', selectedEffort = 'default', query = '';
    const isOpen = () => overlay.classList.contains('show');
    function button(label, action, selected = false) {
        const b = document.createElement('button');
        b.type = 'button'; b.className = 'menu-btn sm-option'; b.textContent = label;
        b.setAttribute('aria-pressed', String(selected));
        b.addEventListener('click', action);
        return b;
    }
    function label(text) { const el = document.createElement('div'); el.className = 'sm-label'; el.textContent = text; body.append(el); }
    function dismiss() {
        overlay.classList.remove('show');
        agentIndicator.setAttribute('aria-expanded', 'false');
        menu = null;
        agentIndicator.focus();
    }
    async function action(actionName, extra = {}) {
        if (busy) return;
        busy = true; failed = false; error.textContent = ''; reload.hidden = true;
        overlay.querySelectorAll('button').forEach(b => b.disabled = true);
        note.textContent = actionName === 'open' ? 'Reading this session’s model choices…' : 'Waiting for the CLI…';
        try {
            const response = await targetFetch('/api/session-model', { method: 'POST', signal: AbortSignal.timeout(20000), headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ target, action: actionName, fingerprint: menu?.fingerprint || '', ...extra }) });
            const data = await response.json();
            if (!data.success) throw new Error(data.error || 'Could not change model settings.');
            target = data.target;
            if (data.closed) {
                dismiss();
                if (data.applied) showStatus('MODEL SETTINGS APPLIED', 'success');
                captureOutput();
                return;
            }
            menu = data.menu;
            if (menu.agent === 'eunice') { selectedModel = menu.options[menu.cursor].id; selectedEffort = menu.effort || 'default'; }
            render();
        } catch (e) {
            failed = true;
            error.textContent = e.message;
            note.textContent = 'Check the terminal if the CLI changed while this picker was open.';
            reload.hidden = false;
            cancel.textContent = 'CLOSE';
        } finally {
            busy = false;
            overlay.querySelectorAll('button').forEach(b => b.disabled = failed && b !== cancel && b !== reload);
            if (['codex', 'hermes'].includes(menu?.agent) && menu.stage !== 'effort') apply.disabled = true;
            if (isOpen() && !overlay.contains(document.activeElement)) overlay.querySelector('.sm-dialog').focus();
        }
    }
    function render() {
        body.replaceChildren();
        cancel.textContent = 'CANCEL';
        if (!menu) return;
        const eunice = menu.agent === 'eunice';
        const hermes = menu.agent === 'hermes';
        const efforts = menu.stage === 'effort';
        label(hermes && menu.stage === 'provider' ? 'PROVIDER' : (efforts ? menu.title.toUpperCase() : 'MODEL'));
        if (eunice) {
            const filter = document.createElement('input');
            filter.className = 'rename-input sm-filter'; filter.placeholder = 'Filter models…'; filter.value = query;
            filter.setAttribute('aria-label', 'Filter models');
            filter.addEventListener('input', () => { query = filter.value; render(); const f = body.querySelector('input'); f.focus(); f.setSelectionRange(query.length, query.length); });
            body.append(filter);
        }
        const options = document.createElement('div'); options.className = 'sm-options';
        menu.options.forEach((opt, index) => {
            if (eunice && !(opt.label + opt.description).toLowerCase().includes(query.toLowerCase())) return;
            const row = button(opt.label, () => {
                if (eunice) { selectedModel = opt.id; if (!opt.efforts.includes(selectedEffort)) selectedEffort = 'default'; render(); }
                else action('select', { index });
            }, eunice ? opt.id === selectedModel : index === menu.cursor);
            if (opt.description) { const desc = document.createElement('span'); desc.className = 'sm-description'; desc.textContent = opt.description; row.append(desc); }
            options.append(row);
        });
        body.append(options);
        if (menu.agent === 'codex' || hermes) {
            note.textContent = efforts ? 'Choose an effort, then Apply. Your conversation stays open.' : (hermes && menu.stage === 'provider' ? 'Choose a provider to see its models.' : (hermes ? 'Choose a model to continue. Models without reasoning controls apply immediately.' : 'Choose a model to see its supported effort levels.'));
            if (efforts) body.append(button('← BACK', () => action('back')));
        } else {
            label('EFFORT');
            const controls = document.createElement('div'); controls.className = 'sm-effort';
            if (eunice) {
                const opt = menu.options.find(m => m.id === selectedModel);
                opt.efforts.forEach(effort => { const b = button(effort.toUpperCase(), () => { selectedEffort = effort; render(); }, effort === selectedEffort); b.style.width = 'auto'; controls.append(b); });
                if (opt.efforts.length === 1) { const hint = document.createElement('span'); hint.className = 'sm-description'; hint.textContent = 'This provider controls effort automatically.'; controls.append(hint); }
            } else if (menu.adjustable) {
                const lower = button('− LESS', () => action('effort', { delta: -1 })); lower.style.width = 'auto';
                const value = document.createElement('span'); value.className = 'sm-effort-value'; value.textContent = (menu.effort || 'default').toUpperCase(); value.setAttribute('aria-live', 'polite');
                const higher = button('+ MORE', () => action('effort', { delta: 1 })); higher.style.width = 'auto';
                controls.append(lower, value, higher);
            } else { controls.textContent = 'This model has no adjustable effort.'; controls.classList.add('sm-description'); }
            body.append(controls);
            note.textContent = hermes && menu.stage === 'provider' ? 'Choose a provider to see its models.' : (menu.agent === 'claude' && !menu.session_only ? 'This CLI version also saves the selection as its default.' : 'Applies to this session. Your conversation stays open.');
        }
        apply.hidden = false;
        apply.disabled = ['codex', 'hermes'].includes(menu.agent) && !efforts;
    }
    agentIndicator.addEventListener('click', () => {
        if (isOpen()) return;
        target = sessionSelect.value; menu = null; query = ''; failed = false;
        body.replaceChildren(); error.textContent = ''; apply.hidden = true;
        overlay.querySelector('.sm-context').textContent = sessionSelect.selectedOptions[0]?.textContent || target;
        overlay.classList.add('show'); agentIndicator.setAttribute('aria-expanded', 'true');
        overlay.querySelector('.sm-dialog').focus();
        action('open');
    });
    cancel.addEventListener('click', () => { if (!busy) { if (failed || !menu) dismiss(); else action('cancel'); } });
    reload.addEventListener('click', () => action('open'));
    apply.addEventListener('click', () => action('apply', menu?.agent === 'eunice' ? { model: selectedModel, effort: selectedEffort } : {}));
    overlay.addEventListener('click', e => { if (e.target === overlay) cancel.click(); });
    document.addEventListener('focusin', e => {
        if (isOpen() && !overlay.contains(e.target)) overlay.querySelector('.sm-dialog').focus();
    });
    // Own keyboard input before the terminal's prefix/shortcut handlers see it.
    document.addEventListener('keydown', e => {
        if (!isOpen()) return;
        e.stopImmediatePropagation();
        if (e.key === 'Escape') { e.preventDefault(); cancel.click(); }
        if (e.key === 'Tab') {
            const focusable = [...overlay.querySelectorAll('button:not(:disabled):not([hidden]), input')];
            if (!focusable.length) { e.preventDefault(); return; }
            const index = focusable.indexOf(document.activeElement);
            if (e.shiftKey && index <= 0) { e.preventDefault(); focusable.at(-1).focus(); }
            else if (!e.shiftKey && (index < 0 || index === focusable.length - 1)) { e.preventDefault(); focusable[0].focus(); }
        }
    }, true);
})();
