# Windows 复验指引（发布用 exe）

对象：`pi-rs-windows-amd64.exe`（发布用，**由 Linux 侧交叉编译**）。

```bat
certutil -hashfile pi-rs-windows-amd64.exe SHA256
pi-rs-windows-amd64.exe --version
```

| 期望 | 说明 |
|---|---|
| sha256 = `37e357fb2d0f3d79…`（完整值见 Release 附件 `SHA256SUMS`） | 与附件清单不一致即不要继续 |
| 第二行 `built <UTC> commit <sha>` | 应为 `commit 0fea302`（或发布时更新的 commit），且**不含 `-dirty`** |

差异说明：Windows 侧自编版本用 MSVC/git-bash 工具链、可能带 `-dirty`；发布用版本是在 Linux 上用
`x86_64-pc-windows-gnu`（rustc 1.99.0 + mingw-w64 GCC 12-win32）**交叉编译**的干净构建。
两者功能应一致；若有行为差异，以发布用版本为准并回报。

---

## 复跑核心用例（沿用 Windows 侧已有的同一套 conhost/ConPTY 脚本）

| # | 用例 | 通过标准 |
|---|---|---|
| 1 | 启动 | 编辑框**为空**，屏幕无字面量转义/垃圾字符（如 `[?1;0c`、`[[??11;;00cc`） |
| 2 | 注入 `abc`（down+up 成对） | 显示 `abc`，**不翻倍**成 `aabbcc` |
| 3 | `Ctrl+D`（编辑框为空） | 退出 TUI，回到 shell 提示符 |
| 4 | `Ctrl+P` | footer 模型名切换；无效项已被启动期过滤（stderr 有 `Warning: No models match pattern "…"`） |
| 5 | `Shift+Tab` | 出现 `Thinking level: <level>`，footer 右侧档位同步 |
| 6 | 含工具一轮（"先 ls 再读 AGENTS.md 前 5 行"） | 工具块**有正文**；`bash`/`powershell` 块末尾有 `Took <时长>` |
| 7 | `Ctrl+D` 退出后 | `To resume this session: …` 独占一行，其后**不多空行** |
| 8 | footer | 多 provider 且宽度足够时显示 `(provider)` 前缀；上下文窗口数字与 `settings.json` 一致 |

## 额外两项（发布前必须确认，属已知未验证）
1. `tools/windows/probe-vt-input.ps1 -Mode default` 与 `-Mode vt` 各跑一次，贴回原始字节输出
   （决定是否实施 VT 输入路径）；
2. Windows Terminal 下：微软拼音候选框是否跟随光标、多行粘贴是否**只**进入编辑框而不触发多次提交。

## 失败时提供
项号 + 完整屏幕文本（或截图）+ 上面 `--version` 两行 + `certutil` 的 sha256。
