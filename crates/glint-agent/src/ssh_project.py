# Executed by python3 on the SSH host; requests and results use JSON over stdio.
import base64, fnmatch, json, os, re, stat, subprocess, sys
LIMIT = 2 * 1024 * 1024
SKIP = {'.git', 'node_modules', 'target', '.venv', '.cache', '.ssh', '.gnupg'}

def run(q):
    root = os.path.realpath(os.path.expanduser(q['root']))
    if not os.path.isdir(root):
        raise ValueError('远程项目目录不存在')
    def text_file(path):
        try:
            metadata = os.stat(path)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > LIMIT: return False
            with open(path, 'rb') as file: sample = file.read(8192)
            if b'\x00' in sample: return False
            import codecs
            codecs.getincrementaldecoder('utf-8')().decode(sample, final=False)
            return True
        except (OSError, UnicodeError):
            return False
    def png_file(path):
        try:
            metadata = os.stat(path)
            return path.lower().endswith('.png') and stat.S_ISREG(metadata.st_mode) and metadata.st_size <= 20 * 1024 * 1024
        except OSError:
            return False
    if q['op'] == 'preview_stat':
        result = {}
        for path in q.get('args', {}).get('paths', [])[:128]:
            resolved = os.path.realpath(os.path.join(root, os.path.expanduser(path)))
            result[path] = 'directory' if os.path.isdir(resolved) else 'image' if png_file(resolved) else 'text' if text_file(resolved) else None
        return result
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
        with open(path, encoding='utf-8', newline='') as f:
            content = f.read(LIMIT + 1)
        if len(content.encode('utf-8')) > LIMIT:
            raise ValueError('文件超过 2MB')
        return content

    if op == 'preview_excerpt':
        if not text_file(path): raise ValueError('不支持预览此文件')
        start = max(1, int(args.get('start_line', 1)))
        lines = max(1, min(25, int(args.get('lines', 25))))
        with open(path, 'rb') as file:
            for _ in range(start - 1):
                if not file.readline(): return ''
            sample = file.read(4097)
        if b'\x00' in sample: raise ValueError('二进制文件不支持预览')
        import codecs
        text = codecs.getincrementaldecoder('utf-8')().decode(sample, final=len(sample) < 4097)
        return ''.join(text.splitlines(keepends=True)[:lines])

    if op == 'preview':
        parent = os.path.dirname(path) if os.path.dirname(path) != path else None
        if os.path.isdir(path):
            entries = []
            with os.scandir(path) as listing:
                for entry in listing:
                    entries.append({'name': entry.name, 'path': entry.path, 'directory': entry.is_dir(), 'previewable': png_file(entry.path) or text_file(entry.path)})
                    if len(entries) > 1000: break
            entries.sort(key=lambda entry: (not entry['directory'], entry['name']))
            return {'kind': 'directory', 'path': path, 'parent': parent, 'entries': entries[:1000], 'truncated': len(entries) > 1000}
        if png_file(path):
            with open(path, 'rb') as file: data = file.read(20 * 1024 * 1024 + 1)
            if len(data) > 20 * 1024 * 1024 or not data.startswith(b'\x89PNG\r\n\x1a\n'): raise ValueError('PNG 图片格式无效或超过 20MB')
            return {'kind': 'image', 'path': path, 'parent': parent, 'content': base64.b64encode(data).decode('ascii')}
        content = read(path)
        if '\x00' in content: raise ValueError('二进制文件不支持文本预览')
        return {'kind': 'text', 'path': path, 'parent': parent, 'content': content}
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
    if op == 'undo_changes':
        import tempfile
        changes = args.get('changes', [])
        def state(change):
            target = change['path']
            if not os.path.isabs(target) or os.path.realpath(target) != target:
                raise ValueError('文件或父目录路径发生变化，未撤销')
            if os.path.lexists(target) and (os.path.islink(target) or not os.path.isfile(target)):
                raise ValueError('文件类型已变化，未撤销')
            current = read(target, missing=True)
            if current == change['before']: return False
            if current != change['after']: raise ValueError(target + ' 的内容已被其他操作修改，未覆盖')
            return True
        for change in changes: state(change)
        restored = []
        for change in changes:
            temporary = None
            try:
                target = change['path']
                if state(change):
                    if change['before'] is None:
                        os.unlink(target)
                    else:
                        permissions = stat.S_IMODE(os.stat(target).st_mode)
                        fd, temporary = tempfile.mkstemp(prefix='.glint-undo-', dir=os.path.dirname(target))
                        with os.fdopen(fd, 'w', encoding='utf-8', newline='') as file:
                            os.fchmod(file.fileno(), permissions)
                            file.write(change['before'])
                            file.flush()
                            os.fsync(file.fileno())
                        if state(change): os.replace(temporary, target)
                restored.append(target)
            except Exception as error:
                return {'restored': restored, 'error': str(error)}
            finally:
                if temporary and os.path.exists(temporary): os.unlink(temporary)
        return {'restored': restored, 'error': None}

    if op == 'resolve_directory':
        if not os.path.isdir(path): raise ValueError('cwd 必须为目录')
        return path
    if op == 'read_file':
        content = read(path)
        start = max(1, int(args.get('start_line', 1))) - 1
        lines = max(1, min(500, int(args.get('lines', 200))))
        page = '\n'.join('%d: %s' % (i + 1, line) for i, line in enumerate(content.splitlines()) if start <= i < start + lines)
        numbered = args.get('line_numbers', True)
        if not numbered: page = ''.join(content.splitlines(keepends=True)[start:start+lines])
        if args.get('include_metadata'):
            import hashlib
            digest = hashlib.sha256(content.encode('utf-8')).hexdigest()
            return {'content':page,'sha256':digest,'expected_sha256':digest,'total_lines':len(content.splitlines()),'start_line':start+1,'path':path,'line_numbers':numbered,'next_line':start+lines+1 if start+lines<len(content.splitlines()) else None,'is_complete':start==0 and lines>=len(content.splitlines())}
        return page
    if op in ('write_file', 'replace_in_file', 'prepare_edit', 'commit_edit'):
        import hashlib, tempfile
        before = read(path, missing=True)
        digest = lambda value: hashlib.sha256(value.encode('utf-8')).hexdigest() if value is not None else None
        expected = args.get('expected_sha256')
        if args.get('create_only') and before is not None: raise ValueError('文件已存在，未覆盖')
        if expected is not None and digest(before) != expected: raise ValueError('文件版本不匹配，请重新读取')
        kind = args.get('kind', op)
        if op == 'commit_edit':
            if path != args.get('canonical_path'): raise ValueError('父目录路径发生变化，未写入')
            if (before is None) != bool(args.get('expected_absent')) or digest(before) != expected: raise ValueError('文件已被其他操作修改，未写入')
            after = args['content']
        elif kind == 'write_file': after = args['content']
        else:
            if before is None: raise ValueError('目标文件不存在')
            old = args['old']
            if not old or before.count(old) != 1: raise ValueError('待替换内容不存在或不唯一，未修改文件')
            after = before.replace(old, args['new'], 1)
        if not isinstance(after, str) or len(after.encode('utf-8')) > LIMIT: raise ValueError('单次写入超过 2MB 或内容不是字符串')
        if op == 'prepare_edit': return {'before':before,'after':after,'path':path,'sha256':digest(before)}
        if before != after:
            import fcntl
            temporary = None
            directory_fd = os.open(os.path.dirname(path), os.O_RDONLY)
            try:
                fcntl.flock(directory_fd, fcntl.LOCK_EX)
                if os.path.realpath(path) != path or read(path, missing=True) != before: raise ValueError('文件已被其他操作修改，未写入')
                if before is not None and os.stat(path).st_mode & 0o222 == 0: raise ValueError('文件为只读，未写入')
                fd, temporary = tempfile.mkstemp(prefix='.glint-write-', dir=os.path.dirname(path))
                with os.fdopen(fd, 'w', encoding='utf-8', newline='') as file:
                    if before is not None: os.fchmod(file.fileno(), stat.S_IMODE(os.stat(path).st_mode))
                    file.write(after);file.flush();os.fsync(file.fileno())
                if os.path.realpath(path) != path or read(path, missing=True) != before: raise ValueError('文件已被其他操作修改，未写入')
                os.replace(temporary,path)
                try: os.fsync(directory_fd)
                except OSError: pass
            finally:
                if temporary and os.path.exists(temporary): os.unlink(temporary)
                os.close(directory_fd)
        return {'before':before,'after':after,'path':path}
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
