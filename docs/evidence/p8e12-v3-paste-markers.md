# P8e-12 V3：Windows 上「真实粘贴」会不会带来括号粘贴标记

实测日期 2026-10-29；机器 Windows 10 LTSC 19044；**conhost**（本机未安装 Windows Terminal / WezTerm）。
被测对象：`tools/windows/probe-paste-markers.ps1`（本批新增）+ 驱动程序（本机脚本，不入库）。

## 方法

1. 探针先置 `ENABLE_VIRTUAL_TERMINAL_PROCESSING`（与 pi-rs 启动时一致），再按需置/清
   `ENABLE_VIRTUAL_TERMINAL_INPUT`；
2. 向**控制台**写 `ESC[?2004h`（括号粘贴开启，与 pi-rs 的启动序列一致）；
3. 驱动把剪贴板设为 `L1\nL2\nL3\n`，用 `SendInput` 发 **Shift+Insert**（真实按键 → conhost 自己执行粘贴），
   每轮前用 `AttachThreadInput` + `SetForegroundWindow` 保证焦点在探针窗口；
4. 探针用 `ReadConsoleInputW` 读原始 `KEY_EVENT` 记录，按 400 ms 空闲切分一轮，记录字符、是否出现
   `ESC[200~` / `ESC[201~`。

## 结果

| 模式 | 有效轮次 | 每次送达的字符（十六进制） | 有 `ESC[200~`/`ESC[201~` |
|---|---|---|---|
| `default`（VT_INPUT=False） | 3/3 | `4C 31 0A 4C 32 0A 4C 33 0A`（`L1\nL2\nL3\n`） | **0/3** |
| `vt`（VT_INPUT=True） | **20/20** | `1B 5B 32 3B 32 7E`（`ESC[2;2~`，即 Shift+Insert 键本身），**没有任何正文** | **0/20** |

- `default` 的后续批次因前台焦点被其它窗口抢走（`keys=0`）而作废，故样本为 3 次；这是自动化限制，
  不是被测行为。
- `vt` 的 20 次全部拿到了按键序列而不是粘贴内容 ⇒ **在 VT 输入模式下 conhost 不再替应用执行粘贴**，
  只把 Shift+Insert 作为 `ESC[2;2~` 交给上层。

## 结论（对 V3 判据）

判据是「VT 模式 20/20 标记完整到达 → 才改走 VT 字节路径」。实测：

1. **标记一次都没出现**（两种模式 0 个 `ESC[200~`）——conhost 不实现括号粘贴，与是否开 VT 输入无关；
2. VT 输入模式还**额外丢掉**了 Shift+Insert 的粘贴动作（只剩按键序列），要在应用层补粘贴就得读剪贴板
   （新依赖或 `unsafe` FFI）。

⇒ **不改走 VT 输入路径**；保留现有的启发式 `PasteBurst`（30 ms 攒批 + 60 ms 回车回看）作为 Windows 粘贴
的兜底。`Windows Terminal` 是否会给 `ESC[200~` 需要另测（本机未安装，未验证）。

## 未做（环境限制）

- V2（WezTerm CLI 场景矩阵）：本机无 WezTerm（`which wezterm` 无命中）；
- V6（光标行列逐帧对比）：需要 `PI_TUI_CURSOR_DUMP` 调试开关（代码里不存在）与 TS 侧同口径录制，Windows 侧无法单独完成。
