# pi-rs

`pi-rs` 是 [pi](https://github.com/badlogic/pi-mono) **v0.87.1**（tag `v0.87.1`，commit `f07218c`）的
Rust 重写：单二进制 coding agent（TUI / print / json / rpc 四种模式），
与原版的 **session 文件、settings.json、models.json、auth.json 互通**。

> 行为规格的唯一来源是上游源码（本仓库开发时以只读 git checkout 放在 `ref/pi-0.87.1/`，
> **不随本仓库分发**）。凡与上游不一致的地方都记录在 `notes/reports/` 的台账里。

## 构建

```bash
# 1) glibc（开发用）
cargo build --release -p pi-cli          # 产物：target/release/pi-rs

# 2) musl 静态（部署用，目标 x86_64-unknown-linux-musl）
rustup target add x86_64-unknown-linux-musl   # 仅首次
cargo build --release --target x86_64-unknown-linux-musl -p pi-cli
# 产物：target/x86_64-unknown-linux-musl/release/pi-rs（static-pie，无动态依赖）
```

工具链固定在 `rust-toolchain.toml`（stable）。**不需要** OpenSSL（TLS 走 rustls + ring，
根证书内置 webpki-roots，不依赖系统 `/etc/ssl`），也不需要 Node/Bun。

## 运行

```bash
pi-rs                 # TUI（默认）
pi-rs -p "问题"        # print 模式（一次性回答）
pi-rs --json          # json 事件流
pi-rs --rpc           # rpc 模式（stdin/stdout JSON）
pi-rs -c              # 续写最近的会话
```

配置目录：`PI_CODING_AGENT_DIR`（缺省 `~/.pi/agent`），会话目录：
`PI_CODING_AGENT_SESSION_DIR`（缺省 `<配置目录>/sessions`）。

## 测试

```bash
# 顺序有讲究：部分 provider 回归测试会用本地 mock 二进制，先把它编出来
cargo build -p mock-provider
cargo test --workspace        # 单元 + 集成测试
scripts/precommit.sh          # 提交前十一项检查（格式/clippy/禁用模式/场景 schema 等）
scripts/check-no-stray.sh     # 残留进程检查（只报告，不自动 kill）
```

> `cargo test` 之前没有 `cargo build -p mock-provider` 时，`crates/pi-ai/tests/golden_provider.rs`
> 这类"经本地 mock 回放"的用例会以 `先构建 mock-provider` 失败（缺二进制，不是断言失败）。

### 差分测试（需要上游 pi 0.87.1 的安装）

`tools/difftest/` 下的运行器会把同一场景分别喂给 `pi-rs` 与上游 `pi`，逐帧/逐字节比较：

```bash
export TS_PI_ENTRY=/path/to/pi-coding-agent/dist/bundle/cli.js
unshare -n sh -c 'ip link set lo up; env PATH="$PATH" DIFTEST_IMPL=pi-rs \
  node tools/difftest/run-tui.mjs --scenario scenarios-tui/T01.json --out /tmp/out'
```

- `scenarios/`：print/json/rpc 场景；`scenarios-tui/`：TUI 场景（100×30）。
- `golden/`、`golden-tui/`：**上游录制结果**（我们自己的录制，不是上游源码）。
  `golden/S*/` 的 JSONL 里有录制时的安装路径字样（例如 pi 包的 docs 目录）。
  这是从上游 prompt 里原样记录下来的**路径**（不含凭据、用户名或主机名），
  且**不能替换成占位符**：S01 的 A/B/C 层比对依赖它和运行时 `PI_PACKAGE_DIR` 完全一致
  （实测替换后 A/B/C 全 different）；
  `golden-tui/*/meta.json` 里的本机路径已替换为 `<REPO>`/`<PI_TS_INSTALL>` 占位。
- `tools/mock-provider/`：本地假 provider（可脚本化 SSE、闸门、断连、错误码），
  所有差分场景都在 `unshare -n` 网络命名空间里跑，绝不触碰真实 provider。

## 主要功能

- 四种模式：TUI（默认）、`-p` print、`--json` 事件流、`--rpc`；
- **无可用模型也能进 TUI**：启动只打印 `Warning: No models available. Use /login …`，
  用 `/login` 配置（当前支持 api_key 型 provider），成功后**当次会话即可对话**；
- `/login`、`/logout`：写/删 `<配置目录>/auth.json`（原子写，Unix `0600`，
  保留未知键）；OAuth 型 provider 提示暂不支持（见 `LIMITATIONS.md`）；
- 主题 `dark`/`light`（`settings.json` 的 `theme`，`/settings` 里可切换并立即重绘）；
- 思考档位 off/minimal/low/medium/high/xhigh/max（`/thinking`、`ctrl+s` 保存）；
- 模型回落与 `enabledModels` 范围过滤（无效项启动时告警）；
- apiKey 支持字面量、`$VAR`/`${VAR}` 与 `!command`；
- 工具结果按 `tool_call_id` 绑定；`grep`/`find` 委托 `rg`/`fd`。

## 与上游的关系

- 基线：pi **0.87.1**（`v0.87.1` / `f07218c`）。
- 许可：**MIT**（见 `LICENSE`）；上游 pi 同为 MIT（Copyright (c) 2025 Mario Zechner），
  派生说明见 `NOTICE`。
- 本仓库不分发上游源码，也不分发上游的二进制/npm 包。

## 已知限制

见 [`LIMITATIONS.md`](LIMITATIONS.md)。几条最常被问到的：

1. **`/bug` 提示行**未实现（已裁定为已知偏差）；
2. 部分 api 未实现（`openai-responses` 已支持；其余见 `LIMITATIONS.md`）；
3. 扩展机制未实现（`--mode rpc` 尚不能承载扩展宿主协议）；
4. 思考档位：`shift+tab` 循环、`ctrl+s` 保存已实现；anthropic 的 `thinking.budget_tokens` 映射未做。

## Windows

`x86_64-pc-windows-gnu` 目标可交叉编译（产物 `pi-rs.exe`）：

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu -p pi-cli
```

Windows 上 `bash` 工具要求 PATH 里有 `bash`（Git Bash / MSYS2，与上游一致）；
另有 `powershell` 工具（`pwsh.exe` / `powershell.exe`），**不在默认工具集**里，需要时：

```bash
pi-rs --tools powershell -p "Get-ChildItem"
```

状态：**实验性**，见 `LIMITATIONS.md`。
