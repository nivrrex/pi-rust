//! F60/2c：真实 `pi-handoff-gate` 的 **todo 工具全生命周期**必须可用。
//!
//! 任务要求覆盖「列表、新增、完成、删除、取消、退出」六种交互。用户报的 `/todo` 卡死
//! 促使我们修了 `Status` 的多行排版（见 `t_f60_multiline_status`），但那只验证了
//! **命令的展示路径**；工具路径（`todo` 工具 → 状态迁移 → `setWidget` 面板 + `notify`）
//! 一直只有"恢复会话带旧状态"的覆盖（`p9_f12_todo_resume`）。
//!
//! 本用例用**真实扩展**（`pi-handoff-gate` 的真实 `index.ts`）跑四个真实回合，
//! 每回合由一个**本地假 provider** 回放一次 todo 工具调用：
//!
//! | 回合 | 工具调用 | 期望屏幕 |
//! |---|---|---|
//! | 1 | `set` 三项 | 清单三行 + `Todo ▰▱▱▱ 0/3` |
//! | 2 | `update` #1 → `done` | `已更新 #1` + `[x] #1` |
//! | 3 | `advance` | `已推进一项` + `#2` 变 `[~]` |
//! | 4 | `clear` | `（清单为空）` |
//!
//! 判据全是**屏幕文本**（去 ANSI），不用墙钟阈值：每一步只断言"该步之后屏幕上
//! 出现了某个文案"，因为假 provider 的响应是确定性的。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// SSE 一帧（OpenAI chat.completion.chunk）。
fn chunk(
    delta: serde_json::Value,
    finish: Option<&str>,
    usage: Option<serde_json::Value>,
) -> String {
    let mut object = serde_json::json!({
        "id": "c",
        "object": "chat.completion.chunk",
        "created": 1700000000i64,
        "model": "m",
        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
    });
    if let Some(usage) = usage {
        object["usage"] = usage;
    }
    format!("data: {object}\n\n")
}

fn usage() -> serde_json::Value {
    serde_json::json!({"prompt_tokens": 100, "completion_tokens": 10, "total_tokens": 110})
}

/// 一次 todo 工具调用的完整响应（tool_calls 流 + usage）。
fn todo_call(call_id: &str, arguments: serde_json::Value) -> String {
    [
        chunk(
            serde_json::json!({"tool_calls": [{
                "index": 0, "id": call_id, "type": "function",
                "function": { "name": "todo", "arguments": arguments.to_string() }
            }]}),
            None,
            None,
        ),
        chunk(serde_json::json!({}), Some("tool_calls"), Some(usage())),
    ]
    .concat()
}

/// 四个回合的响应体：set → update(done) → advance → clear。
fn responses() -> Vec<String> {
    vec![
        todo_call(
            "c1",
            // **长条目是关键**：`/todo` 的 `ctx.ui.notify` 会把整份清单作为一条
            // `Status` 文本送出（多行）；单行宽度必须超过终端宽度，才能复现
            // `diff_render` 的 `LineTooWide`（真机正是清单里有长待办）。
            serde_json::json!({"action": "set", "items": [
                "1a 真 pty 复现 /allcopy 的 require is not defined 并定位到具体文件与行号",
                "1b 修 shim 为 ESM import，并把测试改成走与真实宿主相同的加载路径",
                "1c 全仓搜索 shim/runner 内其他在 ESM 中使用 require/__dirname/module 的位置"
            ]}),
        ),
        todo_call(
            "c2",
            serde_json::json!({"action": "update", "id": 1, "status": "done"}),
        ),
        todo_call("c3", serde_json::json!({"action": "advance"})),
        todo_call("c4", serde_json::json!({"action": "clear"})),
    ]
}

