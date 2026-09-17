const { test } = require('node:test');
const assert = require('node:assert/strict');
const { chromium, devices } = require('playwright');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');

test('host selection, creation default, isolated uploads, offline recovery and mobile layout', async t => {
    const server = http.createServer((req, res) => {
        const filename = path.join(process.cwd(), 'static', req.url === '/' ? 'index.html' : req.url);
        try {
            res.setHeader('Content-Type', { '.html':'text/html', '.js':'text/javascript', '.css':'text/css' }[path.extname(filename)] || 'application/octet-stream');
            res.end(fs.readFileSync(filename));
        } catch { res.writeHead(404); res.end(); }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    t.after(() => new Promise(resolve => server.close(resolve)));
    const browser = await chromium.launch({ executablePath: '/usr/bin/google-chrome', args:['--no-sandbox'] });
    t.after(() => browser.close());
    const context = await browser.newContext({ ...devices['iPhone 13'] });
    const page = await context.newPage();
    const errors = [], requests = [];
    page.on('pageerror', e => errors.push(e.message));
    let releaseRemote, releaseUpload, remoteOffline = false, created = false, hostAttempts = 0;
    const remoteReady = new Promise(resolve => { releaseRemote = resolve; });
    const uploadReady = new Promise(resolve => { releaseUpload = resolve; });
    let holdProjects, releaseProjects;
    await page.route('**/api/**', async route => {
        const request = route.request(), url = new URL(request.url());
        const host = request.headers()['x-tmux-host'] || url.searchParams.get('host') || 'not-invented-here';
        let payload;
        if (request.postData() && url.pathname !== '/api/upload') payload = request.postDataJSON();
        requests.push({ path:url.pathname, host, payload, url });
        if (remoteOffline && host === 'vade') return route.fulfill({ status:503, json:{error:'vade offline',offline:true} });
        let body = {};
        switch (url.pathname) {
            case '/api/hosts':
                if (++hostAttempts === 1) return route.fulfill({status:503,json:{error:'starting'}});
                body = {default_host:'not-invented-here',hosts:[{id:'not-invented-here'},{id:'vade'}]}; break;
            case '/api/windows':
                if (host === 'vade') await remoteReady;
                body = [{target:'0:1',name:'same-name',window_id:'@1'}, {target:'other:2',name:'other',window_id:'@2'}];
                if (host === 'vade' && created) body.push({target:'0:3',name:'new-project',window_id:'@3'});
                break;
            case '/api/capture': body = {content:host+' output /tmp/example.txt',agent:'codex'}; break;
            case '/api/window-status': body = [{target:'0:1',waiting:true}]; break;
            case '/api/project-dirs':
                if (host === 'vade' && holdProjects) await holdProjects;
                body = {dirs:[{name:host === 'vade' ? 'remote-project' : 'local-project',mtime:10}]}; break;
            case '/api/agents': body = {agents:host === 'vade' ? ['codex','eunice'] : ['claude','codex','agy','eunice']}; break;
            case '/api/new-window-named': created = true; body = {success:true,target:'0:3',window_id:'@3',agent:'codex'}; break;
            case '/api/upload': await uploadReady; body = {success:true,path:'/remote/uploaded.txt',name:'uploaded.txt'}; break;
            case '/api/serve-file': return route.fulfill({ body:'remote file' });
            default: body = {success:true};
        }
        return route.fulfill({json:body});
    });
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => sessionSelect.value === 'not-invented-here::0::@1');
    assert.ok(await page.locator('#outputContent').textContent().then(t => t.includes('not-invented-here')),
        'Local output renders while remote discovery is pending');
    releaseRemote();
    await page.waitForFunction(() => allWindows.length === 4);
    assert.equal(hostAttempts, 2, 'Host discovery recovers from an initial outage');
    const labels = await page.locator('#sessionSelect option').allTextContents();
    assert.ok(labels.some(s => s.includes('nih › 0 › same-name')));
    assert.ok(labels.some(s => s.includes('vade › 0 › same-name')));
    await page.selectOption('#sessionSelect', 'vade::0::@1');
    await page.waitForFunction(() => outputContent.dataset.host === 'vade');
    await page.evaluate(async () => { commandInput.value = 'remote command'; await sendCommand(); });
    const sent = requests.find(r => r.path === '/api/send');
    assert.equal(sent.host, 'vade');
    assert.equal(sent.payload.session, '@1');
    await page.evaluate(() => openFileModal('/tmp/example.txt'));
    assert.equal(requests.find(r => r.path === '/api/serve-file').host, 'vade');
    await page.evaluate(() => closeFileModal());

    await page.locator('#newWindowBtn').click();
    assert.equal(await page.evaluate(() => nwHost), 'not-invented-here');
    await page.locator('#nwHostPick button', {hasText:'vade'}).click();
    await page.waitForFunction(() => projectDirs[0]?.name === 'remote-project' && nwCapabilities.has('vade'));
    assert.equal(await page.getByRole('radio', {name:'CLAUDE',exact:true}).isDisabled(), true);
    assert.equal(await page.getByRole('radio', {name:'CODEX',exact:true}).isDisabled(), false);
    for (const [width,height] of [[320,568],[390,300],[844,300]]) {
        await page.setViewportSize({width,height});
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
        assert.equal(overflow, false, `Host controls fit ${width}×${height}`);
    }
    await page.setViewportSize({width:390,height:844});
    await page.evaluate(async () => { closeNewWindowModal(); await submitNewWindow('new-project'); });
    assert.equal(requests.find(r => r.path === '/api/new-window-named').host, 'vade');
    assert.equal(await page.evaluate(() => sessionSelect.value), 'vade::0::@3');
    await page.evaluate(() => createNewWindow());
    assert.equal(await page.evaluate(() => nwHost), 'not-invented-here', 'Host override resets each time');

    // A remote suggestion response must not overwrite the newly selected host.
    holdProjects = new Promise(resolve => { releaseProjects = resolve; });
    await page.locator('#nwHostPick button', {hasText:'vade'}).click();
    await page.locator('#nwHostPick button', {hasText:'nih'}).click();
    releaseProjects();
    await page.waitForFunction(() => projectDirs[0]?.name === 'local-project');
    await page.evaluate(() => closeNewWindowModal());

    await page.selectOption('#sessionSelect', 'vade::0::@1');
    await page.evaluate(() => { uploadPickerTarget = sessionSelect.value; commandInput.value = ''; });
    await page.locator('#fileInput').setInputFiles({name:'uploaded.txt',mimeType:'text/plain',buffer:Buffer.from('remote data')});
    await page.waitForFunction(() => document.querySelector('.upload-item.uploading'));
    await page.evaluate(() => closeUploadModal());
    await page.selectOption('#sessionSelect', 'not-invented-here::0::@1');
    releaseUpload();
    await page.waitForFunction(() => document.querySelector('.upload-item.done'));
    assert.equal(await page.locator('#commandInput').inputValue(), '', 'Remote path is not inserted into another host’s composer');
    const upload = requests.find(r => r.path === '/api/upload');
    assert.equal(upload.host,'vade'); assert.equal(upload.url.searchParams.get('target'),'@1');

    await page.selectOption('#sessionSelect', 'vade::0::@1');
    remoteOffline = true;
    await page.evaluate(() => windowsPoller.request('vade'));
    assert.equal(await page.evaluate(() => sessionSelect.value), 'vade::0::@1');
    assert.match(await page.locator('#sessionSelect option:checked').textContent(), /OFFLINE/);
    const before = requests.filter(r => r.path === '/api/windows' && r.host === 'not-invented-here').length;
    await page.evaluate(() => windowsPoller.request('not-invented-here'));
    assert.ok(requests.filter(r => r.path === '/api/windows' && r.host === 'not-invented-here').length > before);
    remoteOffline = false;
    await page.evaluate(() => windowsPoller.request('vade'));
    assert.equal(await page.evaluate(() => sessionSelect.value), 'vade::0::@1');
    assert.doesNotMatch(await page.locator('#sessionSelect option:checked').textContent(), /OFFLINE/);
    assert.deepEqual(errors, []);
});
