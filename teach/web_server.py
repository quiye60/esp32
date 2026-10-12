#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""本地浏览 cargo doc / rustdoc 生成的文档。

为什么不能直接用 `python -m http.server`：

    rustdoc 的每个页面都用**相对路径**引用共享资源，比如 esp_s3/index.html 里写的是

        <link rel="stylesheet" href="../static.files/rustdoc-xxxx.css">
        <script src="../crates.js"></script>
        <a href="../src/esp_s3/lib.rs.html">Source</a>

    这些文件都在 doc/ 里，不在 doc/esp_s3/ 里。如果把网站根目录设成 HTML 自己所在的
    doc/esp_s3/，上面每一个引用都会 404 —— 页面看起来就是「没样式、点了没反应」。
    所以本脚本会自动往上找到真正的根目录（含 static.files/ 的那一层）。

    另外按需求做了链接改写：把响应 HTML 里的 href/src/action/poster 全部换成
    http://<host>:<port>/... 的绝对地址（保留 ?query 和 #fragment），
    这样链接一定指回本服务器，不依赖浏览器当前地址栏。

用法：

    python3 web_server.py                       # 自动在 ./target/*/doc 里找，监听 0.0.0.0:8000
    python3 web_server.py target/xtensa-esp32s3-none-elf/doc/esp_s3/index.html
    python3 web_server.py target/xtensa-esp32s3-none-elf/doc    # 给目录，自动找入口

