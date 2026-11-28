# F68–F71：扩展 API 缺口（已装扩展实际调用路径）

## 已修（已验正）
| 项 | 修前 | 修后 |
|---|---|---|
| `ctx.getSystemPrompt` (F68) | **方法不存在**（`try/catch` 吞掉 TypeError） | 宿主 `set_ctx_system_prompt` 三入口填入，runner 返回 `hostCtx.systemPrompt` |
| `ctx.signal` (F69) | **字段不存在**（`pi-agent-resilience` 用 `instanceof` 判断后降级） | getter 返回当前操作控制器；`ctx.abort()` 先同步 abort 本地再通知宿主 |
| `pi.getAllTools` (F70) | 只返回扩展注册的表 ⇒ 内置 `bash`/`edit` 永不在列表 | 宿主下发内置清单（`sourceInfo.kind="builtin"`），runner 合并、扩展同名覆盖内置 |

## F69 的两次自我纠正（重要）
1. 第一版用**进程级固定** `AbortController` ⇒ abort 一次后该进程内所有回合都读到 aborted，`pi-agent-resilience` 的取消保护永久生效。这是行为偏差而非缺字段。改为每次构造 ctx（= 新操作）时若已 abort 则换新控制器。
2. 第一版靠"收到新 ctx"触发重建，是**死代码**：`emit_value` 只带 payload，ctx 经 `PI_EXT_CTX` 在握手时下发一次、之后 `set_ctx` 只更新宿主内部状态。
3. 测试也两次修正才有判别力：`update_ctx_model(None, None)` 不做有效下发；取 `seen.first()` 拿到的是闭包累积的上一次记录，须用 `last()`。

## 已知偏差（工程判断，已记 LIMITATIONS）
- ref 的 `signal` 每次**请求**新建、粒度精确；pi-rs 以"事件边界"近似，同一事件处理期间共享一个控制器。

## F71：取证为"无缺陷"（保留回归）
`pi.registerFlag` / `pi.getFlag` 链路完整：CLI 值能下发、未给出的回落注册默认值。
`pi-handoff-gate` 注册 4 个 flag 并在 `session_start` 读取，这条断掉会静默禁用其全部开关。

## 按口径不扩范围（已装扩展代码级 grep 为 0）
- `ctx.ui` 的 14 个成员（`onTerminalInput`/`setEditorText`/`getTheme` 等）：仅出现在 `pi-ai-summary/README.md` 的否决说明里，无代码调用。
- `pi.setModel`（运行期换模型要重建整个 agent 循环）、`pi.setActiveTools`（工具集是构造期不可变 `Arc<Vec<...>>`）、`pi.exec`/`pi.sendMessage`/`pi.registerShortcut` 等。

## 批末 L3
test-gate 286 组 / 1450 项 / 0 失败；precommit 16/16；real-parity-linux 0 格。
