# H1：4096 字节切分假设的机制实证 —— **不成立**

日期 2026-10-31；环境 Windows 10 LTSC + WezTerm `20240203-110809-5046fc22`。
实验程序已入库到 `tools/windows/h1/`（开发时的副本在 `%USERPROFILE%\pi-rs-wintest\h1\`），
**不含任何 pi-rs 代码**；
测试文本全为合成内容（`A`/`L` 填充、占位行），未读取任何凭据。

## 一、结论（先说）

**"控制台按 4096 字节切分写入、切点落在转义序列内部 ⇒ `0m` / `;;` 残留"这一机制在本机
两种宿主上都没有复现。**

- 覆盖切点：`ESC[0m`（4 字节）与 `ESC]8;;BEL`（7 字节）内部的**每一个字节位置**，
  以及在 `std` 的 1024 字节行缓冲边界上的切点；
- 两种宿主：**ConPTY**（WezTerm 窗格，即用户实际路径）与**经典 conhost**
  （`CreateProcess(CREATE_NEW_CONSOLE)`，已按 pi-rs 的做法打开
  `ENABLE_VIRTUAL_TERMINAL_PROCESSING`）；
- 三种写法：`std`（`stdout().write_all`）、`wcw`（单次 `WriteConsoleW` 对照）、
  `split`（按 4096 硬切成两次 `write_all`，直接模拟"被切断"的形态）；
- 合计 72 个宿主×用例：**0 个残留**（3 次读取为空是 `AttachConsole` 竞态，重跑 3 次全部通过，
  见 §四）。

探针已随证据入库：`tools/windows/h1/`（`h1.rs` + `h1_probe.py`）。用**入库版本**重跑整个矩阵
expected `cases=72 bad=0`：

```
cases=72 bad=0 retried=0
```

按批次纪律，H1 不成立 ⇒ **撤回"已确认根因"的说法**，H2 的代码改动不实施（闸门未过）。
`LIMITATIONS.md` 与 `write_frame` 注释里那句根因需要改写，替换文本见 §五。

## 二、实验设计

`h1.exe <shape> <mode> [prefix]`：

| shape | 输出内容 | 目的 |
|---|---|---|
| `csi` | `'A'*prefix + ESC[0m + "TAIL"` + CRLF + `<<END>>` | CSI 跨切分边界 |
| `osc` | `'A'*prefix + ESC]8;;BEL + "TAIL"` + CRLF + `<<END>>` | OSC 8 关闭序列跨边界 |
| `real` | 现状形态：一行文本 + `ESC[0m` + 空格 + `ESC]8;;BEL` + CRLF | 贴近实际帧形状 |
| `real-long` | 同 `real`，行内的 `'L'` 数可调（默认 3000） | 长行（行内序列靠近边界） |

| mode | 写法 |
|---|---|
| `std` | `std::io::stdout().write_all(&bytes)` 一次写完 |
| `wcw` | 一次 `WriteConsoleW`（UTF-16，控制台句柄路径的对照） |
| `split` | 按 4096 字节 `chunks(4096)` 逐块 `write_all`（= 假设里的"被切断"） |

切点矩阵（`split` 模式，`prefix` 决定 4096 边界落在序列的第几个字节）：

| prefix | 4096 边界落在 |
|---|---|
| 4088–4091 | 序列**之前**（对照） |
| 4092 / 4093 / 4094 / 4095 | `ESC[0m` 的第 2 / 3 / 4 字节之间；`ESC]8;;BEL` 的第 2…5 字节之间 |
| 4096 / 4097 | 序列之后（对照） |

第二条矩阵针对 `std` 的**行缓冲**（`LineWriter` 容量 1024）：`real-long` 的行首是 19 字节真彩
前缀，令 `ESC[0m` 的起点 = 1022 / 1023 / 1024 / 1025（`prefix` = 1003…1006）。

读回方式：ConPTY 用 `wezterm cli get-text`；conhost 用 `AttachConsole` +
`ReadConsoleOutputCharacterW` 读整个屏幕缓冲区（400 行）。

## 三、结果

### ConPTY（WezTerm 窗格）

| 组 | 用例数 | 残留 `0m` | 残留 `;;` | TAIL/`<<END>>` 完整 |
|---|---|---|---|---|
| shape×mode（csi/osc/real/real-long × std/wcw/split，prefix 4094 或 3000） | 12 | 0 | 0 | 12/12 |
| 4096 切点扫描（csi/osc × prefix 4088–4097，split） | 20 | 0 | 0 | 20/20 |
| 1024 行缓冲切点（real-long × prefix 1003–1006，std） | 4 | 0 | 0 | 4/4 |

### 经典 conhost（`CREATE_NEW_CONSOLE`，VT 已开）

| 组 | 用例数 | 残留 `0m` | 残留 `;;` | TAIL/`<<END>>` 完整 |
|---|---|---|---|---|
| shape×mode | 12 | 0 | 0 | 12/12（1 次读取为空，重跑通过） |
| 4096 切点扫描 | 20 | 0 | 0 | 20/20（2 次读取为空，重跑通过） |
| 1024 行缓冲切点 | 4 | 0 | 0 | 4/4 |

补充对照（未开 VT 时）：屏幕上出现的是**字面量** `[0m` / `]8;;`（整段序列都没被解释），
这与"切分丢字符"是两种现象 —— 这类情况只有在输出句柄没开
`ENABLE_VIRTUAL_TERMINAL_PROCESSING` 时出现，pi-rs 启动时已通过 `supports_ansi()`
打开，不属于本批讨论范围。

## 四、读取为空的 3 次（harness 竞态，非缺陷）

| 用例 | 现象 | 重跑 |
|---|---|---|
| conhost `real-long/wcw/3000` | `AttachConsole` 后读到空屏 | 3/3 通过 |
| conhost `csi/split/4092` | 同上 | 3/3 通过 |
| conhost `osc/split/4096` | 同上 | 2/3 通过（第 1 次仍空屏） |

空屏的特征是 `srWindow.Bottom = 29`（默认窗口高、光标未移动），而正常组是 38 ⇒ 读到的是
"命令还没输出"的窗口状态，不是输出被吞。ConPTY 组没有该现象。

## 五、需要改写的既有结论（供 Linux 侧合入）

`LIMITATIONS.md`（master `653e941`）第 142–150 行现文把机制写成事实，应改为"未复现的假设 + 防御性做法"：

> ### 行尾多出 `;;` / `0m`（旧构建；**根因未定位**）
>
> 旧构建在 Windows 上曾偶尔在行尾多出 `0m` 或 `;;`。当时的假设是：写入被按固定块切分，
> 切点落在 `ESC` 与参数之间，控制台丢掉半截序列、把剩余字节当普通文本显示。
> **该机制未能复现**（2026-10-31，两种宿主 × 序列内每个字节位置的切点，见
> `docs/evidence/w-p8e17-h1-4096-boundary.md`），因此不再作为结论。
> 现在仍然**按行写入**（每次写入以换行结尾），这是防御性做法：它把切点固定在行尾，
> 使"序列被切开"在该路径上不可能发生，成本是一次 `write_all` 的额外系统调用。
>
> 若仍看到异常字符：设 `PI_TUI_OUTPUT_DUMP=1`（或一个路径）再运行一次，
> 它会把每帧实际写给终端的字节转义后落盘（默认 `%TEMP%\pi-rs-output.log`，
> `\e`/`\r`/`\n` 均转义），并记录异常字符出现在屏幕的第几列。

`crates/pi-cli/src/tui.rs` 的 `write_frame` 文档注释（master `653e941`，第 3541–3553 行）
同样把"真机两轮实测"写成因果，建议改为"当时的假设；H1 未能复现（证据同上），保留按行写入作为防御"。

## 六、真实输出字节（master `653e941` 自编，`PI_TUI_OUTPUT_DUMP`）

构建：`cargo build --release -p pi-cli --target x86_64-pc-windows-gnu`，
`--version` = `pi-rs 0.87.1 / built 2026-10-31T02:35:22Z commit 653e941`，
sha256 前 32 位 `c2a981606af17dbb4b53b7431d4e8ef3`。

场景：WezTerm 窗格启动 → 括号粘贴 12 行合成文本（折叠成 `[paste #1 +12 lines]`）→ 输入 `abc`
→ `/help`。抓到 5 帧：

| 指标 | 实测 |
|---|---|
| 帧数 | 5 |
| 单帧最大字节 | **1 762** |
| 最长一行（转义后字符数） | **347** |
| 超过 4096 字节的行 | **0** |
| 行尾字节形状 | `…\e[0m\e]8;;\a` + `\r\n`（每行都完整，无半截序列） |

即：按行写入后，单次写入最长不到 2 KB，且每块都在换行之后结束 —— 序列不可能被切。

## 七、顺带发现的既有问题（master `653e941`，**本批未改**）

在 Windows 目标上跑 master 的验证门时碰到三处既有问题，与 H1 无关，留给 Linux 侧决定是否修：

| # | 位置 | 现象 | 建议改法 |
|---|---|---|---|
| 1 | `crates/pi-core/src/tools/bash.rs:292` | `clippy -D warnings` 在 Windows 目标失败：`unused import: std::os::windows::process::CommandExt` | 删掉该行（`cmd.creation_flags` 解析到 `tokio::process::Command` 的固有方法，不需要该 trait） |
| 2 | `crates/pi-cli/src/tui.rs:3841` | `cargo test -p pi-cli` 在 Windows 目标 148/1 失败：`startup_bytes_never_enter_alt_screen_for_r3` 断言 `缺 2004h` | 上方已有 `#[cfg(windows)] assert!(!r3.contains("\x1b[?2004h"))`，因此顺序断言（`paste_at < hide_at`）必须整段放进 `#[cfg(not(windows))]`；Windows 分支改断言 `contains("\x1b[?25l")` |
| 3 | `crates/pi-cli/src/tui.rs:4763/4777` | `forward_event` / `forward_sequence` 在 Windows 目标报 `never used`（调用点全在 Unix 分支） | 给两个函数加 `#[cfg(not(windows))]`（同一批问题，不影响行为） |

这些不是我改出来的：本批对仓库的改动只有证据文档与本目录的探针（`git diff` 可核）。

## 八、本次验证跑出的实际数字

| 项 | 对象 | 结果 |
|---|---|---|
| `cargo test -p pi-cli --lib` | master `653e941` / windows-gnu | **148 通过 / 1 失败**（§七 #2） |
| `cargo test -p pi-tui --lib` | 同上 | 80 / 0 |
| `cargo clippy -D warnings` | 同上 | 失败（§七 #1、#3） |
| `cargo fmt --all --check` | 同上 | 0 |
| `cargo test -p pi-cli --lib` | 本地 `win-fixes 30c7e41` / windows-gnu | 139 / 0 |
| `cargo test -p pi-tui --lib` | 同上 | 80 / 0 |
| `cargo build --release -p pi-cli` | 同上 | 成功；`pi-rs 0.87.1 / built 2026-10-31T02:51:33Z commit 30c7e41`，sha256 前 32 位 `4ba8e9e294df00798d395bd83f0a13bf` |

## 九、这次**没有**证明的事

- 没有证明 `;;` / `0m` **再也不会出现**。本次只否证了"切分切断序列"这一条机制；
  终端侧（WezTerm 对 OSC 8 的处理）、ConPTY 的重序列化、或旧构建的其它缺陷仍可能是来源。
- 没有覆盖用户原场景的输入法（rime）+ 真鼠标右键组合；那一路的证据在
  `docs/evidence/w-p8e16-marker-leak.md`（60 次真机矩阵，`0` 残留）。
- 若再次出现，必须同时给：`PI_TUI_OUTPUT_DUMP` 落盘文件 + 异常字符在屏幕第几列，
  才能区分"我们发错"还是"终端解析错"。

## 十、复跑

```bat
cd tools\windows\h1
rustc -O h1.rs -o h1.exe
python h1_probe.py --matrix core        :: shape×mode
python h1_probe.py --matrix boundary    :: 4096 切点全位置
python h1_probe.py --matrix linebuffer  :: 1024 行缓冲切点
python h1_probe.py --matrix all --json h1.json
```

`h1_probe.py` 自带两种宿主（ConPTY 走 `wezterm cli`，conhost 走 `CreateProcess`+屏幕缓冲区），
不依赖 pi-rs 的测试夹具；`end_found=false` 会自动重试（`--retries`，默认 3）。
