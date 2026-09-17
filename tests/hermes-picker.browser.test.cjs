// Opt-in live test. Use only with the isolated backend described in
// docs/session-model-picker.md. It never starts or stops a tmux server/session.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const origin = process.env.HERMES_TEST_URL;
const target = 'hermes-verification:0';

test('Hermes native provider/model/effort picker on desktop and mobile', { skip: !origin }, async () => {
    const url = new URL(origin);
    assert.equal(url.hostname, '127.0.0.1');
    assert.notEqual(url.port, '5533', 'Never use the production backend');
    const hosts = await (await fetch(`${origin}/api/hosts`)).json();
    assert.deepEqual(hosts.hosts, [{ id: 'hermes-test' }]);
    const windows = await (await fetch(`${origin}/api/windows`)).json();
    assert.ok(windows.length && windows.every(w => w.target.startsWith('hermes-verification:')));
    assert.ok(windows.some(w => w.target === target));
    const browser = await chromium.launch({ executablePath: '/usr/bin/google-chrome', args: ['--no-sandbox'] });
    try {
        for (const [width, height, effort] of [[1280, 900, 'high'], [390, 844, 'medium']]) {
            const page = await browser.newPage({ viewport: { width, height } });
            const errors = [];
            page.on('pageerror', error => errors.push(error.message));
            await page.goto(origin);
            await page.waitForFunction(() => document.querySelector('#agentIndicator')?.textContent.includes('HERMES'));
            const modal = page.locator('#sessionModelModal');
            const option = name => modal.getByRole('button', { name, exact: true });
            const enterModels = async () => {
                await page.locator('#agentIndicator').click();
                await modal.getByRole('button', { name: /OpenRouter/ }).click();
                assert.equal(await option('APPLY').isDisabled(), true);
                await option('z-ai/glm-5.3-flash').click();
                await option('Keep current effort').waitFor();
            };
            await enterModels();
            await option('← BACK').click();
            await option('z-ai/glm-5.3-flash').click();
            await option('Keep current effort').waitFor();
            // The current choice has a "current" description in its accessible name.
            await modal.getByRole('button', { name: new RegExp(`^${effort}(\\s*current)?$`) }).click();
            const applied = page.waitForResponse(r => r.url().endsWith('/api/session-model') && r.request().postDataJSON().action === 'apply');
            await option('APPLY').click();
            assert.equal((await (await applied).json()).applied, true);
            await page.locator('#sessionModelModal.show').waitFor({ state: 'hidden' });
            const capture = await (await fetch(`${origin}/api/capture`, {
                method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ target }),
            })).json();
            const confirmation = capture.content.split('✓ Model switched: z-ai/glm-5.3-flash').at(-1);
            assert.ok(confirmation.includes(`Reasoning effort: ${effort}`));
            await enterModels();
            const cancelled = page.waitForResponse(r => r.url().endsWith('/api/session-model') && r.request().postDataJSON().action === 'cancel');
            await option('CANCEL').click();
            const result = await (await cancelled).json();
            assert.equal(result.closed, true);
            assert.equal(result.applied, undefined);
            await page.locator('#sessionModelModal.show').waitFor({ state: 'hidden' });
            assert.deepEqual(errors, []);
            await page.close();
        }
    } finally {
        await browser.close();
    }
});
