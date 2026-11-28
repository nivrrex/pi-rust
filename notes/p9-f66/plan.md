# F66/F67：扩展 `pi.*` 三个空实现

## 已修（已验证）
| API | 修前 | 修后 |
|---|---|---|
| `pi.setSessionName` | `setSessionName() {}`（空） | `host.session.setName` → `append_session_info_to`（写条目 + `session_info_changed` + 刷新 `ctx.sessionName`） |
| `pi.getSessionName` | **重复定义两次**，后者读从未被赋值的 `hostCtx.sessionName` ⇒ 恒 `undefined` | 读宿主下发的 `ctx.sessionName` |
| `pi.setThinkingLevel` | `setThinkingLevel() {}`（空） | `host.thinking.setLevel` → `append_thinking_level_change`（条目 + `thinking_level_changed` + `thinking_level_select` 一并完成） |
| `pi.getThinkingLevel` | 恒返回 `"off"`（假值） | 读 `ctx.thinkingLevel` |

## 顺带修复（代用户扩大范围）
- 两个 `append_session_info*` 出口此前不更新 `ctx.sessionName` ⇒ 扩展改名后立刻读回仍是旧值。已补。

## 测试侧两个坑（已处理，非实现缺陷）
1. `SessionWriter` 是**惰性**的（首条 assistant 前磁盘无文件），夹具必须自己写 header，否则 `SessionFile::read` 报 `MissingHeader`。
2. `pi.setSessionName` 是**异步** RPC（ref 也是 `void emit`，不等待）⇒ 断言必须轮询等待条目出现（5s 有界），不能固定 `sleep`。首次并行跑时因此失败过一次。

## 未实现（维持登记，未扩范围）
- `pi.setModel`：`Provider::stream` 构建时固定 provider，运行期换模型要重建整个 agent 循环；两个已装扩展均未使用（grep 为 0）。
- `pi.setActiveTools`：工具集是构造期的 `Arc<Vec<Arc<dyn Tool>>>` 不可变，需引入共享过滤名单并让 `tool_specs` 与派发都按它过滤；两个已装扩展均未使用。
- `pi.exec` / `pi.sendMessage` / `pi.registerShortcut` 等其余项同理登记在 LIMITATIONS。

## 批末 L3
test-gate 286 组 / 1450 项 / 0 失败；precommit 16/16；real-parity-linux 0 格。
