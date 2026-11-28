# F59 批次计划

规格来源：`ref/pi-0.87.1/`（只读）。所有验证由 agent 自动完成；进程只用
`tools/difftest/safe-spawn.sh` / `safe-kill.sh`。每步 L1、独立提交、勾选。

## R 收口 F58 遗留状态

### R1 回滚确认
- [x] `git status`：仅 `?? AGENTS.md`（本批前即存在）
- [x] `git log -5`：`64c57cf` / `4cbc76b` / `2dce06e` / `344da47` / `dc1df18`
- [x] HEAD = `64c57cf6`；`git diff --stat 4cbc76b HEAD` = **只新增
      `crates/pi-tui/tests/t_f58_render_cost.rs`（45 行）+ plan 文本**，无产品代码差异
- [x] "取消 >8 等待"实验已完全回滚：`crates/pi-cli/src/tui.rs:2008-2015`
      仍是 `batch.len() > FIRST_DRAW_WAIT_MAX_ROWS`（阈值 8 生效），无残留注释/分支
- [x] plan 已记录该假设被推翻及证据（`first_draw_begin` 5671 → 5739/5660 ms）

### R2 晚于 L3 的改动须重跑 L3
- [x] `64c57cf` 晚于 F58 的 L3（跑在 `4cbc76b`），因此**本批必须重跑全量 L3**
      并更新 sha256/bundle，不得沿用旧证据

## E3 首屏剩余差距（统一量具 `measure-firstframe.py`，`first_frame = mark − spawn`）

### E3a `renderers_filled → first_draw_begin`（~1.15 s）细分裁决
两种互相矛盾的归因待定论：
- **H-a1「排版耗时」**：这段主要在 `render_all` 全量排版里。
- **H-a2「轮询延迟」**：这段在等事件循环下一轮 `recv_timeout` / 批量事件到达。

**预测（H-a1）**：在 `draw()` 内部把 `render_all` 前后各打一点，若 `draw` 内部耗时
接近整段 1.15 s，则为排版耗时。
**推翻条件**：若 `first_draw_begin → render_all 完成` 远小于 1.15 s（例如 <300 ms），
则 H-a1 被推翻，改判 H-a2。
- [x] 增设打点：`draw_render_begin` / `draw_render_done`、`draw_diff_begin`、
      `draw_write_begin` / `draw_write_done`，以及等待判定点 `wait_tool_batch` / `wait_first_ui`
- [x] 实测 3 次（真实 116 MB 会话）：
      `renderers_filled 4431 → first_draw_begin 5629`；`draw_render_begin 5629`（**与
      first_draw_begin 同刻**）→ `draw_render_done 5642`（**排版仅 13 ms**）→ 差分 8 ms
      → 写出 28 ms。**结论：H-a1（排版耗时）被推翻，H-a2（事件循环等待）成立**——
      整段 1198 ms 全部发生在 `draw()` 之前
- [x] 等待对象已定位：`wait_tool_batch` 打点 **87 次**，4411 → 5617 连续命中，
      即 **1206 ms 全在等扩展工具行批次**（`tool_row_first_deadline` 的 1500 ms 上限截断）

### E3b `transcript_built`（~1.3 s）细分
- [x] 拆解（真实会话，3 次一致）：
      `transcript_from_messages` = **1442–1455 ms**（`tui_enter 2938 → done 4380`），
      其中条目数 **2917**（`-c` 只渲染 `buildContextEntries`，F49），约 0.50 ms/条；
      `fill_extension_renderers` 仅 **24 ms**（内置工具已被跳过）。
      **主体是 `transcript_from_messages` 里对每个工具调用/结果各发一次同步
      `render_tool_block` RPC**（跨进程往返 × 2917）
- [x] 附带发现：`summary: serde_json::Value::Object(arguments.clone())` 会克隆整份
      `write`/`edit` 参数（单条可达数 MB）。已改为 `tool_summary_from_args`
      （按 key 借用，不克隆）——语义等价，`t_f58_*`/golden 未变

