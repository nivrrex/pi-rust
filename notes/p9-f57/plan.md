# F57 批次计划

规格来源：`ref/pi-0.87.1/`（只读）。所有验证由 agent 在 Linux 上自动完成。

## Q1 dark 并发下 T65/T67 出现 different（**先做**）

- [x] 4 路并发（`JOBS=4`）×2 套（=8 路）→ 未复现；×3 套（=12 路）→ **复现**
      （`T116-toolrow-collapsed` 1/3 套 different，与 T65/T67 同类）
- [x] 取证：失败轮 `T116` 的 `2026h` 帧数 11（golden 9）、快照 offset 19290 落在帧边界
      19276 后 14 字节（帧中间）；快照还原后会话区**整体上滚 11 行**（21 行 vs golden 31 行）
- [x] 根因：首帧被 `waiting_first_tool_batch` **空等最多 1500 ms**（期间零字节输出），
      而 harness 的 `waitStable` 判据是"600 ms 无新字节" ⇒ 把空等当成"已稳定"，
      mark 钉在扩展工具行结果到达**之前**，之后补画的帧让视口上滚
- [x] 修：待定行 ≤ 8 时**不空等首帧**（少量行"写两遍"代价可忽略）；>8 仍按原策略
- [x] 验证：单跑 6/6 same（含 2 次 10 帧抖动轮）；12 路并发 ×3 套复测（见下）

**预测**：T65（`/tree` 已存在分支）与 T67（`--resume` 命名会话）都读**会话目录**，而
`run-tui.mjs` 每轮用 `mkdtemp` 独立的 HOME/agent/sess ⇒ 不该互相干扰。
更可能的原因：两者都对**时序**敏感（`/tree` 打开面板后等 `waitStable`，
`--resume` 的会话发现按 **mtime** 排序），4 路并发时 CPU 争抢让"静止判定"提前成立，
快照落在中间态。
**推翻条件**：若失败 run 的 `raw.bin` 里根本没有目标内容（而不是内容完整但时序不同），
则不是时序而是**读取失败**，方向改为查文件系统/路径。

## Q2 dark nogolden 69 → 0 的对账

- [x] `git log -- scripts/regress-tui.sh`：本批未改它；F56 只加了 `run_batch`（`-r` + 空 `$1` 兜底），未动 nogolden 判定
- [x] 实跑 3 runs：**444 = 375 same + 69 nogolden + 0 different + 0 runfail**
- [x] `run_batch` **没有吞行**（`[ -s list ]` 只拦空清单）。F56 报的"0 nogolden"是我用
      `awk '{print $3}' parallel.log` 统计、漏了 `serial.log` 与汇总行，属**读数错误**，不是脚本 bug

**预测**：nogolden=0 是**对的**——dark 的 148 个场景里 125 个有 golden，
另外 23 个是 `scenarios-tui/` 里存在但 `golden-tui/` 没有的；`nogolden` 应按**run** 计数
（23×3=69）。若现在报 0，说明 `run_batch` 的 `-r` 或空 `$1` 兜底把某些行丢掉了。
**推翻条件**：若 444 对账成立且 nogolden=0 且 `golden-tui` 里确实有全部 148 个目录，
则是我 F56 说错了场景数，需更正口径而不是改代码。

## Q3 首帧口径统一

- [x] 只保留一张表，两个量**分开标注**（见 `measure-firstframe.py` 的口径说明）
- [x] 7.1 s = 进程内 `PI_TUI_STAGES.first_frame_written`（单次）；7.61–7.91 s =
      `measure-firstframe.py` 的 `mark − spawn`（5 次分布，含 `waitStable` 静默窗 + PTY 读延迟）。
      两者不是同一个量，不得混比

## B 粗体差异（用户截图：pi-rs 有粗体、TS 无，以 TS 为准）

- [x] B1 真实会话 110 列，TS 短/长路径与 pi-rs 各导出一次：**bold 单元格 103/103/103**，
      `rs vs ts` 与 `ts vs tsL` 的 bold 差异均 **0 格**；逐行 SGR run 序列 40 行**全同**
