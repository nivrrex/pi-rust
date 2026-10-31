//! `diffLines` 的 Rust 实现——**从 jsdiff 忠实移植**。
//!
//! 出处（`diff@8.0.4` 未压缩源码）：
//! `libesm/diff/base.js`：`diff` :2、`diffWithOptionsObj` :17、`addToPath` :139、
//! `extractCommon` :154、`equals` :171、`removeEmpty` :180、`join` :197、`buildValues` :232；
//! `libesm/diff/line.js`：`tokenize` :44、`diffLines` :37。
//!
//! **移植中最关键的一处**（读代码看不出来，靠与 oracle 逐步对拍才发现）：
//! 主循环的对角线步长是 **`diagonalPath += 2`**（`base.js:73`），不是 +1——
//! Myers 的同一编改长度只处理**同奇偶**的对角线。我原先写 +1，会访问到不该访问的
//! 对角线、把路径记成次优（表现为 added/removed 顺序相反、把两次删除并成一段）。
//!
//! **未实现的选项**（`edit-diff.ts` 都没用）：`callback`、`timeout`、`maxEditLength`、
//! `comparator`、`ignoreCase`、`oneChangePerToken`、`newlineIsToken`、`stripTrailingCr`。
//! 用到时必须先补 oracle 再改。

/// 一个 change 段，字段名与 jsdiff 的 change 对象一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub value: String,
    pub count: usize,
    pub added: bool,
    pub removed: bool,
}

/// Myers 路径上的一个 component（对应 jsdiff 的 `lastComponent` 链）。
#[derive(Debug, Clone)]
struct Node {
    count: usize,
    added: bool,
    removed: bool,
    previous: Option<usize>,
}

/// 一条搜索路径：旧串位置 + component 链的末端。
#[derive(Debug, Clone, Copy)]
struct Path {
    old_pos: i64,
    last: Option<usize>,
}

/// 行切分（`line.js:44-62`）：按 `\n` 切开，把换行并回**前一行**（CRLF 自然保留）。
pub fn tokenize_lines(value: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut rest = value;
    while let Some(index) = rest.find('\n') {
        let end = index + 1;
        tokens.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        tokens.push(rest.to_string());
    }
    tokens
}

/// `removeEmpty`（`base.js:180-188`）。
fn remove_empty(tokens: Vec<String>) -> Vec<String> {
    tokens
        .into_iter()
        .filter(|token| !token.is_empty())
        .collect()
}

/// `diffLines(old, new)`。
pub fn diff_lines(old: &str, new: &str) -> Vec<Change> {
    let old_tokens = remove_empty(tokenize_lines(old));
    let new_tokens = remove_empty(tokenize_lines(new));
    diff_tokens(&new_tokens, &old_tokens)
}

/// `addToPath`（`base.js:139-152`）。
///
/// 合并分支是**替换**末端 component（`count + 1`，`previous` 沿用旧的），不是追加。
fn add_to_path(
    arena: &mut Vec<Node>,
    path: Path,
    added: bool,
    removed: bool,
    old_pos_inc: i64,
) -> Path {
    if let Some(index) = path.last {
        if arena[index].added == added && arena[index].removed == removed {
            let merged = arena.len();
            arena.push(Node {
                count: arena[index].count + 1,
                added,
                removed,
                previous: arena[index].previous,
            });
            return Path {
                old_pos: path.old_pos + old_pos_inc,
                last: Some(merged),
            };
        }
    }
    let pushed = arena.len();
    arena.push(Node {
        count: 1,
        added,
        removed,
        previous: path.last,
    });
    Path {
        old_pos: path.old_pos + old_pos_inc,
        last: Some(pushed),
    }
}

/// `extractCommon`（`base.js:154-169`）：沿对角线吃掉相等的 token。
fn extract_common(
    arena: &mut Vec<Node>,
    base: &mut Path,
    new_tokens: &[String],
    old_tokens: &[String],
    diagonal: i64,
) -> i64 {
    let new_len = new_tokens.len() as i64;
    let old_len = old_tokens.len() as i64;
    let mut old_pos = base.old_pos;
    let mut new_pos = old_pos - diagonal;
    let mut common = 0usize;
    while new_pos + 1 < new_len
        && old_pos + 1 < old_len
        && old_tokens[(old_pos + 1) as usize] == new_tokens[(new_pos + 1) as usize]
    {
        new_pos += 1;
        old_pos += 1;
        common += 1;
    }
    if common > 0 {
        let previous = base.last;
        base.last = Some(arena.len());
        arena.push(Node {
            count: common,
            added: false,
            removed: false,
            previous,
        });
    }
    base.old_pos = old_pos;
    new_pos
}

