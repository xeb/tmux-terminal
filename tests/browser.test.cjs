// Optional browser checks: see docs/mobile-performance.md for setup.
const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const { chromium, webkit, devices } = require('playwright');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
let server, origin;
before(async () => {
    server = http.createServer((req, res) => {
        const filename = path.join(process.cwd(), 'static', req.url === '/' ? 'index.html' : req.url);
        try {
            const content = fs.readFileSync(filename);
            const type = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2' }[path.extname(filename)];
            res.writeHead(200, { 'Content-Type': type || 'application/octet-stream' });
            res.end(content);
        } catch { res.writeHead(404); res.end(); }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    origin = `http://127.0.0.1:${server.address().port}`;
});
after(() => new Promise(resolve => server.close(resolve)));

for (const engine of ['chromium', 'webkit']) {
    test(`${engine}: mobile layout, recovery, history and incremental rendering`, async t => {
        const browser = await (engine === 'webkit' ? webkit.launch() : chromium.launch({ executablePath: '/usr/bin/google-chrome', args: ['--no-sandbox'] }));
        t.after(() => browser.close());
        const context = await browser.newContext({ ...devices['iPhone 13'] });
        const page = await context.newPage();
        const errors = [];
        page.on('pageerror', error => errors.push(error.message));
        let windowAttempts = 0;
        const histories = [];
        let questionsPending = false, questionsOpen = false, questionNumber = 1;
        let sentCommand = null;
        let freeText = false, nativeDraft = '', textOutcome = 'pending';
        const replies = [];
        let holdTextReply = false, releaseTextReply;
        const question = () => ({
            codex_async: true, fingerprint: 'question-' + questionNumber,
            header: `Question ${questionNumber} of 2`, question: 'Which option should we use?',
            text_only: freeText, answer_draft: nativeDraft || undefined,
            cursor: 0, layout: 'list', options: freeText ? [{ number: null, label: 'Write an answer', is_meta: true }] : [
                { number: 1, label: 'First option (Recommended)', is_meta: false },
                { number: 2, label: 'Second option', is_meta: false },
                { number: 3, label: 'Other (write an answer)', is_meta: true },
            ],
        });
        const lines = Array.from({ length: 1100 }, (_, i) => `line ${i}: terminal output`);
        await page.route('**/api/**', async route => {
            const endpoint = new URL(route.request().url()).pathname;
            let body;
            if (endpoint === '/api/windows') {
                if (++windowAttempts === 1) return route.fulfill({ status: 503, body: 'Temporary failure' });
                body = [{ target: '0:1', name: 'A' }, { target: '0:2', name: 'B' }];
            } else if (endpoint === '/api/capture') {
                const { target, history_lines } = route.request().postDataJSON();
                histories.push(history_lines);
                body = { content: lines.slice(-history_lines).join('\n') + '\n' + target, has_more: history_lines < 1000, agent: 'codex' };
                if (questionsPending && target === '0:2') {
                    if (questionsOpen) body.picker = question();
                    else body.question_queue = { count: 2, fingerprint: 'queue' };
                }
            } else if (endpoint === '/api/picker/open') {
                questionsOpen = true;
                body = { success: true, picker: question() };
            } else if (endpoint === '/api/picker/close') {
                questionsOpen = false;
                body = { success: true };
            } else if (endpoint === '/api/picker/select') {
                if (questionNumber++ === 1) body = { success: true, outcome: 'changed', picker: question() };
                else { questionsPending = false; body = { success: true, outcome: 'committed' }; }
            } else if (endpoint === '/api/picker/text') {
                replies.push(route.request().postDataJSON());
                nativeDraft ||= replies.at(-1).text;
                if (textOutcome === 'pending') body = { success: true, outcome: 'pending', picker: question() };
                else {
                    questionNumber++;
                    nativeDraft = '';
                    body = { success: true, outcome: 'changed', picker: question() };
                }
                if (holdTextReply) await new Promise(resolve => { releaseTextReply = resolve; });
            } else if (endpoint === '/api/send') {
                sentCommand = route.request().postDataJSON().command;
                questionsOpen = false;
                body = { success: true };
            } else if (endpoint === '/api/config') body = { large_mode: false };
            else if (endpoint === '/api/window-status') body = [{ target: '0:2', waiting: true }];
            else if (endpoint === '/api/project-dirs') body = { dirs: [] };
            else body = [];
            return route.fulfill({ json: body });
        });
        await page.goto(origin);
        await page.waitForFunction(() => document.querySelector('.terminal-line'), { timeout: 8000 });
        assert.ok(windowAttempts >= 2, 'automatically recovered from initial 503');
        assert.equal(histories[0], 200);
        assert.equal(await page.evaluate(() => document.activeElement.id), '', 'no keyboard autofocus');

        // Overflow checks with every badge present, at small widths and while a
        // software keyboard leaves only 300 CSS pixels of visible height.
        for (const [width, height] of [[320, 568], [390, 844], [390, 300], [844, 300]]) {
            await page.setViewportSize({ width, height });
            await page.evaluate(() => {
                waitingPill.classList.add('show'); busyPill.classList.add('show');
                busyPill.textContent = '◆ 12';
                updateWorking('✻ Working… (12s · ↓ 2 tokens)');
            });
            await page.waitForTimeout(60);
            const layout = await page.evaluate(() => {
                const ids = ['sessionSelect', 'waitingPill', 'busyPill', 'agentIndicator', 'newWindowBtn', 'menuBtn', 'commandInput', 'sendBtn', 'outputContent'];
                return ids.map(id => {
                    const element = document.getElementById(id), r = element.getBoundingClientRect();
                    return { id, x: r.x, y: r.y, right: r.right, bottom: r.bottom, width: r.width, height: r.height };
                });
            });
            for (const r of layout) {
                assert.ok(r.x >= -1 && r.right <= width + 1, `${width}×${height}: ${r.id} horizontal clipping ${JSON.stringify(r)}`);
                assert.ok(r.y >= -1 && r.bottom <= height + 1, `${width}×${height}: ${r.id} vertical clipping ${JSON.stringify(r)}`);
            }
            await page.locator('#menuBtn').click();
            const menu = page.locator('.action-menu');
            const box = await menu.boundingBox();
            assert.ok(box.y >= 0 && box.y + box.height <= height + 1);
            await page.locator('#menuLogout').scrollIntoViewIfNeeded();
            assert.ok(await page.locator('#menuLogout').isVisible());
            await page.evaluate(() => closeActionMenu());
        }
        await page.setViewportSize({ width: 390, height: 844 });
        await page.evaluate(() => { outputContent.scrollTop = 0; });
        await page.waitForFunction(() => historyLines >= 400 && !historyLoading);
        assert.ok(histories.includes(400), 'scrolling back loads more history');
        assert.ok(await page.evaluate(() => outputContent.scrollTop > 0), 'prepended history preserves reading position');
        await page.selectOption('#sessionSelect', '0:2');
        await page.waitForFunction(() => outputContent.textContent.endsWith('0:2'));
        assert.equal(histories.at(-1), 200, 'window switch resets history limit');

        questionsPending = true;
        await page.evaluate(() => captureOutput());
        await page.locator('#questionsBtn').waitFor({ state: 'visible' });
        await page.locator('#commandInput').fill('Keep this normal command draft');
        await page.locator('#questionsBtn').click();
        await page.locator('#pickerCard.show').waitFor();
        await page.screenshot({ path: `/tmp/tmux-terminal-questions-${engine}.png` });
        assert.equal(await page.locator('#commandInput').inputValue(), 'Keep this normal command draft');
        await page.locator('.picker-opt[data-idx="1"]').click();
        await page.locator('#pickerSend').click();
        await page.waitForFunction(() => pickerData?.header === 'Question 2 of 2');
        assert.equal(await page.locator('#commandInput').inputValue(), 'Keep this normal command draft');
        await page.locator('#pickerCancel').click();
        await page.locator('#questionsBtn').waitFor({ state: 'visible' });
        assert.equal(await page.locator('#commandInput').inputValue(), 'Keep this normal command draft');
        await page.locator('#questionsBtn').click();
        await page.locator('#pickerCard.show').waitFor();
        await page.locator('#sendBtn').click();
        await page.waitForFunction(() => commandInput.value === '');
        assert.equal(sentCommand, 'Keep this normal command draft');
        assert.equal(questionNumber, 2, 'normal command did not answer a question');
        await page.evaluate(() => captureOutput());
        await page.locator('#questionsBtn').waitFor({ state: 'visible' });
        await page.locator('#questionsBtn').click();
        await page.locator('#pickerSend').click();
        await page.waitForFunction(() => pickerData === null);

        // Free-text questions open directly into their own answer field. A
        // pending submit keeps the text, while a retry only submits the native
        // draft. Polling must move on when the next question is observed.
        freeText = true;
        questionsPending = true;
        questionsOpen = false;
        questionNumber = 1;
        await page.evaluate(() => captureOutput());
        await page.locator('#commandInput').fill('Ordinary draft stays separate');
        await page.locator('#questionsBtn').click();
        await page.locator('#pickerTextInput').waitFor({ state: 'visible' });
        assert.equal(await page.locator('#pickerSend').isVisible(), false);
        assert.equal(await page.locator('#pickerCancel').isVisible(), true);
        await page.locator('#pickerTextInput').fill('First line\nSecond line');
        await page.locator('#pickerTextInput').press('Enter');
        await page.waitForFunction(() => !sendingText && document.querySelector('#pickerTextInput').readOnly);
        assert.equal(await page.locator('#pickerTextInput').inputValue(), 'First line\nSecond line');
        assert.equal(replies.length, 1);
        assert.equal(replies[0].text, 'First line\nSecond line');
        assert.equal(replies[0].fingerprint, 'question-1');
        assert.match(await page.locator('#status').textContent(), /Not submitted yet/);
        textOutcome = 'changed';
        await page.locator('#pickerTextSend').click();
        await page.waitForFunction(() => pickerData?.header === 'Question 2 of 2');
        assert.equal(replies[1].text, '', 'retry submits existing draft without retyping');
        assert.equal(await page.locator('#pickerTextInput').inputValue(), '');
        assert.equal(await page.locator('#commandInput').inputValue(), 'Ordinary draft stays separate');
        // A native draft left by the original bug is recoverable after reload.
        nativeDraft = 'Answer already in Codex';
        await page.reload();
        await page.locator('#pickerBar.show').waitFor();
        await page.locator('#pickerBar').click();
        await page.locator('#pickerTextInput').waitFor({ state: 'visible' });
        assert.equal(await page.locator('#pickerTextInput').inputValue(), nativeDraft);
        assert.equal(await page.locator('#pickerTextInput').getAttribute('readonly'), '');
        await page.locator('#pickerTextSend').click();
        await page.waitForFunction(() => pickerData?.fingerprint === 'question-3');
        assert.equal(replies[2].text, '');
        // Polling can show the next question before the action response
        // arrives. That late response must not clear the new draft.
        holdTextReply = true;
        await page.locator('#pickerTextInput').fill('Submit this answer');
        await page.locator('#pickerTextSend').click();
        for (let n = 0; n < 100 && !releaseTextReply; n++) await new Promise(resolve => setTimeout(resolve, 10));
        assert.ok(releaseTextReply);
        await page.evaluate(() => captureOutput());
        await page.waitForFunction(() => pickerData?.fingerprint === 'question-4');
        await page.locator('#pickerTextInput').fill('Keep the next answer draft');
        releaseTextReply();
        holdTextReply = false;
        await page.waitForFunction(() => !sendingText);
        assert.equal(await page.locator('#pickerTextInput').inputValue(), 'Keep the next answer draft');
        // Likewise, switching tmux windows while an answer is being confirmed
        // must leave the newly viewed window and its normal text alone.
        holdTextReply = true;
        releaseTextReply = null;
        await page.locator('#pickerTextSend').click();
        for (let n = 0; n < 100 && !releaseTextReply; n++) await new Promise(resolve => setTimeout(resolve, 10));
        assert.ok(releaseTextReply);
        await page.selectOption('#sessionSelect', '0:1');
        await page.locator('#commandInput').fill('Draft in a different window');
        releaseTextReply();
        holdTextReply = false;
        await page.waitForFunction(() => !sendingText);
        assert.equal(await page.locator('#commandInput').inputValue(), 'Draft in a different window');
        await page.selectOption('#sessionSelect', '0:2');
        // External advancement also dismisses old text mode, including while
        // another terminal has submitted the final answer.
        questionNumber = 6;
        await page.evaluate(() => captureOutput());
        await page.waitForFunction(() => pickerData?.fingerprint === 'question-6');
        questionsPending = false;
        await page.evaluate(() => captureOutput());
        await page.waitForFunction(() => pickerData === null);

        const render = await page.evaluate(async () => {
            pausePolling();
            hasMoreHistory = false;
            updateHistoryButton();
            terminalRenderer.reset();
            const lines = Array.from({ length: 1000 }, (_, i) => `\x1b[38;5;${i % 16}mrow ${i}: https://example.com/row/${i}\x1b[0m`);
            terminalRenderer.render(lines.join('\n'), true);
            const original = [...outputContent.children];
            const observer = new MutationObserver(() => {});
            observer.observe(outputContent, { childList: true, subtree: true, characterData: true });
            lines[999] = 'changed timer';
            terminalRenderer.render(lines.join('\n'));
            const mutations = observer.takeRecords().length;
            const retained = original.filter(node => node.isConnected).length;
            terminalRenderer.render(lines.join('\n'));
            const unchangedMutations = observer.takeRecords().length;
            outputContent.scrollTop = outputContent.children[400].offsetTop;
            const anchor = outputContent.children[400];
            const before = anchor.getBoundingClientRect().top;
            terminalRenderer.render('older line\n' + lines.join('\n'));
            const drift = Math.abs(anchor.getBoundingClientRect().top - before);
            const link = outputContent.querySelector('a').getAttribute('href');
            terminalRenderer.render('\x1b[31mred\nstill red\x1b[0m\n<script>alert(1)</script>');
            const inheritedColor = outputContent.children[1].querySelector('span').style.color;
            const escaped = outputContent.querySelector('script') === null;
            terminalRenderer.render('');
            const cleared = outputContent.children.length === 0;
            observer.disconnect();
            return { mutations, retained, unchangedMutations, drift, link, inheritedColor, escaped, cleared };
        });
        assert.equal(render.retained, 999);
        assert.ok(render.mutations <= 2, JSON.stringify(render));
        assert.equal(render.unchangedMutations, 0);
        assert.ok(render.drift < 2, JSON.stringify(render));
        assert.equal(render.link, 'https://example.com/row/0');
        assert.ok(render.inheritedColor);
        assert.ok(render.escaped && render.cleared);

        // Simulate WebKit's visual viewport shrinking/panning independently of
        // the layout viewport, which desktop device emulation doesn't do.
        await page.evaluate(() => {
            Object.defineProperty(window, 'visualViewport', { configurable: true, value: { height: 280, offsetTop: 30, scale: 1 } });
            answerModeTargets.add(sessionSelect.value);
            updatePicker(sessionSelect.value, {
                codex_async: true, fingerprint: 'keyboard-question', header: 'Question 1 of 2',
                question: 'Which option should we use?', cursor: 0, layout: 'list',
                options: [{number: 1, label: 'First option', is_meta: false}, {number: 2, label: 'Other', is_meta: true}],
            });
            updateViewport();
        });
        await page.waitForTimeout(50);
        const viewport = await page.evaluate(() => {
            const r = document.body.getBoundingClientRect();
            return { height: r.height, top: r.top };
        });
        assert.equal(viewport.height, 280);
        assert.equal(viewport.top, 30);
        const controls = await page.evaluate(() => ['pickerCard', 'pickerCancel', 'commandInput', 'sendBtn', 'outputContent'].map(id => {
            const r = document.getElementById(id).getBoundingClientRect();
            return { id, top: r.top, bottom: r.bottom, height: r.height };
        }));
        for (const r of controls) assert.ok(r.top >= 29 && r.bottom <= 311 && r.height > 0, JSON.stringify(r));
        assert.ok(controls[1].bottom <= controls[0].bottom + 1, 'Back to typing remains visible above the command box');
        await page.screenshot({ path: `/tmp/tmux-terminal-keyboard-${engine}.png` });
        await page.evaluate(() => {
            updatePicker(sessionSelect.value, {
                codex_async: true, text_only: true, fingerprint: 'keyboard-text-question', header: 'Question 1 of 6',
                question: 'Which locations did you measure, and were these taken before food and training?',
                cursor: 0, layout: 'list', options: [{number: null, label: 'Write an answer', is_meta: true}],
            });
        });
        await page.locator('#pickerTextInput').fill('Answer with keyboard open');
        const textControls = await page.evaluate(() => ['pickerTextInput', 'pickerTextSend', 'pickerCancel', 'commandInput', 'sendBtn'].map(id => {
            const r = document.getElementById(id).getBoundingClientRect();
            const parent = document.getElementById(id).parentElement.getBoundingClientRect();
            return { id, top: r.top, bottom: r.bottom, height: r.height, parentTop: parent.top, parentBottom: parent.bottom };
        }));
        for (const r of textControls) {
            assert.ok(r.top >= 29 && r.bottom <= 311 && r.height > 0, JSON.stringify(r));
            assert.ok(r.top >= r.parentTop && r.bottom <= r.parentBottom + 1, `text answer control clipped: ${JSON.stringify(r)}`);
        }
        await page.screenshot({ path: `/tmp/tmux-terminal-keyboard-text-${engine}.png` });
        const stopped = await page.evaluate(() => {
            window.dispatchEvent(new Event('offline'));
            return [capturePoller, statusPoller, windowsPoller].every(p => !p.running);
        });
        assert.ok(stopped);
        assert.deepEqual(errors, []);
    });
}
