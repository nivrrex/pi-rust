//! 助手消息渲染出的**任何一行**都不得超过终端列数。
//!
//! 超宽会让终端自己换行（不经过我们的 diff 状态机），在真机上表现为
//! “行尾几个字符被挤到下一行最左”（Windows 135 列下的列表项溢出）。

use pi_tui::{render_all, TranscriptItem, TuiState};

/// 与真机同构的合成长句：括号、数字、`%`、`/`、英文词、顿号、分号、粗体段、行内代码。
fn synthetic_list_item() -> String {
    "1. 六·附新增第 6 节【11/4 增量】：某地 11 月（11/1-11/3）真实动销破冰 16 笔全部真实零资源包（临泉6+太和3 省内侧、阜南 YDY 客户 7 条按量模型）；真实完成率由原判断 66-68% 下修至约 62.5%（427/428 条资源包不可依赖）；缺口 ≈794、真实口径需日均 22 笔；仅滁州（11 月 143 笔全省第 1，值得要打法）/淮北（155% 标杆）/合肥（52% 最大缺口）三家特别提醒一符合\"只做阜阳\"新口径；下期三项校验点".to_string()
}

#[test]
fn assistant_lines_never_exceed_cols() {
    let mut state = TuiState::new("0.87.1", "/tmp/work", "0.0%/128k (auto)");
    state.quiet_startup = true;
    state
        .transcript
        .push(TranscriptItem::Assistant(synthetic_list_item()));
    for cols in [80usize, 90, 100, 110, 120, 125, 130, 133, 135, 140] {
        for (index, line) in render_all(&state, cols, 40).iter().enumerate() {
            let width = pi_tui::diff::visible_width(line);
            assert!(
                width <= cols,
                "cols={cols} 第 {index} 行宽 {width} 超出：{}",
                line.chars().take(40).collect::<String>()
            );
        }
    }
}