/// `buildValues`（`base.js:232-252`）。
fn build_values(
    arena: &[Node],
    last: Option<usize>,
    new_tokens: &[String],
    old_tokens: &[String],
) -> Vec<Change> {
    let mut chain: Vec<usize> = Vec::new();
    let mut cursor = last;
    while let Some(index) = cursor {
        chain.push(index);
        cursor = arena[index].previous;
    }
    chain.reverse();

    let mut out = Vec::with_capacity(chain.len());
    let mut new_pos = 0usize;
    let mut old_pos = 0usize;
    for index in chain {
        let node = &arena[index];
        if node.removed {
            let value = old_tokens[old_pos..old_pos + node.count].concat();
            old_pos += node.count;
            out.push(Change {
                value,
                count: node.count,
                added: false,
                removed: true,
            });
        } else {
            let value = new_tokens[new_pos..new_pos + node.count].concat();
            new_pos += node.count;
            if !node.added {
                old_pos += node.count;
            }
            out.push(Change {
                value,
                count: node.count,
                added: node.added,
                removed: false,
            });
        }
    }
    out
}

/// Myers 主体（`base.js:17-137` 的 `diffWithOptionsObj`）。
fn diff_tokens(new_tokens: &[String], old_tokens: &[String]) -> Vec<Change> {
    let mut arena: Vec<Node> = Vec::new();
    let new_len = new_tokens.len() as i64;
    let old_len = old_tokens.len() as i64;

    let mut best_path: std::collections::HashMap<i64, Path> = std::collections::HashMap::new();
    let mut initial = Path {
        old_pos: -1,
        last: None,
    };
    let mut new_pos = extract_common(&mut arena, &mut initial, new_tokens, old_tokens, 0);
    if initial.old_pos + 1 >= old_len && new_pos + 1 >= new_len {
        return build_values(&arena, initial.last, new_tokens, old_tokens);
    }
    best_path.insert(0, initial);

    let mut edit_length: i64 = 1;
    let mut min_diagonal = i64::MIN;
    let mut max_diagonal = i64::MAX;
    while edit_length <= new_len + old_len {
        // **步长是 2**（`base.js:73`）——同一编改长度只处理同奇偶的对角线。
        let mut diagonal = min_diagonal.max(-edit_length);
        while diagonal <= max_diagonal.min(edit_length) {
            let remove_path = best_path.get(&(diagonal - 1)).copied();
            let add_path = best_path.get(&(diagonal + 1)).copied();
            if remove_path.is_some() {
                best_path.remove(&(diagonal - 1));
            }
            let can_add = match add_path {
                Some(path) => {
                    let add_new_pos = path.old_pos - diagonal;
                    add_new_pos >= 0 && add_new_pos < new_len
                }
                None => false,
            };
            let can_remove = matches!(&remove_path, Some(path) if path.old_pos + 1 < old_len);
            if !can_add && !can_remove {
                best_path.remove(&diagonal);
                diagonal += 2;
                continue;
            }
            // jsdiff：`if (!canRemove || (canAdd && removePath.oldPos < addPath.oldPos))` 走 add，否则走 remove。
            // 四种子情况都写明，不留静默兜底。
            let (path, added, removed, old_pos_inc) = match (remove_path, add_path) {
                (Some(remove), Some(add)) => {
                    if !can_remove || remove.old_pos < add.old_pos {
                        (add, true, false, 0)
                    } else {
                        (remove, false, true, 1)
                    }
                }
                (Some(remove), None) => (remove, false, true, 1),
                (None, Some(add)) => (add, true, false, 0),
                (None, None) => {
                    best_path.remove(&diagonal);
                    diagonal += 2;
                    continue;
                }
            };
            let mut base = add_to_path(&mut arena, path, added, removed, old_pos_inc);
            new_pos = extract_common(&mut arena, &mut base, new_tokens, old_tokens, diagonal);
            if base.old_pos + 1 >= old_len && new_pos + 1 >= new_len {
                return build_values(&arena, base.last, new_tokens, old_tokens);
            }
            best_path.insert(diagonal, base);
            if base.old_pos + 1 >= old_len {
                max_diagonal = max_diagonal.min(diagonal - 1);
            }
            if new_pos + 1 >= new_len {
                min_diagonal = min_diagonal.max(diagonal + 1);
            }
            diagonal += 2;
        }
        edit_length += 1;
    }
    Vec::new()
}

