//! F53d/D：`edit` 工具 diff 的**行内词级比对**（ref `components/diff.ts:33-77` 的
//! `renderIntraLineDiff`）。
//!
//! 语义与 TS 包依赖的 jsdiff `diffWords`（`diff@8.0.4`）对齐：
//! - 词切分（`libcjs/diff/word.js:60-110` 的 `tokenizeIncludingWhitespace` + 空白缝合）：
//!   **词字符**（`[A-Za-z0-9_]` 与若干拉丁扩展区）连续成 token，空白连续成 token，
//!   其余字符各成 token；随后把空白**并入相邻 token**，使每个 token 自带前后空白；
//! - 相等判定是 `trim()` 后比较（`word.js:70-73` 的 `equals`）；
//! - 变更对象后处理（`word.js:183-...` 的 `dedupeWhitespaceInChangeObjects`）：把
//!   删除/插入对象两端与相邻 keep 对象**重复的空白**归并掉。
//!
//! 结果：removed 行由"未被标记为 added 的 token"拼接，added 行由"未被标记为 removed 的
//! token"拼接；首个变更对象的前导空白原样保留（`diff.ts:50-64`）。
//!
//! 复杂度：token 数 n、m ⇒ O(n·m) 时间、O(min(n,m)) 空间（滚动数组），
//! 单行 token 数上限 `MAX_TOKENS = 400`（超限退化为"整行替换"，与 ref 的
//! `maxEditLength` 保守保护同旨；ref 未设上限，此处为**登记偏离**）。

/// 单行 token 上限。超限时退化为整行替换（不做行内高亮）。
const MAX_TOKENS: usize = 400;

/// 词字符判定（jsdiff `word.js:60-63` 的 `extendedWordChars`）。
fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || ch == '_'
        || matches!(ch,
            '\u{AD}' | '\u{C0}'..='\u{166}' | '\u{168}'..='\u{16F}'
            | '\u{170}'..='\u{1FF}' | '\u{250}'..='\u{2C6}' | '\u{2C8}'..='\u{2D7}'
            | '\u{2DE}'..='\u{2FF}' | '\u{1E00}'..='\u{1EFF}')
}

/// jsdiff 的词切分 + 空白缝合（`word.js:76-120`）。
fn tokenize(text: &str) -> Vec<String> {
    // 1) 原始切分：词字符连续段 / 空白连续段 / 其余单字符。
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut mode = 0u8; // 1 = word, 2 = space, 3 = other
    for ch in text.chars() {
        let this = if is_word_char(ch) {
            1
        } else if ch.is_whitespace() {
            2
        } else {
            3
        };
        if this == 3 {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            parts.push(ch.to_string());
            mode = 0;
            continue;
        }
        if mode != this {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            mode = this;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }

    // 2) 空白缝合，照 jsdiff `word.js:96-119` 逐分支：
    //    - 空白 part：若前一个 part 为空 ⇒ 直接 push；否则并入**上一个 token 的尾部**；
    //    - 非空白 part 且前一个 part 是空白：若"上一个 token 恰好等于那段空白" ⇒ 并入它，
    //      否则 push(`空白 + part`)（**这段空白会同时留在前一个 token 尾部**，
    //      jsdiff 就是这么做的，`a  b` 得到 `["a  ", "  b"]`）。
    let mut tokens: Vec<String> = Vec::new();
    let mut prev_part: Option<String> = None;
    for part in parts {
        let is_space = part.chars().all(char::is_whitespace);
        if is_space {
            match prev_part.as_deref() {
                None => tokens.push(part.clone()),
                Some(_) => {
                    if let Some(token) = tokens.pop() {
                        tokens.push(format!("{token}{part}"));
                    } else {
                        tokens.push(part.clone());
                    }
                }
            }
        } else if prev_part
            .as_deref()
            .is_some_and(|prev| prev.chars().all(char::is_whitespace))
        {
            let prev = prev_part.clone().unwrap_or_default();
            if tokens.last().map(String::as_str) == Some(prev.as_str()) {
                let last = tokens.pop().unwrap_or_default();
                tokens.push(format!("{last}{part}"));
            } else {
                tokens.push(format!("{prev}{part}"));
            }
        } else {
            tokens.push(part.clone());
        }
        prev_part = Some(part);
    }
    tokens
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Keep,
    Remove,
    Add,
}

/// LCS 求最小编辑序列；`equals` 按 jsdiff 的 `trim()` 语义。
fn diff_tokens(old: &[String], new: &[String]) -> Vec<Op> {
    // 公共前后缀先剥掉（jsdiff `base.js` 的 extractCommon 同效，且让 O(n·m) 表更小）。
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix].trim() == new[prefix].trim() {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix].trim() == new[new.len() - 1 - suffix].trim()
    {
        suffix += 1;
    }
    let a = &old[prefix..old.len() - suffix];
    let b = &new[prefix..new.len() - suffix];
    let (n, m) = (a.len(), b.len());

    // dp[i][j] = a[i..] 与 b[j..] 的 LCS 长度。
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i].trim() == b[j].trim() {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut ops = vec![Op::Keep; prefix];
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i].trim() == b[j].trim() {
            ops.push(Op::Keep);
            i += 1;
            j += 1;
        } else if dp[i + 1][j] > dp[i][j + 1] {
            ops.push(Op::Remove);
            i += 1;
        } else {
            ops.push(Op::Add);
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Remove);
        i += 1;
    }
    while j < m {
        ops.push(Op::Add);
        j += 1;
    }
    ops.extend(std::iter::repeat_n(Op::Keep, suffix));
    ops
}

