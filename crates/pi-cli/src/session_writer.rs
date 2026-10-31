//! 把一次运行的消息落成会话文件（`spec/c-session.md`）。
//!
//! 形状（`golden/S01/session.jsonl` 实测）：
//! 1. 首行 header：`{"type":"session","version":3,"id":<uuid>,"timestamp":<iso>,"cwd":<cwd>}`；
//! 2. 随后一条 `model_change`（`parentId: null`）；
//! 3. 再逐条 `message`，`parentId` 指向上一条的 id，`timestamp` 为写入时刻。
//!
//! **未实现**（见 notes）：`thinking_level_change`、compaction、toolResult 之外的其他 entry 类型；
//! 会话续写（`--continue`/`--resume`）与树/分支操作。

use std::path::{Path, PathBuf};

use pi_ai::message::Message;
use pi_core::session::{SessionEntry, SessionHeader, CURRENT_SESSION_VERSION};
use pi_core::{session_io::SessionFile, session_path, timefmt};

/// 会话目录里**最新**的会话文件（`--continue`/`--resume` 要续写它）。
///
/// 文件名是 `<ISO 时间戳，`:` 与 `.` 换成 `-`>_<id>.jsonl`，定长且按时间递增，
/// 因此**按文件名取最大**即可（与 ref 的"最近一次会话"语义一致），不必依赖 mtime
/// （mtime 在复制/恢复过的目录里会骗人）。
/// 目录里**最近修改**的会话文件。
///
/// 与 ref `session-manager.ts:749-767` 一致：按 `mtime` 倒序（此前用文件名字典序 `max`，
/// 与"最近"不是一回事），并可要求 header 里的 `cwd` 与给定目录匹配
/// （`sessionCwdMatches`，`session-manager.ts:744-746`；`--session-dir` 指定了共享目录时才需要）。
/// `-c`/`--resume` 是否该按会话文件头里的 `cwd` 过滤。
///
/// ref `session-manager.ts:1791-1796` 的 `continueRecent`：
/// `filterCwd = sessionDir !== undefined && dir !== getDefaultSessionDirPath(cwd)`。
/// 默认分区目录里**不**过滤 ⇒ 目录改名后（`Colibrì` → `colibri`）旧会话仍能被续写。
#[must_use]
pub fn should_filter_by_cwd(
    session_dir: &std::path::Path,
    cwd: &std::path::Path,
    agent_dir: &std::path::Path,
) -> bool {
    !pi_core::session_path::is_default_session_dir(
        session_dir,
        &cwd.to_string_lossy(),
        &agent_dir.to_string_lossy(),
    )
}