// —— 卡片 2：`structuredPatch` / `formatPatch` / `createTwoFilesPatch` ——
//
// 出处：`libesm/patch/create.js`：`structuredPatch` :17、`formatPatch` :146、
// `createTwoFilesPatch` :189、`splitLines` :218。

/// 一个 hunk（字段名与 jsdiff 的 `structuredPatch` 输出一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: i64,
    pub old_lines: i64,
    pub new_start: i64,
    pub new_lines: i64,
    pub lines: Vec<String>,
}

/// `structuredPatch` 的返回结构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredPatch {
    pub old_file_name: String,
    pub new_file_name: String,
    pub old_header: Option<String>,
    pub new_header: Option<String>,
    pub hunks: Vec<Hunk>,
}

/// `splitLines`（`create.js:218-227`）：切行并**保留换行符**；末行无换行时该行不带 `\n`。
pub fn split_lines(text: &str) -> Vec<String> {
    let mut result: Vec<String> = text.split('\n').map(|line| format!("{line}\n")).collect();
    if text.ends_with('\n') {
        result.pop();
    } else if let Some(last) = result.pop() {
        // 末行没有换行：去掉刚才补上的 `\n`。
        result.push(last.trim_end_matches('\n').to_string());
    }
    result
}

/// 给一行加 `+`/`-`/` ` 前缀（jsdiff 里 `contextLines` 与两处 `curRange.push` 的等价形式）。
fn prefixed(prefix: char, line: &str) -> String {
    format!("{prefix}{line}")
}

/// `structuredPatch`（`create.js:17-143`）。`context` 缺省为 4。
pub fn structured_patch(
    old_file_name: &str,
    new_file_name: &str,
    old_str: &str,
    new_str: &str,
    old_header: Option<&str>,
    new_header: Option<&str>,
    context: Option<i64>,
) -> StructuredPatch {
    let context = context.unwrap_or(4);
    // 每段连同它的 `lines` 一起准备；**追加的空段 lines 显式为 `[]`**。
    // jsdiff 里 `current.lines || splitLines(current.value)` 的 `lines: []` 是**真值**，
    // 所以那个空段不会走 splitLines——我原先每段都用 splitLines("")，会多出一行空行
    // （表现为 patch 末尾多出 `" \n\ No newline at end of file"`）。
    let mut segments: Vec<(Change, Vec<String>)> = diff_lines(old_str, new_str)
        .into_iter()
        .map(|change| {
            let lines = split_lines(&change.value);
            (change, lines)
        })
        .collect();
    segments.push((
        Change {
            value: String::new(),
            count: 0,
            added: false,
            removed: false,
        },
        Vec::new(),
    ));

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut old_range_start: i64 = 0;
    let mut new_range_start: i64 = 0;
    let mut cur_range: Vec<String> = Vec::new();
    let mut old_line: i64 = 1;
    let mut new_line: i64 = 1;
    let diff_len = segments.len() as i64;

    for index in 0..segments.len() {
        let (current, lines) = &segments[index];
        let line_count = lines.len() as i64;
        if current.added || current.removed {
            if old_range_start == 0 {
                let previous = if index > 0 {
                    segments.get(index - 1)
                } else {
                    None
                };
                old_range_start = old_line;
                new_range_start = new_line;
                if let Some((_, previous_lines)) = previous {
                    // 取上一段末尾 `context` 行作为开头上下文。
                    let take = if context > 0 {
                        previous_lines
                            .iter()
                            .rev()
                            .take(context as usize)
                            .rev()
                            .map(|line| prefixed(' ', line))
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    cur_range = take;
                    old_range_start -= cur_range.len() as i64;
                    new_range_start -= cur_range.len() as i64;
                }
            }
            let prefix = if current.added { '+' } else { '-' };
            for line in lines {
                cur_range.push(prefixed(prefix, line));
            }
            if current.added {
                new_line += line_count;
            } else {
                old_line += line_count;
            }
        } else {
            if old_range_start != 0 {
                // 未改动段够短且后面还有内容 → 整段并入当前区间（不切 hunk）。
                if line_count <= context * 2 && (index as i64) < diff_len - 2 {
                    for line in lines {
                        cur_range.push(prefixed(' ', line));
                    }
                } else {
                    let context_size = line_count.min(context);
                    for line in lines.iter().take(context_size as usize) {
                        cur_range.push(prefixed(' ', line));
                    }
                    hunks.push(Hunk {
                        old_start: old_range_start,
                        old_lines: old_line - old_range_start + context_size,
                        new_start: new_range_start,
                        new_lines: new_line - new_range_start + context_size,
                        lines: std::mem::take(&mut cur_range),
                    });
                    old_range_start = 0;
                    new_range_start = 0;
                }
            }
            old_line += line_count;
            new_line += line_count;
        }
    }

    // 收尾：去掉行尾换行；末行无换行时其后插一行 `\ No newline at end of file`（`create.js:120-131`）。
    for hunk in &mut hunks {
        let mut index = 0;
        while index < hunk.lines.len() {
            if hunk.lines[index].ends_with('\n') {
                hunk.lines[index].pop();
            } else {
                hunk.lines
                    .insert(index + 1, String::from("\\ No newline at end of file"));
                index += 1;
            }
            index += 1;
        }
    }

    StructuredPatch {
        old_file_name: old_file_name.to_string(),
        new_file_name: new_file_name.to_string(),
        old_header: old_header.map(str::to_string),
        new_header: new_header.map(str::to_string),
        hunks,
    }
}

/// 头选项（`create.js:2-15` 的三个常量里本项目用到的两个）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderOptions {
    pub include_index: bool,
    pub include_underline: bool,
    pub include_file_headers: bool,
}

