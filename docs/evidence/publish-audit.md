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
| `C:\Users` / `vdisksvr` / `系统盘` | 无命中（`scripts/hooks/pre-commit` 的注释提到过该形态，属说明文字） |
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
