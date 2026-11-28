//! 键位定义与用户覆盖（ref `packages/tui/src/keybindings.ts` +
//! `packages/coding-agent/src/core/keybindings.ts`）。
//!
//! 为什么需要这一层：pi-rs 的键位此前**全部硬编码在 `tui.rs` 的 `match` 里**，
//! `keybindings.json` 读都不读。后果有三层：
//! 1. 用户自定义的键位完全不生效（文件存在也没任何效果，且无提示）；
//! 2. `/hotkeys` 面板是对 golden 的逐字转录（编译期常量），与实际键位会脱节；
//! 3. `/reload` 的状态行宣称 "Reloaded keybindings"，实际什么都没重载。
//!
//! 本模块只做**纯转换**：定义表 + 用户覆盖 → 每个动作的生效键列表。
//! 具体按键分派仍在调用方（`tui.rs`），它按 `keys_for(action)` 判断，
//! 做到"改配置即改行为"。

use std::collections::BTreeMap;

/// 一个动作的默认键（`KeyId` 的字符串形式，如 `ctrl+p`、`alt+up`）。
///
/// 多个键是**任一命中生效**（ref `KeybindingsManager.matches` 遍历 `keysById`）。
pub struct Definition {
    pub action: &'static str,
    pub default_keys: &'static [&'static str],
    pub description: &'static str,
}

