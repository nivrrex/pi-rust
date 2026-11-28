# F65：`--session` / `-c` 的历史显示（澄清误判）

## 结论
**不是缺陷。** `--session` 与 `-c` 都能正确显示 user + assistant 历史。

## 误判来源（已验证）
此前记录的"`--session` 不显示历史内容"来自手写夹具：assistant 条目缺
`api`/`provider`/`model`/`usage` 这些必填字段 ⇒ 整条 `Message` 反序列化失败 ⇒
`#[serde(flatten)] extra` 把外层 `parentId` 一并吞进 `extra` ⇒ 条目链断 ⇒
`build_session_projection` 停在第一条（只剩 user）。

`--continue` 另一个独立坑：`newest_session_file` 按 header 的 `cwd` 过滤，
测试必须 `current_dir` 到会话记录的 cwd，否则找不到文件（这属测试写法，不是缺陷）。

## 固化
`crates/pi-cli/tests/p9_f65_session_history.rs`（2 项）：
1. `complete_fixture_projects_both_messages`——**前提守卫**：完整夹具必须投影出 2 条；
2. `session_flag_renders_the_history`——端到端首帧按序含 `user`/`assistant`。

去掉必填字段即复现旧现象（前提断言报"实际 [User]"，端到端报 `["user"]`），
判别力已验证。

## 未验证
真实 TS 会话文件的逐字节对拍未重跑（会话互通已由 `session-interop.mjs` 覆盖）。
