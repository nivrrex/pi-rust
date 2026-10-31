//! edit 工具的**纯逻辑**部分：输入校验、按原文匹配的替换、重叠与空改动检测。
//!
//! 逐条对应 `packages/coding-agent/src/core/tools/edit-diff.ts:250-350`
//! 与 `spec/e-tools.md` 第 4 节。这里不做文件 I/O。
//!
//! **未实现**（见 `notes/p2-reconcile.md`）：`fuzzyFindText` 的模糊匹配、
//! `normalizeForFuzzyMatch`、BOM 与换行符保留、diff/patch 生成。
//! 这些都要等模糊匹配的归一化规则抽清楚再做，否则会与 TS 语义不一致。

/// 一次替换。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// 待替换的原文；必须唯一，且不得与同一次调用里其他 `old_text` 重叠。
    pub old_text: String,
    /// 替换成的新文本。
    pub new_text: String,
}

/// edit 的校验失败。文案逐字照 TS。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// `edits` 不是数组或为空数组（`validateEditInput`）。
    #[error("Edit tool input is invalid. edits must contain at least one replacement.")]
    EmptyEdits,
    /// 单条 edit 的 `oldText` 为空。
    #[error("oldText must not be empty in {path}.")]
    EmptyOldTextSingle {
        /// 文件路径（原始入参）。
        path: String,
    },
    /// 多条 edit 中某条的 `oldText` 为空。
    #[error("edits[{edit_index}].oldText must not be empty in {path}.")]
    EmptyOldTextAt {
        /// 文件路径。
        path: String,
        /// 出问题的条目下标。
        edit_index: usize,
    },
    /// 单条 edit 找不到原文。
    #[error("Could not find the exact text in {path}. The old text must match exactly including all whitespace and newlines.")]
    NotFoundSingle {
        /// 文件路径。
        path: String,
    },
    /// 多条 edit 中某条找不到原文。
    #[error("Could not find edits[{edit_index}] in {path}. The oldText must match exactly including all whitespace and newlines.")]
    NotFoundAt {
        /// 文件路径。
        path: String,
        /// 出问题的条目下标。
        edit_index: usize,
    },
    /// 原文出现多次。
    #[error("Found {occurrences} occurrences of the text in {path}. The text must be unique. Please provide more context to make it unique.")]
    Duplicate {
        /// 文件路径。
        path: String,
        /// 出现次数。
        occurrences: usize,
    },
    /// 两条 edit 的目标区间重叠。
    #[error("edits[{previous}] and edits[{current}] overlap in {path}. Merge them into one edit or target disjoint regions.")]
    Overlap {
        /// 文件路径。
        path: String,
        /// 前一条的下标。
        previous: usize,
        /// 后一条的下标。
        current: usize,
    },
    /// 替换后内容与原内容完全相同。
    #[error("{message}")]
    NoChange {
        /// 完整文案（单条与多条不同）。
        message: String,
    },
}

/// 应用替换；返回替换后的内容。
///
/// 语义要点（与 TS 一致）：
/// - 每条 edit 都针对**原始内容**匹配，不是增量匹配；
/// - 同一 `old_text` 出现多次即报错，不做"替换全部"；
/// - 区间重叠即报错，不做合并。
pub fn apply_edits(content: &str, edits: &[Edit], path: &str) -> Result<String, EditError> {
    // 行尾归一化（`edit-diff.ts` 的 `detectLineEnding` → `normalizeToLF` →
    // …匹配与替换都在 LF 空间里做… → `restoreLineEndings`）。
    //
    // **这一步是 oracle 抓出来的**：不归一化时，`oldText` 用 LF 写、文件是 CRLF 的
    // 场景会走模糊路径，而模糊的归一化会把 `\r` 当行尾空白修剪掉，结果**整份文件
    // 的 CRLF 被降级成 LF**（TS 会原样保留）。
    let line_ending = detect_line_ending(content);
    let normalized_content = normalize_to_lf(content);
    let normalized_edits: Vec<Edit> = edits
        .iter()
        .map(|edit| Edit {
            old_text: normalize_to_lf(&edit.old_text),
            new_text: normalize_to_lf(&edit.new_text),
        })
        .collect();
    let result = apply_edits_lf(&normalized_content, &normalized_edits, path)?;
    Ok(restore_line_endings(&result, line_ending))
}

