"""HTTP integration with independent fake local/SSH hosts and real file writes.

Never touches a real tmux server, SSH host, or project directory.
Run after cargo build --release: python3 tests/test_multi_host.py
"""
import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
TMUX = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
root = Path(os.environ['TEST_HOST_ROOT']) / os.environ.get('TEST_HOST', 'local')
a = sys.argv[1:]
with (root / 'events').open('a') as f: f.write(json.dumps(a) + '\n')
state = root / 'state.json'
windows = json.loads(state.read_text())
target = a[a.index('-t') + 1] if '-t' in a else None
def win():
    if target and target.startswith('%'): return windows[0]
    return next((w for w in windows if target in (w['id'], w['target'])), None)
def field(w, fmt):
    session, index = w['target'].rsplit(':', 1)
    for key,value in {'window_id':w['id'], 'window_name':w['name'], 'session_name':session, 'window_index':index}.items():
        fmt = fmt.replace('#{' + key + '}', value)
    return fmt
if a[0] == 'list-windows':
    for w in windows: print(field(w, a[-1]))
elif a[0] == 'has-session':
    sys.exit(0 if any(w['target'].split(':')[0] == target.lstrip('=') for w in windows) else 1)
elif a[0] in ('new-window', 'new-session'):
    session = a[a.index('-s')+1] if a[0] == 'new-session' else target.lstrip('=').rstrip(':')
    w = dict(id='@9', target=session+':9', name=a[a.index('-n')+1] if '-n' in a else 'new', cwd=a[a.index('-c')+1] if '-c' in a else str(root/'work'))
    windows.append(w); state.write_text(json.dumps(windows)); print(field(w,a[-1]))
elif a[0] == 'display-message':
    w = win()
    if not w: print('no such window', file=sys.stderr); sys.exit(1)
    if ';' in a: print('0\tcodex\n' + os.environ.get('TEST_HOST','local') + ' output')
    elif a[-1] == '#{pane_current_path}': print(w['cwd'])
    elif a[-1] == '#{pane_id}': print('%1')
    else: print('%1\tcodex\t0\t0\t0\t\t')
elif a[0] == 'capture-pane':
    if not win(): sys.exit(1)
    print(os.environ.get('TEST_HOST','local') + ' output')
elif a[0] in ('send-keys','rename-window','kill-window','swap-window','set-option'):
    pass
else:
    raise RuntimeError(a)
'''
SSH = r'''#!/usr/bin/env python3
import base64, json, os, shlex, subprocess, sys, time
from pathlib import Path
root = Path(os.environ['TEST_HOST_ROOT'])
if (root / 'offline').exists(): sys.exit(255)
if (root / 'fail-once').exists():
    (root / 'fail-once').unlink()
    sys.exit(255)
if (root / 'slow').exists(): time.sleep(1)
with (root / 'ssh-events').open('a') as f:
    # Direct SSH must carry control characters as encoded data. In particular,
    # literal tabs in tmux's -F argument must survive the remote shell launch.
    assert '\t' not in sys.argv[-1] and '\n' not in sys.argv[-1]
    argv = shlex.split(sys.argv[-1])
    assert argv[:2] == ['python3', '-c'] and 'os.execvp' in argv[2]
    f.write(json.dumps(json.loads(base64.b64decode(argv[3]))) + '\n')
env = dict(os.environ, TEST_HOST='remote', HOME=str(root/'remote'/'home'))
sys.exit(subprocess.call(shlex.split(sys.argv[-1]), env=env))
'''
BASH = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
root = Path(os.environ['TEST_HOST_ROOT']) / os.environ.get('TEST_HOST', 'local')
with (root / 'events').open('a') as f: f.write(json.dumps(['bash']+sys.argv[1:])+'\n')
if '__TMUX_AGENT__' in sys.argv[-1]: print('__TMUX_AGENT__codex\n__TMUX_AGENT__eunice')
'''


