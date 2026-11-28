/// 切块绝不能切断 UTF-8 多字节序列（终端收到半个字符会显示成乱码）。
#[test]
fn chunks_never_split_utf8_sequences() {
    let payload = "中".repeat(3 * 1024 * 1024); // 9 MiB，全 CJK
    let chunks = pi_tui::diff::split_writes(&payload, 1024 * 1024);
    assert!(chunks.len() > 1, "应分块");
    let joined: String = chunks.iter().copied().collect();
    assert_eq!(joined, payload, "拼回必须完全一致");
    for chunk in &chunks {
        assert!(!chunk.is_empty(), "不应出现空块");
        assert!(
            chunk.chars().all(|ch| ch == '中'),
            "块内出现非预期字符（说明切断了多字节序列）"
        );
    }
}
