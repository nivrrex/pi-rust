//! F49/Y1：首帧渲染成本必须**随行数线性**，不得出现超线性的膨胀。
//!
//! 背景（真实 95 MB 会话实测）：`PI_TUI_TIMING` 给出
//! `lines=506559 bytes=50001977 build=13533ms diff=6409ms write=59004ms`。
//! 本用例把"行数翻 4 倍 ⇒ 耗时不超过 4 倍的宽松上界（含缓存预热与分配器波动）"钉住，
//! 用于防止将来引入 O(n²)（例如每行都重新扫描全部历史）。
//!
//! 判据**不看墙钟绝对值**（受机器负载影响），只看 4 倍规模下的**比值上限**，
//! 且比值阈值取 8（远大于线性，仍能抓住二次增长：真正的 O(n²) 会给 16 倍）。
use pi_tui::diff::{diff_render, DiffState};
use std::time::Instant;

fn frame(rows: usize) -> Vec<String> {
    (0..rows)
        .map(|i| format!("row {i} {}", "z".repeat(80)))
        .collect()
}

fn render_once(rows: usize) -> (u128, usize) {
    let lines = frame(rows);
    let mut state = DiffState::new();
    let started = Instant::now();
    let out = diff_render(&mut state, &lines, 100, 40).expect("渲染应成功");
    (started.elapsed().as_micros(), out.len())
}

#[test]
fn first_frame_cost_is_linear_in_lines() {
    // 预热一次，避免首次分配的噪声落在小规模那侧。
    let _ = render_once(2_000);
    let (small_us, small_bytes) = render_once(4_000);
    let (large_us, large_bytes) = render_once(16_000);
    assert!(small_bytes > 0 && large_bytes > 0, "两帧都应有输出");
    let byte_ratio = large_bytes as f64 / small_bytes as f64;
    assert!(
        byte_ratio < 6.0,
        "行数 4 倍时输出字节应约 4 倍（允许轻微非线性），实际 {byte_ratio:.2} 倍"
    );
    // 耗时至少留 1 µs 下限，避免小规模被计时精度吞掉。
    let small = small_us.max(1) as f64;
    let time_ratio = large_us as f64 / small;
    assert!(
        time_ratio < 8.0,
        "行数 4 倍时耗时上限 8 倍（线性≈4、二次≈16），实际 {time_ratio:.2} 倍\
         （small={small_us}µs large={large_us}µs）"
    );
}