/// `FILE_HEADERS_ONLY`（`create.js:8-12`）——`edit-diff.ts:368` 用的就是它。
pub const FILE_HEADERS_ONLY: HeaderOptions = HeaderOptions {
    include_index: false,
    include_underline: false,
    include_file_headers: true,
};

/// `formatPatch`（`create.js:146-187`）。
pub fn format_patch(patch: &StructuredPatch, headers: HeaderOptions) -> String {
    let mut out: Vec<String> = Vec::new();
    if headers.include_index && patch.old_file_name == patch.new_file_name {
        out.push(format!("Index: {}", patch.old_file_name));
    }
    if headers.include_underline {
        out.push("===================================================================".to_string());
    }
    if headers.include_file_headers {
        out.push(format!(
            "--- {}{}",
            patch.old_file_name,
            patch
                .old_header
                .as_deref()
                .map(|header| format!("\t{header}"))
                .unwrap_or_default()
        ));
        out.push(format!(
            "+++ {}{}",
            patch.new_file_name,
            patch
                .new_header
                .as_deref()
                .map(|header| format!("\t{header}"))
                .unwrap_or_default()
        ));
    }
    for hunk in &patch.hunks {
        // `oldLines === 0` / `newLines === 0` 时起点要回退一行（`create.js:171-176`）。
        let old_start = if hunk.old_lines == 0 {
            hunk.old_start - 1
        } else {
            hunk.old_start
        };
        let new_start = if hunk.new_lines == 0 {
            hunk.new_start - 1
        } else {
            hunk.new_start
        };
        out.push(format!(
            "@@ -{},{old_lines} +{},{new_lines} @@",
            old_start,
            new_start,
            old_lines = hunk.old_lines,
            new_lines = hunk.new_lines
        ));
        out.extend(hunk.lines.iter().cloned());
    }
    format!("{}\n", out.join("\n"))
}

/// `createTwoFilesPatch`（`create.js:189-200`）——`edit-diff.ts:366` 用的就是它。
pub fn create_two_files_patch(
    file_name: &str,
    old_str: &str,
    new_str: &str,
    context: Option<i64>,
) -> String {
    let patch = structured_patch(file_name, file_name, old_str, new_str, None, None, context);
    format_patch(&patch, FILE_HEADERS_ONLY)
}

// —— 卡片 2 剩余：`generateDiffString`（显示用 diff + firstChangedLine）——
//
// 出处：`ref/pi-0.87.1/packages/coding-agent/src/core/tools/edit-diff.ts:376-498`。
// **它不属于 jsdiff**，是 edit-diff.ts 自己的格式化（带行号、`...` 折叠）。

