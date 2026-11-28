//! `edit` 工具：按精确文本替换文件内容。
//!
//! 校验与替换复用 [`super::edit_text`] 的纯逻辑；这里只负责读写与参数解析。
//! 对应 `packages/coding-agent/src/core/tools/edit.ts` 与 `spec/e-tools.md` 第 4 节。
//!
//! **已知差异**： fuzzy 匹配已实现，但 `normalizeForFuzzyMatch` 的 NFKC 步骤仍未实现，
//! 会影响全角/兼容字符；BOM、行尾保留与 diff/patch 已实现。
//!
//! 参数归一由 [`prepare_arguments`] 负责（ref `harness/tools/edit.ts:55-75` 的
//! `prepareEditArguments`），在校验**之前**跑，见 [`Tool::prepare_arguments`]。

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::edit_text::{apply_edits, success_message, Edit, EditError};
use super::tool::{Tool, ToolContext, ToolResult};

/// `edit` 工具。
#[derive(Debug, Default)]
pub struct EditTool;

/// 参数 schema。
pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["path", "edits"],
        "properties": {
            "path": {"type": "string", "description": "Path to the file to edit (relative or absolute)"},
            "edits": {
                "type": "array",
                "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead.",
                "items": {
                    "type": "object",
                    "required": ["oldText", "newText"],
                    "properties": {
                        "oldText": {"type": "string", "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."},
                        "newText": {"type": "string", "description": "Replacement text for this targeted edit."}
                    },
                    "required": ["oldText", "newText"]
                }
            }
        }
    })
}

/// 参数归一到标准形态：`{path, edits: [{oldText, newText}, …]}`。
///
/// 逐条对应 ref `harness/tools/edit.ts:55-75`：
/// 1. `edits` 是 JSON **字符串** ⇒ `JSON.parse`；解析成数组就整体替换，
///    解析成单个 edit 对象就包成数组（`:56-66`）。解析失败保持原样（ref 的 `catch {}`）。
/// 2. `edits` 是单个 edit 对象 ⇒ 包成数组（`:67-69`）。
/// 3. 顶层有 `oldText`/`newText`（legacy 单次替换）⇒ 追加进 `edits` 并删掉两个顶层字段
///    （`:71-75`）；`edits` 不是数组时按空数组起算。
///
/// 非对象入参照原样返回（ref 的 `if (!input || typeof input !== "object")`）。
pub fn prepare_arguments(arguments: Value) -> Value {
    let Some(object) = arguments.as_object() else {
        return arguments;
    };
    let mut object = object.clone();
    if let Some(edits) = object.get("edits").cloned() {
        if let Some(raw) = edits.as_str() {
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                if parsed.is_array() {
                    object.insert("edits".into(), parsed);
                } else if is_single_edit(&parsed) {
                    object.insert("edits".into(), Value::Array(vec![parsed]));
                }
            }
        } else if is_single_edit(&edits) {
            object.insert("edits".into(), Value::Array(vec![edits]));
        }
    }
    let legacy = object
        .get("oldText")
        .and_then(Value::as_str)
        .map(str::to_string)
        .zip(
            object
                .get("newText")
                .and_then(Value::as_str)
                .map(str::to_string),
        );
    if let Some((old_text, new_text)) = legacy {
        let mut list = match object.remove("edits") {
            Some(Value::Array(items)) => items,
            Some(_) | None => Vec::new(),
        };
        list.push(json!({ "oldText": old_text, "newText": new_text }));
        object.remove("oldText");
        object.remove("newText");
        object.insert("edits".into(), Value::Array(list));
    }
    Value::Object(object)
}

/// 是否是单个 edit 对象（ref `isSingleEditInput`：`oldText`/`newText` 都是字符串）。
fn is_single_edit(value: &Value) -> bool {
    value.as_object().is_some_and(|map| {
        map.get("oldText").is_some_and(Value::is_string)
            && map.get("newText").is_some_and(Value::is_string)
    })
}