/// 定义表（ref `TUI_KEYBINDINGS` + `KEYBINDINGS` 的并集，逐项转写）。
///
/// 只收 pi-rs **实际会分派**的动作与 `/hotkeys` 面板会展示的动作；
/// ref 的 altScreen 一族（全屏视口翻页等）pi-rs 未实现，故不收
/// （见 LIMITATIONS 第 24 条的后继条目）。
pub const DEFINITIONS: &[Definition] = &[
    // 编辑器导航（ref `keybindings.ts:71-104`）
    Definition {
        action: "tui.editor.cursorUp",
        default_keys: &["up"],
        description: "Move cursor up",
    },
    Definition {
        action: "tui.editor.cursorDown",
        default_keys: &["down"],
        description: "Move cursor down",
    },
    Definition {
        action: "tui.editor.cursorLeft",
        default_keys: &["left", "ctrl+b"],
        description: "Move cursor left",
    },
    Definition {
        action: "tui.editor.cursorRight",
        default_keys: &["right", "ctrl+f"],
        description: "Move cursor right",
    },
    Definition {
        action: "tui.editor.cursorWordLeft",
        default_keys: &["alt+left", "ctrl+left", "alt+b"],
        description: "Move cursor word left",
    },
    Definition {
        action: "tui.editor.cursorWordRight",
        default_keys: &["alt+right", "ctrl+right", "alt+f"],
        description: "Move cursor word right",
    },
    Definition {
        action: "tui.editor.cursorLineStart",
        default_keys: &["home", "ctrl+home", "ctrl+a"],
        description: "Move to line start",
    },
    Definition {
        action: "tui.editor.cursorLineEnd",
        default_keys: &["end", "ctrl+end", "ctrl+e"],
        description: "Move to line end",
    },
    Definition {
        action: "tui.editor.jumpForward",
        default_keys: &["ctrl+]"],
        description: "Jump forward to character",
    },
    Definition {
        action: "tui.editor.jumpBackward",
        default_keys: &["ctrl+alt+]"],
        description: "Jump backward to character",
    },
    Definition {
        action: "tui.editor.pageUp",
        default_keys: &["pageUp", "ctrl+pageUp"],
        description: "Page up",
    },
    Definition {
        action: "tui.editor.pageDown",
        default_keys: &["pageDown", "ctrl+pageDown"],
        description: "Page down",
    },
    // 编辑（ref `:105-143`）
    Definition {
        action: "tui.editor.deleteCharBackward",
        default_keys: &["backspace"],
        description: "Delete character backward",
    },
    Definition {
        action: "tui.editor.deleteCharForward",
        default_keys: &["delete", "ctrl+d"],
        description: "Delete character forward",
    },
    Definition {
        action: "tui.editor.deleteWordBackward",
        default_keys: &["ctrl+w", "alt+backspace"],
        description: "Delete word backward",
    },
    Definition {
        action: "tui.editor.deleteWordForward",
        default_keys: &["alt+d", "alt+delete"],
        description: "Delete word forward",
    },
    Definition {
        action: "tui.editor.deleteToLineStart",
        default_keys: &["ctrl+u"],
        description: "Delete to line start",
    },
    Definition {
        action: "tui.editor.deleteToLineEnd",
        default_keys: &["ctrl+k"],
        description: "Delete to line end",
    },
    Definition {
        action: "tui.editor.yank",
        default_keys: &["ctrl+y"],
        description: "Yank",
    },
    Definition {
        action: "tui.editor.yankPop",
        default_keys: &["alt+y"],
        description: "Yank pop",
    },
    Definition {
        action: "tui.editor.undo",
        // ref `core/keybindings.ts:72-76`：linux（非 WSL）默认 `ctrl+-`。
        default_keys: &["ctrl+-"],
        description: "Undo",
    },
    // 输入（ref `:144-147`）
    Definition {
        action: "tui.input.newLine",
        default_keys: &["shift+enter", "ctrl+j"],
        description: "Insert newline",
    },
    Definition {
        action: "tui.input.submit",
        default_keys: &["enter"],
        description: "Submit input",
    },
    Definition {
        action: "tui.input.tab",
        default_keys: &["tab"],
        description: "Tab / autocomplete",
    },
    Definition {
        action: "tui.input.copy",
        default_keys: &["ctrl+c"],
        description: "Copy selection",
    },
    // 选择器（ref `:148-156`）
    Definition {
        action: "tui.select.up",
        default_keys: &["up"],
        description: "Move selection up",
    },
    Definition {
        action: "tui.select.down",
        default_keys: &["down"],
        description: "Move selection down",
    },
    Definition {
        action: "tui.select.pageUp",
        default_keys: &["pageUp"],
        description: "Selection page up",
    },
    Definition {
        action: "tui.select.pageDown",
        default_keys: &["pageDown"],
        description: "Selection page down",
    },
    Definition {
        action: "tui.select.confirm",
        default_keys: &["enter"],
        description: "Confirm selection",
    },
    Definition {
        action: "tui.select.cancel",
        default_keys: &["escape", "ctrl+c"],
        description: "Cancel selection",
    },
    // 应用级（ref `core/keybindings.ts:80-236`）
    Definition {
        action: "app.interrupt",
        default_keys: &["escape"],
        description: "Interrupt the current operation",
    },
    Definition {
        action: "app.clear",
        // ref `core/keybindings.ts:94`：`app.clear` 是 **ctrl+c**（不是 ctrl+l；
        // ctrl+l 是 `app.model.select`）。
        default_keys: &["ctrl+c"],
        description: "Clear editor",
    },
    Definition {
        action: "app.exit",
        default_keys: &["ctrl+d"],
        description: "Exit pi",
    },
    Definition {
        action: "app.suspend",
        default_keys: &["ctrl+z"],
        description: "Suspend pi",
    },
    Definition {
        action: "app.thinking.cycle",
        default_keys: &["shift+tab"],
        description: "Cycle thinking level",
    },
    Definition {
        action: "app.model.cycleForward",
        default_keys: &["ctrl+p"],
        description: "Cycle to the next model",
    },
    Definition {
        action: "app.model.cycleBackward",
        default_keys: &["shift+ctrl+p"],
        description: "Cycle to the previous model",
    },
    Definition {
        action: "app.model.select",
        // ref `:116`：`app.model.select` = **ctrl+l**。
        default_keys: &["ctrl+l"],
        description: "Open model selector",
    },
    Definition {
        action: "app.editor.external",
        default_keys: &["ctrl+g"],
        description: "Open external editor",
    },
    Definition {
        action: "app.message.followUp",
        default_keys: &["alt+enter"],
        description: "Queue follow-up message",
    },
    Definition {
        action: "app.message.dequeue",
        // ref `:137-139`：非 Windows 是 `alt+up`。
        default_keys: &["alt+up"],
        description: "Restore queued messages",
    },
    Definition {
        action: "app.clipboard.pasteImage",
        // ref `:141-143`：非 Windows 是 `ctrl+v`。
        default_keys: &["ctrl+v"],
        description: "Paste image or text from clipboard",
    },
    Definition {
        action: "app.tools.expand",
        default_keys: &["ctrl+o"],
        description: "Expand or collapse tool output",
    },
    Definition {
        action: "app.thinking.toggle",
        default_keys: &["ctrl+t"],
        description: "Show or hide thinking",
    },
    Definition {
        action: "app.message.copy",
        default_keys: &["ctrl+x"],
        description: "Copy last agent message",
    },
    Definition {
        action: "app.session.new",
        // ref `:144`：`defaultKeys: []`（无默认键）。
        default_keys: &[],
        description: "Start a new session",
    },
    Definition {
        action: "app.session.tree",
        // ref `:145`：`defaultKeys: []`。
        default_keys: &[],
        description: "Open session tree",
    },
    Definition {
        action: "app.models.save",
        default_keys: &["ctrl+s"],
        description: "Save model selection",
    },
    Definition {
        action: "app.models.enableAll",
        default_keys: &["ctrl+a"],
        description: "Enable all models",
    },
    Definition {
        action: "app.models.clearAll",
        default_keys: &["ctrl+x"],
        description: "Clear all models",
    },
    Definition {
        action: "app.models.toggleProvider",
        default_keys: &["ctrl+p"],
        description: "Toggle all models for provider",
    },
    Definition {
        action: "app.models.reorderUp",
        default_keys: &["alt+up"],
        description: "Move model up in order",
    },
    Definition {
        action: "app.models.reorderDown",
        default_keys: &["alt+down"],
        description: "Move model down in order",
    },
];

