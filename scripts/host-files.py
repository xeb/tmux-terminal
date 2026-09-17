"""Fixed remote filesystem operations. Arguments are JSON; upload bytes use stdin.

Executed with python3 -c over SSH. No installation or daemon on the remote host.
"""
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import signal
import subprocess
from concurrent.futures import ThreadPoolExecutor


def instruction_links(directory):
    root = Path(directory)
    if not (root / 'CLAUDE.md').exists():
        return
    for name in ('AGENTS.md', 'GEMINI.md'):
        try:
            os.symlink('./CLAUDE.md', root / name)
        except FileExistsError:
            pass


def run(op, data):
    if op == 'status':
        listing = subprocess.run(['tmux', 'list-windows', '-a', '-F', '#{session_name}:#{window_index}'], capture_output=True, timeout=3)
        if listing.returncode:
            raise ValueError(listing.stderr.decode(errors='replace').strip())
        def capture(target):
            result = subprocess.run(['tmux', 'capture-pane', '-p', '-t', target, '-S', '-200'], capture_output=True, timeout=3)
            return {'target': target, 'pane': result.stdout.decode(errors='replace')} if not result.returncode else None
        with ThreadPoolExecutor(max_workers=4) as workers:
            return [row for row in workers.map(capture, listing.stdout.decode().splitlines()) if row]
    if op == 'project':
        name = data['name']
        if not re.fullmatch(r'[A-Za-z0-9._][A-Za-z0-9._-]*', name) or name in ('.', '..'):
            raise ValueError('Invalid project name')
        directory = Path.home() / 'p' / name
        directory.mkdir(parents=True, exist_ok=True)
        instruction_links(directory)
        return {'path': str(directory.resolve(strict=True))}
    if op == 'links':
        instruction_links(data['path'])
        return {}
    if op == 'canonical':
        return {'path': str(Path(data['path']).resolve(strict=True))}
    if op == 'dirs':
        result = []
        root = Path.home() / 'p'
        if root.is_dir():
            for path in root.iterdir():
                if re.fullmatch(r'[A-Za-z0-9._][A-Za-z0-9._-]*', path.name) and path.is_dir():
                    try:
                        result.append({'name': path.name, 'mtime': int(path.stat().st_mtime)})
                    except OSError:
                        pass
        result.sort(key=lambda d: (-d['mtime'], d['name']))
        return {'dirs': result}
    if op == 'read':
        path = Path(data['path']).resolve(strict=True)
        ext = path.suffix.lower().lstrip('.')
        allowed = ('jpg jpeg png gif webp bmp svg tiff tif' if data['image'] else 'json txt log csv xml yaml yml toml md ini cfg').split()
        if ext not in allowed or not path.is_file():
            raise ValueError('Not a supported file type')
        limit = 100 * 1024 * 1024 if data['image'] else 5 * 1024 * 1024
        with path.open('rb') as source:
            content = source.read(limit + 1)
        if len(content) > limit:
            raise ValueError('File too large')
        sys.stdout.buffer.write(content)
        return None
    if op == 'upload':
        directory = Path(data['dir']).resolve(strict=True)
        name = data['name']
        if not directory.is_dir() or Path(name).name != name or name in ('', '.', '..'):
            raise ValueError('Invalid upload destination')
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=directory, prefix='.tmux-upload-', delete=False) as output:
                temporary = Path(output.name)
                total = 0
                while True:
                    chunk = sys.stdin.buffer.read(1024 * 1024)
                    if not chunk:
                        break
                    total += len(chunk)
                    if total > 100 * 1024 * 1024:
                        raise ValueError('File too large')
                    output.write(chunk)
                output.flush()
                os.fsync(output.fileno())
            if total != data['size']:
                raise ValueError('Incomplete upload')
            path = Path(name)
            for n in range(10000):
                dest = directory / (name if not n else f'{path.stem}-{n}{path.suffix}')
                try:
                    os.link(temporary, dest)  # Atomic publication, never overwrites.
                    return {'success': True, 'path': str(dest), 'name': dest.name, 'size': total}
                except FileExistsError:
                    continue
            raise ValueError('No unused filename available')
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
    raise ValueError('Unknown filesystem operation')


if __name__ == '__main__':
    def interrupted(signum, frame):
        raise InterruptedError('Transfer interrupted')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    try:
        result = run(sys.argv[1], json.loads(sys.argv[2]))
        if result is not None:
            print(json.dumps(result))
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
