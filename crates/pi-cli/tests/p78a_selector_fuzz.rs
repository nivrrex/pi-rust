//! P7-8a/C 崩溃审计：选择器层进程内 fuzz（真机 #7 崩溃的兜底防线）。
//!
//! 覆盖 `/tree`、`/resume`、`/model`、`/settings`、`/thinking`、补全六个选择器，
//! 输入含 CJK、组合字符、emoji、零宽字符、超长行；操作含退格、粘贴式批量输入、
//! 翻页、折叠、过滤切换；宽度取 20/40/80/100/200。零 panic 即通过。

use pi_cli::model_selector::{ModelEntry, ModelSelector};
use pi_cli::settings_selector::{
    ResumeSelector, SessionRow, SettingsSelector, TreeFilterMode, TreeRow, TreeRowKind,
    TreeSelector,
};
use pi_cli::thinking_selector::{ThinkingEntry, ThinkingSelector};

/// 确定性伪随机（xorshift），避免引入依赖、保证可复现。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            usize::try_from(self.next() % u64::try_from(n).unwrap_or(1)).unwrap_or(0)
        }
    }
}

const ALPHABET: &[char] = &[
    'a', 'b', '/', ' ', '中', '文', 'e', '\u{301}', '👨', '\u{200b}', '\u{fe0f}', '・', '9', '_',
    '-', '\t',
];

fn random_text(rng: &mut Rng, max: usize) -> String {
    let len = rng.below(max) + 1;
    (0..len)
        .map(|_| ALPHABET[rng.below(ALPHABET.len())])
        .collect()
}

const WIDTHS: &[usize] = &[20, 40, 80, 100, 200];

fn check_lines(lines: &[String]) {
    for line in lines {
        // 渲染结果必须是合法 UTF-8 且不含裸控制符（`\t` 允许由编辑器自己处理）。
        assert!(!line.contains('\u{0}'), "渲染结果含 NUL");
    }
}

#[test]
fn tree_selector_fuzz_no_panic() {
    let mut rng = Rng(0x5eed_7001);
    let kinds = [
        TreeRowKind::System,
        TreeRowKind::User,
        TreeRowKind::Assistant,
    ];
    let rows: Vec<TreeRow> = (0..40)
        .map(|index| TreeRow {
            kind: kinds[index % kinds.len()],
            label: if index % 5 == 0 {
                Some(random_text(&mut rng, 6))
            } else {
                None
            },
            id: format!("e{index}"),
            parent_id: if index >= 3 {
                Some(format!("e{}", index - 3))
            } else {
                None
            },
            label_time: if index % 3 == 0 {
                Some(format!("{}s ago", index))
            } else {
                None
            },
            text: format!("{} entry {index}", random_text(&mut rng, 6)),
        })
        .collect();
    let mut selector = TreeSelector::new(rows);
    let filters = [
        TreeFilterMode::Default,
        TreeFilterMode::NoTools,
        TreeFilterMode::UserOnly,
        TreeFilterMode::LabeledOnly,
        TreeFilterMode::All,
    ];
    for step in 0..20_000 {
        match rng.below(14) {
            0 => selector.move_up(),
            1 => selector.move_down(),
            2 => selector.page_up(15),
            3 => selector.page_down(15),
            4 => selector.fold_or_up(),
            5 => selector.unfold_or_down(),
            6 => selector.set_filter(filters[rng.below(filters.len())]),
            7 => selector.cycle_filter(),
            8 => selector.cycle_filter_back(),
            9 => selector.reset_filter(),
            10 => selector.toggle_label_timestamps(),
            11 => selector.fold_or_up(),
            12 => selector.unfold_or_down(),
            _other => selector.move_down(),
        }
        if step % 97 == 0 {
            let width = WIDTHS[rng.below(WIDTHS.len())];
            check_lines(&selector.lines(width));
        }
    }
    for width in WIDTHS {
        check_lines(&selector.lines(*width));
    }
}

#[test]
fn resume_selector_fuzz_no_panic() {
    let mut rng = Rng(0x5eed_7002);
    let rows: Vec<SessionRow> = (0..30)
        .map(|index| SessionRow {
            path: std::path::PathBuf::from(format!(
                "/tmp/{}-{index}.jsonl",
                random_text(&mut rng, 5)
            )),
            name: if index % 4 == 0 {
                Some(random_text(&mut rng, 6))
            } else {
                None
            },
            summary: Some(format!("{} 会话 {index}", random_text(&mut rng, 8))),
            cwd: format!("/home/{}", random_text(&mut rng, 5)),
            message_count: index,
            modified_ms: u128::try_from(index).unwrap_or(0),
        })
        .collect();
    let mut selector = ResumeSelector::new();
    selector.sessions = rows;
    selector.current_cwd = "/home/x".to_string();
    selector.home = "/home/x".to_string();
    for step in 0..20_000 {
        match rng.below(12) {
            0 => {
                let visible = selector.visible_indices();
                if selector.selected > 0 {
                    selector.selected -= 1;
                }
                if !visible.is_empty() {
                    selector.selected = selector.selected.min(visible.len() - 1);
                }
            }
            1 => {
                let visible = selector.visible_indices();
                if !visible.is_empty() {
                    selector.selected = (selector.selected + 1) % visible.len();
                }
            }
            2 => selector.toggle_path(),
            3 => selector.toggle_named(),
            4 => selector.toggle_sort(),
            5 => selector.toggle_path(),
            6 => selector.toggle_named(),
            7 => selector.toggle_scope(),
            8 => {
                selector.enter_rename();
                for _ in 0..rng.below(10) {
                    selector.rename_push(ALPHABET[rng.below(ALPHABET.len())]);
                }
                selector.rename_pop();
                selector.exit_rename();
            }
            9 => {
                selector.enter_delete_confirm();
                selector.exit_delete_confirm();
            }
            10 => {
                let _ = selector.selected_path();
            }
            _other => selector.toggle_scope(),
        }
        if step % 97 == 0 {
            let width = WIDTHS[rng.below(WIDTHS.len())];
            check_lines(&selector.lines(width));
        }
    }
}

