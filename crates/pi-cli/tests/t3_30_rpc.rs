//! P3.5：rpc 的帧格式与五个命令（出处见 `pi_cli::rpc` 各函数注释）。

use pi_cli::rpc::{
    handle_command, handle_line, response_error, response_ok, split_lines, RpcRuntime,
    RpcSessionState,
};
use serde_json::json;

#[derive(Default)]
struct FakeRuntime {
    prompted: Vec<String>,
    steered: Vec<String>,
    followed: Vec<String>,
    aborts: usize,
    levels: Vec<String>,
    models: Vec<String>,
}

impl RpcRuntime for FakeRuntime {
    fn compact(&mut self, _custom_instructions: Option<&str>) -> Result<serde_json::Value, String> {
        Err("fake runtime: compact is not implemented in this double".to_string())
    }

    fn prompt(&mut self, message: &str) -> Result<(), String> {
        self.prompted.push(message.to_string());
        Ok(())
    }
    fn steer(&mut self, message: &str) -> Result<(), String> {
        self.steered.push(message.to_string());
        Ok(())
    }
    fn follow_up(&mut self, message: &str) -> Result<(), String> {
        self.followed.push(message.to_string());
        Ok(())
    }
    fn abort(&mut self) -> Result<(), String> {
        self.aborts += 1;
        Ok(())
    }
    fn set_thinking_level(&mut self, level: &str) -> Result<(), String> {
        self.levels.push(level.to_string());
        Ok(())
    }
    fn set_model(
        &mut self,
        provider: &str,
        model_id: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        self.models.push(format!("{provider}/{model_id}"));
        Ok(Some(
            serde_json::json!({ "provider": provider, "id": model_id }),
        ))
    }
    fn fork(&self, entry_id: &str, position: &str) -> Result<serde_json::Value, String> {
        let _ = (entry_id, position);
        Ok(serde_json::json!({"cancelled": false}))
    }
    fn clear_queue(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }
    fn switch_session(&self, _session_path: &str) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }
    fn new_session(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }
    fn messages(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"messages": []}))
    }
    fn last_assistant_text(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }
    fn entries(&self, _since: Option<&str>) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"entries": [], "leafId": null}))
    }
    fn clone_session(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"cancelled": false}))
    }
    fn fork_messages(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"messages": []}))
    }
    fn tree(&self) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"tree": [], "leafId": null}))
    }
    fn state(&self) -> RpcSessionState {
        RpcSessionState {
            model: Some(serde_json::json!({ "provider": "mock", "id": "mock-model" })),
            is_streaming: false,
            steering_mode: "one-at-a-time".to_string(),
            follow_up_mode: "one-at-a-time".to_string(),
            session_file: Some("/tmp/s.jsonl".to_string()),
            session_id: "sid".to_string(),
            message_count: 2,
            pending_message_count: 0,
            thinking_level: "off".to_string(),
            auto_compaction_enabled: true,
        }
    }
}

#[test]
fn the_five_supported_commands_dispatch_to_the_runtime() {
    let mut runtime = FakeRuntime::default();
    for (line, expect_ok) in [
        (json!({"type": "prompt", "message": "hi"}), true),
        (json!({"type": "steer", "message": "s"}), true),
        (json!({"type": "follow_up", "message": "f"}), true),
        (json!({"type": "abort"}), true),
        (json!({"type": "get_state"}), true),
    ] {
        let response = handle_command(&line, &mut runtime);
        assert_eq!(response["success"], expect_ok, "{line}");
        assert_eq!(response["type"], "response");
    }
    assert_eq!(runtime.prompted, vec!["hi"]);
    assert_eq!(runtime.steered, vec!["s"]);
    assert_eq!(runtime.followed, vec!["f"]);
    assert_eq!(runtime.aborts, 1);
}

#[test]
fn async_commands_succeed_without_data() {
    let mut runtime = FakeRuntime::default();
    let response = handle_command(
        &json!({"type": "prompt", "message": "hi", "id": 7}),
        &mut runtime,
    );
    assert_eq!(response["success"], true);
    assert!(response.get("data").is_none(), "异步命令不带 data");
    assert_eq!(response["id"], 7, "id 原样回显");
}

#[test]
fn get_state_returns_the_session_state() {
    let mut runtime = FakeRuntime::default();
    let response = handle_command(&json!({"type": "get_state"}), &mut runtime);
    assert_eq!(response["success"], true);
    assert_eq!(response["data"]["model"]["provider"], "mock");
    assert_eq!(response["data"]["messageCount"], 2);
    assert_eq!(response["data"]["steeringMode"], "one-at-a-time");
    assert_eq!(response["data"]["sessionFile"], "/tmp/s.jsonl");
}

#[test]
fn unimplemented_commands_say_so_instead_of_pretending_to_work() {
    let mut runtime = FakeRuntime::default();
    let response = handle_command(&json!({"type": "bash", "command": "ls"}), &mut runtime);
    assert_eq!(response["success"], false);
    assert_eq!(response["command"], "bash");
    assert_eq!(
        response["error"],
        "Command not implemented in this version: bash"
    );
}