### E3c 每项先写预测与推翻条件
- [x] **实验 1（已撤销）**：把 `transcript_from_messages` 内的逐条 RPC 延后给
      `fill_extension_renderers`。
      预测：`transcript_from_messages` 从 1455 → 数十 ms。
      实测：确降到 1025/976 ms，首帧 5680 → 5170 ms。
      **但 `T116-toolrow-collapsed` 变 different**——该场景首帧就需要工具行内容，
      延后导致快照前少两行（`$ false` / `(no output)`）。**判据：golden 不得变红 ⇒ 撤销**。
- [x] **实验 2（已撤销）**：取消 `batch.len() > 8` 条件（永不因工具行空等）。
      预测：省 1.2 s。实测：6.62–6.74 s vs 6.82–6.89 s，**仅省约 0.15 s** 且首帧先画原生块
      再回填（多一帧）。**判据：收益不显著 ⇒ 撤销**（与 F58/E4 结论一致）。
- [x] **实验 3（保留）**：`tool_summary_from_args` 不克隆 arguments。
      实测 7.26–7.46 s vs 7.36–7.89 s，收益在噪声内但方向为正、语义等价 ⇒ 保留
- [x] **实验 4（已撤销）**：把 `tool_row_first_deadline` 上限 1500 → 3000 ms 等批次真正完成。
      预测：少画一帧、总字节下降。实测 5861/6214 ms（**更慢**）⇒ 撤销
- [x] 对照 ref 行号确认不改变可见语义：
      `interactive-mode.ts:4056-4062` `renderInitialMessages()` 用
      `buildContextEntries()` 的**引用**逐条建组件；`tool-execution.ts:110-118` 的渲染器在
      建组件的**同一步**同步调用（同进程）。pi-rs 因为跨进程，逐条 RPC 是等价语义的
      实现方式，**去掉它就会改变首帧可见内容**（实验 1 的 `T116` 变红即证据）

### E3d 无优化空间须给打点证据
- [x] 打点证据（非"ref 也如此"）：`draw()` 内排版 **13 ms**、差分 **8 ms**、
      写出 **28 ms**，而首帧前的事件循环等待 **1206 ms**（87 次 `wait_tool_batch`）。
      即瓶颈**不在渲染内核**，而在"等扩展把 2917 行工具行渲染完"。
      这一等待无法在不改变 golden 的前提下消除（实验 1 已证：延后会让 `T116` 变红）。

### E3e 终表
- [x] 终表（统一量具 `measure-firstframe.py`，`first_frame = mark − spawn`，各 3 次）：

      | 实现 | first_frame | 备注 |
      |---|---|---|
      | pi-rs（本批后） | 6.82 / 6.89 / 6.83 s | 保留实验 3 |
      | pi-rs（本批前，F58 末） | 7.36 / 7.37 / 7.41 s | — |
      | pi aot | 4.32 / 4.28 / 4.24 s | `<aot-binary>` |
      | TS bundle | 4.83 / 4.85 / 4.69 s | `pi-ts-0.87.1/dist/bundle/cli.js` |

      pi-rs 阶段（1 次代表值，ms）：`messages_built 903` → `prompt_built 1060` →
      `session_read 1060` → `pre_writer 1709` → `tui_enter 2938` →
      `transcript_from_messages 2938→4380`（1442）→ `renderers_filled 4404` →
      `wait_tool_batch 4404→5635`（1231）→ `first_draw_begin 5635` →
      排版 13 / 差分 8 / 写出 28 → **`first_frame_written 5684`**
- [x] **目标未达**：与 TS bundle 差距 **≈2.0 s**（6.85 vs 4.79 s），未进入 ≤1 s。
      原因已用打点定位且**两处可优化路径均已实验并证明会破坏 golden 或收益不显著**
      （实验 1 / 2）。**写入批次头缺项。**

## G 复核 F58 保留项

### G1 `initial_messages` 借用改动
- [x] 该改动是**纯性能**改动（行为不变）：把 `parts.initial_messages.clone()` 改为借用切片，
      `history` 处用 `to_vec()` 克隆一次（原先是 clone 两次）。**没有任何可见行为差异**
      （`AgentParts` 字段类型未变、调用方未变），全部 golden 保持 same
- [x] 依据：`2dce06e` 实测 `transcript_built` 1463 → 1299 ms（-164 ms，占首帧 2.3%）；
      E3b 细分显示该段（`transcript_from_messages`）总耗时 1442 ms，其中 clone 占 164 ms
