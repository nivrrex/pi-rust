//! 四个内置工具的定义（name / description / parameters）。
//!
//! 判据是 `golden/*/requests.jsonl` 里 TS 实际发出的 `tools` 数组——
//! 三个场景（S02、S12-openai、S25-openai）的 tools 指纹完全一致。
//!
//! 描述文本来自 TS 源码的模板拼接（`tools/{read,write,edit,bash}.ts`），
//! 其中 write 的描述可在 `write.ts` 里逐字找到；read/edit 的含插值，
//! 以 golden 渲染结果为准。

use pi_ai::openai::ToolSpec;

/// 四个内置工具，顺序与 TS 发出的一致：read、bash、edit、write。
pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
    ToolSpec {
        name: "read".to_string(),
        description: "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.".to_string(),
        parameters: serde_json::json!({
                "type": "object",
                "required": [
                        "path"
                ],
                "properties": {
                        "path": {
                                "type": "string",
                                "description": "Path to the file to read (relative or absolute)"
                        },
                        "offset": {
                                "type": "number",
                                "description": "Line number to start reading from (1-indexed)"
                        },
                        "limit": {
                                "type": "number",
                                "description": "Maximum number of lines to read"
                        }
                }
        }),
    },
    ToolSpec {
        name: "bash".to_string(),
        description: "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.".to_string(),
        parameters: serde_json::json!({
                "type": "object",
                "required": [
                        "command"
                ],
                "properties": {
                        "command": {
                                "type": "string",
                                "description": "Shell command to execute"
                        },
                        "timeout": {
                                "type": "number",
                                "description": "Timeout in seconds (optional, no default timeout)"
                        }
                }
        }),
    },
    ToolSpec {
        name: "edit".to_string(),
        description: "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.".to_string(),
        parameters: serde_json::json!({
                "type": "object",
                "required": [
                        "path",
                        "edits"
                ],
                "properties": {
                        "path": {
                                "type": "string",
                                "description": "Path to the file to edit (relative or absolute)"
                        },
                        "edits": {
                                "type": "array",
                                "items": {
                                        "type": "object",
                                        "required": [
                                                "oldText",
                                                "newText"
                                        ],
                                        "properties": {
                                                "oldText": {
                                                        "type": "string",
                                                        "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."
                                                },
                                                "newText": {
                                                        "type": "string",
                                                        "description": "Replacement text for this targeted edit."
                                                }
                                        }
                                },
                                "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead."
                        }
                }
        }),
    },
    ToolSpec {
        name: "write".to_string(),
        description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.".to_string(),
        parameters: serde_json::json!({
                "type": "object",
                "required": [
                        "path",
                        "content"
                ],
                "properties": {
                        "path": {
                                "type": "string",
                                "description": "Path to the file to write (relative or absolute)"
                        },
                        "content": {
                                "type": "string",
                                "description": "Content to write to the file"
                        }
                }
        }),
    },
    ]
}