with tempfile.TemporaryDirectory(prefix='tmux-host-tests-') as directory:
    root = Path(directory)
    for host in ('local', 'remote'):
        base = root / host
        (base / 'home' / 'p' / 'shared').mkdir(parents=True)
        (base / 'home' / 'p' / 'shared' / 'CLAUDE.md').write_text('instructions')
        (base / 'work').mkdir()
        (base / 'events').touch()
        (base / 'state.json').write_text(json.dumps([
            dict(id='@1', target='0:1', name='terminal', cwd=str(base/'work')),
            dict(id='@2', target='other:2', name='shared', cwd=str(base/'work')),
        ]))
    bin_dir = root / 'bin'
    bin_dir.mkdir()
    for name, script in [('tmux', TMUX), ('ssh', SSH), ('bash', BASH)]:
        (bin_dir/name).write_text(script)
        (bin_dir/name).chmod(0o755)
    shutil.copytree(ROOT/'static', root/'static', ignore=shutil.ignore_patterns('assets'))
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    env = dict(os.environ, TEST_HOST_ROOT=str(root), HOME=str(root/'local'/'home'),
               PATH=str(bin_dir)+os.pathsep+os.environ['PATH'], PORT=str(port),
               TMUX_HOSTS=json.dumps([{'id':'not-invented-here'}, {'id':'vade','ssh':'vade'}]))
    log = (root/'server.log').open('w+')
    server = subprocess.Popen([str(ROOT/'target/release/tmux-terminal')], cwd=root, env=env, stdout=log, stderr=log)
    base_url = f'http://127.0.0.1:{port}'

    def request(path, data=None, host=None, raw=False):
        headers = {'Content-Type': 'application/octet-stream' if raw else 'application/json'}
        if host is not None:
            headers['X-Tmux-Host'] = host
        req = urllib.request.Request(base_url+path, data=data if raw else None if data is None else json.dumps(data).encode(), headers=headers)
        try:
            response = urllib.request.urlopen(req, timeout=15)
        except urllib.error.HTTPError as error:
            response = error
        body = response.read()
        try:
            body = json.loads(body)
        except (ValueError, UnicodeDecodeError):
            pass
        return response.status, body

    def events(host):
        return [json.loads(line) for line in (root/host/'events').read_text().splitlines()]

    def upload(host, name, payload, target='@1'):
        return request('/api/upload?'+urllib.parse.urlencode(dict(host=host,target=target,name=name)), payload, raw=True)

    try:
        for _ in range(150):
            try:
                if request('/health')[0] == 200:
                    break
            except OSError:
                time.sleep(.05)
        else:
            raise AssertionError('Server did not start')
        assert request('/api/hosts')[1]['default_host'] == 'not-invented-here'
        for host in (None, 'not-invented-here', 'vade'):
            windows = request('/api/windows', host=host)[1]
            assert windows[0]['target'] == '0:1' and windows[0]['window_id'] == '@1'
            capture = request('/api/capture', {'target':'@1'}, host)[1]
            assert capture['agent'] == 'codex'
            assert ('remote' if host == 'vade' else 'local') in capture['content']
        before = len(events('local'))
        for endpoint, body in [
            ('send', {'session':'@1','command':"literal '$HOME; $(false)\nsecond line"}),
            ('send-key', {'session':'@1','key':'C-c'}),
            ('rename-window', {'target':'@1','name':'renamed'}),
            ('move-window', {'session':'0','from_index':1,'to_index':2}),
            ('kill-window', {'target':'@2'}),
            ('picker/close', {'target':'@1'}),
        ]:
            code, body = request('/api/'+endpoint, body, 'vade')
            assert code == 200, (endpoint, code, body)
        assert len(events('local')) == before, 'Remote actions reached local tmux'
        assert any(a[-1] == "literal '$HOME; $(false)\nsecond line" for a in events('remote'))
        assert request('/api/agents', host='vade')[1]['agents'] == ['codex','eunice']
        assert request('/api/eunice-models', host='vade')[1]['success']
        assert request('/api/window-status', host='vade')[0] == 200
        local_before_model = len(events('local'))
        assert request('/api/session-model', {'target':'@1','action':'open'}, 'vade')[0] == 409
        assert len(events('local')) == local_before_model
        assert request('/api/project-dirs', host='vade')[1]['dirs'][0]['name'] == 'shared'
        code, created = request('/api/new-window-named', {'name':'shared','session':'0','agent':'codex'}, 'vade')
        assert code == 200 and not created['existing'] and created['target'] == '0:9', created
        remote_project = root/'remote'/'home'/'p'/'shared'
        assert (remote_project/'AGENTS.md').is_symlink() and (remote_project/'GEMINI.md').is_symlink()
        assert not (root/'local'/'home'/'p'/'shared'/'AGENTS.md').exists()
        again = request('/api/new-window-named', {'name':'shared','session':'0'}, 'vade')[1]
        assert again['existing'] and again['target'] == '0:9'

        payload = bytes(range(256))*400
        name = "literal '$(touch INJECTED).bin"
        for host, local_name in [('not-invented-here','local'), ('vade','remote')]:
            code, result = upload(host, name, payload)
            assert code == 200, result
            dest = Path(result['path'])
            assert dest.parent == root/local_name/'work' and dest.read_bytes() == payload
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                results = list(pool.map(lambda _: upload(host,name,payload), range(2)))
            assert all(code == 200 for code,_ in results), results
            assert len({result['path'] for _,result in results}) == 2
        assert not (root/'INJECTED').exists()
        assert upload('vade','bad.txt',b'bad','@missing')[0] == 409
        assert not (root/'remote'/'home'/'bad.txt').exists()
        remote_file = root/'remote'/'work'/'preview.txt'
        remote_file.write_text('remote preview')
        assert request('/api/serve-file?'+urllib.parse.urlencode({'host':'vade','path':str(remote_file)}))[1] == b'remote preview'
        ssh_events = [json.loads(line) for line in (root/'ssh-events').read_text().splitlines()]
        assert any(a[0] == 'python3' and a[3] == 'read' for a in ssh_events)
        assert any(a[0] == 'python3' and a[3] == 'status' for a in ssh_events)
        assert request('/api/windows',host='unknown')[0] == 400
        assert request('/api/windows?host=vade',host='not-invented-here')[0] == 400
        (root/'offline').touch()
        before = len(events('local'))
        for path, body in [('/api/windows',None), ('/api/capture',{'target':'@1'}), ('/api/send',{'session':'@1','command':'must not land locally'})]:
            code, result = request(path,body,'vade')
            assert code == 503 and result['offline'], result
        assert upload('vade','offline.txt',b'bad')[0] == 503
        assert len(events('local')) == before
        assert request('/api/windows')[0] == 200
        (root/'offline').unlink()
        assert request('/api/capture',{'target':'@1'},'vade')[1]['content'].startswith('remote')
        before = len(events('remote'))
        (root/'fail-once').touch()
        assert request('/api/send',{'session':'@1','command':'do not retry a failed action'},'vade')[0] == 503
        assert len(events('remote')) == before, 'A failed preflight must stop the mutation even if SSH recovers'
        # An incomplete remote stream must remove the temporary upload and never
        # publish its filename.
        failed = subprocess.run(['python3',str(ROOT/'scripts/host-files.py'),'upload',
            json.dumps({'dir':str(root/'remote'/'work'),'name':'partial.txt','size':999})], input=b'partial',capture_output=True)
        assert failed.returncode != 0
        assert not (root/'remote'/'work'/'partial.txt').exists()
        assert not list((root/'remote'/'work').glob('.tmux-upload-*'))
        remote_state = root/'remote'/'state.json'
        remote_state.write_text(json.dumps([dict(id='@2',target='MASTER:2',name='control',cwd=str(root/'remote'/'work'))]))
        code, fresh = request('/api/new-window-named', {'name':'fresh-zero','session':'0'}, 'vade')
        assert code == 200 and fresh['target'] == '0:9', fresh
        assert any(a[0] == 'new-session' and a[a.index('-s')+1] == '0' for a in events('remote'))
        print('Multi-host HTTP, SSH routing, independent projects, uploads, previews, and outage checks passed')
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill(); server.wait()
        if server.returncode not in (0, -15):
            log.seek(0); print(log.read())
        log.close()