/// 旧动作名 → 新动作名（ref `core/keybindings.ts:239-297` 的 `KEYBINDING_NAME_MIGRATIONS`）。
///
/// 只收与 [`DEFINITIONS`] 有交集的部分：迁移的目的就是"让旧配置继续生效"，
/// 目标动作不在定义表里的迁移项对 pi-rs 无意义。
pub const NAME_MIGRATIONS: &[(&str, &str)] = &[
    ("cursorUp", "tui.editor.cursorUp"),
    ("cursorDown", "tui.editor.cursorDown"),
    ("cursorLeft", "tui.editor.cursorLeft"),
    ("cursorRight", "tui.editor.cursorRight"),
    ("cursorWordLeft", "tui.editor.cursorWordLeft"),
    ("cursorWordRight", "tui.editor.cursorWordRight"),
    ("cursorLineStart", "tui.editor.cursorLineStart"),
    ("cursorLineEnd", "tui.editor.cursorLineEnd"),
    ("pageUp", "tui.editor.pageUp"),
    ("pageDown", "tui.editor.pageDown"),
    ("deleteCharBackward", "tui.editor.deleteCharBackward"),
    ("deleteCharForward", "tui.editor.deleteCharForward"),
    ("deleteWordBackward", "tui.editor.deleteWordBackward"),
    ("deleteWordForward", "tui.editor.deleteWordForward"),
    ("deleteToLineStart", "tui.editor.deleteToLineStart"),
    ("deleteToLineEnd", "tui.editor.deleteToLineEnd"),
    ("yank", "tui.editor.yank"),
    ("undo", "tui.editor.undo"),
    ("newLine", "tui.input.newLine"),
    ("submit", "tui.input.submit"),
    ("tab", "tui.input.tab"),
    ("copy", "tui.input.copy"),
    ("selectUp", "tui.select.up"),
    ("selectDown", "tui.select.down"),
    ("selectPageUp", "tui.select.pageUp"),
    ("selectPageDown", "tui.select.pageDown"),
    ("selectConfirm", "tui.select.confirm"),
    ("selectCancel", "tui.select.cancel"),
    ("interrupt", "app.interrupt"),
    ("clear", "app.clear"),
    ("exit", "app.exit"),
    ("suspend", "app.suspend"),
    ("cycleThinkingLevel", "app.thinking.cycle"),
    ("cycleModelForward", "app.model.cycleForward"),
    ("cycleModelBackward", "app.model.cycleBackward"),
    ("selectModel", "app.model.select"),
    ("expandTools", "app.tools.expand"),
    ("toggleThinking", "app.thinking.toggle"),
    ("newSession", "app.session.new"),
    ("tree", "app.session.tree"),
];

/// 生效的键位表（动作 → 键列表）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    keys: BTreeMap<String, Vec<String>>,
    /// 用户配置里与定义表冲突的键（同一键被多个动作认领，ref `getConflicts`）。
    conflicts: Vec<(String, Vec<String>)>,
}