- [x] B2 未复现用户的"pi-rs 有粗体、TS 无"。证据：同一会话三侧 bold 位置逐格一致。
      差异可能来自 aot 版二进制的终端能力探测（`COLORTERM`/`TERM`），本批无法在
      Linux 上复现 ⇒ 登记，不作为缺陷修复
- [x] B3 **缺陷已修**：`compare-style.mjs` 缺 underline(4/24)/strike(9/29)，其
      `renderStyledScreen` 根本不解析这两个 SGR；`real-style.mjs` 改为**自带解析器**
      并比较全部 8 个属性（fg/bg/bold/dim/italic/underline/strike/inverse）

**ref 锚点**：`packages/tui/src/components/markdown.ts:674`（行内 bold 走 `theme.bold()`）、
`modes/interactive/theme/theme.ts:1182`（`bold: (text) => theme.bold(text)`）、
`packages/tui/src/components/text.ts:60-68`（折行后补位空格继承 SGR）。
**预测**：pi-rs 在**表格/列表项**或**折行补位**处多输出 `SGR 1`（F54 修过"补位空格继承行尾属性"）。
**推翻条件**：若两侧 `SGR 1` 出现位置**完全一致**，则差异来自 aot 二进制的终端能力探测，需给证据。

## C `/allcopy` 与 shim 占位

- [x] C1 取证：`/allcopy`（`pi-handoff-gate/index.ts:183,203`）只调
      `copyToClipboard`（来自 `@earendil-works/pi-coding-agent`）；ref 实现
      `utils/clipboard.ts:74-160` + `utils/clipboard-command.ts:5-53`
- [x] C2 已实现（回退顺序：平台命令 → WSL(OSC52/PowerShell) → OSC 52；
      上限 100_000；失败文案逐条照 ref）
- [x] C2 测试：`tools/ext-host/test/shim-clipboard.test.mjs` **8 项**
      （xclip 收到精确 stdin / xclip 失败退 xsel / 非零退出算失败 / 无可用写命令抛 X11 提示 /
      headless 发 OSC 52 / 远程会话发 OSC 52 / 超长抛 limit 错误 / wayland 走 wl-copy）；
      改前 0/8 通过、改后 8/8
- [x] C3 shim 剩余占位（`ExtensionAPI`/`ExtensionContext`/`SessionEntry`/`Theme` 等
      类型导出为 `undefined`，仅类型用途）已登记；用户已装扩展实际用到的
      `copyToClipboard` 已实现

**ref 回退顺序**（`utils/clipboard.ts:74-160`）：非 linux 先 `getNativeClipboard().setText`；
否则按平台命令（darwin `pbcopy` / win32 `clip` / linux：`termux-clipboard-set`、
`wl-copy`（有 `WAYLAND_DISPLAY`）、`xclip`/`xsel`（有 `DISPLAY`））；再 WSL 分支
（`WT_SESSION` 先 OSC 52，否则 PowerShell via `wslpath`）；最后 OSC 52
（上限 `MAX_OSC52_ENCODED_LENGTH=100_000`，超出抛 `Clipboard unavailable: text exceeds the OSC 52 size limit`）。

## D `/todo` 与宽度错误

- [x] D1 在 `tmp/realcfg`（真实配置副本）与 `--from-home`（真实 Linux 会话）上，cols=120
      跑 `/todo` 全交互（空清单提示、`/todo` 补全菜单、`clear`）；两侧屏幕均无越界行
- [x] D2 **根因**：`unicode-width` 把 `\t` 算 0 列，而未展开的制表符在终端按 8 列制表位渲染
      ⇒ 渲染行远超 `cols`。合成复现：80 列下扩展渲染行 `\t`×100 渲染成 **101 列**；
      120 列下同理可到 121+（`121 > 120` 同源）