/// `generateDiffString` 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayDiff {
    pub diff: String,
    pub first_changed_line: Option<i64>,
}

/// 按 `width` 右对齐（等价 JS 的 `String(n).padStart(width, " ")`）。
fn pad_start(value: &str, width: usize) -> String {
    if value.len() >= width {
        value.to_string()
    } else {
        format!("{}{}", " ".repeat(width - value.len()), value)
    }
}

/// `generateDiffString(oldContent, newContent, contextLines = 4)`。
pub fn generate_diff_string(old: &str, new: &str, context_lines: usize) -> DisplayDiff {
    let parts = diff_lines(old, new);
    let old_lines = old.split('\n').count();
    let new_lines = new.split('\n').count();
    let line_num_width = old_lines.max(new_lines).to_string().len();

    let mut output: Vec<String> = Vec::new();
    let mut old_line_num: i64 = 1;
    let mut new_line_num: i64 = 1;
    let mut last_was_change = false;
    let mut first_changed_line: Option<i64> = None;

    for (index, part) in parts.iter().enumerate() {
        // `part.value.split("\n")`；末元素为空串则 pop。
        let mut raw: Vec<&str> = part.value.split('\n').collect();
        if raw.last() == Some(&"") {
            raw.pop();
        }
        if part.added || part.removed {
            if first_changed_line.is_none() {
                first_changed_line = Some(new_line_num);
            }
            for line in &raw {
                if part.added {
                    output.push(format!(
                        "+{} {}",
                        pad_start(&new_line_num.to_string(), line_num_width),
                        line
                    ));
                    new_line_num += 1;
                } else {
                    output.push(format!(
                        "-{} {}",
                        pad_start(&old_line_num.to_string(), line_num_width),
                        line
                    ));
                    old_line_num += 1;
                }
            }
            last_was_change = true;
            continue;
        }

        let next_part_is_change = parts
            .get(index + 1)
            .map(|next| next.added || next.removed)
            .unwrap_or(false);
        let has_leading = last_was_change;
        let has_trailing = next_part_is_change;

        if has_leading && has_trailing {
            if raw.len() <= context_lines * 2 {
                for line in &raw {
                    output.push(format!(
                        " {} {}",
                        pad_start(&old_line_num.to_string(), line_num_width),
                        line
                    ));
                    old_line_num += 1;
                    new_line_num += 1;
                }
            } else {
                let leading = &raw[..context_lines];
                let trailing = &raw[raw.len() - context_lines..];
                let skipped = raw.len() - leading.len() - trailing.len();
                for line in leading {
                    output.push(format!(
                        " {} {}",
                        pad_start(&old_line_num.to_string(), line_num_width),
                        line
                    ));
                    old_line_num += 1;
                    new_line_num += 1;
                }
                output.push(format!(" {} ...", pad_start("", line_num_width)));
                old_line_num += skipped as i64;
                new_line_num += skipped as i64;
                for line in trailing {
                    output.push(format!(
                        " {} {}",
                        pad_start(&old_line_num.to_string(), line_num_width),
                        line
                    ));
                    old_line_num += 1;
                    new_line_num += 1;
                }
            }
        } else if has_leading {
            let shown = &raw[..raw.len().min(context_lines)];
            let skipped = raw.len() - shown.len();
            for line in shown {
                output.push(format!(
                    " {} {}",
                    pad_start(&old_line_num.to_string(), line_num_width),
                    line
                ));
                old_line_num += 1;
                new_line_num += 1;
            }
            if skipped > 0 {
                output.push(format!(" {} ...", pad_start("", line_num_width)));
                old_line_num += skipped as i64;
                new_line_num += skipped as i64;
            }
        } else if has_trailing {
            let skipped = raw.len().saturating_sub(context_lines);
            if skipped > 0 {
                output.push(format!(" {} ...", pad_start("", line_num_width)));
                old_line_num += skipped as i64;
                new_line_num += skipped as i64;
            }
            for line in &raw[skipped..] {
                output.push(format!(
                    " {} {}",
                    pad_start(&old_line_num.to_string(), line_num_width),
                    line
                ));
                old_line_num += 1;
                new_line_num += 1;
            }
        } else {
            old_line_num += raw.len() as i64;
            new_line_num += raw.len() as i64;
        }
        last_was_change = false;
    }

    DisplayDiff {
        diff: output.join("\n"),
        first_changed_line,
    }
}