impl Keymap {
    /// 只有默认值的键位表。
    #[must_use]
    pub fn defaults() -> Self {
        Self::with_overrides(&BTreeMap::new())
    }

    /// 默认值 + 用户覆盖。
    ///
    /// 覆盖语义照 ref `KeybindingsManager.rebuild`（`packages/tui/src/keybindings.ts:243-268`）：
    /// - 键在用户配置里**出现**（哪怕值是空数组）⇒ 用用户值（空数组 = 禁用该动作）；
    /// - 未出现 ⇒ 用默认值；
    /// - 用户配置里**不在定义表**的动作名被忽略。
    #[must_use]
    pub fn with_overrides(overrides: &BTreeMap<String, Vec<String>>) -> Self {
        let mut keys = BTreeMap::new();
        for definition in DEFINITIONS {
            let value = overrides.get(definition.action).map_or_else(
                || {
                    definition
                        .default_keys
                        .iter()
                        .map(|key| (*key).to_string())
                        .collect()
                },
                Clone::clone,
            );
            keys.insert(definition.action.to_string(), value);
        }
        // 冲突检测：只统计**用户认领**的键（ref 只把 userClaims 计入 conflicts，
        // 默认值之间的重复不算冲突——`ctrl+p` 同时是模型循环与 scoped-models 的默认键）。
        let mut claims: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (action, key_list) in &keys {
            if !overrides.contains_key(action.as_str()) {
                continue;
            }
            for key in key_list {
                claims
                    .entry(key.as_str())
                    .or_default()
                    .push(action.as_str());
            }
        }
        let conflicts = claims
            .into_iter()
            .filter(|(_, actions)| actions.len() > 1)
            .map(|(key, actions)| {
                (
                    key.to_string(),
                    actions.into_iter().map(str::to_string).collect(),
                )
            })
            .collect();
        Self { keys, conflicts }
    }

    /// 生效键表是否与**纯默认值**完全相同（用于"默认情形走 golden 常量"的分叉）。
    #[must_use]
    pub fn is_all_defaults(&self) -> bool {
        DEFINITIONS.iter().all(|definition| {
            let keys = self.keys_for(definition.action);
            keys.len() == definition.default_keys.len()
                && definition
                    .default_keys
                    .iter()
                    .zip(keys.iter())
                    .all(|(default, actual)| default == actual)
        })
    }

    /// 某动作的生效键列表（未知动作 ⇒ 空）。
    #[must_use]
    pub fn keys_for(&self, action: &str) -> &[String] {
        self.keys.get(action).map_or(&[], Vec::as_slice)
    }

    /// 用户配置里的冲突（同一键被多个动作认领）。
    #[must_use]
    pub fn conflicts(&self) -> &[(String, Vec<String>)] {
        &self.conflicts
    }

    /// 显示文本（ref `keyDisplayText`，`keybinding-hints.ts:41-43`）：
    /// 每个键首字母大写，多键用 `/` 连接（如 `Alt+Up/Alt+Down`）。
    #[must_use]
    pub fn display(&self, action: &str) -> String {
        self.keys_for(action)
            .iter()
            .map(|key| {
                key.split('+')
                    // linux（非 darwin）不做 `alt` → `option` 替换（`keybinding-hints.ts:15`）。
                    .map(capitalize)
                    .collect::<Vec<String>>()
                    .join("+")
            })
            .collect::<Vec<String>>()
            .join("/")
    }
}

fn capitalize(part: &str) -> String {
    let mut chars = part.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// 把一个配置值（字符串或字符串数组）读成键列表；非法形状 ⇒ `None`（ref `toKeybindingsConfig`）。
#[must_use]
pub fn coerce_keys(value: &serde_json::Value) -> Option<Vec<String>> {
    match value {
        serde_json::Value::String(text) => Some(vec![text.clone()]),
        serde_json::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(item.as_str()?.to_string());
            }
            Some(out)
        }
        _other => None,
    }
}

