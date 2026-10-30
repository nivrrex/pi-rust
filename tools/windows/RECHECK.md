# Windows 复验指引（发布用 exe）

对象：`pi-rs-windows-amd64.exe`（发布用，**由 Linux 侧交叉编译**）。

```bat
certutil -hashfile pi-rs-windows-amd64.exe SHA256
pi-rs-windows-amd64.exe --version
```

| 期望 | 说明 |
|---|---|
| sha256 = `99a9b351eeb19a34…`（完整值见 Release 附件 `SHA256SUMS`） | 与附件清单不一致即不要继续 |
| 第二行 `built <UTC> commit <sha>` | 应为 `commit a22f4e5`（或发布时更新的 commit），且**不含 `-dirty`** |

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
| 9 | 多行粘贴 | 粘贴 3 行**不**立刻提交；>10 行折叠成 `[paste #N +M lines]`；再按回车才整段发出（`Ctrl+V` 仅 Windows Terminal 有效，conhost 用 Shift+Insert / 右键） |

## 额外两项（发布前必须确认，属已知未验证）
1. `tools/windows/probe-vt-input.ps1 -Mode default` 与 `-Mode vt` 各跑一次，贴回原始字节输出
   （决定是否实施 VT 输入路径）；
2. Windows Terminal 下：微软拼音候选框是否跟随光标、多行粘贴是否**只**进入编辑框而不触发多次提交。

3. **Windows Terminal 下的粘贴**（需真人）：分别用 `Shift+Insert` 与**右键粘贴**各粘一段
   3 行文本与一段 12 行文本——预期整段进入编辑框、**不自动提交**（12 行折叠成
   `[paste #N +M lines]`），再按一次回车才发出。同时确认 `Ctrl+V` 在 Windows Terminal 里
   也能正常工作（`Ctrl+V` 只在 conhost 里不产生按键事件）。

## 粘贴取证（wezterm / 输入法问题定位用）

若在 **wezterm + Rime** 下出现"粘贴折叠不稳定 / 粘贴后上下键失效 / 候选框不跟随光标"，
用同一个 exe 抓一份事件日志（不需要额外工具）：

```bat
set PI_TUI_EVENT_DUMP=%TEMP%\pi-events.log
pi-rs.exe
```

复现一次粘贴后退出，把 `%TEMP%\pi-events.log` 发回。日志每行是
`<毫秒>  <crossterm 事件>`——能看出粘贴是 `Event::Paste("…")`（括号粘贴整段到达）
还是一串 `Key(Char(..))`（逐字符），以及到达间隔。

## 失败时提供
项号 + 完整屏幕文本（或截图）+ 上面 `--version` 两行 + `certutil` 的 sha256。
