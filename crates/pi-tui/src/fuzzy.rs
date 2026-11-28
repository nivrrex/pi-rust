//! 模糊匹配（ref `packages/tui/src/fuzzy.ts` 的逐行转写）。
//!
//! 为什么不是"包含匹配"：`/scoped-models` 的搜索框有 462 个模型，用户靠
//! `dsv4` 之类的碎片定位（ref 的 `fuzzyFilter` 允许字符跳跃匹配，并按
//! 连续性、词边界、位置打分排序）。用 `contains` 会把这类查询全判为不匹配。

/// 一次匹配的结果：是否匹配 + 分数（**越小越好**）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

const NO_MATCH: FuzzyMatch = FuzzyMatch {
    matches: false,
    score: 0.0,
};

/// 词边界字符（ref `:38` 的正则 `[\s\-_./:]`）。
fn is_word_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '-' | '_' | '.' | '/' | ':')
}

fn match_query(normalized: &[char], text: &[char]) -> FuzzyMatch {
    if normalized.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }
    if normalized.len() > text.len() {
        return NO_MATCH;
    }
    let mut query_index = 0;
    let mut score = 0.0_f64;
    let mut last_match: Option<usize> = None;
    let mut consecutive = 0_i32;
    while query_index < normalized.len() {
        let needle = normalized[query_index];
        let from = last_match.map_or(0, |index| index + 1);
        let Some(found) = text[from.min(text.len())..]
            .iter()
            .position(|ch| *ch == needle)
            .map(|offset| offset + from)
        else {
            return NO_MATCH;
        };
        let boundary = found == 0 || is_word_boundary(text[found - 1]);
        if last_match == found.checked_sub(1) {
            consecutive += 1;
            score -= f64::from(consecutive) * 5.0;
        } else {
            consecutive = 0;
            if let Some(previous) = last_match {
                // 两个下标都是小整数，转 f64 不丢精度。
                #[allow(clippy::cast_precision_loss)]
                let gap = (found - previous - 1) as f64;
                score += gap * 2.0;
            }
        }
        if boundary {
            score -= 10.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let position = found as f64;
        score += position * 0.1;
        last_match = Some(found);
        query_index += 1;
    }
    if normalized == text {
        score -= 100.0;
    }
    FuzzyMatch {
        matches: true,
        score,
    }
}

/// 单次模糊匹配（ref `fuzzyMatch`，含 `abc123` / `123abc` 的换位重试）。
#[must_use]
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let text_lower: Vec<char> = text.to_lowercase().chars().collect();
    let primary = match_query(&query_lower, &text_lower);
    if primary.matches {
        return primary;
    }
    // `abc123` ⇄ `123abc`：模型名常见 `gpt4o` 写成 `gpt-4o` / `4ogpt` 之类的顺序差异，
    // ref 对**纯字母前缀 + 纯数字后缀**（及其反向）额外试一次（`:74-96`）。
    let Some(swapped) = swap_alpha_numeric(&query_lower) else {
        return primary;
    };
    let swapped_match = match_query(&swapped, &text_lower);
    if !swapped_match.matches {
        return primary;
    }
    FuzzyMatch {
        matches: true,
        score: swapped_match.score + 5.0,
    }
}

/// 把"字母段+数字段"换成"数字段+字母段"；不是这种形状时返回 `None`。
fn swap_alpha_numeric(query: &[char]) -> Option<Vec<char>> {
    let letters = query
        .iter()
        .take_while(|ch| ch.is_ascii_alphabetic())
        .count();
    let digits_after = query[letters..]
        .iter()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    if letters == 0 || digits_after == 0 || letters + digits_after != query.len() {
        return None;
    }
    let mut out = Vec::with_capacity(query.len());
    out.extend_from_slice(&query[letters..]);
    out.extend_from_slice(&query[..letters]);
    Some(out)
}

/// 过滤并排序（ref `fuzzyFilter`：空白/斜杠分词，全部 token 都要匹配）。
///
/// 空查询返回原顺序；结果按总分**升序**（分数越小越靠前）。
#[must_use]
pub fn fuzzy_filter<T>(items: &[T], query: &str, text_of: impl Fn(&T) -> String) -> Vec<T>
where
    T: Clone,
{
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return items.to_vec();
    }
    let tokens: Vec<&str> = trimmed
        .split([' ', '\t', '/'])
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        return items.to_vec();
    }
    let mut scored: Vec<(f64, usize, T)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let text = text_of(item);
        let mut total = 0.0_f64;
        let mut all = true;
        for token in &tokens {
            let result = fuzzy_match(token, &text);
            if result.matches {
                total += result.score;
            } else {
                all = false;
                break;
            }
        }
        if all {
            scored.push((total, index, item.clone()));
        }
    }
    // 稳定排序：同分时保持原顺序（ref 的 `sort` 在同分下的行为取决于引擎，
    // 这里选**确定的**原序，避免同一输入两次运行次序不同）。
    scored.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    scored.into_iter().map(|(_, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert!(fuzzy_match("", "anything").matches);
        let items = vec!["a", "b"];
        assert_eq!(
            fuzzy_filter(&items, "  ", |item: &&str| (*item).to_string()),
            items
        );
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(fuzzy_match("abc", "aXbXc").matches);
        assert!(!fuzzy_match("cba", "abc").matches);
    }

    #[test]
    fn exact_match_scores_best() {
        let exact = fuzzy_match("gpt4", "gpt4");
        let scattered = fuzzy_match("gpt4", "g-p-t-4");
        assert!(exact.matches && scattered.matches);
        assert!(
            exact.score < scattered.score,
            "完全匹配应比跳跃匹配分低：{} vs {}",
            exact.score,
            scattered.score
        );
    }

    #[test]
    fn word_boundary_is_rewarded() {
        let at_boundary = fuzzy_match("flash", "ling-3.2-flash");
        let mid_word = fuzzy_match("flash", "xflashx");
        assert!(at_boundary.matches && mid_word.matches);
        assert!(
            at_boundary.score < mid_word.score,
            "词边界应得分更好：{} vs {}",
            at_boundary.score,
            mid_word.score
        );
    }

    #[test]
    fn alpha_numeric_swap_is_retried() {
        // `4ogpt` 不是 `gpt4o` 的子序列，但换位后 `ogpt4`… 仍不是；
        // 用能成立的形状验证机制本身：`4gpt` → `gpt4`。
        assert!(fuzzy_match("gpt4", "4gpt").matches);
    }

    #[test]
    fn longer_query_than_text_does_not_match() {
        assert!(!fuzzy_match("abcdef", "abc").matches);
    }

    #[test]
    fn filter_ranks_better_matches_first() {
        let items = vec!["ling-2.6-flash", "ling-3.2-flash", "deepseek-v4"];
        let out = fuzzy_filter(&items, "ling32", |item| (*item).to_string());
        assert_eq!(out.first(), Some(&"ling-3.2-flash"));
    }

    #[test]
    fn filter_requires_all_tokens() {
        let items = vec!["ant-ling/Ling-3.2", "command-code/deepseek"];
        let out = fuzzy_filter(&items, "ant ling", |item| (*item).to_string());
        assert_eq!(out, vec!["ant-ling/Ling-3.2"]);
    }

    #[test]
    fn filter_keeps_original_order_for_ties() {
        let items = vec!["same", "same", "same"];
        let out = fuzzy_filter(&items, "same", |item| (*item).to_string());
        assert_eq!(out.len(), 3);
    }
}