/// 把用户配置对象迁移旧名并归一（ref `migrateKeybindingsConfig`）。
///
/// 语义细节（`:322-344`）：
/// - 旧名迁移到新名；若**新名已存在**，保留新名、丢弃迁移项（`continue`）；
/// - 非法值（非字符串/非字符串数组）被丢弃；
/// - 返回 `(归一后的配置, 是否发生过迁移)`。
#[must_use]
pub fn migrate_config(
    raw: &serde_json::Map<String, serde_json::Value>,
) -> (BTreeMap<String, Vec<String>>, bool) {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut migrated = false;
    for (key, value) in raw {
        let next = NAME_MIGRATIONS
            .iter()
            .find(|(legacy, _new)| legacy == key)
            .map_or_else(|| key.clone(), |(_legacy, new)| (*new).to_string());
        if next != *key {
            migrated = true;
            // 新名已存在 ⇒ 丢弃迁移项（ref `:335-338`）。
            if raw.contains_key(&next) {
                continue;
            }
        }
        let Some(keys) = coerce_keys(value) else {
            continue;
        };
        out.insert(next, keys);
    }
    (out, migrated)
}

/// 解析 `keybindings.json` 文本。
///
/// # Errors
/// 只对"不是 JSON / 不是对象"报错；其它形状问题按 ref 的宽容语义丢弃
/// （ref `loadRawConfig` 解析失败直接返回 `undefined`，即"全部用默认值"）。
pub fn parse_config(text: &str) -> Result<(BTreeMap<String, Vec<String>>, bool), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let parsed: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("keybindings.json 不是合法 JSON：{error}"))?;
    let Some(object) = parsed.as_object() else {
        return Err("keybindings.json 顶层必须是对象".to_string());
    };
    Ok(migrate_config(object))
}

/// 从 `agent_dir/keybindings.json` 读键位（文件不存在 ⇒ 全默认）。
///
/// # Errors
/// 文件存在但读不出或格式非法时返回错误消息。
pub fn load(agent_dir: &std::path::Path) -> Result<Keymap, String> {
    let path = agent_dir.join("keybindings.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let (overrides, _migrated) = parse_config(&text)?;
            Ok(Keymap::with_overrides(&overrides))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Keymap::defaults()),
        Err(error) => Err(format!("{} 读取失败：{error}", path.display())),
    }
}