fn leading_ws(text: &str) -> &str {
    let end = text.len() - text.trim_start().len();
    &text[..end]
}

fn trailing_ws(text: &str) -> &str {
    let start = text.trim_end().len();
    &text[start..]
}

/// 把 `values` 里位于 `at` 的那一段**首部**空白替换成 `to`（jsdiff `replacePrefix`）。
fn replace_prefix(value: &mut String, from: &str, to: &str) {
    if !from.is_empty() && value.starts_with(from) {
        value.replace_range(..from.len(), to);
    } else if from.is_empty() {
        value.insert_str(0, to);
    }
}

/// 把 `values` 里位于 `at` 的那一段**尾部**空白替换成 `to`（jsdiff `replaceSuffix`）。
fn replace_suffix(value: &mut String, from: &str, to: &str) {
    if !from.is_empty() && value.ends_with(from) {
        let start = value.len() - from.len();
        value.replace_range(start.., to);
    } else if from.is_empty() {
        value.push_str(to);
    }
}

fn common_prefix<'a>(left: &'a str, right: &str) -> &'a str {
    let end = left
        .char_indices()
        .zip(right.chars())
        .take_while(|((_, a), b)| a == b)
        .map(|((index, ch), _)| index + ch.len_utf8())
        .last()
        .unwrap_or(0);
    &left[..end]
}

fn common_suffix<'a>(left: &'a str, right: &str) -> &'a str {
    let mut start = left.len();
    let mut right_chars = right.chars().rev();
    for (index, ch) in left.char_indices().rev() {
        match right_chars.next() {
            Some(other) if other == ch => start = index,
            Some(_) | None => break,
        }
    }
    &left[start..]
}

/// jsdiff `maximumOverlap(a, b)`：`b` 的前缀中与 `a` 的**后缀**重合的最长部分。
fn maximum_overlap<'a>(left: &str, right: &'a str) -> &'a str {
    let mut best = 0usize;
    for end in right
        .char_indices()
        .map(|(index, ch)| index + ch.len_utf8())
    {
        if left.ends_with(&right[..end]) {
            best = end;
        }
    }
    &right[..best]
}

/// jsdiff `removePrefix`：`value` 以 `prefix` 开头时去掉它。
fn remove_prefix(value: &mut String, prefix: &str) {
    if !prefix.is_empty() && value.starts_with(prefix) {
        value.replace_range(..prefix.len(), "");
    }
}

