fn main() {
    let mut state = pi_tui::diff::DiffState::new();
    let lines = vec![" pi v0.87.1".to_string(), " second".to_string()];
    let styles = vec![None, None];
    let spans = vec![
        vec![
            pi_tui::style::Span::new(
                1,
                3,
                pi_tui::style::LineStyle::fg(pi_tui::style::ACCENT).bold(),
            ),
            pi_tui::style::Span::new(3, 12, pi_tui::style::LineStyle::fg(pi_tui::style::DIM)),
        ],
        Vec::new(),
    ];
    let (out, _cursor) =
        pi_tui::diff::diff_render_with_cursor_styled(&mut state, &lines, &styles, &spans, 40, 10)
            .expect("render");
    println!("{out}");
}