/// `/hotkeys` 面板的 Markdown 源（ref `interactive-mode.ts:6566-6640` 的模板逐行转写）。
///
/// 与 ref 一致的：三段（Navigation / Editing / Other）、每段一张 GFM 表、
/// 键显示来自**当前生效键位**（`Keymap::display`），而不是编译期常量。
/// `***bold***` 之外的行内代码与表格排版交给 `render_markdown_styled_with_width`。
///
/// **不收**：ref 末尾的 `Extensions` 段（扩展注册的快捷键）与 Windows 专属后缀
/// —— 前者需要扩展注册表接入（未实现，见 LIMITATIONS），后者在 linux 上不出现。
#[must_use]
pub fn hotkeys_markdown(keymap: &Keymap) -> String {
    let editor = |action: &str| keymap.display(action);
    format!(
        "**Navigation**
| Key | Action |
|-----|--------|
| `{cursor_up}` / `{cursor_down}` / `{cursor_left}` / `{cursor_right}` | Move cursor / browse history |
| `{word_left}` / `{word_right}` | Move by word |
| `{line_start}` | Start of line |
| `{line_end}` | End of line |
| `{jump_fwd}` | Jump forward to character |
| `{jump_back}` | Jump backward to character |
| `{page_up}` / `{page_down}` | Scroll by page |

**Editing**
| Key | Action |
|-----|--------|
| `{submit}` | Send message |
| `{new_line}` | New line |
| `{del_word_back}` | Delete word backwards |
| `{del_word_fwd}` | Delete word forwards |
| `{del_line_start}` | Delete to start of line |
| `{del_line_end}` | Delete to end of line |
| `{yank}` | Paste the most-recently-deleted text |
| `{yank_pop}` | Cycle through the deleted text after pasting |
| `{undo}` | Undo |
| `{tab}` | Path completion / accept autocomplete |

**Other**
| Key | Action |
|-----|--------|
| `{interrupt}` | Cancel autocomplete / abort streaming |
| `{clear}` | Clear editor (first) / exit (second) |
| `{exit}` | Exit (when editor is empty) |
| `{suspend}` | Suspend to background |
| `{cycle_thinking}` | Cycle thinking level |
| `{cycle_fwd}` / `{cycle_back}` | Cycle models |
| `{select_model}` | Open model selector |
| `{expand_tools}` | Toggle tool output expansion |
| `{toggle_thinking}` | Toggle thinking block visibility |
| `{external_editor}` | Edit message in external editor |
| `{copy_message}` | Copy selection or last assistant message |
| `{follow_up}` | Queue follow-up message |
| `{dequeue}` | Restore queued messages |
| `{paste_image}` | Paste image or text from clipboard |
| `/` | Slash commands |
| `!` | Run bash command |
| `!!` | Run bash command (excluded from context) |
",
        cursor_up = editor("tui.editor.cursorUp"),
        cursor_down = editor("tui.editor.cursorDown"),
        cursor_left = editor("tui.editor.cursorLeft"),
        cursor_right = editor("tui.editor.cursorRight"),
        word_left = editor("tui.editor.cursorWordLeft"),
        word_right = editor("tui.editor.cursorWordRight"),
        line_start = editor("tui.editor.cursorLineStart"),
        line_end = editor("tui.editor.cursorLineEnd"),
        jump_fwd = editor("tui.editor.jumpForward"),
        jump_back = editor("tui.editor.jumpBackward"),
        page_up = editor("tui.editor.pageUp"),
        page_down = editor("tui.editor.pageDown"),
        submit = editor("tui.input.submit"),
        new_line = editor("tui.input.newLine"),
        del_word_back = editor("tui.editor.deleteWordBackward"),
        del_word_fwd = editor("tui.editor.deleteWordForward"),
        del_line_start = editor("tui.editor.deleteToLineStart"),
        del_line_end = editor("tui.editor.deleteToLineEnd"),
        yank = editor("tui.editor.yank"),
        yank_pop = editor("tui.editor.yankPop"),
        undo = editor("tui.editor.undo"),
        tab = editor("tui.input.tab"),
        interrupt = editor("app.interrupt"),
        clear = editor("app.clear"),
        exit = editor("app.exit"),
        suspend = editor("app.suspend"),
        cycle_thinking = editor("app.thinking.cycle"),
        cycle_fwd = editor("app.model.cycleForward"),
        cycle_back = editor("app.model.cycleBackward"),
        select_model = editor("app.model.select"),
        expand_tools = editor("app.tools.expand"),
        toggle_thinking = editor("app.thinking.toggle"),
        external_editor = editor("app.editor.external"),
        copy_message = editor("app.message.copy"),
        follow_up = editor("app.message.followUp"),
        dequeue = editor("app.message.dequeue"),
        paste_image = editor("app.clipboard.pasteImage"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(action, keys)| {
                (
                    (*action).to_string(),
                    keys.iter().map(|key| (*key).to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn defaults_come_from_the_definition_table() {
        let map = Keymap::defaults();
        assert_eq!(map.keys_for("app.model.cycleForward"), ["ctrl+p"]);
        assert_eq!(map.keys_for("tui.editor.cursorLeft"), ["left", "ctrl+b"]);
        assert_eq!(map.keys_for("app.models.reorderUp"), ["alt+up"]);
        assert!(map.keys_for("no.such.action").is_empty());
    }

    #[test]
    fn user_override_replaces_the_whole_list() {
        let map = Keymap::with_overrides(&overrides(&[("app.model.cycleForward", &["ctrl+n"])]));
        assert_eq!(map.keys_for("app.model.cycleForward"), ["ctrl+n"]);
        // 其它动作不受影响
        assert_eq!(map.keys_for("tui.input.submit"), ["enter"]);
    }

    #[test]
    fn empty_override_disables_the_action() {
        let map = Keymap::with_overrides(&overrides(&[("app.tools.expand", &[])]));
        assert!(
            map.keys_for("app.tools.expand").is_empty(),
            "空数组 = 禁用（ref `normalizeKeys` 允许空）"
        );
    }

    #[test]
    fn unknown_actions_are_ignored() {
        let map = Keymap::with_overrides(&overrides(&[("app.not.real", &["ctrl+q"])]));
        assert!(map.keys_for("app.not.real").is_empty());
        assert_eq!(map.keys_for("tui.input.submit"), ["enter"]);
    }

    #[test]
    fn display_capitalizes_each_key_and_joins_with_slash() {
        let map = Keymap::with_overrides(&overrides(&[
            ("app.models.reorderUp", &["alt+up"]),
            ("app.models.reorderDown", &["alt+down"]),
            ("tui.editor.cursorLeft", &["left", "ctrl+b"]),
        ]));
        assert_eq!(map.display("app.models.reorderUp"), "Alt+Up");
        assert_eq!(map.display("tui.editor.cursorLeft"), "Left/Ctrl+B");
        // 未知动作 ⇒ 空串（不是"未知键名"）
        assert_eq!(map.display("no.such"), "");
    }

    #[test]
    fn legacy_names_are_migrated() {
        let mut raw = serde_json::Map::new();
        raw.insert(
            "cycleModelForward".to_string(),
            serde_json::Value::String("ctrl+n".to_string()),
        );
        let (config, migrated) = migrate_config(&raw);
        assert!(migrated, "应报告发生过迁移");
        assert_eq!(
            config.get("app.model.cycleForward"),
            Some(&vec!["ctrl+n".to_string()])
        );
        assert!(!config.contains_key("cycleModelForward"), "旧名不应保留");
    }

    #[test]
    fn new_name_wins_over_migrated_legacy_name() {
        let mut raw = serde_json::Map::new();
        raw.insert(
            "cycleModelForward".to_string(),
            serde_json::Value::String("ctrl+n".to_string()),
        );
        raw.insert(
            "app.model.cycleForward".to_string(),
            serde_json::Value::String("ctrl+m".to_string()),
        );
        let (config, _migrated) = migrate_config(&raw);
        assert_eq!(
            config.get("app.model.cycleForward"),
            Some(&vec!["ctrl+m".to_string()]),
            "新名优先（ref `:335-338` 的 continue）"
        );
    }

    #[test]
    fn invalid_values_are_dropped() {
        let mut raw = serde_json::Map::new();
        raw.insert("app.model.cycleForward".to_string(), serde_json::json!(42));
        raw.insert(
            "app.tools.expand".to_string(),
            serde_json::json!(["ctrl+o", 7]),
        );
        let (config, _migrated) = migrate_config(&raw);
        assert!(
            !config.contains_key("app.model.cycleForward"),
            "数字值应丢弃"
        );
        assert!(
            !config.contains_key("app.tools.expand"),
            "含非字符串的数组应丢弃"
        );
    }

    #[test]
    fn parse_reports_malformed_input() {
        assert!(parse_config("{ not json").is_err());
        assert!(parse_config("[]").is_err());
        let (config, migrated) =
            parse_config(r#"{"app.model.cycleForward":"ctrl+n"}"#).expect("应解析");
        assert!(!migrated);
        assert_eq!(
            config.get("app.model.cycleForward"),
            Some(&vec!["ctrl+n".to_string()])
        );
    }

    #[test]
    fn bom_is_stripped() {
        let (config, _migrated) =
            parse_config("\u{feff}{\"app.model.cycleForward\":\"ctrl+n\"}").expect("应解析");
        assert_eq!(
            config.get("app.model.cycleForward"),
            Some(&vec!["ctrl+n".to_string()])
        );
    }

    #[test]
    fn conflicts_are_reported_only_for_user_claims() {
        // 默认值里 `ctrl+p` 同时属于 app.model.cycleForward 与 app.models.toggleProvider，
        // 但那不是"用户制造的冲突"（ref 只统计 userClaims）。
        let defaults = Keymap::defaults();
        assert!(defaults.conflicts().is_empty(), "默认值不应报冲突");
        let map = Keymap::with_overrides(&overrides(&[
            ("app.model.cycleForward", &["ctrl+y"]),
            ("app.tools.expand", &["ctrl+y"]),
        ]));
        let conflicts = map.conflicts();
        assert_eq!(conflicts.len(), 1, "应报一处冲突：{conflicts:?}");
        assert_eq!(conflicts[0].0, "ctrl+y");
        let mut actions = conflicts[0].1.clone();
        actions.sort();
        assert_eq!(actions, vec!["app.model.cycleForward", "app.tools.expand"]);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let map = load(tmp.path()).expect("应成功");
        assert_eq!(map, Keymap::defaults());
    }

    #[test]
    fn file_is_read_and_applied() {
        let tmp = tempfile::tempdir().expect("临时目录");
        std::fs::write(
            tmp.path().join("keybindings.json"),
            r#"{"app.model.cycleForward":["ctrl+n","ctrl+m"]}"#,
        )
        .expect("写");
        let map = load(tmp.path()).expect("应成功");
        assert_eq!(map.keys_for("app.model.cycleForward"), ["ctrl+n", "ctrl+m"]);
    }
}
