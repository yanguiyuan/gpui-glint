# Executed by python3 on the SSH host; requests and results use JSON over stdio.
import fnmatch, json, os, re, stat, subprocess, sys
LIMIT = 2 * 1024 * 1024
SKIP = {'.git', 'node_modules', 'target', '.venv', '.cache', '.ssh', '.gnupg'}

def run(q):
    root = os.path.realpath(os.path.expanduser(q['root']))
    if not os.path.isdir(root):
        raise ValueError('远程项目目录不存在')
    op = q['op']
    if op == 'probe':
        return {'root': root}
    args = q.get('args', {})
    relative = args.get('path', '.')
    full = bool(q.get('full_access', False))
    if not isinstance(relative, str) or '\x00' in relative:
        raise ValueError('文件路径无效')
    if not full and (os.path.isabs(relative) or '..' in relative.split('/')):
        raise ValueError('路径必须位于远程项目目录内')
    path = os.path.realpath(os.path.join(root, relative))
    if not full and os.path.commonpath([root, path]) != root:
        raise ValueError('符号链接指向远程项目外部')

    def read(path, missing=False):
        try:
            metadata = os.stat(path)
        except FileNotFoundError:
            if missing: return None
            raise
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > LIMIT:
            raise ValueError('目标必须是 2MB 以内的普通 UTF-8 文件')
        with open(path, encoding='utf-8') as f:
            content = f.read(LIMIT + 1)
        if len(content.encode('utf-8')) > LIMIT:
            raise ValueError('文件超过 2MB')
        return content

    if op == 'workspace_changes':
        status = subprocess.run(['git', '--no-optional-locks', 'status', '--porcelain=v1', '-z', '--untracked-files=all'], cwd=root, capture_output=True, timeout=30)
        if status.returncode:
            raise ValueError('远程目录不是可用的 Git 工作区')
        rows, notes, total = [], [], 0
        entries = iter(status.stdout.decode('utf-8').split('\x00'))
        for entry in entries:
            if not entry: continue
            label, source = entry[3:], entry[3:]
            if 'R' in entry[:2] or 'C' in entry[:2]: source = next(entries, label)
            target = os.path.join(root, label)
            if os.path.islink(target) or os.path.isdir(target):
                notes.append(label + '：跳过链接或子模块')
                continue
            try:
                after = read(target, missing=True) or ''
                base = subprocess.run(['git', 'show', 'HEAD:' + source], cwd=root, capture_output=True, timeout=15)
                if len(base.stdout) > LIMIT: raise ValueError('文件超过 2MB')
                before = base.stdout.decode('utf-8') if base.returncode == 0 else None
                if '\x00' in after or (before is not None and '\x00' in before): raise ValueError('二进制文件')
                total += len(after.encode('utf-8')) + len(base.stdout)
                if total > 8 * LIMIT or len(rows) >= 200:
                    notes.append('变更较多，仅展示前面的文件')
                    break
                if before != after:
                    rows.append({'label': label, 'path': target, 'before': before, 'after': after})
            except (OSError, ValueError, UnicodeError) as error:
                notes.append(label + '：' + str(error))
        return {'changes': rows, 'notes': notes}
    if op == 'read_file':
        content = read(path)
        start = max(1, int(args.get('start_line', 1))) - 1
        lines = max(1, min(500, int(args.get('lines', 200))))
        return '\n'.join('%d: %s' % (i + 1, line) for i, line in enumerate(content.splitlines()) if start <= i < start + lines)
    if op in ('write_file', 'replace_in_file'):
        before = read(path, missing=True)
        if op == 'write_file':
            after = args['content']
        else:
            old = args['old']
            if before is None or not old or before.count(old) != 1:
                raise ValueError('待替换内容不存在或不唯一，未修改文件')
            after = before.replace(old, args['new'], 1)
        if not isinstance(after, str) or len(after.encode('utf-8')) > LIMIT:
            raise ValueError('单次写入超过 2MB 或内容不是字符串')
        if before != after:
            with open(path, 'w', encoding='utf-8', newline='') as f:
                f.write(after)
        return {'before': before, 'after': after, 'path': path}
    if op in ('index', 'list_files', 'search_files'):
        entries, matches, skipped = [], [], 0
        cap = 50000 if op == 'index' else 2000
        pattern = re.compile(args['pattern']) if op == 'search_files' else None
        if not os.path.isdir(path): raise ValueError('目标不是目录')
        for directory, dirs, files in os.walk(path, followlinks=False):
            dirs[:] = sorted(d for d in dirs if d not in SKIP and not os.path.islink(os.path.join(directory, d)))
            files = sorted(f for f in files if not os.path.islink(os.path.join(directory, f)))
            for name in dirs + files:
                item = os.path.join(directory, name)
                rel = os.path.relpath(item, root)
                if op != 'search_files':
                    entries.append({'path': rel, 'name': name, 'directory': os.path.isdir(item)})
                    if len(entries) >= cap: return {'entries': entries, 'truncated': True, 'skipped': skipped}
                elif not os.path.isdir(item) and not name.startswith('.env') and not name.endswith('.pem'):
                    glob = args.get('glob')
                    if glob and not (fnmatch.fnmatch(rel, glob) or fnmatch.fnmatch(name, glob)): continue
                    try:
                        content = read(item)
                    except (OSError, ValueError, UnicodeError):
                        skipped += 1
                        continue
                    hits = 0
                    for number, line in enumerate(content.splitlines(), 1):
                        if pattern.search(line):
                            matches.append('%s:%d:%s' % (rel, number, line[:4000]))
                            hits += 1
                            if len(matches) >= 200: return '\n'.join(matches)
                            if hits >= 30: break
            if op == 'list_files' and os.path.relpath(directory, path).count(os.sep) >= 3:
                dirs[:] = []
        return '\n'.join(matches) if op == 'search_files' else {'entries': entries, 'truncated': False, 'skipped': skipped}
    raise ValueError('不支持的远程文件操作')

try:
    print(json.dumps({'ok': True, 'value': run(json.load(sys.stdin))}, ensure_ascii=False))
except Exception as error:
    print(json.dumps({'ok': False, 'error': str(error)}, ensure_ascii=False))