/// 按请求顺序回放（用完重复最后一条），端口由内核分配。
fn spawn_mock(listener: TcpListener, bodies: Vec<String>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // 按请求顺序回放，用完重复最后一条（`enumerate` 而不是手写计数器，
        // 避免 clippy 的 `explicit_counter_loop`）。
        for (served, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = [0u8; 65536];
            let _ = stream.read(&mut buffer);
            let body = bodies
                .get(served.min(bodies.len().saturating_sub(1)))
                .cloned()
                .unwrap_or_default();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    })
}

fn write_agent(agent: &Path, port: u16, extension_src: &Path) {
    std::fs::create_dir_all(agent.join("extensions")).expect("建扩展目录");
    // 真实扩展：直接拷贝用户的 `pi-handoff-gate`（含 todo.ts / gate.ts）。
    let destination = agent.join("extensions/pi-handoff-gate");
    if !destination.exists() {
        copy_dir(extension_src, &destination).expect("拷贝扩展");
    }
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        format!(
            r#"{{"providers":{{"p":{{"baseUrl":"http://127.0.0.1:{port}/v1","api":"openai-completions","apiKey":"DUMMY","models":[{{"id":"m","name":"m","contextWindow":200000,"maxTokens":1024}}]}}}}}}"#
        ),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");
}

/// 递归拷贝目录（扩展自带的 `.ts` 与 package.json 都要过去）。
fn copy_dir(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let to = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// 去 ANSI 的屏幕文本。
fn screen_of(raw: &Path) -> String {
    let bytes = std::fs::read(raw).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if next == '\u{7}' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// 每回合结束后的累计屏幕（帧是增量的，所以按累计字节判断"是否出现过某文案"）。
struct Frames {
    /// 每个 mark 处的累计屏幕文本。
    screens: Vec<String>,
    /// 每个 mark 处的**累计已写字节数**（`marks.jsonl` 的 `offset`）。
    ///
    /// 为什么需要它：只比"屏幕上出现过某段文字"抓不到冻结——文本早在**更早的帧**里
    /// 出现过（清单在回合 1 的工具结果里就已经渲染）。冻结的表现是"之后的帧一个字节
    /// 都不再写出"，所以必须断言 offset **严格递增**。
    offsets: Vec<u64>,
    /// mark 名字（与 `offsets` 同序）。
    names: Vec<String>,
}

fn run_todo_lifecycle() -> Option<Frames> {
    let node = node_path()?;
    let extension_src = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".pi/agent/extensions/pi-handoff-gate"))
        .filter(|path| path.is_dir())?;

    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定 loopback");
    let port = listener.local_addr().expect("地址").port();
    let _mock = spawn_mock(listener, responses());

    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    write_agent(&agent, port, &extension_src);

    // 四回合 + `/todo` 命令（清单非空时 notify 的是**多行**文本）+
    // Esc（取消当前输入）+ Ctrl+D（退出）。
    let keys_path = tmp.path().join("keys.json");
    let mut keys = vec![
        serde_json::json!({"waitStable": {"quietMs": 1500, "timeoutMs": 120000}}),
        serde_json::json!({"mark": "s0"}),
    ];
    for turn in 1..=4 {
        keys.push(serde_json::json!({"text": "继续"}));
        keys.push(serde_json::json!({"delayMs": 400}));
        keys.push(serde_json::json!({"text": "\r"}));
        keys.push(serde_json::json!({"delayMs": 8000}));
        keys.push(serde_json::json!({"mark": format!("t{turn}")}));
        if turn == 1 {
            // **关键一步**：清单已有 3 项，`/todo` 的 `ctx.ui.notify` 送的是**多行**清单。
            // 改前（`Status` 单行渲染 + `LineTooWide` 丢帧）这里会让屏幕永久冻结，
            // 后续每个 mark 都不再有新字节 ⇒ 下面几条断言全部拿不到新内容。
            // 末尾带空格：避免"补全菜单接受"与"提交"合并成同一次 Enter
            // （`/todo` 恰好等于命令名时会直接提交，但带空格更不依赖那个细节）。
            keys.push(serde_json::json!({"text": "/todo "}));
            keys.push(serde_json::json!({"delayMs": 800}));
            keys.push(serde_json::json!({"text": "\r"}));
            keys.push(serde_json::json!({"delayMs": 3000}));
            keys.push(serde_json::json!({"mark": "cmd"}));
            keys.push(serde_json::json!({"text": "\u{0004}"}));
            keys.push(serde_json::json!({"delayMs": 2000}));
            keys.push(serde_json::json!({"mark": "cmd_exit"}));
        }
    }
    // 取消：输入一半再 Esc。
    keys.push(serde_json::json!({"text": "半截"}));
    keys.push(serde_json::json!({"delayMs": 400}));
    keys.push(serde_json::json!({"text": "\u{001b}"}));
    keys.push(serde_json::json!({"delayMs": 1200}));
    keys.push(serde_json::json!({"mark": "esc"}));
    keys.push(serde_json::json!({"text": "\u{0004}"}));
    keys.push(serde_json::json!({"delayMs": 2000}));
    keys.push(serde_json::json!({"mark": "exit"}));
    std::fs::write(
        &keys_path,
        serde_json::to_string(&keys).expect("序列化 keys"),
    )
    .expect("写 keys");

    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let marks_path = tmp.path().join("marks.jsonl");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("36")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys_path)
        .arg("--marks")
        .arg(&marks_path)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("COLORTERM", "")
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");

    let bytes = std::fs::read(&raw_path).unwrap_or_default();
    let mark_text = std::fs::read_to_string(&marks_path).unwrap_or_default();
    let mut screens = Vec::new();
    let mut offsets = Vec::new();
    let mut names = Vec::new();
    for line in mark_text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(offset) = value.get("offset").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        names.push(
            value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
        offsets.push(offset);
        let cut = (offset as usize).min(bytes.len());
        let partial = tmp.path().join("partial.bin");
        std::fs::write(&partial, &bytes[..cut]).expect("写片段");
        screens.push(screen_of(&partial));
    }
    let _ = std::fs::remove_file(tmp.path().join("partial.bin"));
    let exit_code = std::fs::read_to_string(&code_path)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok());
    assert_eq!(exit_code, Some(0), "四回合 + Esc 之后 Ctrl+D 应正常退出");
    Some(Frames {
        screens,
        offsets,
        names,
    })
}