pub fn newest_session_file(
    session_dir: &std::path::Path,
    cwd: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> =
        std::fs::read_dir(session_dir)
            .ok()?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .filter_map(|path| {
                let mtime = std::fs::metadata(&path).ok()?.modified().ok()?;
                Some((mtime, path))
            })
            .collect();
    // 按 mtime **倒序**（最近的在前）。
    files.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let want = cwd.map(resolve_for_compare);
    for (_, path) in files {
        match &want {
            None => return Some(path),
            Some(want) => {
                let header_cwd = session_header_cwd(&path)
                    .map(|value| resolve_for_compare(std::path::Path::new(&value)));
                if header_cwd.as_deref() == Some(want.as_str()) {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// 比较 cwd 时统一形态（`resolvePath` 的近似：去掉末尾分隔符；能规范化就规范化）。
fn resolve_for_compare(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['/', '\\']);
    trimmed.to_string()
}

/// 读会话文件首行的 `cwd`（ref `readSessionHeaderForDiscovery`）：读不到或不是 JSON 就当没有。
fn session_header_cwd(path: &std::path::Path) -> Option<String> {
    use std::io::BufRead as _;
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(file).read_line(&mut line).ok()?;
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    value.get("cwd")?.as_str().map(str::to_string)
}

/// **预生成**一份会话的 (ISO 时间戳, 会话 id, 文件路径)——**不建文件**。
///
/// 与 TS 对齐：会话 id 与文件名在**启动时**就确定（`get_state` 会报出来），文件在**首次写入**时才创建。
pub fn precompute_session(session_dir: &Path) -> (String, String, std::path::PathBuf) {
    let iso = timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis());
    let id = uuid::Uuid::now_v7().to_string();
    let path = session_dir.join(session_path::session_file_name(&iso, &id));
    (iso, id, path)
}

/// 会话写入器：持有文件路径与"上一条 entry 的 id"。
pub struct SessionWriter {
    path: PathBuf,
    last_id: Option<String>,
    /// 已写入（或从已有文件读到）的条目类型。`--continue` 要靠它判断
    /// "会话里有没有 `thinking_level_change`"（ref `sdk.ts:196`）。
    kinds: Vec<String>,
    /// 是否已经真正落盘（ref `session-manager.ts:1160-1190` 的 `flushed`）。
    ///
    /// **ref 是惰性创建**：第一条 assistant 消息出现之前，所有条目只在内存里攒着，
    /// 会话目录里**没有文件**。这一行为对 `/resume` 的列表可见性有直接影响（P7-7d3）。
    flushed: bool,
    /// 尚未落盘的条目行（header 之后的部分；header 单独由 `header_line` 保存）。
    pending: Vec<String>,
    /// header 那一行的原文。json 模式的**事件流首行**就是它
    /// （golden/S01 的 `events.jsonl` 第 1 行是 `{"type":"session",…}`），
    /// 所以要把原文留着，不能只在建文件时写一次就算。
    header_line: String,
}

impl SessionWriter {
    /// 新建会话文件并写入 header 与 `model_change`（内部先生成 id 与文件名）。
    ///
    /// 需要"先确定 id/文件名、稍后再建文件"的场景（rpc 的 `get_state` 要在写器未建时也报出来，
    /// 见 S35）请用 `precompute_session` + `create_precomputed`。
    ///
    /// `session_dir` 取 `--session-dir` 或 `PI_CODING_AGENT_SESSION_DIR`（由调用方解析好）。
    pub fn create(
        session_dir: &Path,
        cwd: &Path,
        provider: &str,
        model_id: &str,
        thinking_level: &str,
    ) -> Result<Self, String> {
        let (iso, id, path) = precompute_session(session_dir);
        Self::create_precomputed(
            session_dir,
            cwd,
            provider,
            model_id,
            thinking_level,
            &iso,
            &id,
            &path,
        )
    }

    /// 用**预生成**的 `iso`/`id`/`path` 建会话文件（不重新生成 id，保证 `get_state` 早报的值与真实文件一致）。
    #[allow(clippy::too_many_arguments)]
    pub fn create_precomputed(
        _session_dir: &Path,
        cwd: &Path,
        provider: &str,
        model_id: &str,
        thinking_level: &str,
        iso: &str,
        id: &str,
        path: &Path,
    ) -> Result<Self, String> {
        let path = path.to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let header = SessionHeader {
            kind: "session".to_string(),
            version: Some(CURRENT_SESSION_VERSION),
            id: id.to_string(),
            timestamp: iso.to_string(),
            cwd: Some(cwd.to_string_lossy().to_string()),
            extra: serde_json::Map::new(),
        };
        let header_line = serde_json::to_string(&header).map_err(|error| error.to_string())?;
        // **不在这里建文件**：ref 等到第一条 assistant 消息才 `openSync(path, "wx")` 并整体 flush
        // （`session-manager.ts:1174-1184`）。

        // **首条 entry 的 `parentId` 是 `null`**（`golden/S01/session.jsonl` 第 2 行实测），
        // 不是 header 的 id——header 不参与 id 链。
        let mut writer = Self {
            path,
            last_id: None,
            kinds: Vec::new(),
            flushed: false,
            pending: Vec::new(),
            header_line: header_line.clone(),
        };
        let (entry_id, parent_id) = writer.take_id_and_parent();
        let model_change = SessionEntry::ModelChange {
            id: entry_id,
            parent_id,
            timestamp: timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis()),
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            extra: serde_json::Map::new(),
        };
        writer.append_entry(&model_change)?;
        // 紧随 `model_change` 之后是 `thinking_level_change`（golden/S01 第 3 行）。
        // 本项目尚未实现 thinking 等级，固定写 `"off"`（与 `LoopConfig` 的默认一致）。
        let (entry_id, parent_id) = writer.take_id_and_parent();
        writer.append_entry(&SessionEntry::ThinkingLevelChange {
            id: entry_id,
            parent_id,
            timestamp: timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis()),
            thinking_level: thinking_level.to_string(),
            extra: serde_json::Map::new(),
        })?;
        Ok(writer)
    }

    /// 这次运行写了哪个文件。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// header 行的原文（json 事件流首行要用）。
    pub fn header_line(&self) -> &str {
        &self.header_line
    }

    /// 追加 **system 条目**。
    ///
    /// 形状照 `golden/S01` 第 4 行：`content` 是**空串**、正文由 `sections` 承载，
    /// 且带 `toolsAdded`（本 turn 声明的工具）。我原先写的是**渲染后的正文**，与 golden 不符。
    pub fn append_system_message(
        &mut self,
        sections: &[(String, String)],
        tools_added: &[serde_json::Value],
        // `tools_added_raw`：同一份 `toolsAdded` 的**按 TS 键序文本**（空串表示没有工具）。
        // `Message::System` 的手写 `Serialize` 会以 `RawValue` 原样写出它，
        // 从而让 TS 的 `declarationsEqual`（`JSON.stringify` 判等）认为声明没变。
        tools_added_raw: &str,
    ) -> Result<(), String> {
        let (id, parent_id) = self.take_id_and_parent();
        let mut extra = serde_json::Map::new();
        if !sections.is_empty() {
            let map: serde_json::Map<String, serde_json::Value> = sections
                .iter()
                .map(|(name, text)| (name.clone(), serde_json::Value::String(text.clone())))
                .collect();
            extra.insert("sections".to_string(), serde_json::Value::Object(map));
        }
        if !tools_added.is_empty() {
            extra.insert(
                "toolsAdded".to_string(),
                serde_json::Value::Array(tools_added.to_vec()),
            );
        }
        // `toolsRemoved`：ref `packages/ai/src/utils/transcript.ts:150-167` 的
        // `getToolStateChanges`——**上一份**声明里"当前没有**或声明变了**"的工具（只取 `{name}`）。
        // 上一份取**会话文件里最后一条 system 消息**的 `toolsAdded`（`--continue` 的场景里就是
        // 上一次声明的工具集）；读不到就当"没有上一份"，此时 `removed` 为空、不写该键。
        let (added, removed) =
            pi_core::agent::tool_state_changes(&self.last_system_tools_added(), tools_added);
        if !removed.is_empty() {
            extra.insert(
                "toolsRemoved".to_string(),
                serde_json::Value::Array(removed),
            );
        }
        // 有"上一份"时 `toolsAdded` 也应是**差集**（ref 同源）。当前只在"新增/声明变化"这一
        // 分支里沿用调用方给的完整列表（`tools_added_raw` 与之一致）；差集与完整列表不同的
        // 情形（部分工具未变）留待后续：那时需要同时按差集重建原文文本。
        let _ = added;
        if !sections.is_empty() {
            // `sections` 也要**按 TS 的顺序**原样写出（否则 TS 重放会话时按字母序拼系统提示）。
            let mut raw = String::from("{");
            for (index, (name, text)) in sections.iter().enumerate() {
                if index > 0 {
                    raw.push(',');
                }
                raw.push_str(&serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string()));
                raw.push(':');
                raw.push_str(&serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()));
            }
            raw.push('}');
            extra.insert(
                pi_ai::message::SECTIONS_RAW_KEY.to_string(),
                serde_json::Value::String(raw),
            );
        }
        if !tools_added_raw.is_empty() {
            extra.insert(
                pi_ai::message::TOOLS_ADDED_RAW_KEY.to_string(),
                serde_json::Value::String(tools_added_raw.to_string()),
            );
        }
        self.append_entry(&SessionEntry::Message {
            id,
            parent_id,
            timestamp: timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis()),
            message: Message::System {
                content: pi_ai::UserContent::Text(String::new()),
                // TS 的会话里 system 消息带**真实时间**（`core/session-manager.ts:1207…1351` 一律
                // `new Date().toISOString()`；“0”不是 TS 的形态）⇒ 用同一时刻的毫秒时间戳。
                timestamp: timefmt::now_unix_millis(),
                extra,
            },
            extra: serde_json::Map::new(),
        })
    }

    /// 会话是否**已经真正落盘**（ref `session-manager.ts:1136` 的 `isPersisted()`
    /// + `formatResumeCommand` 里的 `existsSync(sessionFile)`）。
    ///
    /// ref 是惰性创建文件：没有任何会话条目时目录里**没有文件**，此时退出**不打印**
    /// `To resume this session:`（T28/T35/T77 三个纯 UI 场景的 golden 尾部实测没有该行）。
    #[must_use]
    pub fn is_flushed(&self) -> bool {
        self.flushed
    }

    /// header 里的会话 id（`get_state` 要报 `sessionId`，golden/S16 实测是 uuid）。
    pub fn header_id(&self) -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(&self.header_line).ok()?;
        value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }

    /// 追加一条 `model_change`（`set_model` 用；**即使模型没变也写**，golden/S16 实测）。
    pub fn append_model_change(&mut self, provider: &str, model_id: &str) -> Result<(), String> {
        let (id, parent_id) = self.take_id_and_parent();
        self.append_entry(&SessionEntry::ModelChange {
            id,
            parent_id,
            timestamp: timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis()),
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            extra: serde_json::Map::new(),
        })
    }

    /// 续写时按需补一条 `thinking_level_change`。
    ///
    /// ref `sdk.ts:196`：`hasThinkingEntry = sessionManager.getBranch().some(e => e.type === "thinking_level_change")`
    /// ——**只有会话里还没有该条目时才写**（S17 的种子没有 → golden 里补了一条；
    /// S14 的第一阶段写过 → 续写阶段不再补）。
    pub fn append_thinking_level_change(&mut self, level: &str) -> Result<(), String> {
        let (id, parent_id) = self.take_id_and_parent();
        self.append_entry(&SessionEntry::ThinkingLevelChange {
            id,
            parent_id,
            timestamp: timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis()),
            thinking_level: level.to_string(),
            extra: serde_json::Map::new(),
        })
    }

    /// 会话里**是否已有**某种类型的条目。
    pub fn has_entry_kind(&self, kind: &str) -> bool {
        self.kinds.iter().any(|existing| existing == kind)
    }

    /// **打开已有会话文件继续写**（`--continue` / `--resume`）。
    ///
    /// 与 `create` 的区别：**不写** header、`model_change`、`thinking_level_change`
    /// （golden/S14 的会话里只有**一条** `session`、且没有重复的 `model_change`），
    /// 并把最后一条 entry 的 id 当作后续 `parentId`。
    pub fn open(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let header_line = text
            .lines()
            .next()
            .ok_or_else(|| "会话文件是空的".to_string())?
            .to_string();
        let file = SessionFile::read(path).map_err(|error| error.to_string())?;
        let last_id = file
            .entries
            .iter()
            .rev()
            .find_map(|entry| entry.id().map(str::to_string));
        Ok(Self {
            path: path.to_path_buf(),
            last_id,
            kinds: file
                .entries
                .iter()
                .map(|entry| entry.kind().to_string())
                .collect(),
            // 打开已存在的会话文件 ⇒ 视为已经落盘（后续一律追加）。
            flushed: true,
            pending: Vec::new(),
            header_line,
        })
    }

    /// **最后一条已写入 entry 的 id**（重试包装器要用它当 `context_edit` 的 `targetId`）。
    pub fn last_entry_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }

    /// 追加 `context_edit`，返回**刚写入的那条 entry 的 JSON**（`entry_appended` 的载荷要用它）。
    /// `replacement: null` 表示"移除该条"（golden/S07 第 7 行）。
    pub fn append_context_edit(&mut self, target_id: &str) -> Result<serde_json::Value, String> {
        let (id, parent_id) = self.take_id_and_parent();
        let entry = SessionEntry::ContextEdit {
            id,
            parent_id,
            timestamp: timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis()),
            target_id: target_id.to_string(),
            replacement: serde_json::Value::Null,
            extra: serde_json::Map::new(),
        };
        self.append_entry(&entry)?;
        serde_json::to_value(&entry).map_err(|error| error.to_string())
    }

    /// 追加一条消息。
    pub fn append_message(&mut self, message: &Message) -> Result<(), String> {
        let (id, parent_id) = self.take_id_and_parent();
        let entry = SessionEntry::Message {
            id,
            parent_id,
            timestamp: timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis()),
            message: message.clone(),
            extra: serde_json::Map::new(),
        };
        self.append_entry(&entry)
    }

    /// 会话里**最后一份**已声明的工具（没有则空）。
    ///
    /// 用于 `toolsRemoved` 的差分（ref `transcript.ts:150-167` 的 `previous`）。
    /// **必须用 `last_declared_tools`**：`compaction` 条目的 `systemMessage` 里也可能带着
    /// 上一份声明（`--continue` 载入的种子会话就常常只有它、没有独立的 system 消息条目）——
    /// 我第一版只扫 system 消息条目，于是 `previous` 恒空、`toolsRemoved` 永远不写（S20c 实测）。
    fn last_system_tools_added(&self) -> Vec<serde_json::Value> {
        let Ok(file) = pi_core::session_io::SessionFile::read(&self.path) else {
            return Vec::new();
        };
        pi_core::agent::last_declared_tools(&file.entries)
    }

    /// 生成新 entry 的 id，并给出**它之前**那一条的 id 作为 `parentId`。
    ///
    /// 必须一步取回两者：我第一版写成"先 `next_id()`（内部更新 `last_id`），
    /// 再读 `self.last_id` 当 parent"，`parentId` 于是等于自己的 id——
    /// 由 golden 对比时的形状检查发现。
    fn take_id_and_parent(&mut self) -> (String, Option<String>) {
        let parent = self.last_id.clone();
        // entry id 是 **uuid 的前 8 位**：`session-manager.ts:277-283` 的
        // `randomUUID().slice(0, 8)`（撞了就重试，最多 100 次，仍撞才退回整条 uuid）。
        // 我原先用整条 uuid，B 层比对的 `entry_appended` 里立刻显出形态不同
        // （golden 归一化成 `<IDn>`、我的是 `<UUID>`）。
        // **必须是 v4（随机）**：v7 的前 8 位是时间戳，同一毫秒内两次 append 会得到相同的 id
        // （我的测试 `t3_32` 当场报出 `left == right`）。TS 用的是 `randomUUID()`（v4）。
        let id = uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>();
        self.last_id = Some(id.clone());
        (id, parent)
    }

    /// 追加一条 `label` 条目（ref `session-manager.ts:1439-1451` 的 `appendLabelChange`）。
    ///
    /// 本项目的 `SessionEntry` 没有 `label` 变体（ref 的 entry 联合里也把它列为"其它类型"），
    /// 因此以 [`SessionEntry::Unknown`] 原样写入，字段顺序与 ref 一致。
    ///
    /// # Errors
    /// 写文件失败时返回错误描述。
    pub fn append_label(&mut self, target_id: &str, label: &str) -> Result<(), String> {
        let (entry_id, parent_id) = self.take_id_and_parent();
        let target = if target_id.is_empty() {
            self.last_id.clone().unwrap_or_else(|| entry_id.clone())
        } else {
            target_id.to_string()
        };
        let mut object = serde_json::Map::new();
        object.insert(
            "type".to_string(),
            serde_json::Value::String("label".into()),
        );
        object.insert("id".to_string(), serde_json::Value::String(entry_id));
        object.insert(
            "parentId".to_string(),
            parent_id.map_or(serde_json::Value::Null, serde_json::Value::String),
        );
        object.insert(
            "timestamp".to_string(),
            serde_json::Value::String(timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis())),
        );
        object.insert("targetId".to_string(), serde_json::Value::String(target));
        object.insert(
            "label".to_string(),
            if label.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(label.to_string())
            },
        );
        self.append_entry(&SessionEntry::Unknown(serde_json::Value::Object(object)))
    }

    fn append_entry(&mut self, entry: &SessionEntry) -> Result<(), String> {
        self.kinds.push(entry.kind().to_string());
        let line = serde_json::to_string(entry).map_err(|error| error.to_string())?;
        // 已经 flush 过 ⇒ 直接追加（ref `:1186-1188`）。
        if self.flushed {
            return SessionFile::append(&self.path, entry).map_err(|error| error.to_string());
        }
        // 还没 flush：遇到第一条 **assistant** 消息才落盘，把 header + 攒下的全部条目 + 本条一次写出
        // （ref `:1167-1184`：`openSync(path, "wx")` 后逐条写）。
        let has_assistant = matches!(entry, SessionEntry::Message { message, .. }
            if matches!(message, pi_ai::message::Message::Assistant { .. }));
        if !has_assistant {
            self.pending.push(line);
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut body = String::with_capacity(self.header_line.len() + line.len() + 64);
        body.push_str(&self.header_line);
        body.push('\n');
        for buffered in self.pending.drain(..) {
            body.push_str(&buffered);
            body.push('\n');
        }
        body.push_str(&line);
        body.push('\n');
        std::fs::write(&self.path, body).map_err(|error| error.to_string())?;
        self.flushed = true;
        Ok(())
    }
    /// 追加一条 `compaction` 条目（ref `session-manager.ts:1259-1285` 的 `appendCompaction`）。
    ///
    /// 字段与 ref 一致：`type/id/parentId/timestamp/summary/firstKeptEntryId/tokensBefore/details/usage/fromHook/systemMessage`。
    /// - `first_kept_entry_id` 为 `None` 时用**本条自己的 id**（ref 的 `?? id`）；
    /// - `details`/`usage` 为 `None` 时不写该键（ref 传 `undefined`，`JSON.stringify` 会丢掉）；
    /// - `fromHook` 恒 `false`（pi 自己生成的压缩；golden/S21 的条目里就是 `false`）；
    /// - `systemMessage` 由调用方给**原始 JSON 文本**（ref 用当前投影的最后一条 system）。
    ///   走 [`pi_core::session_io::SessionFile::append_with_raw_system_message`]：`serde_json::Map`
    ///   会把 `toolsAdded[].parameters` 的键序排序掉，而 TS 的键序参与 A 层判等（`Message::System`
    ///   的 `__toolsAddedRaw`/`__sectionsRaw` 是同一套 `RawValue` 机制）。
    ///
    /// 追加一条 `branch_summary` 条目（`/tree` 选 `Summarize` 后由摘要请求产生）。
    ///
    /// 字段照 TS 会话实测：`{type, id, parentId, timestamp, fromId, details:{readFiles,modifiedFiles},
    /// usage, fromHook:false, summary}`（`fromHook` 恒 `false`——不是扩展钩子生成的）。
    ///
    /// # Errors
    /// 写文件失败时返回错误描述。
    pub fn append_branch_summary(
        &mut self,
        from_id: &str,
        summary: &str,
        usage_input: u64,
        usage_output: u64,
    ) -> Result<(), String> {
        let (entry_id, parent_id) = self.take_id_and_parent();
        let mut usage = serde_json::Map::new();
        usage.insert("input".to_string(), serde_json::Value::from(usage_input));
        usage.insert("output".to_string(), serde_json::Value::from(usage_output));
        usage.insert("cacheRead".to_string(), serde_json::Value::from(0));
        usage.insert("cacheWrite".to_string(), serde_json::Value::from(0));
        usage.insert("reasoning".to_string(), serde_json::Value::from(0));
        usage.insert(
            "totalTokens".to_string(),
            serde_json::Value::from(usage_input + usage_output),
        );
        let mut cost = serde_json::Map::new();
        for key in ["input", "output", "cacheRead", "cacheWrite", "total"] {
            cost.insert(key.to_string(), serde_json::Value::from(0));
        }
        usage.insert("cost".to_string(), serde_json::Value::Object(cost));

        let mut details = serde_json::Map::new();
        details.insert(
            "readFiles".to_string(),
            serde_json::Value::Array(Vec::new()),
        );
        details.insert(
            "modifiedFiles".to_string(),
            serde_json::Value::Array(Vec::new()),
        );

        let mut object = serde_json::Map::new();
        object.insert(
            "type".to_string(),
            serde_json::Value::String("branch_summary".into()),
        );
        object.insert("id".to_string(), serde_json::Value::String(entry_id));
        object.insert(
            "parentId".to_string(),
            parent_id.map_or(serde_json::Value::Null, serde_json::Value::String),
        );
        object.insert(
            "timestamp".to_string(),
            serde_json::Value::String(timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis())),
        );
        object.insert(
            "fromId".to_string(),
            serde_json::Value::String(from_id.to_string()),
        );
        object.insert("details".to_string(), serde_json::Value::Object(details));
        object.insert("usage".to_string(), serde_json::Value::Object(usage));
        object.insert("fromHook".to_string(), serde_json::Value::Bool(false));
        object.insert(
            "summary".to_string(),
            serde_json::Value::String(summary.to_string()),
        );
        self.append_entry(&SessionEntry::Unknown(serde_json::Value::Object(object)))
    }

    pub fn append_compaction(
        &mut self,
        summary: &str,
        first_kept_entry_id: Option<&str>,
        tokens_before: u64,
        details: Option<serde_json::Value>,
        usage: Option<serde_json::Value>,
        system_message_raw: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let (id, parent_id) = self.take_id_and_parent();
        let mut extra = serde_json::Map::new();
        extra.insert("fromHook".to_string(), serde_json::Value::Bool(false));
        if let Some(details) = details {
            extra.insert("details".to_string(), details);
        }
        if let Some(usage) = usage {
            extra.insert("usage".to_string(), usage);
        }
        if let Some(raw) = system_message_raw {
            // 内存里保留解析后的对象（投影要读它）；**写出的那一行**用原文（保序）。
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
                extra.insert("systemMessage".to_string(), value);
            }
        }
        let entry = SessionEntry::Compaction {
            id: id.clone(),
            parent_id,
            timestamp: timefmt::unix_millis_to_iso8601(timefmt::now_unix_millis()),
            summary: summary.to_string(),
            first_kept_entry_id: first_kept_entry_id.unwrap_or(&id).to_string(),
            tokens_before,
            extra,
        };
        match system_message_raw {
            Some(raw) => {
                self.kinds.push(entry.kind().to_string());
                pi_core::session_io::SessionFile::append_with_raw_system_message(
                    &self.path, &entry, raw,
                )
                .map_err(|error| error.to_string())?;
            }
            None => self.append_entry(&entry)?,
        }
        serde_json::to_value(&entry).map_err(|error| error.to_string())
    }
}