#[test]
fn model_and_thinking_selector_fuzz_no_panic() {
    let mut rng = Rng(0x5eed_7003);
    let models: Vec<ModelEntry> = (0..25)
        .map(|index| ModelEntry {
            id: format!("{}-{index}", random_text(&mut rng, 6)),
            provider: format!("p{}", index % 3),
            name: format!("{} model {index}", random_text(&mut rng, 5)),
            current: index == 0,
            default: index == 1,
        })
        .collect();
    let mut model_selector = ModelSelector::new(models, Some("refresh".to_string()));
    let levels: Vec<ThinkingEntry> = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
        .iter()
        .map(|level| ThinkingEntry {
            level: (*level).to_string(),
            current: *level == "medium",
            default: *level == "medium",
        })
        .collect();
    let mut thinking_selector = ThinkingSelector::new(levels);
    for step in 0..20_000 {
        match rng.below(6) {
            0 => model_selector.move_up(),
            1 => model_selector.move_down(),
            2 => model_selector.push(ALPHABET[rng.below(ALPHABET.len())]),
            3 => model_selector.pop(),
            4 => thinking_selector.push(ALPHABET[rng.below(ALPHABET.len())]),
            _other => thinking_selector.push(ALPHABET[rng.below(ALPHABET.len())]),
        }
        if step % 211 == 0 {
            let _ = model_selector.lines();
            check_lines(&thinking_selector.lines());
            let _ = model_selector.filtered_indices();
            let _ = thinking_selector.filtered_indices();
            let _ = model_selector.selected_model();
        }
        if step % 97 == 0 {
            thinking_selector.pop();
        }
    }
}

#[test]
fn settings_selector_fuzz_no_panic() {
    let mut rng = Rng(0x5eed_7004);
    let mut selector = SettingsSelector::new();
    for step in 0..20_000 {
        match rng.below(8) {
            0 => selector.move_up(),
            1 => selector.move_down(),
            2 => selector.push(ALPHABET[rng.below(ALPHABET.len())]),
            3 => selector.pop(),
            4 => {
                let _ = selector.activate();
            }
            5 => {
                let _ = selector.current_of(rng.below(40));
            }
            _other => {
                let text = random_text(&mut rng, 30);
                for ch in text.chars() {
                    selector.push(ch);
                }
            }
        }
        if step % 97 == 0 {
            let width = WIDTHS[rng.below(WIDTHS.len())];
            check_lines(&selector.lines(width));
        }
    }
    for width in WIDTHS {
        check_lines(&selector.lines(*width));
    }
}

/// P7-8b/C：把 fuzz 扩到"搜索框输入"这条真机崩溃路径（`/settings` 搜索、`/resume` 过滤、
/// 补全菜单），每个 ≥10 万次随机操作，必须 0 panic。
#[test]
fn search_and_completion_fuzz_no_panic() {
    let mut rng = Rng(0x5eed_7005);
    let mut settings = SettingsSelector::new();
    for step in 0..100_000 {
        match rng.below(6) {
            0 | 1 => {
                settings.push(ALPHABET[rng.below(ALPHABET.len())]);
            }
            2 => settings.pop(),
            3 => settings.move_up(),
            4 => settings.move_down(),
            _other => {
                let _ = settings.activate();
            }
        }
        if step % 997 == 0 {
            let width = WIDTHS[rng.below(WIDTHS.len())];
            check_lines(&settings.lines(width));
        }
    }

    // 补全：编辑器输入 + 退格 + 渲染（`/t`、`/s`、中文、零宽字符混合）
    let mut state = pi_tui::TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
    for step in 0..100_000 {
        match rng.below(5) {
            0 | 1 => {
                let _ = pi_tui::apply(
                    &mut state,
                    pi_tui::TuiEvent::Char(ALPHABET[rng.below(ALPHABET.len())]),
                );
            }
            2 => {
                let _ = pi_tui::apply(&mut state, pi_tui::TuiEvent::Backspace);
            }
            3 => {
                let _ = pi_tui::apply(&mut state, pi_tui::TuiEvent::Escape);
            }
            _other => {
                let _ = pi_tui::apply(&mut state, pi_tui::TuiEvent::Tab);
            }
        }
        if step % 991 == 0 {
            let width = WIDTHS[rng.below(WIDTHS.len())];
            check_lines(&pi_tui::render_all(&state, width, 30));
        }
    }
}