/// jsdiff `removeSuffix`：`value` 以 `suffix` 结尾时去掉它。
fn remove_suffix(value: &mut String, suffix: &str) {
    if !suffix.is_empty() && value.ends_with(suffix) {
        let start = value.len() - suffix.len();
        value.replace_range(start.., "");
    }
}

/// 仅测试用：暴露词切分结果，便于与 oracle 对齐。
#[doc(hidden)]
#[must_use]
pub fn debug_tokens(text: &str) -> Vec<String> {
    tokenize(text)
}

/// 行内词级比对：返回 `(removed 行, added 行)`，语义同 `renderIntraLineDiff`。
#[must_use]
pub fn render_intra_line_diff(old: &str, new: &str) -> (String, String) {
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    if old_tokens.len() > MAX_TOKENS || new_tokens.len() > MAX_TOKENS {
        // 退化：整行替换（不做行内高亮）。
        return (old.to_string(), new.to_string());
    }
    let ops = diff_tokens(&old_tokens, &new_tokens);

    // 组装变更对象序列。jsdiff 的 `buildValues` 用 `join()`（`word.js:121-131`）
    // 把**同一分量内的 token** 拼接起来，规则是：除第一个 token 外，其余 token 去掉
    // 自己的前导空白（空白已并进各 token，去掉才不会重复）。
    fn join_tokens(tokens: &[String]) -> String {
        let mut out = String::new();
        for (index, token) in tokens.iter().enumerate() {
            if index == 0 {
                out.push_str(token);
            } else {
                out.push_str(token.trim_start());
            }
        }
        out
    }
    #[derive(Clone)]
    struct Change {
        op: Op,
        value: String,
    }
    let mut changes: Vec<Change> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut run_op = Op::Keep;
    let mut run: Vec<String> = Vec::new();
    let flush = |op: Op, run: &mut Vec<String>, changes: &mut Vec<Change>| {
        if run.is_empty() {
            return;
        }
        changes.push(Change {
            op,
            value: join_tokens(run),
        });
        run.clear();
    };
    for op in ops {
        if op != run_op {
            flush(run_op, &mut run, &mut changes);
            run_op = op;
        }
        match op {
            Op::Keep => {
                run.push(new_tokens[j].clone());
                i += 1;
                j += 1;
            }
            Op::Remove => {
                run.push(old_tokens[i].clone());
                i += 1;
            }
            Op::Add => {
                run.push(new_tokens[j].clone());
                j += 1;
            }
        }
    }
    flush(run_op, &mut run, &mut changes);
    let _ = (i, j);

    // dedupeWhitespaceInChangeObjects（`word.js:183-295`）——按 jsdiff 的**五个分支**
    // 逐个照抄（deletion+insertion 两端 keep / insertion-only / deletion-only × 三种 keep 布局）。
    {
        let mut index = 0usize;
        let mut last_keep: Option<usize> = None;
        while index <= changes.len() {
            let mut deletion: Option<usize> = None;
            let mut insertion: Option<usize> = None;
            while index < changes.len() && changes[index].op != Op::Keep {
                match changes[index].op {
                    Op::Remove => deletion = Some(index),
                    Op::Add => insertion = Some(index),
                    Op::Keep => {}
                }
                index += 1;
            }
            let end_keep = if index < changes.len() {
                Some(index)
            } else {
                None
            };
            match (deletion, insertion) {
                (Some(d), Some(ins)) => {
                    let old_prefix = leading_ws(&changes[d].value).to_string();
                    let old_suffix = trailing_ws(&changes[d].value).to_string();
                    let new_prefix = leading_ws(&changes[ins].value).to_string();
                    let new_suffix = trailing_ws(&changes[ins].value).to_string();
                    if let Some(keep) = last_keep {
                        let shared = common_prefix(&old_prefix, &new_prefix).to_string();
                        replace_suffix(&mut changes[keep].value, &new_prefix, &shared);
                        remove_prefix(&mut changes[d].value, &shared);
                        remove_prefix(&mut changes[ins].value, &shared);
                    }
                    if let Some(keep) = end_keep {
                        let shared = common_suffix(&old_suffix, &new_suffix).to_string();
                        replace_prefix(&mut changes[keep].value, &new_suffix, &shared);
                        remove_suffix(&mut changes[d].value, &shared);
                        remove_suffix(&mut changes[ins].value, &shared);
                    }
                }
                (None, Some(ins)) => {
                    // insertion-only：每个变更对象保留**尾随**空白，重复的前导空白删掉。
                    if last_keep.is_some() {
                        let ws = leading_ws(&changes[ins].value).to_string();
                        remove_prefix(&mut changes[ins].value, &ws);
                    }
                    if let Some(keep) = end_keep {
                        let ws = leading_ws(&changes[keep].value).to_string();
                        remove_prefix(&mut changes[keep].value, &ws);
                    }
                }
                (Some(d), None) => match (last_keep, end_keep) {
                    (Some(start), Some(end)) => {
                        let new_ws_full = leading_ws(&changes[end].value).to_string();
                        let del_ws_start = leading_ws(&changes[d].value).to_string();
                        let del_ws_end = trailing_ws(&changes[d].value).to_string();
                        let new_ws_start = common_prefix(&new_ws_full, &del_ws_start).to_string();
                        remove_prefix(&mut changes[d].value, &new_ws_start);
                        let rest = &new_ws_full[new_ws_start.len().min(new_ws_full.len())..];
                        let new_ws_end = common_suffix(rest, &del_ws_end).to_string();
                        remove_suffix(&mut changes[d].value, &new_ws_end);
                        replace_prefix(&mut changes[end].value, &new_ws_full, &new_ws_end);
                        let take = new_ws_full.len() - new_ws_end.len();
                        replace_suffix(
                            &mut changes[start].value,
                            &new_ws_full,
                            &new_ws_full[..take.min(new_ws_full.len())],
                        );
                    }
                    (None, Some(end)) => {
                        let end_prefix = leading_ws(&changes[end].value).to_string();
                        let del_suffix = trailing_ws(&changes[d].value).to_string();
                        let overlap = maximum_overlap(&del_suffix, &end_prefix).to_string();
                        remove_suffix(&mut changes[d].value, &overlap);
                    }
                    (Some(start), None) => {
                        let start_suffix = trailing_ws(&changes[start].value).to_string();
                        let del_prefix = leading_ws(&changes[d].value).to_string();
                        let overlap = maximum_overlap(&start_suffix, &del_prefix).to_string();
                        remove_prefix(&mut changes[d].value, &overlap);
                    }
                    (None, None) => {}
                },
                (None, None) => {}
            }
            if end_keep.is_none() {
                break;
            }
            last_keep = end_keep;
            index += 1;
        }
    }

    // 拼 removed / added（`diff.ts:50-73`）。
    let mut removed = String::new();
    let mut added = String::new();
    let mut first_removed = true;
    let mut first_added = true;
    for change in &changes {
        match change.op {
            Op::Remove => {
                let mut value = change.value.as_str();
                if first_removed {
                    let ws = leading_ws(value);
                    removed.push_str(ws);
                    value = &value[ws.len()..];
                    first_removed = false;
                }
                removed.push_str(value);
            }
            Op::Add => {
                let mut value = change.value.as_str();
                if first_added {
                    let ws = leading_ws(value);
                    added.push_str(ws);
                    value = &value[ws.len()..];
                    first_added = false;
                }
                added.push_str(value);
            }
            Op::Keep => {
                removed.push_str(&change.value);
                added.push_str(&change.value);
            }
        }
    }
    (removed, added)
}

