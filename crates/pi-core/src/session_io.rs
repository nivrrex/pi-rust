//! 会话文件的读写与追加（I/O 边界层）。
//!
//! 格式见 `spec/c-session.md` 第 1 节：JSONL，每行一个 entry；首行是 header。
//! 解析本身是纯函数（见 [`crate::session`]），这里只负责与文件系统打交道。
//!
//! 失败路径都显式返回错误，不 panic。

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::session::{parse_line, SessionEntry, SessionHeader, SessionLine};
use pi_ai::message::Message;

/// 读写会话文件时的失败。
#[derive(Debug, thiserror::Error)]
pub enum SessionIoError {
    /// 文件系统操作失败。
    #[error("{path} 的 I/O 失败: {source}")]
    Io {
        /// 出问题的路径。
        path: PathBuf,
        /// 底层错误。
        #[source]
        source: std::io::Error,
    },
    /// 某一行不是合法 JSON 或不符 entry 结构。
    #[error("{path} 第 {line} 行解析失败: {source}")]
    Parse {
        /// 文件路径。
        path: PathBuf,
        /// 行号（1 基）。
        line: usize,
        /// 底层错误。
        #[source]
        source: serde_json::Error,
    },
    /// 文件里没有 header。
    #[error("{0} 的首行不是 session header")]
    MissingHeader(PathBuf),
    /// `type: "message"` 的条目**没能解析成消息**（`parse_line` 把它降级成了 `Unknown`）。
    ///
    /// `transcript()` 必须把它当**错误**——不然这就是又一条静默丢弃路径
    /// （条目级解析失败 → `Unknown` → 被跳过；S27 的 assistant 就是先掉进这里）。
    #[error("会话条目 {entry_id} 的 type 是 message，但消息解析失败: {detail}")]
    EntryMessageUnparsed {
        /// 出问题的条目 id。
        entry_id: String,
        /// 二次解析拿到的错误文本。
        detail: String,
    },
    /// 会话条目**读不回**成消息（`transcript()` 的读回路径）。
    ///
    /// **不允许静默丢弃**：以前这里是 `from_value(...).ok()`，于是"少了一个字段的条目"
    /// （例如手写文件缺 `usage.cost`）会让消息**无声消失**——S27 的 assistant 就是这么丢的。
    #[error("会话条目 {entry_id} 读不回成消息: {source}")]
    EntryMessageParse {
        /// 出问题的条目 id（**按条目**的读回路径里没有行号——条目已经解析过了）。
        entry_id: String,
        /// 底层错误。
        #[source]
        source: serde_json::Error,
    },
}

/// 一份会话文件的内容。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionFile {
    /// 首行 header。
    pub header: SessionHeader,
    /// 其余 entry（按文件顺序）。
    pub entries: Vec<SessionEntry>,
}

impl SessionFile {
    /// ref `packages/coding-agent/src/core/session-manager.ts:476-509` 的
    /// `buildContextEntries`：**只取最后一次 `compaction` 及其之后**的条目，并且从该
    /// compaction 的 `firstKeptEntryId` 起保留（system 消息除外）。
    ///
    /// 为什么恢复渲染必须用它（F49/Y1）：`renderInitialMessages()`
    /// （`interactive-mode.ts:4056-4062`）渲染的是 `buildContextEntries()` 的结果。
    /// 压缩前的历史**不再进入上下文**，因此也不应被渲染 —— 一个压缩过 23 次、95 MB 的会话
    /// 若把全部 56k 条都渲染出来，首帧就要写 50 MB（实测 47 秒；TS 同会话 1.8 MB、3 秒）。
    #[must_use]
    pub fn context_entries(&self) -> Vec<&SessionEntry> {
        context_entries(&self.entries)
    }

    /// 同 [`Self::context_entries`]，但只取 **message 条目**。
    ///
    /// **注意**：这条路径会丢掉 `compaction` 与 `branch_summary` 条目——
    /// 而 ref 的 `sessionEntryToContextMessages`（`session-manager.ts:458-465`）
    /// 会把它们转成 `CompactionSummary` / `BranchSummary` 消息送进上下文。
    /// 需要与 ref 一致的上下文时请用 [`Self::projected_messages`]。
    #[must_use]
    pub fn context_messages(&self) -> Vec<Message> {
        Self::messages_of(&self.entries)
    }

    /// 与 ref `buildContextEntries` + `sessionEntryToContextMessages` 一致的上下文消息。
    ///
    /// 走 `projection::build_session_projection`（它保留了 `compaction` 与
    /// `branch_summary` 条目并各自投影成一条消息），因此与自己实际发出的请求体同源。
    ///
    /// 为什么单独加而不是改 `context_messages`：后者有几个**只需要裸消息**的调用方
    /// （例如树导航后的屏幕重放，那里 `[branch]` / `[compaction]` 块由界面自己渲染，
    /// 再塞一条摘要消息会重复显示）。切换调用点比改语义安全。
    #[must_use]
    pub fn projected_messages(&self) -> Vec<Message> {
        crate::agent::projection::build_session_projection(
            &self.entries,
            crate::agent::projection::LeafSelector::Unspecified,
        )
        .map(|projection| projection.messages)
        .unwrap_or_default()
    }