/// P5-4a：`compact` 现在是**已实现**命令（`SUPPORTED` 计数 +1、`RECOGNIZED_UNIMPLEMENTED` -1）。
///
/// 钉住"已分发"这件事：响应来自假运行时的 `compact`（错误文本是假运行时自己的），
/// **不是** `Command not implemented in this version: compact`。反证：未实现的命令仍是后者。
#[test]
fn compact_is_dispatched_instead_of_reported_unimplemented() {
    let mut runtime = FakeRuntime::default();
    let response = handle_command(
        &json!({"type": "compact", "customInstructions": "只保留结论"}),
        &mut runtime,
    );
    assert_eq!(response["command"], "compact");
    assert_eq!(response["success"], false);
    assert_eq!(
        response["error"], "fake runtime: compact is not implemented in this double",
        "响应必须来自已分发的 compact，而不是未实现分支"
    );
    assert_ne!(
        response["error"],
        "Command not implemented in this version: compact"
    );
}

#[test]
fn unknown_commands_use_the_spec_error_text() {
    let mut runtime = FakeRuntime::default();
    let response = handle_command(&json!({"type": "nonsense"}), &mut runtime);
    assert_eq!(response["error"], "Unknown command: nonsense");
    assert_eq!(response["command"], "nonsense");
}

#[test]
fn a_broken_line_reports_a_parse_error_with_the_parse_command_name() {
    let mut runtime = FakeRuntime::default();
    let response = handle_line("{not json", &mut runtime);
    assert_eq!(response["success"], false);
    assert_eq!(response["command"], "parse");
    assert!(response.get("id").is_none(), "解析失败时没有 id");
    assert!(
        response["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Failed to parse command: "),
        "实际：{}",
        response["error"]
    );
}

#[test]
fn runtime_errors_become_error_responses_carrying_the_id() {
    struct Failing;
    impl RpcRuntime for Failing {
        fn compact(
            &mut self,
            _custom_instructions: Option<&str>,
        ) -> Result<serde_json::Value, String> {
            Err("fake runtime: compact is not implemented in this double".to_string())
        }

        fn prompt(&mut self, _message: &str) -> Result<(), String> {
            Err("boom".to_string())
        }
        fn steer(&mut self, _message: &str) -> Result<(), String> {
            Ok(())
        }
        fn follow_up(&mut self, _message: &str) -> Result<(), String> {
            Ok(())
        }
        fn abort(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn set_thinking_level(&mut self, _level: &str) -> Result<(), String> {
            Ok(())
        }
        fn set_model(
            &mut self,
            _provider: &str,
            _model_id: &str,
        ) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }
        fn fork(&self, entry_id: &str, _position: &str) -> Result<serde_json::Value, String> {
            let _ = entry_id;
            Ok(serde_json::json!({"cancelled": false}))
        }
        fn clear_queue(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
        fn switch_session(&self, _session_path: &str) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
        fn new_session(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
        fn messages(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"messages": []}))
        }
        fn last_assistant_text(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
        fn entries(&self, _since: Option<&str>) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"entries": [], "leafId": null}))
        }
        fn clone_session(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"cancelled": false}))
        }
        fn fork_messages(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"messages": []}))
        }
        fn tree(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"tree": [], "leafId": null}))
        }
        fn state(&self) -> RpcSessionState {
            RpcSessionState {
                thinking_level: "off".to_string(),
                auto_compaction_enabled: true,
                model: None,
                is_streaming: false,
                steering_mode: "one-at-a-time".to_string(),
                follow_up_mode: "one-at-a-time".to_string(),
                session_file: None,
                session_id: "sid".to_string(),
                message_count: 0,
                pending_message_count: 0,
            }
        }
    }
    let response = handle_command(
        &json!({"type": "prompt", "message": "x", "id": "a"}),
        &mut Failing,
    );
    assert_eq!(response["success"], false);
    assert_eq!(response["error"], "boom");
    assert_eq!(response["id"], "a");
}

#[test]
fn frames_split_only_on_lf_and_strip_carriage_returns() {
    // U+2028 在 JSON 字符串里是合法内容，**不能**被当成分隔符。
    let text = "{\"a\":\"x\u{2028}y\"}\r\n{\"b\":1}\n";
    let lines = split_lines(text);
    assert_eq!(lines.len(), 3, "两行 + 末尾空串");
    assert_eq!(lines[0], "{\"a\":\"x\u{2028}y\"}", "\r 被剥掉、U+2028 保留");
    assert_eq!(lines[1], "{\"b\":1}");
}

#[test]
fn response_helpers_produce_the_spec_shapes() {
    let ok = response_ok(Some(&json!(1)), "prompt", None);
    assert_eq!(
        ok,
        json!({"id": 1, "type": "response", "command": "prompt", "success": true})
    );
    let err = response_error(None, "prompt", "bad");
    assert_eq!(
        err,
        json!({"type": "response", "command": "prompt", "success": false, "error": "bad"})
    );
}

