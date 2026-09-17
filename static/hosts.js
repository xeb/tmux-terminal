// Host identity stays separate from tmux targets and user-facing labels.
let hostRegistry = [{ id: 'not-invented-here' }];
let defaultHost = hostRegistry[0].id;
const hostOffline = new Set();
const hostWindows = new Map();
const hostStatuses = new Map();
const hostsReady = (async () => {
    let delay = 1000;
    for (;;) {
        try {
            const response = await fetch('/api/hosts', { signal: AbortSignal.timeout(8000) });
            if (response.status === 404) return; // Older server during an upgrade.
            if (!response.ok) throw new Error('Hosts unavailable');
            const data = await response.json();
            if (!Array.isArray(data.hosts) || !data.hosts.length) return;
            hostRegistry = data.hosts;
            defaultHost = data.default_host || data.hosts[0].id;
            return;
        } catch (_) {
            // A temporary startup outage must not permanently hide remote hosts.
            await new Promise(resolve => setTimeout(resolve, delay));
            delay = Math.min(delay * 2, 30000);
            while (document.hidden || !navigator.onLine) {
                await new Promise(resolve => setTimeout(resolve, 1000));
            }
        }
    }
})();

function windowKey(host, win) {
    const session = win.target.slice(0, win.target.lastIndexOf(':'));
    // Old-server responses have no stable window ID; retain their local keys.
    if (!win.window_id) return host === defaultHost ? win.target : `${host}::${win.target}`;
    return `${host}::${session}::${win.window_id}`;
}

function windowInfo(key) {
    const win = allWindows.find(w => w.target === key);
    if (win) return win;
    const parts = (key || '').split('::');
    const nativeTarget = parts.length > 1 ? parts[parts.length - 1] : key;
    return { host: parts.length > 1 ? parts[0] : defaultHost, nativeTarget,
        session: parts.length === 3 ? parts[1] : (nativeTarget || '').split(':')[0],
        window_id: nativeTarget?.startsWith('@') ? nativeTarget : null, target: key };
}

function normalizeWindow(host, win) {
    const [session, index] = win.target.split(':');
    return { ...win, host, nativeTarget: win.target, session, index: Number(index), target: windowKey(host, win) };
}

function hostLabel(host) {
    return host === 'not-invented-here' ? 'nih' : host;
}

function windowLabel(win) {
    return `${hostLabel(win.host)} › ${win.session} › ${win.name} (${win.index})`;
}

function hostFetch(url, options = {}, host = defaultHost) {
    return fetch(url, { ...options, headers: { ...options.headers, 'X-Tmux-Host': host } });
}

function targetFetch(url, options = {}) {
    const body = JSON.parse(options.body || '{}');
    const field = Object.hasOwn(body, 'target') ? 'target' : 'session';
    const info = windowInfo(body[field]);
    body[field] = info.window_id || info.nativeTarget;
    return hostFetch(url, { ...options, body: JSON.stringify(body) }, info.host);
}

// Each host has its own adaptive polling clock, timeout, and backoff. A slow
// remote host cannot delay painting local windows or refreshing their status.
class HostPoller {
    constructor(work, options) {
        this.running = false;
        this.ready = hostsReady.then(() => {
            this.pollers = new Map(hostRegistry.map(host => [host.id,
                new AdaptivePoller((signal, current) => work(host.id, signal, current), options)]));
            if (this.running) for (const poller of this.pollers.values()) poller.start();
        });
    }
    start() {
        this.running = true;
        if (this.pollers) for (const poller of this.pollers.values()) poller.start();
    }
    stop() {
        this.running = false;
        if (this.pollers) for (const poller of this.pollers.values()) poller.stop();
    }
    async request(host) {
        await this.ready;
        if (host) return this.pollers.get(host)?.request();
        return Promise.all([...this.pollers.values()].map(p => p.request()));
    }
}