    /// 便捷入口：从条目切片算 `buildContextEntries` 的 message 子集。
    #[must_use]
    pub fn messages_of(entries: &[SessionEntry]) -> Vec<Message> {
        context_entries(entries)
            .into_iter()
            .filter_map(|entry| match entry {
                SessionEntry::Message { message, .. } => Some(message.clone()),
                _other => None,
            })
            .collect()
    }
}

/// ref `session-manager.ts:476-509` 的实现（纯函数，便于单测）。
///
/// 语义：沿**文件顺序**取到最后一个 `compaction` 为止；
/// 从该 compaction 的 `firstKeptEntryId` 起保留条目（`system` 消息除外），
/// 再接上 compaction 自身与它之后的全部条目。
#[must_use]
pub fn context_entries(entries: &[SessionEntry]) -> Vec<&SessionEntry> {
    let Some(last) = entries
        .iter()
        .rposition(|entry| matches!(entry, SessionEntry::Compaction { .. }))
    else {
        return entries.iter().collect();
    };
    let SessionEntry::Compaction {
        first_kept_entry_id,
        ..
    } = &entries[last]
    else {
        return entries.iter().collect();
    };
    let mut out: Vec<&SessionEntry> = vec![&entries[last]];
    let mut found_first_kept = false;
    for entry in &entries[..last] {
        if entry.id() == Some(first_kept_entry_id.as_str()) {
            found_first_kept = true;
        }
        let is_system_message = matches!(
            entry,
            SessionEntry::Message {
                message: Message::System { .. },
                ..
            }
        );
        if found_first_kept && !is_system_message {
            out.push(entry);
        }
    }
    out.extend(entries[last + 1..].iter());
    out
}

/// `branch_summary` 拼进 user 消息时的前缀——**逐字照抄**
/// `packages/coding-agent/src/core/messages.ts:19-23`。
pub const BRANCH_SUMMARY_PREFIX: &str =
    "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n";

/// 后缀——逐字照抄 `packages/coding-agent/src/core/messages.ts:24`。
pub const BRANCH_SUMMARY_SUFFIX: &str = "</summary>";

impl SessionFile {
    /// 读取会话文件。
    ///
    /// 空文件视为"缺 header"而不是空会话——静默接受空文件会让后续追加写出没有 header 的
    /// 文件，那种文件 TS 读不了。
    pub fn read(path: &Path) -> Result<Self, SessionIoError> {
        let text = std::fs::read_to_string(path).map_err(|source| SessionIoError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(path, &text)
    }

    /// 从文本解析；便于测试里不落盘。
    ///
    /// **坏行只跳过、不整份失败**（ref `session-manager.ts:616-624` 的 `parseSessionEntryLine`：
    /// 空行与解析失败的行都返回 `null`，读取继续）。此前任一坏行都让整个会话加载失败
    /// （真机 `-c` 报"第 6147 行解析失败"后整份会话打不开）。
    /// 剥掉开头的 UTF-8 BOM（Windows 编辑器会加）。
    ///
    /// 三处读会话文件的地方都要用：`SessionFile::parse`、`/resume` 的扫描、
    /// rpc 的条目读取。不剥时首行会变成 `\u{FEFF}{"type":"session"…}`，
    /// 解析失败 ⇒ 报「首行不是 session header」或把整份会话当不存在。
    #[must_use]
    pub fn strip_bom(text: &str) -> &str {
        text.strip_prefix('\u{feff}').unwrap_or(text)
    }

    pub fn parse(path: &Path, text: &str) -> Result<Self, SessionIoError> {
        // **先剥 UTF-8 BOM**：Windows 上的记事本等编辑器保存 UTF-8 时会加 BOM，
        // 带 BOM 时首行会变成 `\u{FEFF}{"type":"session"…}`，`parse_line` 解析失败
        // 整行被跳过 ⇒ `MissingHeader`，报「首行不是 session header」且 `-c` 直接失败。
        // TS 侧容忍（`utils/text.ts` 的 `stripBom`，实测同一份带 BOM 的会话能续写）。
        let text = Self::strip_bom(text);
        let mut header = None;
        let mut entries = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(parsed) = parse_line(line) else {
                continue;
            };
            match parsed {
                SessionLine::Header(parsed) => {
                    if header.is_none() {
                        header = Some(parsed);
                    }
                }
                SessionLine::Entry(mut entry) => {
                    // **保序信息在解析时会丢**（`serde_json::Map` 是 `BTreeMap`），而
                    // `Message::System` 的手写 `Serialize` 需要 `extra` 里的
                    // `__toolsAddedRaw`/`__sectionsRaw` 才能把 `sections`/`toolsAdded`
                    // **按原文键序**写回去（会话文件与事件载荷都要）。所以读回时从**原始行**
                    // 把这两段文本取出来挂上——与 `run.rs`/`session_writer.rs` 组装时同源；
                    // 内部键由 `Serialize` 消费，**不会**写进文件（`t3_38_no_internal_keys`）。
                    attach_raw_system_fields(line, &mut entry);
                    entries.push(entry)
                }
            }
        }
        let header = header.ok_or_else(|| SessionIoError::MissingHeader(path.to_path_buf()))?;
        Ok(Self { header, entries })
    }

