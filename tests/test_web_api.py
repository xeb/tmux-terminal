"""Exercise HTTP actions against a fake tmux; never sends keys to real sessions.
Run after cargo build --release: python3 tests/test_web_api.py
"""
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
FAKE_TMUX = r'''#!/usr/bin/env python3
import json, os, sys, time
from pathlib import Path
root = Path(__file__).parent
path = root / 'state.json'
s = json.loads(path.read_text())
a = sys.argv[1:]
def pane():
    if 'capture_text' in s: return s['capture_text']
    if s['mode'] == 'queued': return (root / 'queued.txt').read_text()
    if s['mode'] == 'done': return '› Ask Codex to do anything\n  GPT-6-Astra xhigh\n'
    if s.get('text_only'):
        return (root / 'text.txt').read_text().replace('1 of 6', str(s['question']) + ' of 6').replace('Narrowest point and high near the thickest part\n  Before food', s.get('draft') or 'Type your answer')
    p = (root / 'active.txt').read_text().replace('1 of 2', str(s['question']) + ' of 2')
    if s['question'] == 2: p = p.replace(' main prompt', ' prev question')
    if s['cursor'] == 2: p = p.replace('› 1.', '  1.').replace('  3. Other', '› 3. ' + (s.get('draft') or 'Type your answer'))
    return p
if a[0] == 'display-message':
    if a[-1] == '#{pane_id}': print('%1')
    elif ';' in a:
        print('1200\t' + s.get('foreground', 'codex'))
        print(pane(), end='')
    else: print('0')
elif a[0] == 'list-windows': print('0:1\tTest' if '#{window_name}' in a[-1] else '0:1')
elif a[0] == 'capture-pane': print(pane(), end='')
elif a[0] == 'send-keys':
    keys = a[a.index('-t') + 2:]
    s['keys'].append(keys)
    if '-l' in keys:
        s['text'].append({'mode': s['mode'], 'text': keys[-1]})
        if s['mode'] == 'active':
            s['draft'] = s.get('draft', '') + keys[-1]
            s['last_paste'] = time.monotonic()
            if s.get('advance_during_paste'): s['question'] += 1

    elif keys == ['S-Left']: s['mode'] = 'active'
    elif keys == ['S-Right']:
        if s['question'] > 1: s['question'] -= 1
        else: s['mode'] = 'queued'
    elif keys == ['3']: s['cursor'] = 2
    elif s['mode'] == 'active' and keys == ['Enter'] and time.monotonic() - s.get('last_paste', 0) < .12:
        s['draft'] += '\n'
        s['early_enters'] = s.get('early_enters', 0) + 1
    elif s['mode'] == 'active' and keys == ['Enter'] and s.get('ignore_enter'):
        pass
    elif s['mode'] == 'active' and (keys in [['1'], ['2']] or keys == ['Enter']):
        s['draft'] = ''
        if s['question'] == 1: s['question'] = 2
        else: s['mode'] = 'done'
    temporary = root / ('state-' + str(os.getpid()) + '.json')
    temporary.write_text(json.dumps(s)); temporary.replace(path)
'''

