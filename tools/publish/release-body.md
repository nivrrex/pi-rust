pi **0.87.1** 的 Rust 重写：单二进制 coding agent（TUI / print / json / rpc），
与上游 v0.87.1 的 session 文件、`settings.json`、`models.json`、`auth.json` 互通。

## 下载

| 平台 | 文件 |
|---|---|
| Linux x86_64（glibc） | `pi-rs-linux-amd64` |
| Linux x86_64（musl，static-pie） | `pi-rs-linux-amd64-musl` |
| Linux aarch64（musl，static-pie） | `pi-rs-linux-arm64` |
| Windows x86_64（实验性） | `pi-rs-windows-amd64.exe` |

校验：`sha256sum -c SHA256SUMS`。musl 产物是静态链接，无动态依赖，
也不依赖 OpenSSL（TLS 走 rustls，根证书内置）。

## 状态

- Linux 三份产物：与上游 pi 0.87.1 做逐字节差分（CLI 93 个场景、TUI 84 个场景）全部一致；
- Windows：交叉编译 + 有限真机验证，按实验性对待；已知输入法、经典 conhost 粘贴等边界见
  `LIMITATIONS.md`。

## 支持范围

- api：`openai-completions`、`openai-responses`、`anthropic-messages`
- TUI：编辑器（行编辑、补全、大段粘贴折叠）、`/model` `/settings` `/tree` `/resume` `/thinking`、
  差分渲染、`shift+tab` 切思考档位、`ctrl+s` 保存档位
- 四种模式：TUI（默认）、`-p/--print`、`--json`、`--rpc`；`-c` 续写会话
- Windows 额外提供 `powershell` 工具（不在默认工具集，用 `--tools powershell` 启用）

## 已知限制

见仓库 `LIMITATIONS.md`：`/bug` 提示行、扩展机制、OAuth 登录、部分键位与 CLI 子命令未实现；
Windows 为实验性（`bash` 工具与上游一致地要求系统里有 bash，例如 Git Bash）。

## 许可

MIT（派生自 pi 0.87.1，MIT © 2025 Mario Zechner）。
