pi **0.87.1** 的 Rust 重写：单二进制 coding agent（TUI / print / json / rpc），
与上游 pi v0.87.1 的会话文件、`settings.json`、`models.json`、`auth.json` **双向互通**。

## 下载

| 平台 | 文件 |
|---|---|
| Linux x86_64（glibc） | `pi-rs-linux-amd64` |
| Linux x86_64（musl，static-pie） | `pi-rs-linux-amd64-musl` |
| Linux aarch64（musl，static-pie） | `pi-rs-linux-arm64` |
| Windows x86_64（实验性） | `pi-rs-windows-amd64.exe` |

校验：`sha256sum -c SHA256SUMS`。musl 产物静态链接、无动态依赖，也不依赖 OpenSSL
（TLS 走 rustls，根证书内置）。安装即把二进制放进 `PATH`：

```sh
chmod +x pi-rs-linux-amd64-musl
mv pi-rs-linux-amd64-musl ~/.local/bin/pi-rs
pi-rs
```

## 主要功能

- 四种模式：TUI（默认）、`-p/--print`、`--mode json`、`--mode rpc`；`-c` 续写会话
- api：`openai-completions`、`openai-responses`、`anthropic-messages`
- TUI：差分渲染、编辑器（行编辑、补全、大段粘贴折叠）、`/model` `/settings` `/tree`
  `/resume` `/thinking` `/scoped-models` `/session` `/hotkeys` `/trust` `/reload`
  `/changelog` `/compact` `/copy` `/name` `/clone` `/fork` `/import` `/export` `/new`
- 思考档位 off…max（`shift+tab` 切换、`ctrl+s` 保存）；主题 dark/light
- 会话条目完整：消息、`session_info`、`branch_summary`、`compaction`、`context_edit` 等，
  未知类型原样往返
- 扩展（需要本机有 `node`）：工具/命令/事件注册、`ctx.ui`、Markdown 变换器、`pi.events`；
  `.ts` 扩展需 jiti，其 shim 已随程序内嵌，无需另装
- `keybindings.json` 生效（含旧键名迁移与冲突检测）
- Windows 额外提供 `powershell` 工具（不在默认工具集，用 `--tools powershell` 启用）

## 兼容与回退

本版**不覆盖、不修改**已安装的上游 `pi`：两者可并存，读写同一份配置与会话目录，
格式一致（已双向验证）。想切回上游时直接运行 `pi`；删除本版只需删掉二进制
（不写系统目录、不装服务）。

## 本批重点（F244–F245）

- 修复 PTY 回归把超时 SIGKILL 误判为通过的问题；TUI 回归现在同时检查 helper 与真实退出码。
- edit/write 增加同路径文件变更队列；bash 输出改为有界累加器并流式保存完整输出。
- `auth.json` / `settings.json` 增加跨进程锁与原子更新；扩展帧在读取阶段限制大小。
- YAML frontmatter 支持锚点、别名、注释、flow 集合与多行标量。
- 内置 `/llama` 支持目录刷新及 `load` / `unload` / `download` 子命令。
- `-c` 首屏优化：23 MB 会话本机热缓存首帧约 4.76 s → 3.66 s；119 MB 约 12.01 s → 9.11 s。

## 已知限制

完整清单见仓库 `LIMITATIONS.md`。摘要：

- `/share`（需 GitHub gist）、`/bug`（上传/导出 zip）未实现；
  `/export` 支持 `.jsonl` 与单文件 HTML 两条分支
- 扩展事件已全部派发（含 `agent_before_settle` 续跑、`session_compact`、
  `before_provider_request` 等）；少数 `ctx`/`ctx.ui` 成员与 `pi.setModel` 未实现，
  `pi.setActiveTools` 已实现
- `/login` 只支持 api_key 型 provider，OAuth 型 provider 明确提示不支持
- Windows 为实验性：输入法候选框、经典 conhost 粘贴等边界未覆盖；
  `bash` 工具与上游一致，要求系统里有 bash（如 Git Bash）
- 大会话 `-c` 仍受跨进程扩展启动与全历史 eager 渲染影响；119 MB 会话首屏仍慢于 AOT

## 许可

MIT（派生自 pi 0.87.1，MIT © 2025 Mario Zechner）。派生说明见 `NOTICE`。