with tempfile.TemporaryDirectory(prefix='tmux-terminal-api-') as directory:
    root = Path(directory)
    shutil.copytree(ROOT / 'static', root / 'static', ignore=shutil.ignore_patterns('assets'))
    (root / 'tmux').write_text(FAKE_TMUX)
    (root / 'tmux').chmod(0o755)
    for name, fixture in [('queued.txt', 'codex-queued.txt'), ('active.txt', 'codex-async.txt'), ('text.txt', 'codex-async-text.txt')]:
        # Exercise current compact chords and display-name casing throughout
        # opening, returning to the composer, answering, and normal text sends.
        pane = (ROOT / 'tests/fixtures/picker' / fixture).read_text()
        (root / name).write_text(pane.replace(' + ', '+').replace('gpt-6-astra', 'GPT-6-Astra'))
    state_file = root / 'state.json'
    state_file.write_text(json.dumps({'mode': 'queued', 'question': 1, 'cursor': 0, 'keys': [], 'text': []}))
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    env = dict(os.environ, PORT=str(port), PATH=str(root) + os.pathsep + os.environ['PATH'])
    log = (root / 'server.log').open('w+')
    server = subprocess.Popen([str(ROOT / 'target/release/tmux-terminal')], cwd=root, env=env, stdout=log, stderr=log)
    base = f'http://127.0.0.1:{port}'
    def request(path, body=None):
        req = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(), headers={'Content-Type': 'application/json'})
        try: response = urllib.request.urlopen(req, timeout=10)
        except urllib.error.HTTPError as error: response = error
        with response:
            data = response.read()
            return response.status, response.headers, json.loads(data) if 'json' in response.headers.get('Content-Type', '') else data.decode()
    try:
        for _ in range(100):
            try:
                if request('/health')[0] == 200: break
            except urllib.error.URLError: time.sleep(.05)
        else: raise AssertionError('test server did not start')
        status, headers, html = request('/')
        assert status == 200 and headers['Cache-Control'] == 'no-cache, must-revalidate'
        asset = re.search(r'href="(/assets/[^"]+)"', html)[1]
        assert request(asset)[1]['Cache-Control'] == 'public, max-age=31536000, immutable'
        assert request('/assets/00000000000000000000.missing.js')[1]['Cache-Control'] == 'no-store'
        _, headers, capture = request('/api/capture', {'target': '0:1', 'history_lines': 200})
        assert headers['Cache-Control'] == 'no-store' and capture['has_more']
        assert capture['question_queue']['count'] == 2
        assert not request('/api/capture', {'target': '0:1'})[2]['has_more']

        # A Codex conversation about Hermes must keep its Codex badge, including
        # while its own footer is missing. Python and shells need stronger proof.
        state = json.loads(state_file.read_text())
        state['capture_text'] = 'Quoted Hermes UI:\n ☤ glm-5.3-flash │ ctx --\n❯ Ask anything\n'
        for command, expected in [('codex', 'codex'), ('python', 'hermes'), ('bash', None), ('node', None)]:
            state['foreground'] = command
            state_file.write_text(json.dumps(state))
            captured = request('/api/capture', {'target': '0:1'})[2]
            assert captured.get('agent') == expected, (command, captured)
            assert captured['content'] == state['capture_text']
            assert captured['styled_content'] == state['capture_text']
        state.pop('foreground'); state.pop('capture_text')
        state_file.write_text(json.dumps(state))
        assert request('/api/window-status')[2][0]['waiting']
        status, _, opened = request('/api/picker/open', {'target': '0:1', 'fingerprint': capture['question_queue']['fingerprint']})
        assert status == 200 and opened['picker']['codex_async']
        fingerprint = opened['picker']['fingerprint']
        assert request('/api/picker/select', {'target': '0:1', 'fingerprint': 'stale', 'index': 1})[0] == 409
        status, _, chosen = request('/api/picker/select', {'target': '0:1', 'fingerprint': fingerprint, 'index': 1})
        assert status == 200 and chosen['outcome'] == 'changed'
        assert chosen['picker']['header'] == 'Question 2 of 2'
        assert request('/api/send', {'session': '0:1', 'command': 'ordinary message'})[2]['success']
        state = json.loads(state_file.read_text())
        assert state['text'] == [{'mode': 'queued', 'text': 'ordinary message'}]
        assert state['keys'][-6:] == [['S-Right'], ['S-Right'], ['-l', 'ordinary message'], ['Enter'], ['Enter'], ['Enter']]
        # Reopen and choose Other. Custom answer goes to the question editor,
        # while the preceding normal command went to the ordinary composer.
        queue = request('/api/capture', {'target': '0:1'})[2]['question_queue']
        opened = request('/api/picker/open', {'target': '0:1', 'fingerprint': queue['fingerprint']})[2]
        fp = opened['picker']['fingerprint']
        selected = request('/api/picker/select', {'target': '0:1', 'fingerprint': fp, 'index': 2})[2]
        assert selected['outcome'] == 'awaiting_text'
        keys_before = json.loads(state_file.read_text())['keys']
        assert request('/api/picker/select', {'target': '0:1', 'fingerprint': fp, 'index': 2})[2]['outcome'] == 'awaiting_text'
        assert json.loads(state_file.read_text())['keys'] == keys_before, 'reopening Other must not type its digit into the editor'
        reply = request('/api/picker/text', {'target': '0:1', 'text': 'my answer', 'fingerprint': fp})[2]
        assert reply['success'] and reply['outcome'] == 'changed', reply
        assert not json.loads(state_file.read_text()).get('early_enters')
        assert json.loads(state_file.read_text())['text'][-1] == {'mode': 'active', 'text': 'my answer'}
        assert request('/api/picker/close', {'target': '0:1'})[2]['success']
        assert json.loads(state_file.read_text())['mode'] == 'queued'
        assert request('/api/picker/text', {'target': '0:1', 'text': 'stale answer', 'fingerprint': fp})[0] == 409
        # The reported iPhone layout: six free-text questions, blank spacing
        # after the counter, and a multiline draft. Immediate Enter is modeled
        # as a paste newline, as in Codex's paste-burst handler.
        state.update(mode='active', question=1, cursor=0, text_only=True, draft='', keys=[], text=[], ignore_enter=True)
        state_file.write_text(json.dumps(state))
        capture = request('/api/capture', {'target': '0:1'})[2]
        fp = capture['picker']['fingerprint']
        assert capture['picker']['header'] == 'Question 1 of 6'
        answer = 'First paragraph\n\n1. A detail\n2. Another detail'
        status, _, reply = request('/api/picker/text', {'target': '0:1', 'text': answer, 'fingerprint': fp})
        assert status == 200 and reply['outcome'] == 'pending', reply
        state = json.loads(state_file.read_text())
        assert state['question'] == 1 and state['draft'] == answer
        assert not state.get('early_enters')
        assert state['keys'].count(['Enter']) == 1, 'no blind repeated Enter'
        state['ignore_enter'] = False
        state_file.write_text(json.dumps(state))
        status, _, reply = request('/api/picker/text', {'target': '0:1', 'text': '', 'fingerprint': fp})
        assert status == 200 and reply['outcome'] == 'changed', reply
        state = json.loads(state_file.read_text())
        assert state['question'] == 2 and len(state['text']) == 1, 'retry must not duplicate the draft'
        assert request('/api/picker/text', {'target': '0:1', 'text': answer, 'fingerprint': fp})[0] == 409
        # Last-answer completion must be observed in the main composer.
        fp = reply['picker']['fingerprint']
        reply = request('/api/picker/text', {'target': '0:1', 'text': 'Final answer', 'fingerprint': fp})[2]
        assert reply['outcome'] == 'committed', reply
        # Reload recovery submits text already in the native editor, without
        # adding a single literal keystroke or changing an unrelated command.
        state.update(mode='active', question=1, draft='Existing answer', keys=[], text=[])
        state_file.write_text(json.dumps(state))
        fp = request('/api/capture', {'target': '0:1'})[2]['picker']['fingerprint']
        assert request('/api/picker/text', {'target': '0:1', 'text': 'Different answer', 'fingerprint': fp})[0] == 409
        assert request('/api/picker/text', {'target': '0:1', 'text': '', 'fingerprint': fp})[2]['outcome'] == 'changed'
        assert json.loads(state_file.read_text())['keys'] == [['Enter']]
        # A question changing while text is settling must never receive Enter.
        state.update(mode='active', question=1, draft='', keys=[], text=[], advance_during_paste=True)
        state_file.write_text(json.dumps(state))
        fp = request('/api/capture', {'target': '0:1'})[2]['picker']['fingerprint']
        reply = request('/api/picker/text', {'target': '0:1', 'text': 'Delayed answer', 'fingerprint': fp})[2]
        assert reply['outcome'] == 'pending'
        assert ['Enter'] not in json.loads(state_file.read_text())['keys']
        print('HTTP checks passed: versioned caching, history limits, queued questions, selection, safe text submission, and stale-answer rejection.')
    except Exception:
        log.flush(); log.seek(0); print(log.read()[-3000:])
        raise
    finally:
        server.terminate()
        try: server.wait(timeout=5)
        except subprocess.TimeoutExpired: server.kill(); server.wait()
        log.close()
