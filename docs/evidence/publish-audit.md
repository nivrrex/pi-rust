# 发布前公开内容审计（最终 HEAD，P8e-11 复跑）

> 本文件脱敏：不含凭据值，也不含完整本机路径。

## 结论
可以发布（B1 已按批内规则处置）。剩余一项**需知情**：录制基准里含本机全局
`AGENTS.md` 的原文（工作规则文本，无姓名/邮箱/账号/密钥）。

## 规模
| 项 | 值 |
|---|---|
| 跟踪文件 | 3 655 |
| 最大单文件 | 1 406 404 B（`golden/S51/events.jsonl`），远低于 100 MB |

## 模式扫描（全量跟踪文件）
| 模式 | 结果 |
|---|---|
| `sk-…`（≥16 位）/ `ghp_…` / `github_pat_…` / `BEGIN … PRIVATE KEY` | 无命中（命中项仅本文件对模式名的自述，已改写为文字描述） |
| `Bearer <≥20 位>` | 无命中（golden 里的 Authorization 是 `test-key`） |
| 邮箱 | 无命中 |
| `C:\Users`、内网主机名、网络盘共享名 | 无命中（`scripts/hooks/pre-commit` 的注释已改成不点名的描述） |
| `/root/Desktop/pi-ts-<ver>` | **0**（B1 归一化后清零，见下） |

## B1 处置（已完成）
- 现象：`golden/` 下 1 503 个文件含 TS 安装目录的绝对路径（原 8 598 处）。
- 处置：把这些路径统一替换为 `<PI_PKG>`，并在两个比较器里加同一条归一化
  （`tools/difftest/normalize.mjs`、`tools/difftest/compare-tui.mjs`），保证重新录制时
  仍判 same。
- 验证：`check-s01-pin.sh` → `A=same B=same C=same fs=same exit=same`；TUI 全量
  `same=68/0`。

## 需知情的残留内容
`golden/**/requests.jsonl` 里保存了录制的完整 system prompt，其中包含本机全局
`AGENTS.md` 的正文（编码风格、记录规范等**工作规则文本**）。检查结论：
- 无姓名、邮箱、账号、主机名、内网地址、密钥；
- 含 `/root/...` 形态的文档路径（已被 `<PI_PKG>` 归一化的部分除外），属通用 home 路径。

若不允许公开这段工作规则文本，需要在发布前排除这批 golden 或对其中的 system prompt
段做占位替换（会削弱 event/request 基准的可比性，属取舍）。

## 索引可执行位与编码约定
`scripts/check-repo-conventions.sh`（precommit 内）保证：`scripts/*.sh`、`tools/**.sh`、
`tools/windows/*.ps1`、`scripts/hooks/*` 索引为 `100755`；含非 ASCII 的 `.ps1` 必须带
UTF-8 BOM。当前全绿。

## 许可证与说明文件
`LICENSE`（MIT）、`NOTICE`、`README.md`、`LIMITATIONS.md` 均在仓库根且为跟踪文件。

## G''c（P8e-14）：用户全局提示词特征短语扫描

脚本：`tools/publish/privacy-scan.sh`（只读；命中即非零退出）。两组模式：
密钥/本机路径/邮箱，以及**用户全局提示词特征短语**（`N先生`、`以上回答已遵循所有设定规则`、
`全局提示词`、`末尾固定`、`逐条检查`）。

| 组 | 跟踪文件 | `golden*/` |
|---|---|---|
| 密钥/本机路径/邮箱 | 命中 6 个（见下） | 无 |
| 提示词特征短语 | 无 | **72 个**（全部是 `逐条检查`） |

### 跟踪文件里的命中（逐项处置）
| 文件 | 命中内容 | 处置 |
|---|---|---|
| `crates/pi-cli/src/runtime.rs` | 测试用 `/root/...` 字面量 | 保留（单元测试夹具） |
| `crates/pi-cli/src/settings_selector.rs` | 测试用 `/root/projects/x` | 保留（`shorten_path` 用例） |
| `docs/evidence/*.md` | 本机路径（证据文档） | 保留（已脱敏，仅出现通用 `~`/临时路径） |
| `tools/difftest/pty_retry_check.py` | 测试用路径 | 保留 |

### golden 里的命中（**阻塞发布**）
`golden/S23b`、`S24`、`S51`、`S60` 等 72 个文件里出现 `逐条检查`——即用户全局
`AGENTS.md` 的正文。原因：这些场景录制时 harness 未把 `HOME` 也指向临时目录，
`project_context` 读到了真实的 `~/.pi/agent/AGENTS.md`。

**未处置**，因为两条路都超出"只改文案"的范围：
1. 用 `<USER_AGENTS_MD>` 占位归一化：需要**两侧**同时归一化，而我们的运行时会再次读到
   同一份真实 AGENTS.md（当前比较之所以 same，正是因为在读同一份）⇒ 单边改必红；
   要彻底解决得让 harness 固定 `HOME`，然后**重录这批 golden**（涉及 4 个场景 × 3 次）。
2. 从公开树排除这 72 个文件：会让 S23b/S24/S51/S60 失去基准（回归里变 `nogolden`）。

在决定之前，**发布包不可用于发布**。