/// 行尾已归一化为 LF 的实现主体。
fn apply_edits_lf(content: &str, edits: &[Edit], path: &str) -> Result<String, EditError> {
    if edits.is_empty() {
        return Err(EditError::EmptyEdits);
    }
    let single = edits.len() == 1;

    // 只要有一条 edit 需要模糊匹配，就走"整体归一化 + 保留未变更部分"那条路径
    // （`edit-diff.ts:300-320`）。空 oldText 仍先按原样报错。
    if edits
        .iter()
        .any(|edit| !edit.old_text.is_empty() && !content.contains(&edit.old_text))
    {
        for (index, edit) in edits.iter().enumerate() {
            if edit.old_text.is_empty() {
                return Err(if single {
                    EditError::EmptyOldTextSingle {
                        path: path.to_string(),
                    }
                } else {
                    EditError::EmptyOldTextAt {
                        path: path.to_string(),
                        edit_index: index,
                    }
                });
            }
        }
        let base = normalize_for_fuzzy_match(content);
        return apply_edits_preserving_unchanged(content, &base, edits, path);
    }

    // 先逐条校验空 oldText 与唯一性，同时记录替换区间。
    let mut plan: Vec<(usize, usize, &Edit)> = Vec::with_capacity(edits.len());
    for (index, edit) in edits.iter().enumerate() {
        if edit.old_text.is_empty() {
            return Err(if single {
                EditError::EmptyOldTextSingle {
                    path: path.to_string(),
                }
            } else {
                EditError::EmptyOldTextAt {
                    path: path.to_string(),
                    edit_index: index,
                }
            });
        }
        let occurrences = content.matches(&edit.old_text).count();
        if occurrences == 0 {
            return Err(if single {
                EditError::NotFoundSingle {
                    path: path.to_string(),
                }
            } else {
                EditError::NotFoundAt {
                    path: path.to_string(),
                    edit_index: index,
                }
            });
        }
        if occurrences > 1 {
            return Err(EditError::Duplicate {
                path: path.to_string(),
                occurrences,
            });
        }
        let start = content.find(&edit.old_text).unwrap_or(0);
        plan.push((start, start + edit.old_text.len(), edit));
    }

    // 重叠检查：按起点排序后看前一条的结束是否越过后一条的起点。
    let mut by_start: Vec<usize> = (0..plan.len()).collect();
    by_start.sort_by_key(|index| plan[*index].0);
    for pair in by_start.windows(2) {
        let previous = plan[pair[0]];
        let current = plan[pair[1]];
        if previous.1 > current.0 {
            return Err(EditError::Overlap {
                path: path.to_string(),
                previous: pair[0],
                current: pair[1],
            });
        }
    }

    // 从后往前替换，避免前面的替换影响后面的偏移。
    let mut result = content.to_string();
    let mut ordered = plan;
    ordered.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, edit) in ordered {
        result.replace_range(start..end, &edit.new_text);
    }

    if result == content {
        return Err(EditError::NoChange {
            message: if single {
                format!(
                    "No changes made to {path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected."
                )
            } else {
                format!("No changes made to {path}. The replacements produced identical content.")
            },
        });
    }
    Ok(result)
}

/// 成功文案：`Successfully replaced N block(s) in <path>.`
pub fn success_message(path: &str, blocks: usize) -> String {
    format!("Successfully replaced {blocks} block(s) in {path}.")
}

/// 换行符归一化：`\r\n` 与单独的 `\r` 都变成 `\n`（`edit-diff.ts:19-21`）。
pub fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// 检测文件原本的换行风格（`edit-diff.ts:11-17`）。
///
/// 规则：找第一个 `\r\n` 与第一个 `\n` 的位置，`\r\n` 在前就是 CRLF，否则 LF。
pub fn detect_line_ending(content: &str) -> &'static str {
    match (content.find("\r\n"), content.find('\n')) {
        // `\r\n` 出现在第一个 `\n` 之前 → CRLF；其余（含没有任何换行）都是 LF。
        (Some(crlf), Some(lf)) if crlf < lf => "\r\n",
        (Some(_), None) | (Some(_), Some(_)) | (None, Some(_)) | (None, None) => "\n",
    }
}

