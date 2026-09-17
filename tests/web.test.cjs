const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const flush = () => new Promise(resolve => setImmediate(resolve));

test('Hermes token-count footer clears stale working status after a turn', () => {
    const source = fs.readFileSync('static/app.js', 'utf8');
    const context = vm.createContext({});
    vm.runInContext(source.slice(source.indexOf('const HERMES_SPINNER'), source.indexOf('function isEuniceFooter')) +
        source.slice(source.indexOf('function isHermesStatus'), source.indexOf('function parseCodexWorking')), context);
    const idle = fs.readFileSync('tests/fixtures/hermes/idle_after_turn.txt', 'utf8').trimEnd().split('\n');
    const busy = ['( •_•)>⌐■-■ mulling... ⏱ 4s', '☤ ❯ msg=interrupt · /queue · /bg · /steer · Ctrl+C cancel'];
    assert.equal(context.parseHermesWorking(idle), null);
    assert.equal(context.parseHermesWorking([...busy, ...idle]), null);
    assert.equal(context.parseHermesWorking([...idle, ...busy]).verb, 'Mulling');
    assert.equal(context.isHermesStatus(busy[1]), false);
    assert.equal(context.isHermesStatus('╭─ ☤ Hermes ───╮'), false);
});

test('white and cream ANSI foregrounds render black without changing terminal state', () => {
    const context = vm.createContext({});
    vm.runInContext(fs.readFileSync('static/terminal.js', 'utf8') +
        '\nglobalThis.render = renderTerminalOutput; globalThis.state = freshAnsiState();', context);
    for (const sgr of ['37', '97', '38;5;7', '38;5;15', '38;5;231', '38;5;230', '38;5;250', '38;2;255;255;255', '38;2;248;248;240']) {
        assert.match(context.render(`\x1b[${sgr}mtext`), /color:#000[;"]/);
    }
    // The house status bar and reverse-video selections must not become
    // black-on-black when their white foreground is remapped.
    assert.match(context.render('\x1b[38;5;250;48;5;234mstatus'), /color:#000;background-color:rgb\(227,227,227\)/);
    assert.match(context.render('\x1b[40mdefault'), /color:#000;background-color:rgb\(238,238,238\)/);
    assert.match(context.render('\x1b[7mreverse'), /color:#000;background-color:var\(--terminal-bg\)/);
    assert.match(context.render('\x1b[30;107;7mreverse'), /color:#000;background-color:rgb\(238,238,238\)/);
    assert.match(context.render('\x1b[30;107mlight background'), /color:rgb\(17,17,17\);background-color:rgb\(255,255,255\)/);
    assert.match(context.render('\x1b[38;5;240mgray'), /color:rgb\(88,88,88\)/);
    assert.match(context.render('\x1b[97mfirst', context.state), /color:#000/);
    assert.equal(context.state.fg, 'rgb(255,255,255)');
    assert.match(context.render('next line', context.state), /color:#000/);
    assert.equal(context.render('\x1b[0mreset', context.state), 'reset');
});

function harness() {
    let now = 0, id = 0;
    const timers = new Map();
    const context = vm.createContext({
        AbortController,
        setTimeout(fn, delay) { timers.set(++id, { fn, at: now + delay }); return id; },
        clearTimeout(id) { timers.delete(id); },
    });
    vm.runInContext(fs.readFileSync('static/terminal.js', 'utf8') + '\nglobalThis.Poller = AdaptivePoller;', context);
    return {
        Poller: context.Poller,
        async advance(ms) {
            const end = now + ms;
            while (true) {
                const due = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
                if (!due) break;
                now = due[1].at;
                timers.delete(due[0]);
                due[1].fn();
                await flush();
            }
            now = end;
            await flush();
        },
    };
}

test('slow captures never overlap; bursts coalesce and await a fresh result', async () => {
    const { Poller, advance } = harness();
    const requests = [];
    const poller = new Poller(() => new Promise(resolve => requests.push(resolve)));
    poller.start();
    await flush();
    let completed = 0;
    poller.request().then(() => completed++);
    poller.request().then(() => completed++);
    await advance(2000);
    assert.equal(requests.length, 1);
    requests[0](true);
    await flush();
    assert.equal(completed, 0);
    await advance(0);
    assert.equal(requests.length, 2);
    requests[1](false);
    await flush();
    assert.equal(completed, 2);
    await advance(4999);
    assert.equal(requests.length, 2);
    await advance(1);
    assert.equal(requests.length, 3);
    poller.stop();
    requests[2](false);
});

test('responses from a stopped generation cannot repaint a newly selected target', async () => {
    const { Poller, advance } = harness();
    const pending = [];
    const painted = [];
    let target = 'A';
    const poller = new Poller(async (signal, current) => {
        const capturedTarget = target;
        await new Promise(resolve => pending.push({ resolve, signal }));
        if (current()) painted.push(capturedTarget);
        return false;
    });
    poller.start();
    await flush();
    target = 'B';
    poller.start();
    assert.equal(pending[0].signal.aborted, true);
    pending[0].resolve();
    await flush();
    await advance(0);
    pending[1].resolve();
    await flush();
    assert.deepEqual(painted, ['B']);
    poller.stop();
});

test('failed startup retries, backs off, and pauses until resumed', async () => {
    const { Poller, advance } = harness();
    let attempts = 0;
    const poller = new Poller(async () => {
        attempts++;
        if (attempts < 3) throw new Error('offline');
        return true;
    }, { active: 1000, idle: 3000, maximum: 6000 });
    poller.start();
    await flush();
    await advance(2999);
    assert.equal(attempts, 1);
    await advance(1);
    assert.equal(attempts, 2);
    poller.stop();
    await advance(60000);
    assert.equal(attempts, 2);
    poller.start();
    await flush();
    assert.equal(attempts, 3);
    await advance(1000);
    assert.equal(attempts, 4);
    poller.stop();
});

test('a hung request times out and does not accumulate more requests', async () => {
    const { Poller, advance } = harness();
    let attempts = 0;
    const poller = new Poller(signal => new Promise((resolve, reject) => {
        attempts++;
        signal.addEventListener('abort', () => reject(new Error('aborted')), { once: true });
    }));
    poller.start();
    await flush();
    await advance(9999);
    assert.equal(attempts, 1);
    await advance(1);
    await advance(3000);
    assert.equal(attempts, 2);
    poller.stop();
});
