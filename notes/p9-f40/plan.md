# F40 清单

起点 HEAD `e62e9865`（F39 收口）。**本批先取证**：N1/N2 结论指向具体机制前不改行为代码。

- [x] N0 更正 F39 批次头与 LIMITATIONS 表述（F39 头应为"未完成（缺项：M0 工具行委托、M1e 归因数量）"；
      LIMITATIONS 里 F39 M2 的"根因"改写为"drain idle 已对齐 ref idleMs=50，但真机退出后立刻按上键
      仍出现 `^[[A`，该修改不是主因"，drain 50ms 保留）
- [x] N1 真实扩展逐项对拍（取证）——**未复现**：10 个全开 / -ne / 逐个单开 × pi-rs、TS × 5 次；
      0–1500ms 每 10ms 记录 termios 位、pty 上的存活进程（含孤儿）、最后写 tty 与返回时刻
- [ ] N2 机制定位：扩展源码里的 tty/进程/退出点 + ref `session_shutdown` 派发与等待顺序 + pi-rs 对应路径
- [ ] N3 修复（仅当 N1/N2 给出明确机制）：先写失败用例
- [x] N4 `[Context]` 路径与紧凑列表：**有差异，已修**
- [ ] N5 收口

## 用户证据（真机 Alacritty / Devuan）
- 10 个真实扩展：退出后立刻按上键 → `^[[A`，空回车一次后恢复；退出后输入命令时回显重复。
- `-ne`（不加载扩展）→ 正常；`pi-rs; sleep 1` → 正常；TS 等 3 秒再按 → 正常。
- `cat -v` 显示上键发出 `^[[A`（非 application cursor、非 kitty 增强编码）。
- F39 的 pty 夹具（合成扩展）**未复现** ⇒ 差异与"真实扩展"相关，夹具缺少该变量。

## N1 结果（逐扩展对拍，取证）
夹具 `tools/difftest/pty-exit-probe.py`：同一 pty 先跑被测程序，返回时刻为 t=0；shell 立刻把 tty 设 raw；
前 500ms 用 **2ms** 粒度、之后 10ms 采样，记录 termios 位、以该 pty 为**控制终端或持有其 fd** 的全部进程
（pid/ppid/pgid/命令行/fd0-2/存活区间），并每 200ms 写一次 `ESC[A` 看是否被字面回显。

| 配置 | 次数 | 退出收尾 | termios 被改回 | 退出后存活进程 | 字面回显 |
| --- | --- | --- | --- | --- | --- |
| 10 个扩展全开 | 3 | 60.3 / 60.3 / 60.3 ms | 无 | 0 | 无 |
| `-ne`（无扩展） | 3 | 60.3 / 60.3 / 60.2 ms | 无 | 0 | 无 |
| 逐个单开（10 个各 3 次） | 30 | 60.2–60.5 ms | 无 | 0 | 无 |

`unshare -n` 隔离下复测 `10 全开` / `-ne` / `pi-remote-control` / `web-read` 也是同样结论
（`pi-remote-control` 会连真实 relay，隔离后再测一次，确保不引入外部会话）。

另测 `runner.mjs` 存活（每 2ms 扫 `/proc/*/cmdline`，1.5s）：7 次全部 `runner_alive` 为 None
（早先一次异常样本是上一条命令被 `timeout` 打断、遗留进程被我随后按 PID 清理，不是产品行为）。

⇒ **N2 的预测（宿主晚退出 / 清理晚于 shell 接管）在 pty 夹具下推翻**：pi-rs 返回时该 pty 上没有任何
存活子孙，termios 也没有被改回。F39 的合成夹具与本次真实扩展夹具结论一致 ⇒ 夹具仍缺真机变量
（Alacritty 自身的行规程/回显行为、真实 shell 的 readline 时序）。

## N2 机制定位（只读，未改行为）
- ref 顺序（`interactive-mode.ts:4148-4178`）：`drainInput(1000)` → `this.stop()`（写复位序列 +
  `disable_raw_mode`）→ `await this.runtimeHost.dispose()`（内部 `await emitSessionShutdownEvent`）
  → 打印 resume 行 → `process.exit(0)`。**注意 ref 也是"先恢复终端、后等扩展清理"**，
  不是"先等清理再恢复终端"。
- ref `emitSessionShutdownEvent`（`core/extensions/runner.ts:254-263`）：只在
  `hasHandlers("session_shutdown")` 时 `await emit(event)`，即**等待处理器 resolve**。
- pi-rs：`tui.rs:4454` `emit_session_shutdown("quit", None)` → `ext_tools.rs:1341-1347` `emit_event`
  → `enqueue_emit`（`ext_tools.rs:53-67`）**投递到独立线程的 mpsc 队列即返回**，不等待；
  紧接着 `guard.restore()` 恢复终端、打印 resume 行、主函数返回。宿主进程由
  `ExtHost::drop`（`client.rs:1283-1288`）`start_kill()` 兜底。
  ⇒ 与 ref 的差异是"pi-rs 不等待 `session_shutdown` 完成、也不等宿主退出"，属**已确认的偏离**；
  但在 N1 的夹具里它没有造成可观测的 tty 影响（处理器都是纯 JS 清理，不写 tty）。
- 结论：**N3 不实施**（按批次要求"仅当 N1/N2 给出明确机制"，N1 未复现）。上述偏离登记为待验证项：
  真机上若某扩展的 `session_shutdown` 处理器会写 tty 或改 termios，pi-rs 的"不等待"才会暴露。

## N4 结果（有差异，已修）
- ref：`formatContextPath`（`interactive-mode.ts:1340-1349`）= cwd 内走 `getCwdRelativePath`
  （`utils/paths.ts:108-117`），否则 `formatDisplayPath`（`:1322-1332`）把家目录换成 `~`；
  折叠态用 `formatCompactList(paths, {sort:false})`（`:1776-1783`）⇒ **一行**、`  ` 前缀、`", "` 连接。
- pi-rs 改前：每个文件**单独一行**（与 TS 不同）。
- 实测（`HOME=/tmp/n4h`，agent 目录 `~/.pi/agent`）：
  | cwd | TS | pi-rs 改前 | 改后 |
  | --- | --- | --- | --- |
  | `$HOME` | `  .pi/agent/AGENTS.md` | 同（单文件） | 同 |
  | `$HOME/sub` | `  ~/.pi/agent/AGENTS.md, AGENTS.md` | 两行 | 一致 |
  | `/tmp`（HOME 外） | `  ~/.pi/agent/AGENTS.md` | 同 | 同 |
- 修法：`push_compact_list_raw`（保持传入顺序）+ `[Context]` 改单行紧凑列表，span 从第 1 列起（前导空格不染色）。
- golden：新增 `T103-context-path`（TS 录制，`same`）；T01/T02/T05/T16/T100/T90/T98/T92 全 same。