#[test]
fn get_tree_is_supported_and_returns_tree_shape() {
    // 对照必须真的触发：`get_tree` 必须在**已实现**列表里（此前它在"已识别但未实现"里）。
    let mut fake = FakeRuntime::default();
    let response = pi_cli::rpc::handle_command(
        &serde_json::json!({"id": "r1", "type": "get_tree"}),
        &mut fake,
    );
    assert_eq!(
        response["success"],
        serde_json::json!(true),
        "{response:#?}"
    );
    assert_eq!(response["command"], "get_tree");
    let data = &response["data"];
    assert!(data.get("tree").is_some(), "响应必须带 tree：{data:#?}");
    assert!(data.get("leafId").is_some(), "响应必须带 leafId：{data:#?}");
}

#[test]
fn get_fork_messages_is_supported_and_returns_messages_shape() {
    let mut fake = FakeRuntime::default();
    let response = pi_cli::rpc::handle_command(
        &serde_json::json!({"id": "r2", "type": "get_fork_messages"}),
        &mut fake,
    );
    assert_eq!(
        response["success"],
        serde_json::json!(true),
        "{response:#?}"
    );
    assert_eq!(response["command"], "get_fork_messages");
    assert!(
        response["data"]["messages"].is_array(),
        "响应必须带 messages 数组：{response:#?}"
    );
}

#[test]
fn fork_is_supported_and_returns_cancelled_shape() {
    // 对照必须真的触发：`fork` 必须在**已实现**列表里（此前它在"已识别但未实现"里，
    // S28 的 B 层第 3 行就是 `Command not implemented in this version: fork`）。
    let mut fake = FakeRuntime::default();
    let response = pi_cli::rpc::handle_command(
        &serde_json::json!({"id": "r3", "type": "fork", "entryId": "m3a"}),
        &mut fake,
    );
    assert_eq!(
        response["success"],
        serde_json::json!(true),
        "{response:#?}"
    );
    assert_eq!(response["command"], "fork");
    assert!(
        response["data"]["cancelled"].is_boolean(),
        "响应必须带 cancelled：{response:#?}"
    );
}

#[test]
fn fork_reports_failure_with_the_ts_error_text() {
    struct Refusing;
    impl RpcRuntime for Refusing {
        fn compact(
            &mut self,
            _custom_instructions: Option<&str>,
        ) -> Result<serde_json::Value, String> {
            Err("fake runtime: compact is not implemented in this double".to_string())
        }

        fn prompt(&mut self, _m: &str) -> Result<(), String> {
            Ok(())
        }
        fn steer(&mut self, _m: &str) -> Result<(), String> {
            Ok(())
        }
        fn follow_up(&mut self, _m: &str) -> Result<(), String> {
            Ok(())
        }
        fn abort(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn set_thinking_level(&mut self, _l: &str) -> Result<(), String> {
            Ok(())
        }
        fn set_model(&mut self, _p: &str, _m: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }
        fn tree(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"tree": [], "leafId": null}))
        }
        fn clear_queue(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
        fn switch_session(&self, _session_path: &str) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
        fn new_session(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
        fn messages(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"messages": []}))
        }
        fn last_assistant_text(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({}))
        }
        fn entries(&self, _since: Option<&str>) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"entries": [], "leafId": null}))
        }
        fn clone_session(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"cancelled": false}))
        }
        fn fork_messages(&self) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"messages": []}))
        }
        fn fork(&self, _entry_id: &str, _position: &str) -> Result<serde_json::Value, String> {
            // 与 TS 一致的文案〔`agent-session-runtime.ts:275`〕。
            Err("Invalid entry ID for forking".to_string())
        }
        fn state(&self) -> RpcSessionState {
            RpcSessionState {
                model: None,
                is_streaming: false,
                steering_mode: "one-at-a-time".to_string(),
                follow_up_mode: "one-at-a-time".to_string(),
                session_file: None,
                session_id: String::new(),
                message_count: 0,
                pending_message_count: 0,
                thinking_level: "off".to_string(),
                auto_compaction_enabled: true,
            }
        }
    }
    let response = pi_cli::rpc::handle_command(
        &serde_json::json!({"id": "r4", "type": "fork", "entryId": "nope"}),
        &mut Refusing,
    );
    assert_eq!(response["success"], serde_json::json!(false));
    assert_eq!(
        response["error"],
        serde_json::json!("Invalid entry ID for forking")
    );
}

#[test]
fn clone_is_supported_and_returns_cancelled_shape() {
    // `clone` 此前在"已识别但未实现"里；它复用 `fork` 的 `position: "at"`.
    let mut fake = FakeRuntime::default();
    let response =
        pi_cli::rpc::handle_command(&serde_json::json!({"id": "r5", "type": "clone"}), &mut fake);
    assert_eq!(
        response["success"],
        serde_json::json!(true),
        "{response:#?}"
    );
    assert_eq!(response["command"], "clone");
    assert!(
        response["data"]["cancelled"].is_boolean(),
        "响应必须带 cancelled：{response:#?}"
    );
}