    /// 追加一条 entry 到文件末尾。
    ///
    /// 逐行追加而不是整体重写：会话文件是 append-only 的，整体重写会在并发场景下
    /// 丢失另一个进程写入的内容。
    /// 会话里的**消息序列**（供 `--continue` 重建上下文）。
    ///
    /// - 只取 `message` 条目，按文件顺序；
    /// - **system 消息不返回**：系统提示由配置重新组装（不是从会话回放）；
    /// - **应用 `context_edit`**：`replacement` 是**部分字段补丁**（golden/S17 的种子文件里是
    ///   `{"content":[…]}`，只有 `content`），把它合并进目标消息后使用；`replacement: null`
    ///   表示**移除**该条（S07 的 `replacement: null` 就是这个语义）。
    ///
    /// 文件里**已经有 system 条目**吗？
    ///
    /// `transcript()` 会把 system 消息过滤掉（它不进上下文），所以要判断"要不要再写一条
    /// system"，只能直接看条目。〔续写路径用；golden 里续写后 system 只出现一次〕
    pub fn has_system_message(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry,
                SessionEntry::Message {
                    message: pi_ai::message::Message::System { .. },
                    ..
                }
            )
        })
    }

    pub fn transcript(&self) -> Result<Vec<pi_ai::message::Message>, SessionIoError> {
        use std::collections::HashMap;
        // 1) 消息按 id 存放（保持文件顺序）。
        let mut order: Vec<String> = Vec::new();
        let mut by_id: HashMap<String, serde_json::Value> = HashMap::new();
        for entry in &self.entries {
            // `custom_message` → **user** 消息〔packages/coding-agent/src/core/messages.ts:163-169〕；
            // `branch_summary` → **user** 消息（前缀 + summary + 后缀）〔:170-176〕。
            // 两者都**参与上下文**；我此前没建模这两类，`--continue` 会少消息。
            match entry {
                SessionEntry::Message { id, message, .. } => {
                    if let Ok(value) = serde_json::to_value(message) {
                        order.push(id.clone());
                        by_id.insert(id.clone(), value);
                    }
                }
                SessionEntry::CustomMessage { id, content, .. } => {
                    let blocks = match content {
                        serde_json::Value::String(text) => serde_json::json!([
                            { "type": "text", "text": text }
                        ]),
                        other => other.clone(),
                    };
                    order.push(id.clone());
                    by_id.insert(
                        id.clone(),
                        serde_json::json!({ "role": "user", "content": blocks, "timestamp": 0 }),
                    );
                }
                SessionEntry::BranchSummary { id, summary, .. } => {
                    order.push(id.clone());
                    by_id.insert(
                        id.clone(),
                        serde_json::json!({
                            "role": "user",
                            "content": [{
                                "type": "text",
                                "text": format!("{BRANCH_SUMMARY_PREFIX}{summary}{BRANCH_SUMMARY_SUFFIX}"),
                            }],
                            "timestamp": 0,
                        }),
                    );
                }
                SessionEntry::ThinkingLevelChange { .. }
                | SessionEntry::ModelChange { .. }
                | SessionEntry::Compaction { .. }
                | SessionEntry::ContextEdit { .. } => {}
                // **降级成 `Unknown` 的 message 条目要报错**（否则又是静默丢消息）。
                SessionEntry::Unknown(value) => {
                    if value.get("type").and_then(serde_json::Value::as_str) == Some("message") {
                        let entry_id = value
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("<无 id>")
                            .to_string();
                        let detail = match serde_json::from_value::<pi_ai::message::Message>(
                            value
                                .get("message")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null),
                        ) {
                            Ok(_) => {
                                "（二次解析竟然成功——请检查 parse_line 的降级条件）".to_string()
                            }
                            Err(error) => error.to_string(),
                        };
                        return Err(SessionIoError::EntryMessageUnparsed { entry_id, detail });
                    }
                }
            }
        }
        // 2) 按 entry 顺序应用 `context_edit`（后写的覆盖先写的）。
        let mut removed: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in &self.entries {
            if let SessionEntry::ContextEdit {
                target_id,
                replacement,
                ..
            } = entry
            {
                match replacement {
                    serde_json::Value::Null => {
                        removed.insert(target_id.clone());
                    }
                    serde_json::Value::Object(patch) => {
                        removed.remove(target_id);
                        if let Some(serde_json::Value::Object(object)) = by_id.get_mut(target_id) {
                            for (key, value) in patch {
                                object.insert(key.clone(), value.clone());
                            }
                        }
                    }
                    other => {
                        // 不是对象也不是 null：整体替换（保持与"补丁"同样的宽容度）。
                        by_id.insert(target_id.clone(), other.clone());
                        removed.remove(target_id);
                    }
                }
            }
        }
        // 3) 反序列化回 `Message`，滤掉被移除的与 system。
        let mut out = Vec::new();
        // **只取当前叶子的路径**，不是文件线性顺序：分支会话里，同级兄弟不属于当前上下文。
        // 等价于 ref `session-manager.ts:390` 的 `buildSessionPath(entries, leafId, byId)`：
        // 叶子 = 文件最后一条（`getLeafId()`），沿 `parentId` 上溯，再反转。
        let mut parents: HashMap<String, Option<String>> = HashMap::new();
        for entry in &self.entries {
            if let Some(id) = entry.id() {
                parents.insert(id.to_string(), entry.parent_id().map(str::to_string));
            }
        }
        let mut path: Vec<String> = Vec::new();
        let mut seen_path: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut current = order.last().cloned();
        while let Some(id) = current {
            if !seen_path.insert(id.clone()) {
                break;
            }
            current = parents.get(&id).cloned().flatten();
            path.push(id);
        }
        path.reverse();
        for id in path {
            if removed.contains(&id) {
                continue;
            }
            let Some(value) = by_id.get(&id).cloned() else {
                continue;
            };
            // **解析失败就报错**（带条目 id），不再 `.ok()` 掉——静默丢消息是 S27 那次真缺陷。
            let message =
                serde_json::from_value::<pi_ai::message::Message>(value).map_err(|source| {
                    SessionIoError::EntryMessageParse {
                        entry_id: id.clone(),
                        source,
                    }
                })?;
            if !matches!(message, pi_ai::message::Message::System { .. }) {
                out.push(message);
            }
        }
        Ok(out)
    }

    pub fn append(path: &Path, entry: &SessionEntry) -> Result<(), SessionIoError> {
        let line = serde_json::to_string(entry).map_err(|source| SessionIoError::Parse {
            path: path.to_path_buf(),
            line: 0,
            source,
        })?;
        Self::append_line(path, &line)
    }

    /// 追加一条条目，但把 `systemMessage` 字段用**原始文本**（`RawValue`）原样写出。
    ///
    /// 与 `Message::System` 的 `__toolsAddedRaw` / `__sectionsRaw` 是**同一套机制**：需要
    /// 原样输出的字段走 `RawValue`（`serde_json::Map` 是 `BTreeMap`，会把它排序）。
    /// 这里原文来自**会话文件的原始行**（[`extract_raw_message_object`]），因为压缩条目要
    /// 回写那条 system 消息（ref `session-manager.ts:1262-1274`），而解析后的 `Map` 已经把
    /// `toolsAdded[].parameters` 的键序排序掉了。
    ///
    /// `entry` 里若带 `systemMessage`，会被 `raw_system_message` 覆盖（内部键 `__systemMessageRaw`
    /// 从不入库）。
    pub fn append_with_raw_system_message(
        path: &Path,
        entry: &SessionEntry,
        raw_system_message: &str,
    ) -> Result<(), SessionIoError> {
        // **不能**用 `to_value(entry)` 重建这一行：`serde_json::Map` 是 `BTreeMap`，
        // 会把条目的字段整体排序（`details`/`firstKeptEntryId`/`fromHook`/`id`…），
        // 而归一化工具按**原文出现顺序**给 id 编号（`tools/difftest/normalize.mjs:26`），
        // 顺序一变、占位符编号就整体错位（S19/S22 的 A 层正是这样被我对错的）。
        // 所以：先按条目自己的字段顺序序列化，再把 `systemMessage` 的值**原位替换**成原文。
        let mut line = serde_json::to_string(entry).map_err(|source| SessionIoError::Parse {
            path: path.to_path_buf(),
            line: 0,
            source,
        })?;
        match replace_json_field_value(&line, "systemMessage", raw_system_message) {
            Some(replaced) => line = replaced,
            None => {
                return Err(SessionIoError::Parse {
                    path: path.to_path_buf(),
                    line: 0,
                    source: serde_json::Error::io(std::io::Error::other(
                        "条目里没有 systemMessage 字段，无法原位替换",
                    )),
                })
            }
        }
        Self::append_line(path, &line)
    }

    /// 追加一条条目，但把 `systemMessage` 字段用**原始文本**（`RawValue`）原样写出。
    ///
    /// 把一行 JSON 追加到文件末尾（必要时先补一个换行）。
    fn append_line(path: &Path, line: &str) -> Result<(), SessionIoError> {
        let mut line = line.to_string();
        // **先补换行**：已有内容如果不是以 `\n` 结尾（手写/生成的种子文件可能如此），
        // 直接 append 会把新条目**粘在上一行后面**，整个文件就再也解析不了。
        // 这是 S27 对拍发现的真缺陷（种子末尾没有换行，`thinking_level_change` 被粘到
        // `branch_summary` 那行上，表现为"A 层少一条"）。
        let needs_newline = match std::fs::metadata(path) {
            Ok(meta) if meta.len() > 0 => {
                use std::io::{Read, Seek, SeekFrom};
                let mut file = std::fs::File::open(path).map_err(|source| SessionIoError::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                file.seek(SeekFrom::End(-1))
                    .map_err(|source| SessionIoError::Io {
                        path: path.to_path_buf(),
                        source,
                    })?;
                let mut last = [0u8; 1];
                file.read_exact(&mut last)
                    .map_err(|source| SessionIoError::Io {
                        path: path.to_path_buf(),
                        source,
                    })?;
                last[0] != b'\n'
            }
            // 文件不存在或为空：不需要补换行。
            Ok(_) | Err(_) => false,
        };
        if needs_newline {
            line.insert(0, '\n');
        }
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map_err(|source| SessionIoError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        file.write_all(line.as_bytes())
            .map_err(|source| SessionIoError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(())
    }

    /// 把整份会话写到文件（覆盖）。用于新建会话。
    pub fn write(&self, path: &Path) -> Result<(), SessionIoError> {
        let mut text =
            serde_json::to_string(&self.header).map_err(|source| SessionIoError::Parse {
                path: path.to_path_buf(),
                line: 0,
                source,
            })?;
        text.push('\n');
        for entry in &self.entries {
            let line = serde_json::to_string(entry).map_err(|source| SessionIoError::Parse {
                path: path.to_path_buf(),
                line: 0,
                source,
            })?;
            text.push_str(&line);
            text.push('\n');
        }
        std::fs::write(path, text).map_err(|source| SessionIoError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// 由**原始**会话条目构造 TS `getTree()` 的树形结构（纯函数，便于单测）。
///
/// 形状取自 golden/S28 实测：节点是 `{"entry": <原始条目>, "children": [...]}`，
/// `parentId` 为 `null`/缺省（或指向不存在的条目）的条目是**根**；
/// `type == "session"` 的文件头**不是**树节点。返回 `(tree, leafId)`，
/// `leafId` 是**最后一条**条目的 id（TS 的 `getLeafId()`）。〔`session-manager.ts:1522` `getTree()`〕
/// 会话树：返回**已排好键序的 JSON 字符串**与叶子 id。
///
/// 为什么返回字符串而不是 `Value`：`serde_json` 的对象是 `BTreeMap`，会把
/// 节点 `{entry, children}` 排成 `{children, entry}`。而这里的键序是判据
/// （rpc 响应要与 TS 逐字节互通，`golden/S28` 实测节点是 `entry` 在前），
/// 所以整棵树在**这一层**就拼成字符串，不再经过 `Value` 的排序。
/// 〔session-manager.ts:1522；rpc-mode.ts:651〕
pub fn build_tree(entries: &[serde_json::Value]) -> (String, Option<String>) {
    build_tree_with_raw(entries, None)
}

/// 同 `build_tree`，但可传入**每条目的原始 JSON 文本**（与 `entries` 同序）。
///
/// 条目的键序也是判据：`serde_json::from_str::<Value>` 会按字母序重排，
/// 而 rpc 的树响应里 `entry` 必须保持会话文件里的原始键序
/// （`golden/S28`：`{type, id, parentId, timestamp, message}`）。
pub fn build_tree_with_raw(
    entries: &[serde_json::Value],
    raw_lines: Option<&[String]>,
) -> (String, Option<String>) {
    let mut by_id: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    // 条目 id → 原始 JSON 文本（仅当调用方给了 `raw_lines` 且长度对得上时）。
    let mut raw_by_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut kids: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut leaf: Option<String> = None;
    for (index, entry) in entries.iter().enumerate() {
        if entry.get("type").and_then(serde_json::Value::as_str) == Some("session") {
            continue;
        }
        let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let id = id.to_string();
        leaf = Some(id.clone());
        if let Some(parent) = entry.get("parentId").and_then(serde_json::Value::as_str) {
            kids.entry(parent.to_string()).or_default().push(id.clone());
        } else {
            // 没有 parent（null 或缺省）→ 根；用空串做根的伪 parent 键。
            kids.entry(String::new()).or_default().push(id.clone());
        }
        if let Some(raw) = raw_lines.and_then(|lines| lines.get(index)) {
            raw_by_id.insert(id.clone(), compact_keep_order(raw));
        }
        by_id.insert(id.clone(), entry.clone());
        order.push(id);
    }
    // 指向**不存在**的父条目：也当根（会话文件可能被手工编辑过）。
    for id in &order {
        if let Some(parent) = by_id
            .get(id)
            .and_then(|e| e.get("parentId"))
            .and_then(serde_json::Value::as_str)
        {
            if !by_id.contains_key(parent) {
                kids.entry(String::new()).or_default().push(id.clone());
                kids.entry(parent.to_string())
                    .or_default()
                    .retain(|c| c != id);
            }
        }
    }
    fn node(
        id: &str,
        by_id: &std::collections::HashMap<String, serde_json::Value>,
        raw_by_id: &std::collections::HashMap<String, String>,
        kids: &std::collections::HashMap<String, Vec<String>>,
    ) -> String {
        // **显式栈的前序遍历，直接写进一个缓冲**。
        //
        // 两件事都不能靠递归/自底向上拼串：
        // 1. 递归：真实会话上万条，3000 层就 `has overflowed its stack` 并 abort
        //    整个进程（实测 2000 层正常、3000 层 SIGABRT）；
        // 2. 自底向上拼串：每层都要把子树字符串复制进父节点，深链下总复制量
        //    O(n²)（实测 10000 层 4.03s）。
        // 前序写缓冲则每段只写一次，总代价 O(输出大小)。
        //
        // 输出形状：`{"entry":E,"children":[<子…>]}`——`entry` 在前（golden/S28 实测）。
        struct Frame {
            child_ids: Vec<String>,
            next_child: usize,
            written: usize,
        }
        let entry_json = |id: &str| match raw_by_id.get(id) {
            Some(raw) => raw.clone(),
            None => by_id
                .get(id)
                .cloned()
                .unwrap_or(serde_json::Value::Null)
                .to_string(),
        };
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        seen.insert(id.to_string());
        let mut out = String::new();
        out.push_str("{\"entry\":");
        out.push_str(&entry_json(id));
        out.push_str(",\"children\":[");
        let mut stack: Vec<Frame> = vec![Frame {
            child_ids: kids.get(id).cloned().unwrap_or_default(),
            next_child: 0,
            written: 0,
        }];
        while let Some(top) = stack.last_mut() {
            let mut advanced = None;
            while top.next_child < top.child_ids.len() {
                let child = top.child_ids[top.next_child].clone();
                top.next_child += 1;
                if seen.insert(child.clone()) {
                    advanced = Some(child);
                    break;
                }
            }
            match advanced {
                Some(child) => {
                    // 逗号按"父帧已写出的子节点数"决定，与遍历进度无关。
                    if top.written > 0 {
                        out.push(',');
                    }
                    top.written += 1;
                    out.push_str("{\"entry\":");
                    out.push_str(&entry_json(&child));
                    out.push_str(",\"children\":[");
                    stack.push(Frame {
                        child_ids: kids.get(&child).cloned().unwrap_or_default(),
                        next_child: 0,
                        written: 0,
                    });
                }
                None => {
                    out.push_str("]}");
                    stack.pop();
                }
            }
        }
        out
    }
    let root_ids: Vec<String> = kids.get("").cloned().unwrap_or_default();
    let mut roots: Vec<String> = Vec::new();
    for id in root_ids {
        roots.push(node(&id, &by_id, &raw_by_id, &kids));
    }
    (format!("[{}]", roots.join(",")), leaf)
}

/// TS `getUserMessagesForForking()`：可 fork 的**用户消息**列表（纯函数）。
///
/// 规则照 `coding-agent/src/core/agent-session.ts:3782-3796`：遍历**全部**条目
/// （不分叉路径），保留 `type=="message"` 且 `message.role=="user"`，文本用
/// `contentText(content, "")` 求（字符串原样；内容块只取 `type=="text"` 的 `text`，
/// 用 `""` 拼接〔`ai/src/utils/text.ts:6-12`〕），**空文本跳过**；返回 `{entryId, text}`，
/// 顺序即**文件顺序**。golden/S28 实测为 `[m1, m3a, m3b]`（m2 是 assistant，不入选）。
pub fn fork_messages(entries: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for entry in entries {
        if entry.get("type").and_then(serde_json::Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = entry.get("message") else {
            continue;
        };
        if message.get("role").and_then(serde_json::Value::as_str) != Some("user") {
            continue;
        }
        let text = match message.get("content") {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(serde_json::Value::Array(blocks)) => blocks
                .iter()
                .filter(|block| {
                    block.get("type").and_then(serde_json::Value::as_str) == Some("text")
                })
                .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
            Some(serde_json::Value::Null) | None => String::new(),
            // 既不是字符串也不是内容块数组（数字/对象等）：`contentText` 也不会给出文本。
            Some(_not_text) => String::new(),
        };
        if text.is_empty() {
            continue;
        }
        let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        out.push(serde_json::json!({"entryId": id, "text": text}));
    }
    out
}

/// 把一行 JSON **紧凑化**，同时保留字段的原始顺序。
///
/// 为什么不直接原样回吐：场景夹具里的会话行带空格
/// （`{"type": "message", "id": "m1", …}`），而 TS 侧是 `JSON.stringify` 的
/// **紧凑**输出。rpc 的树响应要与之逐字节互通，所以这里按原文的键序重新紧凑输出
/// （`serde_json::to_string` 会把键排成字母序，不能用）。
pub fn compact_keep_order(raw: &str) -> String {
    // **不能**先 `from_str::<Value>` 再回吐：`serde_json::Map` 是 `BTreeMap`，
    // 解析后键序已经变成字母序。这里直接从原文里剥掉结构空白，
    // 保留字段的原始出现次序。
    let mut out = String::with_capacity(raw.len());
    let mut in_string = false;
    let mut escaped = false;
    for ch in raw.trim_end().chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        // 结构空白丢掉，其余原样（用 `if` 而不是 `_ =>`，本项目禁通配分支）。
        if ch == '"' {
            in_string = true;
            out.push(ch);
        } else if !matches!(ch, ' ' | '\t' | '\n' | '\r') {
            out.push(ch);
        }
    }
    out
}

/// 按**给定键序**把一个 JSON 对象渲染成字符串（无新依赖）。
///
/// 为什么需要它：`serde_json` 的对象是 `BTreeMap`——**按键名排序**输出。所以在"键序是判据"
/// 的地方（TS 会读回并逐字节比较的 rpc 响应，例如 `get_tree` 的 `data` 是 `{tree, leafId}`
/// 而我这边会排成 `{leafId, tree}`），必须自己出字符串。值的转义交给 `serde_json`。
pub fn ordered_object(pairs: &[(&str, &serde_json::Value)]) -> String {
    let mut out = String::from("{");
    for (index, (key, value)) in pairs.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&serde_json::Value::String((*key).to_string()).to_string());
        out.push(':');
        out.push_str(&value.to_string());
    }
    out.push('}');
    out
}

/// 若这条条目是 `system` 消息，从**原始行**取出 `sections`/`toolsAdded` 的文本挂到
/// `extra` 的内部键上（保序用；见 `SessionFile::parse` 的说明）。
fn attach_raw_system_fields(line: &str, entry: &mut SessionEntry) {
    let SessionEntry::Message { message, .. } = entry else {
        return;
    };
    let pi_ai::message::Message::System { extra, .. } = message else {
        return;
    };
    let Some(raw_message) = extract_raw_message_object(line) else {
        return;
    };
    for (field, key) in [
        ("sections", pi_ai::message::SECTIONS_RAW_KEY),
        ("toolsAdded", pi_ai::message::TOOLS_ADDED_RAW_KEY),
    ] {
        if let Some(raw) = raw_json_field_value(raw_message, field) {
            extra.insert(key.to_string(), serde_json::Value::String(raw.to_string()));
        }
    }
}

/// 取一段 JSON 文本里**顶层**某个字段的**原文**（含对象/数组的配对括号）。
#[must_use]
pub fn raw_json_field_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            if text[index..].starts_with(&needle) {
                let mut cursor = index + needle.len();
                while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                    cursor += 1;
                }
                if bytes.get(cursor) == Some(&b':') {
                    cursor += 1;
                    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                        cursor += 1;
                    }
                    let end = balanced_value_end(text, cursor)?;
                    return text.get(cursor..end);
                }
            }
            in_string = true;
            index += 1;
            continue;
        }
        index += 1;
    }
    None
}

