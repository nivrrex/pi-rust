# Windows 真机测试清单（交给 Windows 侧 agent 执行）

被测对象：`pi-rs.exe`（`--version` 第二行应为 `commit 5a31915` 或更新）。
本文件只列**需要在 Windows 上验证**的项；Linux 侧已自动化覆盖的不在此列。

请在 `%USERPROFILE%\pi` 下执行，逐项记录「通过 / 不通过 + 实际输出或截图」。
**不要**读取或回传任何密钥内容（`auth.json` 只看结构，不看值）。

---

## 0. 准备

```bat
cd %USERPROFILE%\pi
pi-rs.exe --version
dir %USERPROFILE%\.pi\agent
```

| 期望 | 说明 |
|---|---|
| 第二行为 `built <UTC> commit <sha>` | sha 应 ≥ `5a31915` |
| `%USERPROFILE%\.pi\agent` 存在且含 `settings.json`、`auth.json`、`models.json` | 若不存在，记下实际目录，后续项全部标记"配置缺失" |

---

## 1. 启动与配置目录（B-e 修复项）

```bat
pi-rs.exe
```

| 检查 | 期望 |
|---|---|
| 是否进入 TUI | 是（启动屏 + 编辑器框 + footer） |
| footer 右侧 | `openrouter/free • high`（或你 `settings.json` 里的默认模型与档位） |
| 是否报 `没有可用的 API key` | **不应出现**；若出现，记录完整报错并跳转第 7 项 |

退出：编辑框为空时按 `Ctrl+D`（见第 2 项）。

---

## 2. 键位（Windows 输入层修复项，重点）

在**空编辑框**状态下逐项测试，每项记录"有反应 / 无反应"：

| # | 键 | 期望 |
|---|---|---|
| 2.1 | `Ctrl+D` | 退出 TUI，终端回到 `%USERPROFILE%\pi>` 提示符 |
| 2.2 | `Ctrl+P` | footer 右侧模型名切换（在 `enabledModels` 的 3 个之间循环）；只有 1 个模型时显示 `Only one model available` |
| 2.3 | `Ctrl+O` | 工具输出展开/折叠切换 |
| 2.4 | `Ctrl+L` | 打开模型选择器面板 |
| 2.5 | `Esc` | 关闭已打开的面板；空闲时无副作用 |
| 2.6 | `Shift+Tab` | 思考档位循环（footer 右侧 `• high` 等变化） |
| 2.7 | 方向键 `↑`/`↓` | 在输入历史中翻动 |
| 2.8 | `Ctrl+C` | 清空编辑框内容（不退出进程） |

**已知缺口**：`Alt+P`（`app.model.cycleBackward`）未实现，不需测试。

---

## 3. 输入法与粘贴

| # | 操作 | 期望 |
|---|---|---|
| 3.1 | 用微软拼音输入「你好」 | 候选框跟随光标、不闪烁；提交后文字进入编辑框，无乱码 |
| 3.2 | 粘贴一段多行文本（`Ctrl+V` 或右键） | 显示为折叠占位（`[Pasted ~N lines]` 形式），不撑爆编辑框 |

---

## 4. 一轮含工具的对话（真实 provider）

发送：

```
先 ls 一下当前目录，再读 AGENTS.md 前 5 行
```

| 检查 | 期望 |
|---|---|
| 工具块头部 | 形如 ` bash ls` / ` read ...`（名字 + 参数） |
| **工具块正文** | **必须有内容**（此前缺陷是空块）；`bash`/`powershell` 块末尾有 `Took <时长>` |
| 折叠提示 | 长输出显示 `... (N earlier lines, ctrl+o to expand)`，按 `Ctrl+O` 能展开 |
| thinking 块 | 有内容（灰斜体） |
| 回合收尾 | 助手给出文字总结；footer 左侧用量变化（`↑… ↓… x.x%/200k`） |
| 错误块（可选） | 发一条必然失败的命令（如 `!type Z:\nope.txt`），工具块底色应为**浅红**（light 主题） |

---

## 5. 退出行为（F2 修复项）

| # | 操作 | 期望 |
|---|---|---|
| 5.1 | 空编辑框按 `Ctrl+D` | 退出后 `To resume this session: …` 打印在**单独一行**，其**后不多出空行**（与同目录下原版 `pi` 的退出输出逐行对比） |
| 5.2 | 退出后终端状态 | 光标可见、无残留颜色块；`Ctrl+D`/普通命令可正常输入 |

对照方法（若本机也装了原版 pi）：

```
node <pi 安装目录>\dist\bundle\cli.js      ← 原版
pi-rs.exe                                  ← 本次被测
```

两次都用 `Ctrl+D` 退出，比较最后两行的行数与空行差异。

---

## 6. `--help` 与模型回落（B-d/B-b 修复项）

```bat
pi-rs.exe --help
```

| 检查 | 期望 |
|---|---|
| 是否出现 `PI_TUI_RENDERER` | **不应出现**（已删除） |
| `--provider` / `--model` 文案 | `provider 名（缺省：settings 的 defaultProvider / 可用模型回落）`，**不写"本版本必填"** |
| 缺省启动能否工作 | 不给 `--provider/--model` 直接 `pi-rs.exe` 也应能进 TUI（走回落） |

---

## 7. 失败时的取证包

若某项不通过，请提供：

1. 该项编号 + 操作步骤；
2. **完整**屏幕文本（复制控制台，或截图）；
3. `pi-rs.exe --version` 的两行输出；
4. 如下目录清单（**不要**贴文件内容）：

```bat
dir %USERPROFILE%\.pi\agent
dir %USERPROFILE%\.pi\agent\*.json
```

5. 若与键位有关，额外记录：按键时**编辑框是否为空**、是否有面板打开。

---

## 8. 记录模板

```
[ ] 0  版本与配置目录      通过 / 不通过（实际：            ）
[ ] 1  启动与 footer        通过 / 不通过（实际：            ）
[ ] 2.1 Ctrl+D 退出        通过 / 不通过
[ ] 2.2 Ctrl+P 切模型      通过 / 不通过（footer 变化：      ）
[ ] 2.3 Ctrl+O 展开        通过 / 不通过
[ ] 2.4 Ctrl+L 选择器      通过 / 不通过
[ ] 2.5 Esc                通过 / 不通过
[ ] 2.6 Shift+Tab 档位     通过 / 不通过
[ ] 2.7 方向键历史          通过 / 不通过
[ ] 2.8 Ctrl+C 清空编辑框   通过 / 不通过
[ ] 3.1 输入法候选框        通过 / 不通过
[ ] 3.2 多行粘贴折叠        通过 / 不通过
[ ] 4   含工具一轮         通过 / 不通过（工具正文：有/空；Took：有/无）
[ ] 5.1 退出无多余空行      通过 / 不通过
[ ] 5.2 退出后终端状态      通过 / 不通过
[ ] 6   --help 文案与回落   通过 / 不通过
```
