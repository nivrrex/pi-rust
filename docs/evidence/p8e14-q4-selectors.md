# Q4：选择器文案对齐（进行中）

判定依据：**TS 录制的 golden 是唯一真值**（golden 由 TS 产出，不得改 golden 迁就 pi-rs）。
因此对已有 golden 覆盖的选择器，先查 golden 文本，再决定是否要改我们自己的字符串。

## 1. thinking selector —— 已确认一致（无需改）
| 文本 | 我们的实现 | TS golden（`golden-tui/T28*/run-1/raw.bin`） |
|---|---|---|
| 副标题 | `Shift+Tab cycles thinking levels in-session` | `Shift+Tab cycles thinking levels in-session`（逐字一致） |
| 档位描述 | `No reasoning` / `Very brief reasoning (~1k tokens)` / … | 与 ref `thinking-selector.ts:23-31` 的 `LEVEL_DESCRIPTIONS` 一致（此前已对齐） |
| 标题 | `Thinking Level` | 同 ref `thinking-selector.ts:78` |

> 备注：ref 源码里 `keyDisplayText("app.thinking.cycle")` 渲染成 `Shift+Tab`（大写），
> 与"键名一律小写"的直觉不同；以 golden 为准。

## 2. 其余七个选择器（待办）
`settings`、`model`、`theme`、`resume`、`tree`、`summarize`、`warnings`：
需要逐个先取 ref 文案（`settings-selector.ts` 等）→ 与我们的字符串对照 → 有差异则改 →
再补 TS 录制 golden（dark/light）。本轮未做，下一窗口按"每个选择器一次提交"推进。
