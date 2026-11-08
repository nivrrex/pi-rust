# F41 清单

起点 HEAD `d8dee2cd`。目标：修"退出后首个按键字面回显 `^[[A`"（F40 未定位，本批有真机 strace）。

- [x] P0 复现：新增 `crates/pi-cli/tests/p9_f41_host_no_tty.rs` + `tools/difftest/tty-run.sh`。
      关键：**必须让 pi-rs 自己挂在 pty 上**（`script(1)` 分配），否则子进程天然无 tty、测不出来。
      改前 FAILED：`[(0, "pipe:[…]"), (1, "/dev/pts/5"), (2, "/dev/pts/5")]` —— 宿主 fd 1/2 是 tty。
- [x] P1 机制（行号）：
      - `crates/pi-ext-host/src/client.rs:372-377`（改前）：`.stdin(piped).stdout(inherit).stderr(inherit)`
        ⇒ 宿主 node 的 fd 1/2 **继承 pty**，fd 0 是管道（真机 strace 也显示 fd 0 为 ENOTTY）。
      - `tools/ext-host/runner.mjs`：**没有任何** `setRawMode`/`tcsetattr`/`isTTY` 依赖；
        对 tty 的写回来自 Node/libuv 自身的 `uv_tty_reset_mode`（进程在 tty 上时退出会写回
        启动时保存的 cooked 模式），不是扩展或 runner 的显式代码。
      - 复现三个前提同时成立才会出现：①宿主继承 tty ②宿主晚于 pi-rs 退出 ③shell 已在两者之间
        把 tty 设为 raw。
- [x] P2①：宿主 stdio 全部改为**管道**（`stdin=piped`、`stdout=piped`、`stderr=piped`），
      并由宿主把「协议外的 stdout」与「stderr」两条管道**转发到自己的 stderr**
      （`client.rs` 新增 `StreamKind` + 两个 `tokio::spawn` 读取任务）。实测扩展的
      `process.stderr.write` 与 stray `console.log` 都出现在宿主 stderr（3/3 条），stdout 未被污染。
      runner 侧依赖 `isTTY/columns` 的地方：无（已 grep 确认）。
- [x] P2②：退出顺序对齐 ref `interactive-mode.ts:4148-4178`：
      `emit_session_shutdown_sync(≤1s)` → `shutdown_host(≤1s，超时按 PID kill)` → `guard.restore()`
      → 打印 resume 行。新增 `ext_tools::emit_session_shutdown_sync` / `ext_tools::shutdown_host`。
      `emit_session_shutdown` 保留为同步入口（内部转调，1s）。
- [x] 退出耗时：改前 55.8 ms / 改后 55.9 ms（`--list-tools` + 单扩展，pty 夹具），
      TS 60.3 ms；`runner.mjs` 退出后残留：无；termios 未被改回。
- [x] 用例：`p9_f41_host_no_tty` 改前 FAILED → 改后 ok；
      `p9_f36_exit_terminal` 3/3、`p9_f38_ctrlc_double` 2/2、`p9_f39_exit_timeline` 1/1 全通过。
- [x] golden 回归：T01/T05/T16/T90/T100/T103/T98/T89i/T96 全 same。