/// 把一段 JSON 文本里**顶层**某个字段的值原位替换成 `raw`（字符串感知，保留其余字段的原文顺序）。
///
/// 用于"字段要用原文写出、但整行不能重排"的场景（压缩条目的 `systemMessage`）：
/// `serde_json::Map` 是 `BTreeMap`，一旦经它重建，字段顺序就变了，而归一化工具按原文
/// 出现顺序给 id 编号（`tools/difftest/normalize.mjs:26`）。
/// 找不到该字段或值不配对时返回 `None`（调用方按错误处理，不猜）。
#[must_use]
pub fn replace_json_field_value(text: &str, key: &str, raw: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            if text[index..].starts_with(&needle) {
                let mut cursor = index + needle.len();
                while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                    cursor += 1;
                }
                if bytes.get(cursor) == Some(&b':') {
                    cursor += 1;
                    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                        cursor += 1;
                    }
                    let end = balanced_value_end(text, cursor)?;
                    let mut out = String::with_capacity(text.len() + raw.len());
                    out.push_str(text.get(..cursor)?);
                    out.push_str(raw);
                    out.push_str(text.get(end..)?);
                    return Some(out);
                }
            }
            in_string = true;
            index += 1;
            continue;
        }
        index += 1;
    }
    None
}

/// 从 `start` 起取一个 JSON 值（对象/数组/字符串/数字等）的结束下标（不含）。
fn balanced_value_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    match bytes.get(start)? {
        b'{' | b'[' => {
            let (open, close) = (bytes[start], if bytes[start] == b'{' { b'}' } else { b']' });
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            for (offset, byte) in bytes[start..].iter().enumerate() {
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if *byte == b'\\' {
                        escaped = true;
                    } else if *byte == b'"' {
                        in_string = false;
                    }
                    continue;
                }
                match byte {
                    b'"' => in_string = true,
                    value if *value == open => depth += 1,
                    value if *value == close => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(start + offset + 1);
                        }
                    }
                    _not_structural => {}
                }
            }
            None
        }
        b'"' => {
            let mut escaped = false;
            for (offset, byte) in bytes[start + 1..].iter().enumerate() {
                if escaped {
                    escaped = false;
                } else if *byte == b'\\' {
                    escaped = true;
                } else if *byte == b'"' {
                    return Some(start + offset + 2);
                }
            }
            None
        }
        _scalar => {
            let end = bytes[start..]
                .iter()
                .position(|byte| byte.is_ascii_whitespace() || matches!(byte, b',' | b'}' | b']'))
                .map_or(bytes.len(), |offset| start + offset);
            Some(end)
        }
    }
}