- [x] D3 修：扩展 `renderCall`/`renderResult` 行折行**前**把 `\t` 展开成 3 空格
      （ref `packages/tui/src/components/text.ts:61`）。用例 `t_f57_tab_width`（3 项），
      改前 3/3 失败（`cols=80 时有 1 行超宽 [(3, 101)]`）、改后全过。**不用 clamp**
- [x] D4 ref `tui-main-screen.ts:520-546`：写 crash log 后 `stop()` + **throw**，屏幕不留错误文字。
      pi-rs 原来 `eprintln!("跳过本帧")`——TUI raw 模式下 stderr 在同一 PTY ⇒ **会打烂画面**
      （实测 `raw.bin` 里能搜到该字符串）。已改为计数 + 可选落盘（`PI_TUI_RENDER_ERRORS`）
- [x] D5 除宽度错误外**未复现**其它 `/todo` 问题（空清单提示、补全菜单、`clear` 均正常）。
      需要用户提供的最小输入见最终回复（一条命令的输出），本项以"给出最小输入"收口，不算未完成

## E 首屏耗时拆解

- [x] E1 拆 `tui_enter → first_draw_begin`（实测 2757→5387 = **2630 ms**）：
      - `tui_enter → transcript_built` = 1463 ms（`transcript_from_messages`：6.4 万条会话里
        构造转录条目，含 markdown/工具行条目切分）
      - `transcript_built → renderers_filled` = 16 ms（扩展开机渲染回填，几乎为 0）
      - `renderers_filled → first_draw_begin` = 1151 ms（首帧 `render_all` 全量排版）
      首帧总 `first_frame_written` = **5420 ms**
- [x] E2 **方案：只排版可见视口的行**（省掉 1151 ms 的大部分）
      - 预测：若 ref 也只渲染视口，则可行；否则会改变 `render()` 的返回值语义（`ScrollView`
        需要全量行才能算 `contentHeight`/滚动位置）
      - **推翻条件（已成立）**：`ref/packages/tui/src/components/scroll-view.ts:215-219`
        `render()` 里是 `this.child.render(contentWidth)`——**全量渲染**，
        视口裁剪在之后的布局层（`[LAYOUT_NODE]`）。裁剪会改 `contentHeight`、破坏
        `followEnd`/滚动条的语义 ⇒ **不做**。
- [x] E3 剩余构成与不可优化证据：
      - `tui_enter → transcript_built` 1463 ms：`transcript_from_messages` 遍历 6.4 万条
        会话条目构造转录项。ref `renderInitialMessages()`（`interactive-mode.ts:4056-4062`）
        同样遍历 `buildContextEntries()` 的全量结果并逐条建组件，**同阶**。
      - `renderers_filled → first_draw_begin` 1151 ms：`render_all` 全量排版，与 ref 同构（见 E2）。
      - 与 pi aot（约 3.7 s）的差距 ≈ 1.7 s，主要来自"Rust 侧额外一次 clone 全量转录"
        （`AgentParts::initial_messages` + `resumed_entries` + `transcript` 三份），
        属**数据结构冗余**而非算法差异，本批不做（会动 `AgentParts` 的公共形状）。

## L3 批末

cargo clean → test-gate、clippy -D warnings、precommit、regress dark（并发+串行各一次）+ light、
self-verify、real-parity、real-parity-linux、privacy-scan、readonly、check-orphans、
git status 干净、删 `tmp/realcfg*`。

## L3 批末实测（HEAD 见下）

`cargo clean` → 全量重建 →
- `test-gate` **225 组 / 1181 passed / 0 failed**；`cargo fmt --check` 干净；
  `clippy --workspace --all-targets -D warnings` **0 告警**
- `precommit` **16/16 通过**
- `regress-tui` dark **并发 375 same / 0 different / 69 nogolden**、
  **串行（`--jobs 1`）375 same / 0 different / 69 nogolden**；light **81 same / 0 / 0**
- `self-verify` **26/26 same, rc=0**；`real-parity-linux` **8/8 same, rc=0**
- `privacy-scan` 干净；`readonly` 8+2 一致；`check-orphans` 0
