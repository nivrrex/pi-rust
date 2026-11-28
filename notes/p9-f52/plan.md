批次状态：完成

# F52 清单

起点 HEAD `6aecc3cd`（F51 最终）。F51 的"完成"在本批作废，以本批首行为准。

## 默认取值（无决策点，按此执行）
1. 每个按键序列拆成独立场景（比较器只比最后快照）。
2. `-ne` 与合成扩展夹具两种配置都要覆盖；light 场景放 `scenarios-tui-light/`。
3. 诊断一律写文件（eprintln 在 pty 下不可见）。

## 步骤
- [x] 0 取证：P1–P3 预测 + 文件诊断（tools_expanded、collapsed_row、ext.render.tool 的 expanded/命中、hide_thinking、ext.render.thinking 返回、渲染缓存键成分）
- [x] A ctrl+o 双向
- [x] B ctrl+t 实效
- [x] C 工具行折行 + 三条不变量
- [x] TS 金标场景：8 个（F52a–F52g + L-F52a）全部 same
- [x] D 取证（E5）：-c / --session / git 分支 / 会话 cwd 四场景 same；根因是 --session 未用会话头 cwd
- [x] F51 债务：缓存键三用例（折叠行 / hide 标志 / 标签）改前 2 FAILED
- [x] L3：dark 122/0、light 27/0、workspace 1131/0、precommit 11/11

## 0 取证结论（读源码 + 靶向实验）

| 预测 | 结论 | 证据 |
| --- | --- | --- |
| P1 `ctrl+o` 展开时折叠行被丢弃、回折不重取 | **前半成立**：展开态渲染仍优先用 `collapsed_row`（`pi-tui/src/lib.rs` 的 ToolCall 分支），所以 ctrl+o 屏上无变化；回折其实不丢（F51 已改为保留旧行） | 真机 E3 + 单元用例 `ctrl_o_toggles_both_ways`（改前 FAILED） |
| P2 `ctrl+t` 缓存键缺 hide 标志 / 扩展标签路径无视它 | **一半成立**：缓存键里已有 hide（F51 加的），真正的原因是我们把**全文丢掉、只存标签**，可见态无内容可渲染；且隐藏态把标签丢进 markdown ⇒ `+ ` 变 `- ` | `tool-render` 无此行号，改看 `assistant-message.ts:129-152`；用例 `ctrl_t_switches_between_label_and_full_text`（改前 FAILED） |
| P3 native 头部未按 ref Text 处理换行/制表符/折行 | **成立** | `text.ts:60-68`（`replace(/\t/g,"   ")` + `wrapTextWithAnsi`）对照我们原来的 `clamp_to_width`；用例 `t_native_invariants` 改前 3/4 FAILED（裸 LF/TAB、长路径被截断） |

## 已完成（A/B/C）

- **A**（`04c5a2fa`）：`collapsed_row` 只在 `!state.tools_expanded` 时生效；展开态忽略扩展 `renderCall`/`renderResult` 降级行（它们是按 `expanded=false` 生成的预览）。ref：`interactive-mode.ts:4397-4416`、`tool-execution.ts:244-247`、`tool-render.ts:186-196`。
- **B**（`04c5a2fa`）：`TranscriptItem::Thinking` 改为 `{text, label}`，隐藏态渲染标签（原样输出、不走 markdown），可见态渲染全文。ref：`interactive-mode.ts:4429-4434`、`assistant-message.ts:129-152`、`:30`。
- **C**（`04c5a2fa`）：native 头部与正文按 `text.ts:60-68` 处理——制表符→3 空格、按 `\n` 拆行、超宽折行。`clamp_to_width_ansi` 只剩"扩展返回的超宽行"这一处来源。

## A 的二次修正（`8056b86d`）

第一版把"展开态忽略扩展 renderCall/renderResult"也一起改了，导致 `T90-render-hook` 从 same 变
different（该 golden 里 ` expanded=true` 那行正是扩展按展开态渲染的结果）。
ref 里 `ToolExecutionComponent` **不做**这种过滤（`tool-execution.ts:110-118` 始终用渲染器组件），
展开态的差别由"按 `expanded` 重新请求"负责（`:352`）。已回退该过滤，T90 恢复 same。

## 仍未完成：TS 金标场景

已建 5 个场景（`F52a..F52e`）+ 夹具（`f52-bash-heredoc.sse`、`f52-collapse-ext.ts`），TS 录制成功但 pi-rs 对拍 **different**，两点原因**均非渲染**：
1. 夹具的 `edit` 参数用的是 `{path, oldText, newText}`，TS 接受、pi-rs 报
   `Validation failed for tool "edit"`（两边 schema 不同）；
2. 末帧整体上滚位置不同（TS 顶部保留 ` hi` 与助手正文，pi-rs 从工具行开始）。

为避免把"已知 different"塞进 regress，**场景与 golden 已删除**，只保留夹具生成器与扩展夹具；
两者修好后可原样重建。F52 因此未完成。
