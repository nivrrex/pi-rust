# pi-rs

`pi-rs` 是 [pi](https://github.com/badlogic/pi-mono) **v0.87.1** 的 Rust 重写：
单二进制 coding agent，提供 TUI / print / json / rpc 四种模式，
与原版的 **session 文件、`settings.json`、`models.json`、`auth.json` 互通**
（可以拿原版的会话与配置直接继续用）。

不依赖 Node/Bun，不需要 OpenSSL（TLS 走 rustls，根证书内置，不读系统 `/etc/ssl`）。

## 当前状态

| 平台 | 产物 | 状态 |
|---|---|---|
| Linux x86_64 | `pi-rs-linux-amd64`（glibc） | 已验证 |
| Linux x86_64 | `pi-rs-linux-amd64-musl`（static-pie） | 已验证 |
| Linux aarch64 | `pi-rs-linux-arm64`（static-pie） | 已验证 |
| Windows x86_64 | `pi-rs-windows-amd64.exe` | **实验性**，交叉编译 + 有限真机验证 |

Linux 侧的验证方式是与上游 `pi` 0.87.1 做**逐字节差分**：同一批场景分别喂给两侧
（provider 为本地 mock，全程不联网、不动真实凭据），比对原始输出流、
终端终态与请求内容。当前 93 个 CLI 场景、72+12 个 TUI 场景全部一致。

未实现的部分、与上游的行为差异、Windows 侧的已知问题见 [`LIMITATIONS.md`](LIMITATIONS.md)。

## 安装

从 Releases 下载与平台对应的二进制，放到 `PATH` 里即可（musl 产物是静态链接，
可直接拷到任意 x86_64 Linux）：

```bash
chmod +x pi-rs-linux-amd64-musl
mv pi-rs-linux-amd64-musl ~/.local/bin/pi-rs
pi-rs                 # TUI
```

## 构建

工具链固定在 `rust-toolchain.toml`（stable）。

```bash
# glibc（开发用）
cargo build --release -p pi-cli          # 产物：target/release/pi-rs

# musl 静态（部署用）
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl -p pi-cli

# Windows 交叉编译
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p pi-cli
```

## 运行

```bash
pi-rs                 # TUI（默认）
pi-rs -p "问题"        # print 模式（一次性回答）
pi-rs --json          # json 事件流
pi-rs --rpc           # rpc 模式（stdin/stdout JSON）
pi-rs -c              # 续写最近的会话
```

配置目录：`PI_CODING_AGENT_DIR`（缺省 `~/.pi/agent`）；
会话目录：`PI_CODING_AGENT_SESSION_DIR`（缺省 `<配置目录>/sessions`）。

## 主要功能

- 四种模式：TUI（默认）、`-p` print、`--json` 事件流、`--rpc`；
- **无可用模型也能进 TUI**：启动只打印 `Warning: No models available. Use /login …`，
  用 `/login` 配置（支持 api_key 型 provider），成功后**当次会话即可对话**；
- `/login`、`/logout`：写/删 `<配置目录>/auth.json`（原子写，Unix `0600`，保留未知键）；
- 主题 `dark`/`light`（`settings.json` 的 `theme`，`/settings` 里可切换并立即重绘）；
- 思考档位 off/minimal/low/medium/high/xhigh/max（`/thinking`、`ctrl+s` 保存）；
- 模型回落与 `enabledModels` 范围过滤（无效项启动时告警，不会卡住）；
- apiKey 支持字面量、`$VAR`/`${VAR}` 与 `!command`；
- 工具结果按 `tool_call_id` 绑定；`grep`/`find` 委托 `rg`/`fd`；
- 自动压缩、`/tree`、`/resume`、`/fork`、`/copy` 等会话命令。

## 测试

```bash
# 部分 provider 回归用本地 mock 二进制，先编出来
cargo build -p mock-provider
cargo test --workspace
```

> 不先构建 `mock-provider` 时，`crates/pi-ai/tests/golden_provider.rs` 这类
> "经本地 mock 回放"的用例会以"先构建 mock-provider"失败（缺二进制，不是断言失败）。

### 差分测试（需要同时装有上游 pi 0.87.1）

`tools/difftest/` 下的运行器把同一个场景分别喂给 `pi-rs` 与上游 `pi`，
逐帧/逐字节比较：

```bash
export TS_PI_ENTRY=/path/to/pi-coding-agent/dist/bundle/cli.js
unshare -n sh -c 'ip link set lo up; env PATH="$PATH" DIFTEST_IMPL=pi-rs \
  node tools/difftest/run-tui.mjs --scenario scenarios-tui/T01.json --out /tmp/out'
```

- `scenarios/`：print/json/rpc 场景；`scenarios-tui/`、`scenarios-tui-light/`：TUI 场景；
- `golden/`、`golden-tui*/`：上游录制结果（本仓库自行录制，不含上游源码）；
  `golden-tui/*/meta.json` 里的本机路径已替换为占位符；
- `tools/mock-provider/`：本地假 provider（可脚本化 SSE、闸门、断连、错误码），
  所有差分场景都在 `unshare -n` 网络命名空间里跑，不会触碰真实 provider。

## Windows 使用说明

- `bash` 工具要求 PATH 里有 `bash`（Git Bash / MSYS2 / Cygwin，与上游一致）；
- 另有 `powershell` 工具，**不在默认工具集**里，需要时 `pi-rs --tools powershell -p "Get-ChildItem"`；
- 输入法候选框跟随终端光标位置。默认与上游一致**隐藏**硬件光标；部分终端在光标隐藏时
  不更新坐标，候选框可能不跟随编辑位置。可设 `settings.json` 的
  `"showHardwareCursor": true`（或环境变量 `PI_HARDWARE_CURSOR=1`）显示硬件光标；
- 经典 conhost（`cmd.exe` 窗口）的粘贴会被拆成一串按键，按到达速率合并；
  Windows Terminal / WezTerm 走正常粘贴路径。细节见 `LIMITATIONS.md`。

## 与上游的关系

- 基线：pi **0.87.1**（`v0.87.1` / `f07218c`），行为以该版本源码为准；
- 本仓库**不分发**上游源码，也不分发上游的二进制/npm 包；
- 许可：MIT（见 [`LICENSE`](LICENSE)）。上游 pi 同为 MIT（Copyright (c) 2025 Mario Zechner），
  派生说明见 [`NOTICE`](NOTICE)。