/// 从会话文件的**原始行文本**里取出 `message` 字段的 JSON 对象文本（原样、保序）。
///
/// 为什么需要它：会话文件里 system 消息的 `toolsAdded`/`sections` 是**原始文本**（TS 写的键序），
/// 但解析成 `serde_json::Map`（`BTreeMap`）后键序就没了，而内部保序键（`__toolsAddedRaw` /
/// `__sectionsRaw`）按约定**不入库**（`t3_38_no_internal_keys`）⇒ 读回时无从恢复。
/// 压缩条目要**回写**这条 system 消息（ref `agent-session.ts:1262-1274`），所以直接从原始行取。
///
/// 扫描是**字符串感知**的：`"message"` 出现在字符串字面量里（例如系统提示正文提到 message）
/// 不会被误匹配。找不到或不配对时返回 `None`（调用方按"没有该字段"处理，不猜）。
#[must_use]
pub fn extract_raw_message_object(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            // 顶层键：`"message"` 后跟 `:`（允许空白）
            if line[index..].starts_with("\"message\"") {
                let mut cursor = index + "\"message\"".len();
                while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                    cursor += 1;
                }
                if bytes.get(cursor) == Some(&b':') {
                    cursor += 1;
                    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                        cursor += 1;
                    }
                    if bytes.get(cursor) == Some(&b'{') {
                        return balanced_object(&line[cursor..]);
                    }
                }
            }
            in_string = true;
            index += 1;
            continue;
        }
        index += 1;
    }
    None
}

