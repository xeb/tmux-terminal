const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const flush = () => new Promise(resolve => setImmediate(resolve));

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