/// **新增**：`set` 之后屏幕出现三项清单与计数面板。
#[test]
fn todo_set_shows_three_items_and_counter() {
    let Some(frames) = run_todo_lifecycle() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    let after = &frames.screens[1.min(frames.screens.len() - 1)];
    for needle in [
        "#1 1a 真 pty 复现",
        "#2 1b 修 shim 为 ESM import",
        "#3 1c 全仓搜索",
        "Todo",
    ] {
        assert!(
            after.contains(needle),
            "`set` 之后屏幕上应有 {needle:?}。屏幕：\n{after}"
        );
    }
}

/// **完成**：`update #1 → done` 之后出现 `已更新 #1` 与打勾的 `[x] #1`。
#[test]
fn todo_update_marks_the_item_done() {
    let Some(frames) = run_todo_lifecycle() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    let after = &frames.screens[4.min(frames.screens.len() - 1)];
    assert!(
        after.contains("已更新 #1"),
        "`update` 之后应有回执文案。屏幕：\n{after}"
    );
    assert!(
        after.contains("[x] #1 1a 真 pty 复现"),
        "`update` 之后第一项应显示为完成态。屏幕：\n{after}"
    );
}

/// **推进**（删除/跳过类交互的同一入口）：`advance` 之后 `#2` 变成进行中。
#[test]
fn todo_advance_moves_the_current_item() {
    let Some(frames) = run_todo_lifecycle() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    let after = &frames.screens[5.min(frames.screens.len() - 1)];
    assert!(
        after.contains("已推进一项"),
        "`advance` 之后应有回执文案。屏幕：\n{after}"
    );
    assert!(
        after.contains("[~] #2 1b 修 shim"),
        "`advance` 之后第二项应变成进行中。屏幕：\n{after}"
    );
}