打开 http://0.0.0.0:8000/ （或 http://127.0.0.1:8000/ ）会自动跳到入口页，
想只让本机访问就加 --host 127.0.0.1，想换端口加 --port。
其它开关：--no-open 不自动开浏览器，--no-rewrite 不改写链接，--verbose 打印每个请求。
"""

import argparse
import errno
import mimetypes
import os
import posixpath
import re
import socket
import threading
import urllib.parse
import webbrowser
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# --------------------------------------------------------------------------
# 一、链接改写
# --------------------------------------------------------------------------

# 只改「跳转/加载类」属性，别碰正文里的代码示例
ATTR_RE = re.compile(
    r'(?P<pre>\b(?:href|src|action|poster)\s*=\s*)(?P<q>["\'])(?P<url>[^"\']*)(?P=q)',
    re.IGNORECASE,
)
SCHEME_RE = re.compile(r'^[a-zA-Z][a-zA-Z0-9+.\-]*:')   # http: https: data: mailto: javascript:
SKIP_PREFIX = ('#', '//')                                # 纯锚点 / 协议相对地址
SHARED_ASSET_DIRS = ('static.files',)                    # rustdoc 共享资源目录（根目录的标记）
HOST_HEADER_RE = re.compile(r'^[A-Za-z0-9.\-\[\]:]+$')    # 用来校验 Host 头，防注入


def normalize_url_path(path: str):
    """把 '/a/b/../c' 规范化成 '/a/c'；越出网站根目录时返回 None。"""
    out = []
    for seg in path.split('/'):
        if seg in ('', '.'):
            continue
        if seg == '..':
            if not out:
                return None
            out.pop()
        else:
            out.append(seg)
    return '/' + '/'.join(out)


def absolutize(url: str, page_url_path: str, base_url: str) -> str:
    """把一个链接换成指向本服务器的绝对地址；不该动、动不了的原样返回。"""
    u = url.strip()
    if not u or u.startswith(SKIP_PREFIX) or SCHEME_RE.match(u):
        return url
    path, hash_sep, fragment = u.partition('#')
    path, query_sep, query = path.partition('?')
    if not path:
        return url                                        # 只有 ?query#frag
    if path.startswith('/'):
        joined = path                                     # 以 / 开头，视作站点根目录
    else:
        joined = posixpath.join(posixpath.dirname(page_url_path), path)
    norm = normalize_url_path(joined)
    if norm is None:
        return url                                        # 跑到根目录外面，交给浏览器
    out = base_url + norm
    if query_sep:
        out += '?' + query
    if hash_sep:
        out += '#' + fragment
    return out


def rewrite_html(text: str, page_url_path: str, base_url: str, enabled: bool = True):
    """返回 (改写后的 HTML, 改写的链接个数)。"""
    if not enabled:
        return text, 0
    counter = [0]

    def repl(m):
        new = absolutize(m.group('url'), page_url_path, base_url)
        if new != m.group('url'):
            counter[0] += 1
        return m.group('pre') + m.group('q') + new + m.group('q')

    return ATTR_RE.sub(repl, text), counter[0]


# --------------------------------------------------------------------------
# 二、找入口页面 & 找真正的网站根目录
# --------------------------------------------------------------------------

def detect_root(start_dir: Path) -> Path:
    """从 start_dir 往上找第一个含 static.files/ 的目录 —— 那就是网站根目录。"""
    start_dir = start_dir.resolve()
    for anc in (start_dir, *start_dir.parents):
        if any((anc / d).is_dir() for d in SHARED_ASSET_DIRS):
            return anc
    return start_dir


def crate_name_near(start: Path):
    """往上找 Cargo.toml，取出 crate 名（- 换成 _，和 rustdoc 的目录名一致）。"""
    for anc in (start, *start.parents):
        cargo = anc / 'Cargo.toml'
        if cargo.is_file():
            m = re.search(r'(?m)^\s*name\s*=\s*"([^"]+)"', cargo.read_text(errors='replace'))
            return m.group(1).replace('-', '_') if m else None
    return None


def _depth_key(base: Path, p: Path):
    return (len(p.relative_to(base).parts), str(p))


def find_entry(target: Path) -> Path:
    """target 是文件就直接用；是目录就自动找一个入口 index.html。"""
    target = target.resolve()
    if target.is_file():
        return target
    direct = target / 'index.html'
    if direct.is_file():
        return direct

    indexes = [
        p for p in target.rglob('index.html')
        if 'src' not in p.parts and not any(d in p.parts for d in SHARED_ASSET_DIRS)
    ]
    crate = crate_name_near(target)
    if crate:                                            # 优先本项目自己的 crate
        for p in indexes:
            if p.parent.name == crate:
                return p
    if indexes:                                          # 其次选层级最浅的
        return min(indexes, key=lambda p: _depth_key(target, p))
    pages = sorted(target.rglob('*.html'))
    if pages:
        return min(pages, key=lambda p: _depth_key(target, p))
    raise SystemExit(f'❌ {target} 里找不到任何 HTML 文件')


def find_default_target() -> Path:
    """不给路径时：在 ./target/*/doc 里找文档目录。"""
    cwd = Path.cwd()
    docs = [p for p in sorted(cwd.glob('target/*/doc')) if p.is_dir()]
    crate = crate_name_near(cwd)
    if crate:
        preferred = [p for p in docs if (p / crate).is_dir()]
        if preferred:
            return preferred[0]
    if docs:
        return docs[0]
    raise SystemExit(
        '❌ 没找到文档目录，请显式给路径，例如：\n'
        '   cargo doc && python3 web_server.py target/xtensa-esp32s3-none-elf/doc/esp_s3/index.html'
    )


# --------------------------------------------------------------------------
# 三、HTTP 服务
# --------------------------------------------------------------------------

CONTENT_TYPES = {
    '.js': 'text/javascript',
    '.mjs': 'text/javascript',
    '.css': 'text/css',
    '.html': 'text/html',
    '.htm': 'text/html',
    '.json': 'application/json',
    '.map': 'application/json',
    '.svg': 'image/svg+xml',
    '.png': 'image/png',
    '.ico': 'image/x-icon',
    '.woff': 'font/woff',
    '.woff2': 'font/woff2',
    '.ttf': 'font/ttf',
    '.wasm': 'application/wasm',
    '.txt': 'text/plain',
    '.webmanifest': 'application/manifest+json',
}
TEXTY_TYPES = (
    'application/javascript', 'application/json', 'application/manifest+json', 'image/svg+xml',
)


def content_type(path: str) -> str:
    ext = os.path.splitext(path)[1].lower()
    ctype = CONTENT_TYPES.get(ext) or mimetypes.guess_type(path)[0] or 'application/octet-stream'
    if ctype.startswith('text/') or ctype in TEXTY_TYPES:
        ctype += '; charset=utf-8'
    return ctype


