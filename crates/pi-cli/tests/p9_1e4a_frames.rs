//! S4：渲染钩子降级帧的展开（宿主侧宽度裁剪 + 不支持时回落）。

use serde_json::json;

#[test]
fn text_frame_pads_and_clamps() {
    let frame = json!({"kind":"text","text":"hello","paddingX":1,"paddingY":1});
    let lines = pi_cli::ext_tools::flatten_render_frame(&frame, 80).expect("应可展开");
    assert_eq!(
        lines,
        vec!["".to_string(), " hello".to_string(), "".to_string()]
    );

    // 宽度裁剪：80 → 5 列时截断。
    let lines = pi_cli::ext_tools::flatten_render_frame(&frame, 5).expect("应可展开");
    assert!(lines
        .iter()
        .all(|line| pi_tui::clamp_to_width(line, 5) == *line));
}

#[test]
fn stack_and_spacer_frames_concatenate() {
    let frame = json!({
        "kind":"stack",
        "children":[
            {"kind":"text","text":"a","paddingX":0,"paddingY":0},
            {"kind":"spacer","lines":2},
            {"kind":"text","text":"b","paddingX":0,"paddingY":0}
        ]
    });
    let lines = pi_cli::ext_tools::flatten_render_frame(&frame, 80).expect("应可展开");
    assert_eq!(lines, vec!["a", "", "", "b"]);
}

#[test]
fn unsupported_frames_fall_back() {
    let frame = json!({"kind":"unsupported","reason":"Box"});
    assert!(pi_cli::ext_tools::flatten_render_frame(&frame, 80).is_none());
    // 栈里嵌套不支持项 ⇒ 整块回落（不静默丢一半）。
    let nested = json!({
        "kind":"stack",
        "children":[{"kind":"text","text":"a","paddingX":0,"paddingY":0},{"kind":"unsupported","reason":"SelectList"}]
    });
    assert!(pi_cli::ext_tools::flatten_render_frame(&nested, 80).is_none());
}