/// 把 LF 还原成目标换行风格（`edit-diff.ts:23-25`）。
pub fn restore_line_endings(text: &str, ending: &str) -> String {
    if ending == "\r\n" {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

/// 模糊匹配前的归一化（`edit-diff.ts:34-57`）。
///
/// 已实现的部分：每行去尾空白、智能引号→ASCII、各种破折号→`-`、各种特殊空格→普通空格。
///
/// **未实现 NFKC**（源码第一步是 `text.normalize("NFKC")`）：NFKC 需要完整的 Unicode
/// 兼容映射表，手写子集必然静默产生与 TS 不一致的结果。要补就得引入 `unicode-normalization`
/// 依赖（按 AGENTS 需先记入 questions.md 等确认）；当前如实缺省，影响范围是
/// 全角/兼容字符（如全角括号、ﬁ 连字）的模糊匹配。
pub fn normalize_for_fuzzy_match(text: &str) -> String {
    text.split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .map(|ch| match ch {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            '\u{00A0}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
            '\u{2002}'..='\u{200A}' => ' ',
            other => other,
        })
        .collect()
}

/// 模糊查找的结果（`FuzzyMatchResult`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// 是否找到。
    pub found: bool,
    /// 命中的下标（`used_fuzzy` 为真时是**归一化空间**里的下标）。
    pub index: usize,
    /// 命中长度。
    pub match_length: usize,
    /// 是否走了模糊匹配。
    pub used_fuzzy: bool,
    /// 应当用于替换的基准内容（模糊命中时是归一化后的内容）。
    pub content_for_replacement: String,
}

/// 先精确、再模糊地查找 `old_text`（`edit-diff.ts:207-244`）。
pub fn fuzzy_find_text(content: &str, old_text: &str) -> FuzzyMatch {
    if let Some(index) = content.find(old_text) {
        return FuzzyMatch {
            found: true,
            index,
            match_length: old_text.len(),
            used_fuzzy: false,
            content_for_replacement: content.to_string(),
        };
    }
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old = normalize_for_fuzzy_match(old_text);
    match fuzzy_content.find(&fuzzy_old) {
        Some(index) => FuzzyMatch {
            found: true,
            index,
            match_length: fuzzy_old.len(),
            used_fuzzy: true,
            content_for_replacement: fuzzy_content,
        },
        None => FuzzyMatch {
            found: false,
            index: 0,
            match_length: 0,
            used_fuzzy: false,
            content_for_replacement: content.to_string(),
        },
    }
}

/// 在**归一化空间**里数出现次数（`edit-diff.ts:247-251`）。
pub fn count_occurrences(content: &str, old_text: &str) -> usize {
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old = normalize_for_fuzzy_match(old_text);
    if fuzzy_old.is_empty() {
        return 0;
    }
    fuzzy_content.matches(&fuzzy_old).count()
}

/// 模糊路径的替换：任一条 edit 走了模糊匹配时，基准内容整体归一化，
/// 但**只有命中区间被替换**，未改动部分保留**原始文本**（含原始行尾空白）。
///
/// 规则取自 `edit-diff.ts:300-320、137-165`。
///
/// 可行性依据：`normalize_for_fuzzy_match` 逐行处理、**不改变行数**（NFKC 未实现，
/// 不会引入行数变化），因此基准内容与原始内容的行号一一对应，可以按行回填。
///
/// **已与 TS 逐字节对账**（`tools/oracle/edit-oracle.mjs` 的 40 组用例 + 本仓库测试
/// `t3_23_edit_oracle`）。当初担心的"整行 vs 字节偏移"差异**没有出现**：只覆盖行内
/// 一部分的用例（`partial_line_prefix_and_suffix` → `'prefix CENTER suffix\n'`、
/// `partial_line_with_trailing_ws_on_line` → `'aa BB cc   \n'`）与
/// `fuzzy_single_line_trailing_ws`（→ `'Y   \n'`，尾随空白保留）全部逐字节一致。
///
/// 仍与 TS 不同的一处只有 **NFKC**（见 `t3_23_edit_oracle::known_differences`）。
pub fn apply_edits_preserving_unchanged(
    original: &str,
    base: &str,
    edits: &[Edit],
    path: &str,
) -> Result<String, EditError> {
    // 每条 edit 在基准内容里定位（精确优先、模糊兜底）。
    let mut plan: Vec<(usize, usize, &Edit)> = Vec::with_capacity(edits.len());
    for (index, edit) in edits.iter().enumerate() {
        // 不唯一检查也在**归一化空间**里做（与 `countOccurrences` 一致）。
        let occurrences = count_occurrences(base, &edit.old_text);
        if occurrences > 1 {
            return Err(EditError::Duplicate {
                path: path.to_string(),
                occurrences,
            });
        }
        let found = fuzzy_find_text(base, &edit.old_text);
        if !found.found {
            return Err(if edits.len() == 1 {
                EditError::NotFoundSingle {
                    path: path.to_string(),
                }
            } else {
                EditError::NotFoundAt {
                    path: path.to_string(),
                    edit_index: index,
                }
            });
        }
        plan.push((found.index, found.index + found.match_length, edit));
    }

    let original_lines: Vec<&str> = if original.is_empty() {
        Vec::new()
    } else {
        original.split('\n').collect()
    };
    let base_lines: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('\n').collect()
    };
    if original_lines.len() != base_lines.len() {
        // 行数对不上就不做"保留"这层——退回整体替换（宁可与 TS 有差异，也不要错位）。
        let mut result = base.to_string();
        let mut ordered = plan.clone();
        ordered.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        for (start, end, edit) in ordered {
            result.replace_range(start..end, &edit.new_text);
        }
        if result == original {
            return Err(EditError::NoChange {
                message: no_change_message(path, edits.len()),
            });
        }
        return Ok(result);
    }

    // 命中区间 → 行号；相邻或重叠的合成一组。
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for (start, end, _) in &plan {
        let start_line = line_of_offset(base, *start);
        // 命中的**最后一个字符**所在行。
        //
        // 不能写 `end - 1`：`end` 是字节偏移，减一可能落在多字节字符中间
        // （`"中文\n第二行"` 的 end=13，13-1=12 在「二」里面），
        // 于是 `line_of_offset` 的切片会 panic。取 `base[..end]` 的最后一个
        // **字符起点**才是边界安全的。
        let end_line = {
            let last = base[..*end]
                .char_indices()
                .next_back()
                .map(|(offset, _)| offset)
                .unwrap_or(0);
            line_of_offset(base, last)
        };
        match groups.last_mut() {
            // 与前一组相邻或重叠 → 合并；否则新起一组。
            Some(last) if start_line <= last.1 + 1 => last.1 = last.1.max(end_line),
            Some(_) | None => groups.push((start_line, end_line)),
        }
    }

    // 逐行回填：组内用基准行（归一化空间）并应用偏移替换，组外用**原始行**。
    let mut out: Vec<String> = Vec::with_capacity(original_lines.len());
    let mut index = 0;
    for (group_start, group_end) in groups {
        while index < group_start {
            out.push(original_lines[index].to_string());
            index += 1;
        }
        // 组内：把基准行拼起来，按偏移做替换，再拆回行。
        let base_start = base_lines[..group_start]
            .iter()
            .map(|l| l.len() + 1)
            .sum::<usize>();
        let group_base: String = base_lines[group_start..=group_end].join("\n");
        let mut replaced = group_base.clone();
        let mut ordered: Vec<(usize, usize, &Edit)> = plan
            .iter()
            .filter(|(start, _, _)| {
                *start >= base_start && *start < base_start + group_base.len() + 1
            })
            .cloned()
            .collect();
        ordered.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        for (start, end, edit) in ordered {
            let local_start = start.saturating_sub(base_start);
            let local_end = end.saturating_sub(base_start).min(replaced.len());
            if local_start <= local_end
                && replaced.is_char_boundary(local_start)
                && replaced.is_char_boundary(local_end)
            {
                replaced.replace_range(local_start..local_end, &edit.new_text);
            }
        }
        for line in replaced.split('\n') {
            out.push(line.to_string());
        }
        index = group_end + 1;
    }
    while index < original_lines.len() {
        out.push(original_lines[index].to_string());
        index += 1;
    }

    let result = out.join("\n");
    if result == original {
        return Err(EditError::NoChange {
            message: no_change_message(path, edits.len()),
        });
    }
    Ok(result)
}

/// 偏移落在第几行（0 基）。
///
/// **不 panic**：偏移若落在多字节字符中间，先退到该字符的起点。调用方算错偏移时
/// 结果会偏一行，而不会把整个工具打崩。
fn line_of_offset(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    text[..offset].matches('\n').count()
}

/// 空改动错误的文案（单条/多条两套）。
fn no_change_message(path: &str, total: usize) -> String {
    if total == 1 {
        format!("No changes made to {path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected.")
    } else {
        format!("No changes made to {path}. The replacements produced identical content.")
    }
}