class DocHandler(SimpleHTTPRequestHandler):
    """静态文件 + HTML 链接改写。"""

    server_version = 'RustdocServer/1.0'

    root: Path = Path('.')
    base_url: str = ''
    entry_url: str = '/'
    rewrite: bool = True
    verbose: bool = False

    # 用自己的日志，关掉默认的
    def log_message(self, fmt, *args):
        pass

    # ---- 请求分发 ----

    def do_GET(self):
        self._serve(with_body=True)

    def do_HEAD(self):
        self._serve(with_body=False)

    def _resolve(self):
        """把请求路径映到磁盘路径，顺便挡掉 ../ 越界。"""
        raw = urllib.parse.urlsplit(self.path).path
        raw = urllib.parse.unquote(raw, errors='surrogatepass')
        parts = []
        for seg in raw.split('/'):
            if seg in ('', '.'):
                continue
            if seg == '..':
                if not parts:
                    return None
                parts.pop()
            elif '/' in seg or (os.altsep and os.altsep in seg) or '\x00' in seg:
                return None
            else:
                parts.append(seg)
        return self.root.joinpath(*parts)

    def _redirect(self, location: str):
        self.send_response(302)
        self.send_header('Location', location)
        self.send_header('Content-Length', '0')
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()

    def _rewrite_base(self) -> str:
        """改写用的前缀要跟着「浏览器实际用的地址」走。

        用 --host 0.0.0.0 时，本机访问是 127.0.0.1、手机访问是 192.168.x.x，
        如果统一改写成 http://0.0.0.0:8000/...，手机上的 0.0.0.0 就指向手机自己了。
        所以直接用请求里的 Host 头。
        """
        host = (self.headers.get('Host') or '').strip()
        if host and HOST_HEADER_RE.match(host):
            return f'http://{host}'
        return self.base_url

    def _serve(self, with_body: bool):
        target = self._resolve()
        if target is None:
            return self.send_error(403, 'Bad request path')

        path_only = urllib.parse.urlsplit(self.path).path
        if target.is_dir():
            if path_only in ('', '/'):
                if not (self.root / 'index.html').is_file() and self.entry_url != '/':
                    return self._redirect(self.entry_url)      # 根目录没 index，直接送入口页
            elif not path_only.endswith('/'):
                return self._redirect(path_only + '/')
            index = target / 'index.html'
            if index.is_file():
                target = index
            else:
                return self.list_directory(str(target))

        if not target.is_file():
            return self._not_found(target)

        try:
            data = target.read_bytes()
        except OSError:
            return self.send_error(404, 'Cannot read file')

        ctype = content_type(str(target))
        rewritten = 0
        if ctype.startswith('text/html'):
            page_url = '/' + target.relative_to(self.root).as_posix()
            text = data.decode('utf-8', errors='replace')
            text, rewritten = rewrite_html(text, page_url, self._rewrite_base(), self.rewrite)
            data = text.encode('utf-8')

        self.send_response(200)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(data)))
        self.send_header('Cache-Control', 'no-store')          # 重新生成文档后刷新即可见
        self.end_headers()
        if with_body:
            try:
                self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError):
                pass

        if rewritten or self.verbose:          # 默认安静：只报 HTML 页面和错误
            note = f'  改写 {rewritten} 处链接' if rewritten else ''
            print(f'  {self.command:4} {self.path}  → 200  ({len(data) / 1024:.1f} KiB){note}', flush=True)

    def _not_found(self, target: Path):
        shown = '/' + target.relative_to(self.root).as_posix() if target.is_relative_to(self.root) else str(target)
        body = (
            '<!doctype html><meta charset="utf-8">'
            '<title>404</title>'
            '<style>body{font-family:sans-serif;margin:2rem;line-height:1.7}'
            'code{background:#f2f2f2;padding:.1em .3em;border-radius:3px}</style>'
            f'<h1>404</h1><p>服务器上没有这个文件：<code>{shown}</code></p>'
            f'<p>网站根目录：<code>{self.root}</code></p>'
            f'<p><a href="{self.entry_url}">→ 打开入口页 {self.entry_url}</a></p>'
        ).encode('utf-8')
        self.send_response(404)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        if self.command == 'GET':
            self.wfile.write(body)
        print(f'  {self.command:4} {self.path}  → 404', flush=True)


