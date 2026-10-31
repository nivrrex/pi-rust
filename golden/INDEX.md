# golden/INDEX.md — 差分基线总览（批次 B5 / E4 重算）

「sane」= 产出是否成形；「stableLayers」= 连跑 3 次一致命中的层（A=session+文件树+退出码，B=事件流，C=provider 请求体）。
全部基线在批次 B5 用**修复后的 mock 与统一归一化器**重录过（各连跑 3 次）。

变体集只在 B 层不稳时才保留；B 层已稳定的场景其 `variants/` 已删除。

| 场景 | 模式 | sane | stableLayers | 签收分类 | 备注 |
| S09a | rpc | True | A/B/C | A/B/C 硬判据 | abort 已发出的请求（闸门在第 2 块之后）|
|---|---|---|---|---|---|
| S01 | print | True | A/B/C | A/B/C 硬判据 |  |
| S02 | print | True | A/B/C | A/B/C 硬判据 |  |
| S03 | print | True | A/B/C | A/B/C 硬判据 |  |
| S04 | print | True | A/B/C | A/B/C 硬判据 |  |
| S04b | print | True | A/C | A/C 硬判据（B 软判据） |  |
| S04c | print | True | A/B/C | A/B/C 硬判据 |  |
| S05 | print | True | A/C | A/C 硬判据（B 软判据） |  |
| S06 | print | True | A/C | A/C 硬判据（B 软判据） |  |
| S07 | print | True | A/C | A/C 硬判据（B 软判据） |  |
| S08 | print | True | A/C | A/C 硬判据（B 软判据） |  |
| S09a | rpc | False | — | 不作基线 | abort 已发出的请求（**未通过**：abort 落在 settle 之后，mock 未收到请求） |
| S09b | rpc | True | A/B/C | A/B/C 硬判据 | abort 阻塞工具（tool_execution_start 之后） |
| S10 | rpc | True | A/B/C | A/B/C 硬判据 |  |
| S11 | rpc | True | A/C | A/C 硬判据（B 软判据） |  |
| S12-anthropic | print | True | A/B/C | A/B/C 硬判据 |  |
| S12-openai | print | True | A/B/C | A/B/C 硬判据 |  |
| S13a | print | True | A/B/C | A/B/C 硬判据 |  |
| S13b | print | True | A/B/C | A/B/C 硬判据 |  |
| S13b-text | print | True | A/B/C | A/B/C 硬判据 |  |
| S13b-toolcall | print | True | A/B/C | A/B/C 硬判据 |  |
| S13c | print | True | A/B/C | A/B/C 硬判据 |  |
| S14 | print | True | A/C | A/C 硬判据（B 软判据） |  |
| S15 | print | True | A/B/C | A/B/C 硬判据 |  |
| S16 | rpc | True | A/C | A/C 硬判据（B 软判据） |  |
| S17 | print | True | A/C | A/C 硬判据（B 软判据） |  |
| S18 | print | True | A/B/C | A/B/C 硬判据 | P4-1 新录：usage 与 cost（含 cached_tokens/cache_write_tokens，Q7 口径）|
| X01 | print | True | A/B/C | A/B/C 硬判据 | P4-2 新录（全 TS：TS→TS→TS）：TS 写 → Rust 续写 → TS 再读 |
| X02 | print | True | A/B/C | A/B/C 硬判据 | P4-2 新录（全 TS）：Rust 写 → TS 续写 → Rust 再读 |
| S19 | print | True | A/B/C | A/B/C 硬判据 |  |
| S20 | rpc | True | A/B/C | A/B/C 硬判据 |  |
| S21 | print | True | A/B/C | A/B/C 硬判据 |  |
| S22 | print | True | A/B/C | A/B/C 硬判据 |  |
| S23 | print | True | A/B/C | A/B/C 硬判据 |  |
| S23b | print | True | A/B/C | A/B/C 硬判据 | skills 与 project_context 并存 |
| S24 | print | True | A/B/C | A/B/C 硬判据 | 斜杠命令展开（print 通道） |
| S26 | print | True | A/B/C | A/B/C 硬判据 | 工具参数校验：缺必填（TS 的原文）|
| S26b | print | True | A/B/C | A/B/C 硬判据 | 类型错**不报错**（TS 放行）|
| S26c | print | True | A/B/C | A/B/C 硬判据 | 多余字段**不报错**（TS 忽略）|
| S26d | print | True | A/B/C | A/B/C 硬判据 | write 缺必填 content 的校验原文 |
| S26e | print | True | A/B/C | A/B/C 硬判据 | edit 缺必填 edits 的校验原文 |
| S26f | print | True | A/B/C | A/B/C 硬判据 | bash 缺必填 command 的校验原文 |
| S09a | rpc | True | A/B/C | A/B/C 硬判据 | abort 已发出的请求 |
| S24b | rpc | True | A/B/C | A/B/C 硬判据 | 斜杠命令展开（rpc 通道） |
| S25-anthropic | print | True | A/C | A/C 硬判据（B 软判据） | 图片读取的 toolResult（Anthropic） |
| S25-openai | print | True | A/C | A/C 硬判据（B 软判据） | 图片读取的 toolResult（OpenAI） |

合计：A/B/C 硬判据 24 个、A/C 硬判据（B 软判据）11 个、不作基线 1 个。

**已移出基线**：`S09`（场景脚本等待的 `text_start` 事件在 rpc 事件流里从不出现；旧通过是坏 runner 的假象）。现场保留在 `golden/_invalid/S09/`。

**S09a 已取代 S09**：覆盖「取消已发出的请求」——闸门卡在第 2 块（首个文本增量已到达）之后，abort 落在流进行中；结果为 `aborted` + 部分文本 `Hello`，请求日志 1 行，`gateHits=1` 的注入断言通过，连跑 3 次 A/B/C 全稳定。

## golden/sessions

S01（纯对话）、S02（带工具）、S19（带压缩）三份 TS 写出的会话，供 X01 使用。
| S27 | print | True | A/B/C | A/B/C 硬判据 | 手写种子含 custom_message/branch_summary，`--continue` 续写 |
| S28 | rpc | True | A/B/C | A/B/C 硬判据 | **真分叉**种子（m2 下 m3a/m3b）+ `--continue`；命令 get_tree/get_fork_messages/fork/get_tree/prompt |
| S29 | rpc | True | A/B/C | A/B/C 硬判据 | `clone`（以当前叶子为界 fork）的差分场景；命令 get_tree/clone/get_tree/prompt |
| S30 | rpc | True | A/B/C | A/B/C 硬判据 | `get_entries`（全量与 `since` 过滤）的差分场景 |
| S31 | rpc | True | A/B/C | A/B/C 硬判据 | `get_last_assistant_text`（空闲时取种子里的助手回复）的差分场景 |
| S33 | rpc | True | A/B/C | A/B/C 硬判据 |  的差分场景 |
| S34 | rpc | True | A/B/C | A/B/C 硬判据 | new_session + switch_session（`<SESSION_DIR>` 占位符切回种子）的差分场景 |