/// 解析一行 diff（ref `components/diff.ts:11-20` 的 `parseDiffLine`）：
/// `^([+-\s])(\s*\d*)\s(.*)$` ⇒ `(prefix, lineNum, content)`。
fn parse_diff_line(line: &str) -> Option<(char, &str, &str)> {
    let mut chars = line.char_indices();
    let (_, prefix) = chars.next()?;
    if !matches!(prefix, '+' | '-' | ' ') && !prefix.is_whitespace() {
        return None;
    }
    let rest = &line[prefix.len_utf8()..];
    // `\s*\d*` 段：跳过空白与数字，直到第一个"非空白非数字"字符为止。
    // `(\s*\d*)\s(.*)` 的贪婪语义：先吃空白、再吃数字，然后**一个**空白作分隔。
    // 注意 `-159 | …` 里数字后紧跟空格 ⇒ `lineNum` 是 `"159"`（不含空格）。
    let mut index = 0usize;
    for (offset, ch) in rest.char_indices() {
        if !ch.is_whitespace() {
            index = offset;
            break;
        }
        index = offset + ch.len_utf8();
    }
    let num_start = index;
    for (offset, ch) in rest[num_start..].char_indices() {
        if ch.is_ascii_digit() {
            index = num_start + offset + ch.len_utf8();
        } else {
            break;
        }
    }
    let line_num = &rest[num_start..index];
    let sep = rest[index..].chars().next()?;
    if !sep.is_whitespace() {
        return None;
    }
    Some((prefix, line_num, &rest[index + sep.len_utf8()..]))
}