# --------------------------------------------------------------------------
# 四、启动
# --------------------------------------------------------------------------

def make_handler(root: Path, entry_url: str, rewrite: bool, verbose: bool):
    return type('BoundDocHandler', (DocHandler,), {
        'root': root,
        'entry_url': entry_url,
        'rewrite': rewrite,
        'verbose': verbose,
    })


def bind_server(host: str, port: int, handler):
    """端口被占用就自动往后找一个（最多试 20 个）。"""
    for candidate in range(port, port + 20):
        try:
            httpd = ThreadingHTTPServer((host, candidate), handler)
        except OSError as e:
            if e.errno == errno.EADDRINUSE:
                continue
            raise
        httpd.daemon_threads = True
        return httpd, httpd.server_address[1]     # 真的绑定到的端口（--port 0 时尤其重要）
    raise SystemExit(f'❌ {port}~{port + 19} 端口都被占用了，用 --port 换一个')


def lan_ip():
    """猜一下本机在局域网的 IP（不会真的发包），失败就返回 None。"""
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.connect(('8.8.8.8', 80))
            return s.getsockname()[0]
    except OSError:
        return None


def main(argv=None):
    ap = argparse.ArgumentParser(
        description='本地浏览 rustdoc 文档：自动定位正确的网站根目录，并把页面里的链接改写成绝对地址。',
    )
    ap.add_argument('path', nargs='?', help='HTML 文件或目录；不填就自动在 ./target/*/doc 里找')
    ap.add_argument('--port', type=int, default=8000, help='端口，默认 8000（占用会自动 +1 往后找）')
    ap.add_argument('--host', default='0.0.0.0',
                    help='监听地址，默认 0.0.0.0（本机+局域网）；只想本机访问用 127.0.0.1')
    ap.add_argument('--no-open', action='store_true', help='不自动打开浏览器')
    ap.add_argument('--no-rewrite', action='store_true', help='不改写链接（只靠相对路径，需根目录正确）')
    ap.add_argument('--verbose', action='store_true', help='打印每一个请求')
    args = ap.parse_args(argv)

    target = Path(args.path).expanduser() if args.path else find_default_target()
    if not target.exists():
        raise SystemExit(f'❌ 路径不存在：{target}')

    entry = find_entry(target)
    root = detect_root(entry.parent)
    try:
        entry_url = '/' + entry.relative_to(root).as_posix()
    except ValueError:
        raise SystemExit(f'❌ 入口 {entry} 不在根目录 {root} 下')

    handler = make_handler(root, entry_url, not args.no_rewrite, args.verbose)
    httpd, port = bind_server(args.host, args.port, handler)

    bind_host = args.host or '0.0.0.0'
    home_url = f'http://{bind_host}:{port}/'                 # 打开这个就会落到入口页
    local_url = f'http://127.0.0.1:{port}{entry_url}'
    handler.base_url = f'http://127.0.0.1:{port}'            # 拿不到 Host 头时的兜底

    print(f'📄 入口页面  : {entry_url}', flush=True)
    print(f'📁 网站根目录: {root}', flush=True)
    print(f'🔗 打开地址  : {home_url}    ← 会自动跳到 {entry_url}', flush=True)
    print(f'   （等效地址: {local_url}）', flush=True)
    if bind_host in ('0.0.0.0', '::'):
        ip = lan_ip()
        if ip:
            print(f'🌐 局域网    : http://{ip}:{port}{entry_url}   （手机/平板同网段可用）', flush=True)
        print('⚠  监听 0.0.0.0：同一网段的人都能看到这份文档；只想本机用请加 --host 127.0.0.1',
              flush=True)
    print(f'🔧 链接改写  : {"关" if args.no_rewrite else "开（按你访问用的地址改写，换 IP 也不会失效）"}',
          flush=True)
    print('按 Ctrl+C 停止服务器', flush=True)

    if not args.no_open:
        threading.Timer(0.5, lambda: webbrowser.open(home_url)).start()

    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print('\n正在关闭服务器……')
    finally:
        httpd.server_close()


if __name__ == '__main__':
    main()