- [x] **判据：收益虽小但方向明确、零风险（少一次全量克隆）、不改公共形状 ⇒ 保留**。
      不撤销的理由：不是把"无效改动"留着，而是"与实验 1/2 不同，它不改变任何可见语义"。

### G2 T116 阈值 8（LIMITATIONS 21）
- [x] LIMITATIONS 21 已含推翻条件（①在 ref 找到等价的"首帧等待渲染器"点 ⇒ 照抄；
      ②折叠行改回本进程内渲染 ⇒ 删除该阈值）
- [x] 已探索两条"比空等更对齐 ref"的方案并给出实测结论：
      ① **首帧前阻塞一次批量 `render_tool_rows`**：这正是当前 `wait_tool_batch` 做的事
         （批次请求已发出，等待它回填），实测该批次需要 **1206 ms**（2917 行）；
         把上限从 1500 加到 3000 反而更慢（5861/6214 ms），说明**不是空等而是真在算**；
      ② **不等、先画原生块再回填**（实验 2）：只省 0.15 s，且多写一帧 ⇒ 撤销。
      结论：pi-rs 已经在做"一次 RPC 往返"而非固定 sleep，阈值 8 只是"小批量不值得等"的
      经验分界。**保留现状，理由：两种替代方案实测均无净收益**。
- [x] 只登记不修的理由：`wait_tool_batch` 的 1206 ms 是扩展**真实计算时间**（2917 行 ×
      每行一次组件渲染），消除它只能靠"不等"（实验 2，收益 0.15 s 但改变首帧内容）
      或"延后渲染"（实验 1，`T116` 变红）。两者均不可接受 ⇒ 登记为架构性差异。

## L3 批末

`cargo clean` → test-gate、clippy `-D warnings`、precommit、regress dark（并发+串行）+ light、
self-verify、real-parity、real-parity-linux、privacy-scan、readonly、check-orphans、
git status 干净、删 `tmp/realcfg*`。
**任何 different、未解项、E3 未达标，一律写"未完成"并列缺项。**

## L3 批末实测（HEAD eaaef84）

`cargo clean` → 全量重建 →
- `test-gate` **228 组 / 1188 passed / 0 failed**；`fmt --check` 干净；`clippy -D warnings` 0 告警
- 其余项见下方追加记录
- `precommit` **16/16**；`privacy-scan` 干净；`readonly` 8+2；`check-orphans` 0
- dark **并发 375 same / 0 different / 69 nogolden**、**串行（`--jobs 1`）375/0/69**
- light **81 same / 0 different / 0 nogolden**
- `self-verify` **26/26 rc=0**；`real-parity-linux` **8/8 rc=0**

## E3f：进一步实验（批次体积 / payload / 块大小），全部撤销

| 实验 | 预测 | 实测 | 裁决 |
|---|---|---|---|
| `CHUNK` 50 → 400（59 块合并成 8 块） | 少 51 次 RPC 往返 ⇒ 更快 | 7.53 / 7.56 s（基线 6.82 / 6.89） | **更慢 ⇒ 撤销**（单条 RPC 行数过多反增开销） |
| `truncate_result_payload` 上限 8 KB → 512 B | payload 变小 ⇒ 传输更快 | 7.32 / 7.29 s | **无改善 ⇒ 撤销**（payload 体积不是瓶颈） |

**为什么这些都不是瓶颈**：用 `PI_TOOL_ROW_LOG` 采集行级统计，1938 行里
**1862 行返回 `arr(1)`、76 行 `arr(2)`，无一行 null** —— 扩展对每一行都真的
构造了 `ToolExecutionComponent` 并 `render()`。这 1206 ms 是扩展的真实计算，
不是空等、不是传输、不是调度。

**与 ref 的差异根因**：ref 在同一进程内同步渲染（`tool-execution.ts:110-118`），
没有"发 RPC + 等回包"的往返；pi-rs 因为扩展跑在独立 Node 进程，必须跨进程。
要消除这 1.2 s 只能把扩展渲染搬回 Rust 进程内（等于放弃扩展渲染器），
与规格冲突 ⇒ **本批不做，登记为架构性差异**。E3e 差距 2.0 s 的缺项由此确认。