/// 取一段以 `{` 开头的文本里**配对**的对象（含花括号），字符串感知。
fn balanced_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return text.get(..=index);
                }
            }
            _not_structural => {}
        }
    }
    None
}

#[cfg(test)]
mod raw_message_tests {
    use super::extract_raw_message_object;

    #[test]
    fn extracts_the_message_object_verbatim() {
        let line = r#"{"type":"message","id":"a","timestamp":"t","message":{"role":"system","content":"","toolsAdded":[{"name":"read","description":"d","parameters":{"type":"object"},"constrainedSampling":false}]}}"#;
        let raw = extract_raw_message_object(line).expect("能取到 message 对象");
        // 原样：键序与原文一致（`name` 在 `constrainedSampling` 之前）
        assert!(
            raw.starts_with(r#"{"role":"system""#),
            "取到的是 message 对象本身"
        );
        let name = raw.find("\"name\"").expect("有 name");
        let constrained = raw
            .find("\"constrainedSampling\"")
            .expect("有 constrainedSampling");
        assert!(name < constrained, "键序必须原样保留（name 在前）");
        assert!(raw.ends_with('}'), "到配对的花括号为止");
        assert!(!raw.contains("\"type\":\"message\""), "不含条目自己的字段");

        // 负向：字符串里出现 `"message":` 不得被当成字段
        let tricky = r#"{"type":"message","message":{"role":"system","content":"提到 \"message\": {\"x\":1} 这段","timestamp":1}}"#;
        let raw = extract_raw_message_object(tricky).expect("仍能取到");
        assert!(raw.contains("这段"), "取到的是真正的 message 对象");
        assert_eq!(
            raw.matches("timestamp").count(),
            1,
            "没有把字符串里的内容当字段"
        );

        // 负向：没有该字段、或花括号不配对 ⇒ None（不猜）
        assert!(extract_raw_message_object(r#"{"type":"model_change","id":"a"}"#).is_none());
        assert!(extract_raw_message_object(r#"{"message":{"role":"system""#).is_none());
        assert!(extract_raw_message_object("").is_none());
    }
}

#[cfg(test)]
mod raw_system_message_tests {
    use super::SessionFile;
    use crate::session::SessionEntry;

    fn compaction_entry() -> SessionEntry {
        serde_json::from_value(serde_json::json!({
            "type": "compaction", "id": "c1", "parentId": "a1", "timestamp": "2026-01-01T00:00:00.000Z",
            "summary": "s", "firstKeptEntryId": "a1", "tokensBefore": 10,
            "systemMessage": {"role": "system", "content": "", "toolsAdded": [
                {"name": "read", "description": "d", "parameters": {"type": "object"}}]}
        }))
        .expect("压缩条目可解析")
    }

    #[test]
    fn raw_system_message_is_written_verbatim_and_plain_append_sorts() {
        let dir = tempfile::tempdir().expect("临时目录");
        let raw = r#"{"role":"system","content":"","timestamp":5,"toolsAdded":[{"name":"read","description":"d","parameters":{"type":"object"},"constrainedSampling":false}]}"#;

        // ① 原始文本写出：`name` 在 `constrainedSampling` 之前（原序）
        let with_raw = dir.path().join("raw.jsonl");
        SessionFile::append_with_raw_system_message(&with_raw, &compaction_entry(), raw)
            .expect("写出成功");
        let text = std::fs::read_to_string(&with_raw).expect("读回");
        let name = text.find("\"name\"").expect("有 name");
        let constrained = text
            .find("\"constrainedSampling\"")
            .expect("有 constrainedSampling");
        assert!(name < constrained, "原始文本必须原样写出（name 在前）");
        assert!(
            text.contains("\"timestamp\":5"),
            "原文里的 timestamp 原样保留"
        );

        // ② 反证：同样的条目走普通 `append` ⇒ 键被排序（说明这条路径确实需要 RawValue）
        let sorted = dir.path().join("sorted.jsonl");
        SessionFile::append(&sorted, &compaction_entry()).expect("写出成功");
        let text = std::fs::read_to_string(&sorted).expect("读回");
        let name = text.find("\"name\"").expect("有 name");
        let description = text.find("\"description\"").expect("有 description");
        assert!(
            description < name,
            "普通 append 会排序（description 在 name 前）——这正是本修复要绕开的机制"
        );

        // ③ 写出后的那一行仍能解析，且 systemMessage 在内存里可用
        let line = std::fs::read_to_string(&with_raw).expect("读回");
        let entry: SessionEntry = serde_json::from_str(line.trim_end()).expect("写出的行仍可解析");
        let value = serde_json::to_value(&entry).expect("可序列化");
        assert_eq!(value["systemMessage"]["role"], "system");
        assert_eq!(value["type"], "compaction");
    }
}
