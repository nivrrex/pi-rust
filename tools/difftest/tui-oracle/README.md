# tui-oracle：主屏差分渲染的 TS oracle 夹具

生成 **TS（pi 0.87.1）真实 `TuiMainScreen` 写出的字节**，供 pi-rs 的 `diff_render` 逐字节对拍。

## 依赖与来源
- **TS 版本**：pi **0.87.1**（tag `v0.87.1` / commit `f07218c`）。
- **源码引用方式**：运行时直接从**本机已安装的 pi 包**里 import `packages/tui/src/{tui.ts,tui-main-screen.ts,utils.ts}`（node ≥22.18 的类型剥离），**不复制、不修改上游源码**；路径由环境变量 `PI_TUI_REF_DIR` 指定（缺省指向开发机上的只读 checkout `ref/pi-0.87.1/packages/tui/src/`）。
- **裸包名解析**：`get-east-asian-width`、`marked` 由 `resolve-hook.mjs` 映射到**已安装 pi 包自带的** `node_modules`（基线由 `TS_PI_ENTRY` 推导：`<pkg>/dist/bundle/cli.js` ⇒ `<pkg>/node_modules`），**不新增任何 npm 依赖**。
- **node 版本**：v24.21.0（`node --version`）。

## 生成命令
```bash
export PATH="$(ls -d $HOME/.local/share/fnm/node-versions/*/installation/bin | tail -1):$HOME/.cargo/bin:$PATH"
export TS_PI_ENTRY=<PI_TS_INSTALL>/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js
node --import ./tools/difftest/tui-oracle/register.mjs \
     tools/difftest/tui-oracle/gen.mjs tools/difftest/tui-oracle/fixtures.json
```
可选：`PI_TUI_REF_DIR=/path/to/packages/tui/src/` 覆盖 ref 路径。

## 夹具内容（`fixtures.json`）
- `cases[]`：每项 `{ id, seed, frames: [{ width, height, lines }], outputs: [base64…] }`
- **手写 10 条**（`h01`–`h10`）：首次渲染、内容不变、末尾追加、改末行、改中间行、改视口之上的行、内容缩短、内容从空增长到超过 rows、宽度变化、高度变化。
- **随机 200 条**（`r000`–`r199`）：宽度 20–80、高度 5–30、每例 2–12 帧，内容含 ASCII / CJK 全角 / SGR 颜色序列 / 空行，每帧随机增/删/改行；**种子固定**（`mulberry32`，种子 = `1000+n`）。
- 总大小 **≈0.89 MB**（上限 2 MB）。
- `outputs[i]` = 第 i 帧 `doRender()` **写入假终端的字节**（`terminal.write` 的拼接，`hideCursor/showCursor` 等非字节流操作不计入）。

## 稳定性
连续生成 3 次，sha256 全部为 `2f6462347f9043a4a6ddfe1d712e77fa1ea8eac6a264d995c89bdc2827d201c0`（912660 字节），`cmp` 逐字节一致。

## 无依赖文本侧车（Rust 单测读取）
`gen.mjs` 同时输出 `fixtures.lines.txt`（同一份数据，避免为读 JSON 给 `pi-tui` 新增 serde 依赖）：
```
case <id> <seed> <帧数>
frame <width> <height> <行数>
line <base64(utf8)>
out  <base64(utf8)>
```

## 光标夹具（独立文件）
- `gen-cursor.mjs` → `fixtures-cursor.json`（+ 文本侧车 `fixtures-cursor.lines.txt`，多一列 `show <0|1>`）。
- 与 v1 的差别：① 行内注入 `CURSOR_MARKER`（`\x1b_pi:c\x07`，`tui.ts:168`）；② 假终端把 `hideCursor/showCursor` 记为**真实字节** `\x1b[?25l` / `\x1b[?25h`（v1 忽略）；③ 覆盖 `showHardwareCursor` 两种取值。
- 用例：**手写 12**（marker 行首/中/尾、CJK 后、SGR 后、视口内/外、无 marker、marker 行被删、宽度变化、`showHardwareCursor` 两种）+ **随机 100**（在 v1 随机序列基础上每帧随机插 marker，`showHardwareCursor` 交替）。
- 规模 **524.6 KB**（上限 2 MB）；连续生成 3 次 sha256 全同 `7322b57542f4999b49261ed1f384bfd56886f062`（524562 字节），`cmp` 逐字节一致。

生成命令（同 v1，只换脚本与输出）：
```bash
node --import ./tools/difftest/tui-oracle/register.mjs \
     tools/difftest/tui-oracle/gen-cursor.mjs tools/difftest/tui-oracle/fixtures-cursor.json
```

## 退出序列夹具（独立文件）
- `gen-exit.mjs` → `fixtures-exit.json`（+ 文本侧车 `fixtures-exit.lines.txt`，`case` 行末附 base64 的 `stop` 字节）。
- 做法：沿用光标夹具的帧序列（含 marker 在不同行/无 marker/未满屏/刚好满屏/超屏），**最后一帧之后**调用真实 `TuiMainScreen.beforeTerminalStop({preserveScreen:false})`（`tui-main-screen.ts:168-176`），单独记录它写出的字节。
- 规模：**112 用例**（手写 12 + 随机 100），全部用例 `stop` 非空（样例 `" \x1b[2B\r\n"`），文件 **527024 字节**（<2 MB）。
- 稳定性：连续生成 3 次 sha256 全同 `8ddb002f842e558ae7d999dd8ec0d9467cbd9f2e`，`cmp` 逐字节一致。

## 输入夹具（独立文件）
- `gen-keys.mjs` → `fixtures-input.json`：用 ref 的真实 `StdinBuffer` + `keys.ts` 解析函数，喂入**字节分片序列**，记录 `data`/`paste` 事件与 `parseKey`/`isKeyRelease`/`isKeyRepeat` 结果。
- 用例 **220**（手写 20 + 随机 200），**63.7 KB**；连续生成 3 次 sha256 全同 `23b2cb503eb448c8030cb8855fa8e29a6db315c3`。
- 超时确定性：对孤立 ESC 等用例在 `process()` 后**手动 `flush()`** 模拟超时（`timeoutEmulated: true`），不使用真实定时器。
- 生成命令：`node --import ./tools/difftest/tui-oracle/register.mjs tools/difftest/tui-oracle/gen-keys.mjs tools/difftest/tui-oracle/fixtures-input.json`