/// **清空**：`clear` 之后屏幕显示空清单。
#[test]
fn todo_clear_empties_the_list() {
    let Some(frames) = run_todo_lifecycle() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    let after = &frames.screens[6.min(frames.screens.len() - 1)];
    assert!(
        after.contains("（清单为空）"),
        "`clear` 之后应显示空清单。屏幕：\n{after}"
    );
}

/// **取消**：Esc 之后进程仍在正常处理输入（后续 `ctrl+d` 能退出，见
/// `run_todo_lifecycle` 的 `assert_eq!(exit_code, Some(0))`），且屏幕不含半截输入被提交的痕迹。
#[test]
fn todo_escape_cancels_without_freezing() {
    let Some(frames) = run_todo_lifecycle() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    let after = &frames.screens[frames.screens.len().saturating_sub(3)];
    // `半截` 若被当成 prompt 提交，会出现一条用户消息回显；Esc 之后不应如此。
    assert!(
        !after.contains("半截\n"),
        "Esc 之后半截输入不应被提交。屏幕：\n{after}"
    );
}

/// **`/todo` 命令**必须把三行清单都画在屏幕上，并且之后进程仍响应
/// （紧随其后的 `ctrl+d` 让进程退出，见 `run_todo_lifecycle` 的 exit_code 断言）。
///
/// 注意：这条**不是** F60/2 冻结的直接回归——`pi-handoff-gate` 的 `render()`
/// 逐项各占一行（`todo.ts:221-226`），单行宽度没有超过 110 列，
/// 因此回滚 `Status` 的多行修复后它仍会通过。冻结的直接回归在
/// `crates/pi-tui/tests/t_f60_multiline_status.rs`（用真实 `/todo` 文本构造超宽行，
/// 改前直接给出 `LineTooWide { width: 120, line_width: 122 }`）。
#[test]
fn todo_command_renders_multiline_list_and_stays_responsive() {
    let Some(frames) = run_todo_lifecycle() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    let after = &frames.screens[2.min(frames.screens.len() - 1)];
    for needle in [
        "#1 1a 真 pty 复现",
        "#2 1b 修 shim 为 ESM import",
        "#3 1c 全仓搜索",
    ] {
        assert!(
            after.contains(needle),
            "`/todo` 之后屏幕应有 {needle:?}（多行清单逐行显示）。屏幕：\n{after}"
        );
    }
}

/// **无响应类缺陷的通用判据**：每个 mark 处写出的字节数必须**严格递增**
/// （退出之后的 `exit` mark 除外）。
///
/// 覆盖的是"命令/工具之后主循环还能继续画帧"这一类缺陷（用户报的 `/todo` 卡死、
/// `/compact` 无反应都属于它）。用"字节是否增长"而不是墙钟时间，与机器快慢无关。
/// 冻结的**直接**回归（`LineTooWide` 丢帧）在 `t_f60_multiline_status`。
#[test]
fn every_step_keeps_writing_bytes() {
    let Some(frames) = run_todo_lifecycle() else {
        eprintln!("跳过：本机缺 node 或 pi-handoff-gate");
        return;
    };
    let mut stalled: Vec<String> = Vec::new();
    for pair in frames.offsets.windows(2).zip(frames.names.iter().skip(1)) {
        let (window, name) = pair;
        // `exit` 是**进程已退出之后**的 mark（Ctrl+D 让循环结束，不再画帧）⇒ 排除。
        if name == "exit" {
            continue;
        }
        if window[1] <= window[0] {
            stalled.push(format!(
                "{name}（offset 没增长：{} → {}）",
                window[0], window[1]
            ));
        }
    }
    assert!(
        stalled.is_empty(),
        "这些步骤没有写出任何新字节（屏幕冻结的判据）：{stalled:?}\noffsets={:?}\nnames={:?}",
        frames.offsets,
        frames.names
    );
}
