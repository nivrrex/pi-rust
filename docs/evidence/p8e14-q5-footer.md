# Q5：footer 与无模型布局（P8e-14）

## ref 取证
| 项 | ref | 我们的实现 |
|---|---|---|
| cwd 家目录缩写 | `components/footer.ts:32-44` `formatCwdForFooter` | `pi-tui/src/gitinfo.rs::format_cwd_for_footer`（canonicalize 后比较，home 外保持绝对路径） |
| 分支来源 | `core/footer-data-provider.ts:239-248` 读 `<gitdir>/HEAD`：`ref: refs/heads/X` ⇒ X，否则 `detached` | `gitinfo::git_branch` 同规则 |
| `.git` 查找 | `footer-data-provider.ts:14-48`：向上找，支持 worktree 的 `.git` 文件 + `commondir` | 同（`find_git_paths`） |
| cwd 行拼接 | `footer.ts:115-121`：`<pwd> (<branch>)` | 同（`footer_cwd_line`） |
| 无模型上下文 | `footer.ts:112-114` `contextWindow ?? 0`、`percent ?? 0` | `0.0%/0 (auto)`（此前是 `0.0%/128k`，已改） |
| 无模型右侧模型名 | 实测 TS 显示 `unknown`（`footer.ts:170-171` 取 `state.model?.id`，该状态下占位模型 id 是 `unknown`） | 改为 `unknown`（此前 `no-model`） |
| docs 路径 | `config.ts:445-447` `getPackageDir()/docs` | `PI_DOCS_DIR` → 可执行文件旁 `docs/` → 公开仓库 docs 地址（见 LIMITATIONS.md） |

## 整屏 diff（同状态、同尺寸、`tmux capture-pane -p`）
路径文本归一化（`<DOCS>`）后：

| 尺寸 | fresh | paste3 | paste12 |
|---|---|---|---|
| 100×30 | **0** | **0** | **0** |
| 80×24 | 14 | 12 | 14 |

## 80×24 变差的原因（结论）
不是布局算法差异，而是**路径长度导致的折行**：
- TS 的 docs 路径是 76 字符的绝对路径，80 列下每个条目折成 2 行 ⇒ 提示块多占 4 行；
- 我们的回落地址只有 55 字符，不折行；
- 24 行屏幕装不下 TS 的整块内容 ⇒ TS 整屏**上滚 4 行**（首行变成 `Press ctrl+o …`），
  我们不上滚。实测把 `PI_DOCS_DIR` 设成接近长度的路径（70 字符）后，两侧都上滚、
  差异从 14 行降到 1 行的错位（折行粒度不同）。

结论：80×24 的差异属"安装布局不同带来的路径长度差"，无法通过改布局消除；
已在 `LIMITATIONS.md` 登记。
