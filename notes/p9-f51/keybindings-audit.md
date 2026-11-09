# Z6 快捷键审计

ref 权威来源：`packages/coding-agent/src/core/keybindings.ts`（动作定义 + 默认键）。
"pi-rs 是否实现" 以当前 `crates/pi-cli/src/tui.rs` 的按键分派与 `crates/pi-tui/src/lib.rs`
的 `TuiEvent` 处理为准。

| 动作名 | 默认键 | pi-rs | 分类 |
| --- | --- | --- | --- |
| `app.tools.expand` | `ctrl+o` | 已实现（`TuiEvent::CtrlO`） | — |
| `app.thinking.toggle` | `ctrl+t` | **本批实现**（切换 + 写设置 + 状态行） | — |
| `app.model.select` | `ctrl+l` | 已实现（`TuiEvent::CtrlL`） | — |
| `app.model.cycleForward` | `ctrl+p` | 已实现（`TuiEvent::CtrlP`） | — |
| `app.model.cycleBackward` | `shift+ctrl+p` | 已实现（`TuiEvent::CtrlShiftO`? 见注 1） | 待确认 |
| `app.session.toggleNamedFilter` | `ctrl+n` | 已实现（`TuiEvent::CtrlN`） | — |
| `app.editor.external` | `ctrl+g` | **未实现**（提示行见到 `ctrl+g` 文案但无行为） | 缺陷 |
| `app.message.copy` | `ctrl+x` | 已实现（F43：无消息时报 `Error: `） | — |
| `app.input.submit` | `enter` | 已实现 | — |
| `app.input.newline` | `shift+enter`/`ctrl+j` | 已实现（`Newline`） | — |
| `app.input.clear` | `ctrl+c`（双击退出） | 已实现（F37：500 ms 双击） | — |
| `app.input.quit` | `ctrl+d` | 已实现 | — |
| `app.history.prev`/`next` | `up`/`down` | 已实现 | — |
| `app.editor.deleteWordBackward` | `ctrl+w` | 已实现（`DeleteWordBackward`） | — |
| `app.editor.deleteToLineStart` | `ctrl+u` | 已实现 | — |
| `app.editor.deleteToLineEnd` | `ctrl+k` | 已实现 | — |
| `app.editor.lineStart` | `ctrl+a` | 已实现 | — |
| `app.editor.lineEnd` | `ctrl+e` | 已实现 | — |
| `app.tree.filter.noTools` | `ctrl+t`（**仅 /tree 选择器内**） | 已实现（选择器分支优先） | — |
| `app.tree.filter.userOnly` | `ctrl+u` | 已实现 | — |
| `app.tree.filter.labeledOnly` | `ctrl+l` | 已实现 | — |
| `app.tree.filter.default` | `ctrl+d` | 已实现 | — |
| `/model` 等斜杠命令 | `/` | 已实现 | — |
| `!` bash 模式 | `!` | 已实现 | — |

注 1：`shift+ctrl+p`（非 Windows）在我们的映射里是 `CtrlShiftO`？**待确认**——
`tui.rs` 的按键分派里 `char 'p' + SHIFT` 目前与 `ctrl+p` 同义（循环前进），
与 ref 的"后退"不同。**缺陷（未修）**。

## 本批实现

`app.thinking.toggle`（`ctrl+t`）：`crates/pi-tui/src/lib.rs` 的 `TuiEvent::CtrlT` →
翻转 `state.hide_thinking_block` + `push_status("Thinking blocks: hidden|visible")` +
返回 `TuiAction::ToggleThinking(bool)`；`crates/pi-cli/src/tui.rs` 收到该 action 后调
`settings::set_hide_thinking_block()` 写回 `settings.json`（保留未知键）。
用例 `crates/pi-cli/tests/p9_f51_ctrl_t_thinking.rs`：**改前 FAILED**（无状态行、设置未写），
改后通过。

## 其余缺项（只登记，不在本批实现）

1. `app.editor.external`（`ctrl+g`）：需要拉起外部编辑器并回填，属独立功能（ref
   `interactive-mode.ts:4436-4460`）。
2. `app.model.cycleBackward`（`shift+ctrl+p`）：见注 1，行为与 ref 相反。