/// 把 `details.diff` 渲染成行文本（ref `components/diff.ts:79-160` 的 `renderDiff`，
/// 不含颜色：颜色由调用方的整行样式负责）。
///
/// 规则：
/// - 连续的 `-` 行之后紧跟**恰好 1 行** `+`、且 `-` 也**恰好 1 行**时，
///   这两行走行内词级比对（`renderIntraLineDiff`）；
/// - 否则删除行先全列、再列新增行（`:122-132`）；
/// - 独立的 `+` 行、上下文行原样输出（`:134-145`）。
#[must_use]
pub fn render_diff_lines(diff: &str) -> Vec<String> {
    let lines: Vec<&str> = diff.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let Some((prefix, _, _)) = parse_diff_line(lines[index]) else {
            out.push(lines[index].replace('\t', "   "));
            index += 1;
            continue;
        };
        if prefix == '-' {
            let mut removed: Vec<(String, String)> = Vec::new();
            while index < lines.len() {
                match parse_diff_line(lines[index]) {
                    Some(('-', num, content)) => {
                        removed.push((num.to_string(), content.replace('\t', "   ")));
                        index += 1;
                    }
                    Some(('+' | ' ', _, _)) | Some((_, _, _)) | None => break,
                }
            }
            let mut added: Vec<(String, String)> = Vec::new();
            while index < lines.len() {
                match parse_diff_line(lines[index]) {
                    Some(('+', num, content)) => {
                        added.push((num.to_string(), content.replace('\t', "   ")));
                        index += 1;
                    }
                    Some(('-' | ' ', _, _)) | Some((_, _, _)) | None => break,
                }
            }
            if removed.len() == 1 && added.len() == 1 {
                let (removed_num, removed_content) = &removed[0];
                let (added_num, added_content) = &added[0];
                let (removed_line, added_line) =
                    render_intra_line_diff(removed_content, added_content);
                out.push(format!("-{removed_num} {removed_line}"));
                out.push(format!("+{added_num} {added_line}"));
            } else {
                for (num, content) in &removed {
                    out.push(format!("-{num} {content}"));
                }
                for (num, content) in &added {
                    out.push(format!("+{num} {content}"));
                }
            }
            continue;
        }
        if prefix == '+' {
            let (_, num, content) = parse_diff_line(lines[index]).unwrap_or(('+', "", ""));
            out.push(format!("+{num} {}", content.replace('\t', "   ")));
            index += 1;
            continue;
        }
        let (_, num, content) = parse_diff_line(lines[index]).unwrap_or((' ', "", ""));
        out.push(format!(" {num} {}", content.replace('\t', "   ")));
        index += 1;
    }
    out
}
