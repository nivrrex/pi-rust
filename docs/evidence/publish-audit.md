# 发布前公开内容审计（P8e-10/G-b）

> 本文件脱敏；不含任何凭据值或完整本机路径。

## 结论（先看这一段）
**当前不可直接发布**，有 1 项阻塞：

| # | 发现 | 影响 | 处置建议 |
|---|---|---|---|
| B1 | `golden/` 下 **2 858** 个文件里出现本机 TS 安装路径 `/root/Desktop/pi-ts-<ver>/…`（共 8 598 处），来自录制时 TS 运行时打印的路径（`requests.jsonl` / `events.jsonl` / `session.jsonl`） | 公开仓库会暴露本机目录布局（**不是密钥**，但属环境痕迹） | 二选一：① 发布前做归一化替换为 `<PI_TS_DIR>/…` 并重新校验 golden 仍 same；② 不发布 `golden*/`（但这些是差分基准，去掉会让仓库失去可验证性）。建议 ① |

其余项**已通过**：

## 跟踪文件规模
- 跟踪文件 **3 649** 个；最大单文件 1 376 KB（`golden/S51/events.jsonl` 及其 `_runs` 副本），**远低于 100 MB 限制**。

## 密钥与个人信息模式扫描（全量跟踪文件）
| 模式 | 结果 |
|---|---|
| `sk-…`（≥16 位） | 无命中 |
| `ghp_…` / `github_pat_…` | 无命中 |
| `BEGIN … PRIVATE KEY` | 无命中 |
| `Bearer <≥20 位>` | 无命中（golden 里的 Authorization 是 `test-key` 占位） |
| 邮箱 `@domain.tld` | 无命中 |
| `vdisksvr` / `系统盘` | 无命中 |
| `C:\Users…` | 无命中（本机路径已在上批改成 `%USERPROFILE%`） |
| `/root/…` | **仅 B1 那一类**（`golden/` 里的 TS 安装路径）；源码里唯一命中是 `settings_selector.rs:1631` 的单元测试用字面量 `"/root/projects/x"`（`shorten_path` 用例，可保留或改成 `/home/u`） |

## 不应入库的内容确认
| 项 | 状态 |
|---|---|
| `notes/`（决策、证据、真机记录、P9 清点） | **已在 `.git/info/exclude`**，不在跟踪列表 |
| 真机验收清单（含本机操作习惯） | 已移入 `notes/local/` 并 exclude |
| `auth.json` / `settings.json` / `models.json` 副本 | 无（仓库内只有测试夹具中合成的假 key） |
| 录制数据含个人路径 | 见 B1 |

## 索引可执行位
`scripts/check-repo-conventions.sh` 已在 precommit 中强制：`scripts/*.sh`、`tools/**.sh`、`tools/windows/*.ps1`、`scripts/hooks/*` 必须 `100755`；`.ps1` 含非 ASCII 必须带 UTF-8 BOM。当前全绿。

## NOTICE / LICENSE / README
`LICENSE`（MIT）、`NOTICE`、`README.md` 均在仓库根且为跟踪文件；README 的功能与限制条目需与 `LIMITATIONS.md` 同步核对（G-c 步骤），本轮未改。
