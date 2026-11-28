# V5：Linux 括号粘贴与折叠阈值对拍（P8e-12）

## 方法
`tmux` 100×30，`PI_CODING_AGENT_DIR` 指向空临时目录，用 **`tmux paste-buffer -p`** 注入
**真正的括号粘贴**（等价于终端的 `ESC[200~…ESC[201~`）。TS 与 `pi-rs` 各跑同一条。

> 注意：`tmux send-keys -H 1b 5b 32 30 30 7e` 这种逐键注入**不是**括号粘贴（tmux 会把
> 每个字节当独立按键送出，且中间可能被输入超时切开），用它得出的"没折叠"是注入方式
> 的问题，不是实现差异——本文件的数据全部来自 `paste-buffer -p`。

## 折叠阈值（ref）
`packages/tui/src/components/editor.ts:1296-1312`：
- `pastedLines.length > 10` 或 `totalChars > 1000` ⇒ 折叠；
- 折叠文案：>10 行 → `[paste #N +L lines]`，否则（>1000 字符）→ `[paste #N C chars]`。
我们的实现（`crates/pi-tui/src/lib.rs:922-940`）与之一致（阈值与两种文案）。

## 对拍结果（11 行是阈值边界：`> 10`）
| 行数 | TS | pi-rs | 判定 |
|---|---|---|---|
| 3 | 展开（`L1`…） | 展开 | same |
| 8 | 展开 | 展开 | same |
| **11** | `[paste #1 +11 lines]` | `[paste #1 +11 lines]` | same |
| 30 | `[paste #1 +30 lines]` | 同上 | same |

## 顺带修的输入层缺陷
`crates/pi-tui/src/input.rs`：起始标记原先要求 `ESC[200~` **六字节同时到齐**才进入粘贴模式，
分块到达时首个 `ESC` 会被当成孤立 Esc 发出去（真机 ConPTY 就是逐字节送）。已改为
"只要当前缓冲仍是 `ESC[200~` 的真前缀就继续等"，超时仍由 `process()` 的 wake / `flush()`
兜底（孤立 Esc 不受影响）。
