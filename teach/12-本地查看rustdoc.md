# 12 · 在本机浏览器里看 `cargo doc` 的文档

配套脚本：[`web_server.py`](./web_server.py)

## 12.1 为什么「随便起个服务器」打开是坏的

`cargo doc` 的输出长这样（`target/xtensa-esp32s3-none-elf/doc/`）：

```
doc/
├── static.files/          ← 所有页面**共享**的 CSS / JS / 字体 / 图标
├── src/                   ← 源码高亮页面（点 “Source” 跳这里）
├── crates.js  search-index.js …
├── esp_s3/                ← 本项目 crate
│   ├── index.html
│   └── all.html
└── esp_hal/ aligned/ …    ← 依赖 crate，一共 100 多个
```

关键是每个页面里的路径都是**相对 crate 目录往上一级**写的，比如 `esp_s3/index.html` 里：

```html
<link rel="stylesheet" href="../static.files/rustdoc-17e0aaed.css">
<script src="../crates.js"></script>
<a href="../src/esp_s3/lib.rs.html">Source</a>
```

所以**网站根目录必须是 `doc/`**：

| 把哪个目录当根 | `../static.files/…css` 解析成 | 结果 |
| --- | --- | --- |
| `doc/` ✅ | `/static.files/…css`（存在） | 样式/搜索/源码链接都正常 |
| `doc/esp_s3/` ❌ | `/static.files/…css`（不存在） | 所有 CSS/JS 404 → 页面光秃秃、点不动 |

「打开 HTML 所在目录」这种直觉写法正好踩在第二行上，这就是「文档没有样式、点了没反应」的根因。

## 12.2 用法

```bash
# 最省事：不给路径，自动在 ./target/*/doc 里找本项目 crate 的入口，监听 0.0.0.0:8000
python3 teach/web_server.py

# 指定入口 HTML（走哪个路径都行，脚本会自己往上找根目录）
python3 teach/web_server.py target/xtensa-esp32s3-none-elf/doc/esp_s3/index.html

# 给目录：自动递归找 index.html（优先本项目 crate，其次层级最浅的）
python3 teach/web_server.py target/xtensa-esp32s3-none-elf/doc

# 只想本机访问 / 换端口 / 不自动开浏览器
python3 teach/web_server.py --host 127.0.0.1
python3 teach/web_server.py --port 9000 --no-open
```

启动后浏览器打开 **http://0.0.0.0:8000/** （等于 http://127.0.0.1:8000/ ），
它会自动跳到入口页 `http://127.0.0.1:8000/esp_s3/index.html`；
终端会打印入口、真正的网站根目录和局域网地址，按 `Ctrl+C` 结束。

## 12.3 脚本替你处理了什么

1. **自动定位根目录**：从入口 HTML 往上找第一个含 `static.files/` 的目录当网站根目录，
   于是 `../static.files/…`、`../crates.js`、`../src/…` 全部能命中。
2. **自动找入口**：给目录时递归找 `index.html`（跳过 `src/`、`static.files/`），
   优先选与 `Cargo.toml` 里 `name` 同名的 crate；访问 `/` 时自动跳到入口页。
3. **链接改写**（默认开，`--no-rewrite` 可关）：把响应 HTML 里所有 `href/src/action/poster`
   改写成 `http://你访问用的地址:端口/…` 的绝对地址，`?query` 和 `#fragment` 原样保留，
   `https://…`、`data:`、`javascript:`、纯 `#锚点` 不动，跳出根目录的 `../` 不动。
   这样页面里点任何链接都指回本服务器，不依赖地址栏当前路径。
   前缀取请求里的 `Host` 头，所以本机用 127.0.0.1、手机用 192.168.x.x 访问都不会串地址。
4. **友好的 404 / 防越界**：找不到文件时给一个带根目录和入口链接的提示页；
   `/../../etc/passwd` 这类请求直接 403。
5. **端口被占用自动 +1** 往后找一个（最多 20 个）；HTML 响应带 `Cache-Control: no-store`，
   重新 `cargo doc` 后直接刷新就能看到新内容。

> 脚本只读磁盘上的 HTML，不改文件——`file://` 直接双击打开仍然可用（只是浏览器会拦掉搜索用的
> XHR，侧边栏搜索失灵）。

## 12.4 最简等价做法（不需要这个脚本时）

```bash
cd target/xtensa-esp32s3-none-elf/doc   # 注意：cd 到 doc，不是 cd 到 esp_s3
python3 -m http.server 8080             # 然后浏览器开 http://127.0.0.1:8080/esp_s3/index.html
```

只要把根目录设对，`http.server` 也够用；`web_server.py` 多出来的只是自动找入口、链接改写、
404 提示和日志，省得每次手敲路径。

## 12.5 参数表

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `path` | 自动 | HTML 文件或目录；不给就在 `./target/*/doc` 里找 |
| `--port` | `8000` | 被占用会自动往后找，启动后以打印的地址为准 |
| `--host` | `0.0.0.0` | 本机+局域网都能访问；只想本机用 `--host 127.0.0.1` |
| `--no-open` | 关 | 加了这个不自动开浏览器 |
| `--no-rewrite` | 关 | 不改写链接（根目录已正确时也能正常浏览） |
| `--verbose` | 关 | 连 CSS/JS/字体等每个请求都打印 |

## 12.6 常见坑

| 现象 | 原因 / 解决 |
| --- | --- |
| 页面没样式、点不动 | 根目录设成了 crate 目录。用本脚本，或 `cd` 到 `doc/` 再 `http.server` |
| 点 “Source” 404 | 同上，`../src/` 也在根目录下 |
| 端口被占、地址栏的端口和预期不符 | 8000 上还挂着上次没关的服务器；先关掉它（`pkill -f web_server.py`），或看终端打印的实际端口 |
| 侧边栏搜索不出结果 | 用 `http://` 而不是 `file://` 打开（浏览器拦 file 的 XHR） |
| 改完代码看不到变化 | `cargo doc` 重新生成，然后刷新（脚本已带 `no-store`） |
| 只想看某个依赖的文档 | 直接传它的入口，例如 `…/doc/esp_hal/index.html` |
