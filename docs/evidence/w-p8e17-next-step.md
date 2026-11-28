# 下一步：抓一次真实残留（H1 已否证切分假设）

H1 结论已接受并合入主分支：**"4096 切分切断序列"不成立**，
`LIMITATIONS.md` 与 `write_frame` 注释已改成"根因未定位 + 按行写入属防御性做法"。

现在没有证据支持任何具体机制，唯一能推进的是**拿到残留发生瞬间的原始字节**。
请在 Windows 侧按下面流程做一次（或多次）复现，并把结果提交到本仓 `win-fixes`。

## 一、用哪个二进制

用 master 最新 HEAD 重编（本次刚修掉 Windows 目标上的三处既有问题：
死导入导致 clippy 失败、`startup_bytes_never_enter_alt_screen_for_r3` 缺平台门、
`forward_event`/`forward_sequence` 的 never-used 警告）。复现时请记录：

```
pi-rs --version        # 期望形如 "built … commit <HEAD>"，不要带 -dirty
sha256sum pi-rs.exe
```

## 二、复现步骤（贴近用户原场景）

1. **WezTerm** 窗口，输入法切到 **rime/小狼毫 中文态**（用户报障时的状态）；
2. 启动 `pi-rs.exe`，等首屏画完；
3. 用 **Shift+Insert** 粘贴 12 行以上文本（>10 行会折叠成 `[paste #1 +N lines]`）；
4. 按 ↑/↓ 各 10 次、End/Right 连点 20 次、Home 两次；
5. 输入 `/help` 回车，再输入一行中文（触发候选框）；
6. 全程开启 `PI_TUI_OUTPUT_DUMP`：

```bat
set PI_TUI_OUTPUT_DUMP=%TEMP%\pi-rs-output.log
set PI_TUI_CURSOR_DUMP=%TEMP%\pi-rs-cursor.log
pi-rs.exe
```

7. 出现异常字符（`;;` / `0m` / 其它残片）时**立即截图**，截图里要能数出它落在第几列
   （或直接用 `wezterm cli get-text` 取当前窗格文本，连坐标一起存）。

## 三、要提交的东西

| 项 | 说明 |
|---|---|
| `docs/evidence/w-p8e17-repro-<n>.md` | 复现记录：二进制 sha、操作序列、残留字符串、落在第几列/哪一行、是否每次都能重现（k/N 次） |
| `%TEMP%\pi-rs-output.log` | 原样附上（若太大，截取残留下标前后各 3 帧，并注明截取规则） |
| 截图 / `get-text` 输出 | 与 dump 同一时刻 |

## 四、判读口径（提交前先自查）

1. **在 dump 里找到残留那一行的字节**：看行尾序列是否完整（`\e[0m\e]8;;\a`）。
   - 完整 ⇒ 我们发的是对的，问题在 ConPTY/终端侧（下一步对比官方 pi 在同一机器同一流程的表现）；
   - 不完整（半截序列）⇒ 我们发出去就是坏的，回查 `diff::write_frame` 与实际写入路径。
2. 记录残留所在行是**该帧第几条写入**、该帧总字节数与是否含 `\e[?2026h`。
3. 若 20 次内一次都复现不了，也请提交记录（k=0），并补一组**同机器上用官方 pi 0.87.1
   跑同样操作**的对照（有没有残片），这能直接区分"我们的输出形态"与"终端/ConPTY 本身"。

## 五、不要做的事

- 不要再按"少发 OSC 8"或"少发 2004h"去改：前者已被证据否掉（当时把行尾复位换成
  `ESC[0m` 后，残留就从 `;;` 变成了 `0m`，说明被打坏的是**行尾任意序列**，不是 OSC 8 特有）；
- 不要在复现前先改视觉/写入逻辑，否则又变成"改一版猜一版"。
