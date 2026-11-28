# F123–F137 批次收口（15 个提交，HEAD `68fa834`）

## L3 结果（全部实跑）
- `scripts/precommit.sh`：**18 项全部通过**（首轮因我 F132 验证留下的 1 个
  `mock-provider` 残留报红，按 pidfile 三元组清理后重跑通过）。
- `scripts/regress-tui.sh`：**125 场景全 same，0 diff、0 缺失**。
- `scripts/regress-cli.sh`：**92 个可跑，75 same / 17 different / 0 runfail**——
  与 F124 的基线名单**完全一致**（见下"差异性质"）。
- `cargo test`（precommit 内含）：全过。
- `check-no-stray`：无残留。

## 本批唯一的功能改动
**F132**：`compaction_start` 的派发从"压缩成功之后"提到**摘要请求之前**，
与 ref `agent-session.ts:2761-2772` 一致，并删掉成功/失败分支里的两处重复。

**效果**：`S13c` 的事件序列与 golden **逐条相同**（此前 `summarization_retry_*`
排在 `compaction_start` 之前）。差异从**两处减为一处**——只剩
`estimatedTokensAfter` 那 35。因为计数按"场景是否 same"算，它仍计为 different，
但从"事件顺序错"变成了"只有一个展示数字差"。

## 差异性质（CLI 的 17 个）
与 F124 基线名单一致 ⇒ 本批没有让任何一个变好或变坏。其中 7 个
（`S19 S20 S20b S20e S21 S22 S24b`）的差异都是 `estimatedTokensAfter` 差 35，
已在 LIMITATIONS 里留下精确测量与下一步指向。

## 本批最大的教训（三次自我纠正，全部入库）
围绕 `S13c`/那 35 token，先后做了 **F125→F137 十三张卡**，其中：
- **F128–F130 三张全部作废**：把 `run.mjs` 的默认 `runs = 3`（三个独立进程）
  误读成"一次运行里调了三次压缩"，据此推断出"重复压缩缺陷"——**不存在**。
- **F133 的键序归因不成立**：加 `preserve_order` 后数字一个没变，
  因为 `eta` 算的是内存投影，不经过 `to_value`。
- **F126 的撤回过度**：只看 `A=same` 就否掉整个发现，而 **B 层差异是真实的**。

⇒ 两条方法结论：①看单次运行的行为必须 `--runs 1` 或只看 `run-1/`；
②判断场景状态要**分层看**（A/B/C 各自独立）。

## 主动停止的一项
`estimatedTokensAfter` 的 35：F135/F136 已拿到精确分解（内存 `sections` 比文件
多 349、`toolsAdded` 少 123）与下一步方向，但 F137 判定**性价比不足**——
它只影响一个展示用数字，不参与压缩判定、不影响会话内容，且该 7 个场景的
`A`/`C` 层都已 same。停在一个"已知、有测量、有指向"的位置。