/// 从参数里解析 `edits`；缺字段按空字符串处理（与 `edit-diff.ts` 的宽松读取一致）。
fn parse_edits(arguments: &Value) -> Vec<Edit> {
    arguments
        .get("edits")
        .and_then(Value::as_array)
        .map(|edits| {
            edits
                .iter()
                .map(|edit| Edit {
                    old_text: crate::tools::tool::arg_string(edit.get("oldText")),
                    new_text: crate::tools::tool::arg_string(edit.get("newText")),
                    // 保持 String（下游按值使用），见 `Edit` 的字段类型。
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        r#"Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes."#
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(schema)
    }

    fn prepare_arguments(&self, arguments: Value) -> Value {
        prepare_arguments(arguments)
    }

    fn execute<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let path = crate::tools::tool::arg_string(arguments.get("path"));
            let path = path.as_str();
            let edits = parse_edits(&arguments);

            if cancel.is_cancelled() {
                return ToolResult::error("Operation aborted");
            }
            if edits.is_empty() {
                // 与 `validateEditInput` 的文案一致（JSON 解析层没拦住时由这里兜）。
                return ToolResult::error(
                    "Edit tool input is invalid. edits must contain at least one replacement.",
                );
            }

            let absolute = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                context.cwd.join(path)
            };
            let queued_path = absolute.clone();
            match super::file_mutation_queue::with_file_mutation_queue(&absolute, move || async move {
                // 先确认可读可写（TS 同样是 access 先行）。
                if let Err(error) = tokio::fs::metadata(&queued_path).await {
                    let kind = if error.kind() == std::io::ErrorKind::NotFound {
                        "ENOENT: no such file or directory"
                    } else {
                        "EACCES: permission denied"
                    };
                    return ToolResult::error(format!(
                        "Could not edit file: {path}. {kind}, access '{}'.",
                        queued_path.display()
                    ));
                }
                let content = match tokio::fs::read_to_string(&queued_path).await {
                    Ok(content) => content,
                    Err(error) => {
                        return ToolResult::error(format!("Could not edit file: {path}. {error}."));
                    }
                };
                if cancel.is_cancelled() {
                    return ToolResult::error("Operation aborted");
                }

                match apply_edits(&content, &edits, path) {
                    Ok(updated) => {
                        if let Err(error) = tokio::fs::write(&queued_path, &updated).await {
                            return ToolResult::error(format!("Could not edit file: {path}. {error}."));
                        }
                        // `details` 的三个字段（golden/S04 实测 `{diff, firstChangedLine, patch}`）：
                        // - `diff`/`firstChangedLine` 来自 `edit-diff.ts:376` 的 `generateDiffString`；
                        // - `patch` 来自 `edit-diff.ts:368` 的 `createTwoFilesPatch`
                        //   （`context: 4` + `FILE_HEADERS_ONLY`，文件名用**传入的 path**，golden 里是 `input.txt`）。
                        let display =
                            crate::tools::diff::generate_diff_string(&content, &updated, 4);
                        let patch = crate::tools::diff::create_two_files_patch(
                            path,
                            &content,
                            &updated,
                            Some(4),
                        );
                        let mut result = ToolResult::text(success_message(path, edits.len()));
                        let mut details = serde_json::json!({
                            "diff": display.diff,
                            "patch": patch,
                        });
                        // JS 的 `JSON.stringify` 会**省略** `undefined` 属性，所以仅在有值时写。
                        if let Some(first) = display.first_changed_line {
                            details["firstChangedLine"] = serde_json::json!(first);
                        }
                        result.details = Some(details);
                        result
                    }
                    Err(error) => ToolResult::error(match error {
                        EditError::EmptyEdits => {
                            "Edit tool input is invalid. edits must contain at least one replacement."
                                .to_string()
                        }
                        other => other.to_string(),
                    }),
                }
            })
            .await
            {
                Ok(result) => result,
                Err(error) => ToolResult::error(error.to_string()),
            }
        })
    }
}
