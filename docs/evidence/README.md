# 证据摘要（脱敏）

本目录是**可公开**的证据摘要：只保留"做了什么 + 怎么复现 + 结果"，不含本机路径、主机名、
帐号或任何凭据。完整原始输出保留在本地 `notes/evidence/`（不入库）。

## 条目

| 文件 | 覆盖 |
|---|---|
| `parity-tui.md` | 渲染/交互对拍方法与结论（文本级 + 样式级） |
| `retry.md` | 重试判据、退避、显示 |
| `esc-latency.md` | Esc 中止延迟 |
| `tools-output.md` | 工具结果正文渲染与 ctrl+o |
| `thinking.md` | 思考档位与 footer 显示 |

## 复现前提

- 需要 Node（跑录制/比较脚本）与 `fd`/`rg`（TS 侧工具探测用）。
- 所有涉及启动被测进程的步骤都要在 `unshare -n`（独立 netns，`ip link set lo up`）里执行，
  并让 mock provider 与被测进程处于**同一 netns**。
- 每次运行使用临时配置目录（`PI_CODING_AGENT_DIR` / `PI_CODING_AGENT_SESSION_DIR`），
  不要指向真实配置。