/// 按 ref `footer.ts:86-105` 汇总**整个会话**的用量（footer 显示的是累计值）：
/// - `message` 且 `role == assistant` → `message.usage`
/// - `message` 且 `role == toolResult` 且带 `usage` → `message.usage`
/// - `branch_summary` / `compaction` 且带 `usage` → `entry.usage`
///
/// 返回 `(input, output, cost)`。读不到的条目按 0 计（不 panic、不猜测）。
#[must_use]
pub fn session_usage_totals(path: &std::path::Path) -> (u64, u64, f64) {
    let Ok(file) = pi_core::session_io::SessionFile::read(path) else {
        return (0, 0, 0.0);
    };
    let mut input = 0_u64;
    let mut output = 0_u64;
    let mut cost = 0.0_f64;
    for entry in &file.entries {
        let usage = match entry {
            pi_core::session::SessionEntry::Message { message, .. } => match message {
                pi_ai::message::Message::Assistant(assistant) => Some(assistant.usage),
                pi_ai::message::Message::ToolResult { extra, .. } => extra
                    .get("usage")
                    .and_then(|value| serde_json::from_value::<pi_ai::Usage>(value.clone()).ok()),
                _other => None,
            },
            pi_core::session::SessionEntry::BranchSummary { extra, .. }
            | pi_core::session::SessionEntry::Compaction { extra, .. } => extra
                .get("usage")
                .and_then(|value| serde_json::from_value::<pi_ai::Usage>(value.clone()).ok()),
            _other => None,
        };
        if let Some(usage) = usage {
            input = input.saturating_add(usage.input);
            output = output.saturating_add(usage.output);
            cost += usage.cost.total;
        }
    }
    (input, output, cost)
}
