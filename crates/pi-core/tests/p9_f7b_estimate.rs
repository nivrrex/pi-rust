//! P9-F7b：`bashExecution` 的 token 估算与 compaction 切点，按 ref 口径
//! （`compaction.ts:359-362` 的 `chars = command.length + output.length`、`:377`/`:391` 的切点判定）。

use pi_ai::message::Message;

#[test]
fn token_estimate_and_cut_points_match_ref() {
    let message: Message = serde_json::from_value(serde_json::json!({
        "role":"bashExecution","command":"echo hi","output":"hi\n",
        "exitCode":0,"cancelled":false,"truncated":false,"timestamp":1
    }))
    .expect("反序列化");
    // "echo hi"(7) + "hi\n"(3) = 10 ⇒ ceil(10/4) = 3
    assert_eq!(pi_core::agent::compaction::estimate_tokens(&message), 3);
    assert!(pi_core::agent::compaction::is_cut_point_message(&message));
    assert!(pi_core::agent::compaction::is_turn_start_message(&message));
}
