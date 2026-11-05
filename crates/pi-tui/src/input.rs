//! 键盘输入层的**纯函数**移植（规格：`ref/pi-0.87.1/packages/tui/src/{stdin-buffer.ts,keys.ts}`）。
//!
//! **本批不接线**：`pi-cli` 仍用 crossterm 事件；本模块只提供纯函数与夹具驱动的单测。
//!
//! 设计：无全局可变状态、无定时器。超时以"显式时间 + 下次唤醒时刻"表达：
//! `process()` 返回 `(事件, Option<下次唤醒的绝对毫秒>)`，调用方在到期时调用 [`InputState::flush`]。
//!
//! **线程安全**：`InputState` 为纯数据（`Send + Sync`），函数无共享状态。

/// 输入事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// 一个完整序列（`stdin-buffer.ts` 的 `data` 事件）。
    Data(String),
    /// 括号粘贴的整段内容（`stdin-buffer.ts` 的 `paste` 事件）。
    Paste(String),
}

/// 孤立 ESC 的超时（`stdin-buffer.ts:24` `DEFAULT_ESCAPE_TIMEOUT_MS = 10`）。
pub const ESCAPE_TIMEOUT_MS: u64 = 10;
/// 普通未完成序列的超时（`DEFAULT_SEQUENCE_TIMEOUT_MS`）。
pub const SEQUENCE_TIMEOUT_MS: u64 = 100;

const ESC: char = '\u{1b}';
const PASTE_START: &str = "\u{1b}[200~";
const PASTE_END: &str = "\u{1b}[201~";

/// 切分状态（对应 `StdinBuffer` 的私有字段，`stdin-buffer.ts:282-288`）。
#[derive(Debug, Default, Clone)]
pub struct InputState {
    /// **字节缓冲**（P7-6al/B）：多字节字符可能跨 `read()` 边界，
    /// 必须按字节累积、只在完整 UTF-8 前缀上切分（旧实现用 `from_utf8_lossy` 会切出 U+FFFD，
    /// 并在粘贴分支按字节偏移 `drain` 到字符中间而 panic）。
    buffer: Vec<u8>,
    paste_mode: bool,
    paste_buffer: String,
    /// ↔ `keys.ts:25` 的 `_kittyProtocolActive`（显式传入，不用全局状态）。
    pub kitty_active: bool,
}

impl InputState {
    /// 新建状态。
    #[must_use]
    pub fn new(kitty_active: bool) -> Self {
        Self {
            kitty_active,
            ..Self::default()
        }
    }

    /// 是否处于括号粘贴累积中。
    #[must_use]
    pub fn is_pasting(&self) -> bool {
        self.paste_mode
    }

    /// 喂入一段字节（对应 `StdinBuffer.process`，`stdin-buffer.ts:296`）。
    ///
    /// 返回本批事件与"下次唤醒时刻"（`Some(now + timeout)`，`None` 表示缓冲已空、无需唤醒）。
    pub fn process(&mut self, chunk: &[u8], now_ms: u64) -> (Vec<InputEvent>, Option<u64>) {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        self.drain(&mut events);
        let wake = if self.buffer.is_empty() {
            None
        } else if self.buffer == [0x1b] {
            Some(now_ms + ESCAPE_TIMEOUT_MS)
        } else {
            Some(now_ms + SEQUENCE_TIMEOUT_MS)
        };
        (events, wake)
    }

    /// 超时触发（对应 `StdinBuffer` 的 `setTimeout(flush)`，`stdin-buffer.ts:388-395`）。
    pub fn flush(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::new();
        if self.paste_mode {
            // 粘贴未闭合：按 ref 的行为把已累积内容作为粘贴发出（`stdin-buffer.ts:337/371` 的超时分支）。
            if !self.paste_buffer.is_empty() {
                events.push(InputEvent::Paste(std::mem::take(&mut self.paste_buffer)));
            }
            self.paste_mode = false;
        }
        if !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            events.push(InputEvent::Data(decode_lossy(&rest)));
        }
        events
    }

    /// 把缓冲里的**完整序列**取出并发出；不完整的留在 `buffer`（`stdin-buffer.ts:194-257`）。
    fn drain(&mut self, events: &mut Vec<InputEvent>) {
        loop {
            if self.buffer.is_empty() {
                return;
            }
            // 括号粘贴：起止标记是纯 ASCII，可在**字节**层查找（UTF-8 续字节 ≥0x80，不会误匹配）。
            if self.paste_mode {
                if let Some(index) = find_sub(&self.buffer, PASTE_END.as_bytes()) {
                    let head = decode_lossy(&self.buffer[..index]);
                    self.paste_buffer.push_str(&head);
                    let rest = self.buffer[index + PASTE_END.len()..].to_vec();
                    self.buffer = rest;
                    self.paste_mode = false;
                    events.push(InputEvent::Paste(std::mem::take(&mut self.paste_buffer)));
                    continue;
                }
                // 未闭合：交出"除尾部可能半截结束标记之外"的**完整 UTF-8 前缀**
                let keep = PASTE_END.len() - 1;
                let take = self.buffer.len().saturating_sub(keep);
                let take = floor_char_boundary(&self.buffer, take);
                if take > 0 {
                    let part = decode_lossy(&self.buffer[..take]);
                    self.buffer.drain(..take);
                    self.paste_buffer.push_str(&part);
                }
                return;
            }
            // 起始标记可能**分块到达**（例如 tmux `send-keys -l` 一次一个字符）：
            // 只要当前缓冲还是 `ESC[200~` 的真前缀，就等后续字节，别当成孤立 ESC。
            // 超时由 `process()` 的 wake/`flush()` 兜底（孤立 Esc 仍会正常发出）。
            if PASTE_START.as_bytes().starts_with(self.buffer.as_slice()) {
                return;
            }
            if self.buffer.starts_with(PASTE_START.as_bytes()) {
                self.buffer.drain(..PASTE_START.len());
                self.paste_mode = true;
                continue;
            }
            if self.buffer[0] == 0x1b {
                let text = match std::str::from_utf8(&self.buffer) {
                    Ok(text) => text,
                    Err(error) if error.error_len().is_none() => {
                        // 尾部是半截 UTF-8：只在有效前缀上判断是否完整序列
                        match std::str::from_utf8(&self.buffer[..error.valid_up_to()]) {
                            Ok(prefix) => prefix,
                            Err(_) => return,
                        }
                    }
                    Err(_) => {
                        // 非法字节：丢弃 1 字节并交出一个替换字符（ref 用 lossy 解码，行为等价）
                        self.buffer.drain(..1);
                        events.push(InputEvent::Data("\u{fffd}".to_string()));
                        continue;
                    }
                };
                // WezTerm 的 `"\x1b\x1b"` 特例：只发第一个 ESC（`stdin-buffer.ts:219-232`）
                if text.starts_with("\u{1b}\u{1b}") {
                    let third = text.chars().nth(2);
                    if matches!(third, Some('[' | ']' | 'O' | 'P' | '_')) {
                        self.buffer.drain(..1);
                        events.push(InputEvent::Data(ESC.to_string()));
                        continue;
                    }
                }
                match complete_len(text) {
                    Some(len) => {
                        let seq = decode_lossy(&self.buffer[..len]);
                        self.buffer.drain(..len);
                        events.push(InputEvent::Data(seq));
                    }
                    // 序列未完整或只有半截 UTF-8 ⇒ 等更多字节
                    None => return,
                }
                continue;
            }
            // 非 ESC 起始：取一个**完整 UTF-8 字符**
            let text = match std::str::from_utf8(&self.buffer) {
                Ok(text) => text,
                Err(error) if error.error_len().is_none() => {
                    // 半截多字节字符 ⇒ 等下次读取
                    match std::str::from_utf8(&self.buffer[..error.valid_up_to()]) {
                        Ok(prefix) if !prefix.is_empty() => prefix,
                        other => {
                            let _ = other;
                            return;
                        }
                    }
                }
                Err(_) => {
                    self.buffer.drain(..1);
                    events.push(InputEvent::Data("\u{fffd}".to_string()));
                    continue;
                }
            };
            let ch = text.chars().next().unwrap_or(' ');
            self.buffer.drain(..ch.len_utf8());
            events.push(InputEvent::Data(ch.to_string()));
        }
    }
}

/// 在字节缓冲里查找子串（ASCII 标记用）。
fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// 把 `index` 向下取整到 UTF-8 字符边界（保证切片不 panic）。
fn floor_char_boundary(bytes: &[u8], index: usize) -> usize {
    let mut i = index.min(bytes.len());
    while i > 0 && (bytes[i] & 0xc0) == 0x80 {
        i -= 1;
    }
    i
}

/// 有损解码（与 ref 的 lossy 行为一致）。
fn decode_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 判断以 ESC 起始的缓冲里第一个完整序列的长度（`stdin-buffer.ts` 的 `isCompleteSequence`）。
fn complete_len(buffer: &str) -> Option<usize> {
    let bytes: Vec<char> = buffer.chars().collect();
    if bytes.first() != Some(&ESC) {
        return None;
    }
    if bytes.len() == 1 {
        return None; // 孤立 ESC：等待超时
    }
    match bytes[1] {
        '[' => {
            // CSI：终止字节在 0x40..=0x7e
            if bytes.len() >= 3 && bytes[2] == 'M' {
                return if bytes.len() >= 6 { Some(6) } else { None };
            }
            for (index, ch) in bytes.iter().enumerate().skip(2) {
                if ('\u{40}'..='\u{7e}').contains(ch) {
                    return Some(index + 1);
                }
            }
            None
        }
        ']' | 'P' | '_' => {
            // OSC/DCS/APC：需以 BEL 或 ST(ESC \) 结尾
            for index in 2..bytes.len() {
                if bytes[index] == '\u{7}' {
                    return Some(index + 1);
                }
                if bytes[index] == ESC && bytes.get(index + 1) == Some(&'\\') {
                    return Some(index + 2);
                }
            }
            None
        }
        'O' => {
            if bytes.len() >= 3 {
                Some(3)
            } else {
                None
            }
        }
        single => {
            let _ = single;
            Some(2) // ESC + 单字符 = meta
        }
    }
}

/// 解析单个序列为键名（`keys.ts:1251` `parseKey` 的子集，按夹具覆盖范围实现）。
#[must_use]
pub fn parse_key(sequence: &str, kitty_active: bool) -> Option<String> {
    // kitty 协议开了 flag 2 时，**每个**按键都带事件类型后缀（`:1` 按下、`:2` 重复、`:3` 抬起），
    // 连方向键也不例外（`\x1b[1;1:2B`）。ref 只丢弃 release（`tui.ts:1074`），重复事件按普通按键处理；
    // 此前这里直接拿原串去查 legacy 表 ⇒ 方向键的重复事件全部解析失败（粘着按住 ↓ 只走一格）。
    let normalized = normalize_event_type_suffix(sequence);
    let sequence = normalized.as_str();

    // **精确匹配表**（顺序与内容逐条对齐 `keys.ts:1278-1291`）：
    //   `\r` / `\x1bOM` ⇒ enter；`\x00` ⇒ ctrl+space；`" "` ⇒ space；
    //   `\x7f` ⇒ backspace；`\x08` ⇒ backspace（ref 在 Windows Terminal 下为 ctrl+backspace，本实现取非 WT 分支）；
    //   `\x1b[Z` ⇒ shift+tab；`\t` ⇒ tab（`CODEPOINTS.tab = 9`，`keys.ts:301-309`）；
    //   `\n`：kitty 未激活 ⇒ enter（`:1283`）；kitty 激活 ⇒ shift+enter（fixture `lf-kitty` 的 ref 实测）。
    // **kitty 激活态优先**（`keys.ts:1264-1267`）：`\x1b\r` 与 `\n` 都映射为 shift+enter
    // （注释原文：`\x1b\r` = Kitty 的 "map shift+enter send_text all \e\r"；`\n` = Ghostty 的绑定）。
    if kitty_active && (sequence == "\u{1b}\r" || sequence == "\n") {
        return Some("shift+enter".to_string());
    }
    // ref `keys.ts:1283-1291`（逐条）：
    //   1283 `\r` / `\x1bOM` ⇒ enter；`\n`（kitty 未激活）⇒ enter
    //   1285 `" "` ⇒ space；1286 `\x7f` ⇒ backspace；1287 `\x08` ⇒ backspace；1288 `\x1b[Z` ⇒ shift+tab
    //   1289 `\x1b\r`（kitty 未激活）⇒ alt+enter
    //   1290 `\x1b `（kitty 未激活）⇒ alt+space
    //   1291 `\x1b\x7f` / `\x1b\b` ⇒ alt+backspace
    //   1294-1300 `0x01..=0x1a` 且 **长度 1** ⇒ ctrl+<字母>（多字节序列不进入该分支）
    match sequence {
        "\r" | "\u{1b}OM" => return Some("enter".to_string()),
        "\n" if !kitty_active => return Some("enter".to_string()),
        "\u{1b}\r" if !kitty_active => return Some("alt+enter".to_string()),
        "\u{1b} " if !kitty_active => return Some("alt+space".to_string()),
        "\u{1b}\u{7f}" | "\u{1b}\u{8}" => return Some("alt+backspace".to_string()),
        "\u{0}" => return Some("ctrl+space".to_string()),
        " " => return Some("space".to_string()),
        "\u{7f}" => return Some("backspace".to_string()),
        "\u{8}" => return Some("backspace".to_string()),
        "\u{1b}[Z" => return Some("shift+tab".to_string()),
        "\t" => return Some("tab".to_string()),
        "\n" => {
            return Some(if kitty_active { "shift+enter" } else { "enter" }.to_string());
        }
        other => {
            let _ = other;
        }
    }

    if let Some(key) = parse_kitty(sequence) {
        return Some(key);
    }
    if let Some(key) = parse_modify_other_keys(sequence) {
        return Some(key);
    }
    // legacy 表（`keys.ts:368/394/408`）
    if let Some(key) = legacy_key(sequence) {
        return Some(key);
    }
    if sequence == "\u{1b}" {
        return Some("escape".to_string());
    }
    // ref `keys.ts:1278-1281`（注释见 `:1156`：ctrl+alt+key 是 ESC 后跟控制字符）：
    //   `"\x1b\x1b"` ⇒ `ctrl+alt+[`；`"\x1b\x1c"` ⇒ `ctrl+alt+\`；
    //   `"\x1b\x1d"` ⇒ `ctrl+alt+]`；`"\x1b\x1f"` ⇒ `ctrl+alt+-`。
    match sequence {
        "\u{1b}\u{1b}" => return Some("ctrl+alt+[".to_string()),
        "\u{1b}\u{1c}" => return Some("ctrl+alt+\\".to_string()),
        "\u{1b}\u{1d}" => return Some("ctrl+alt+]".to_string()),
        "\u{1b}\u{1f}" => return Some("ctrl+alt+-".to_string()),
        _other => {}
    }
    let mut chars = sequence.chars();
    let first = chars.next()?;
    if first == ESC {
        let rest: String = chars.collect();
        if rest.chars().count() == 1 {
            let ch = rest.chars().next().unwrap_or(' ');
            if kitty_active {
                return None; // 夹具（r 系列）表明 kitty 激活时 ESC+单字符不产生键名
            }
            if ch == 'b' {
                // ref 的 emacs 兼容映射（夹具 h08 期望 "alt+left"）
                return Some("alt+left".to_string());
            }
            // ref `keys.ts:1296-1300`：`ESC` + **控制字符**（0x01..=0x1a）⇒ `ctrl+alt+<字母>`
            let ch_code = ch as u32;
            if (1..=26).contains(&ch_code) {
                let letter = char::from_u32(ch_code + 96).unwrap_or('?');
                return Some(format!("ctrl+alt+{letter}"));
            }
            if ch.is_ascii_uppercase() {
                return Some(format!("alt+{}", ch.to_ascii_lowercase()));
            }
            return Some(format!("alt+{ch}"));
        }
        return None;
    }
    // 其余控制字符 ⇒ ctrl+<字母>，**仅在序列长度为 1 时**（ref `keys.ts:1318` `if (data.length === 1)`；
    // 否则 `\r\n` 这类多字节序列会被误判成 `ctrl+m`，而 ref 返回 `undefined`）。
    let code = first as u32;
    if sequence.chars().count() == 1 && (1..=26).contains(&code) {
        let letter = char::from_u32(code + 96).unwrap_or('?');
        return Some(format!("ctrl+{letter}"));
    }
    if first.is_ascii_graphic() || first == ' ' {
        return Some(first.to_string());
    }
    None // 多字节可打印字符 ⇒ 夹具期望 None
}

/// 去掉 CSI 参数里的 kitty 事件类型（`:` 后跟 1/2/3，且紧挨终止字节或 `;`）。
///
/// 例：`\x1b[1;1:2B` → `\x1b[1;1B`；`\x1b[97;5:2u` → `\x1b[97;5u`；`\x1b[27;5;97~` 不变。
fn normalize_event_type_suffix(sequence: &str) -> String {
    if !sequence.starts_with("\u{1b}[") {
        return sequence.to_string();
    }
    let bytes = sequence.as_bytes();
    let mut out = String::with_capacity(sequence.len());
    let mut index = 0_usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b':'
            && index + 1 < bytes.len()
            && matches!(bytes[index + 1], b'1' | b'2' | b'3')
            && (index + 2 == bytes.len()
                || bytes[index + 2].is_ascii_alphabetic()
                || bytes[index + 2] == b'~'
                || bytes[index + 2] == b';')
        {
            index += 2;
            continue;
        }
        out.push(byte as char);
        index += 1;
    }
    out
}

/// kitty CSI u（`keys.ts:587` `parseKittySequence` + `:1333` 正则）。
fn parse_kitty(sequence: &str) -> Option<String> {
    let body = sequence.strip_prefix("\u{1b}[")?.strip_suffix('u')?;
    let mut parts = body.split(';');
    let code_part = parts.next()?;
    let mod_part = parts.next();
    let mut code_parts = code_part.split(':');
    let codepoint: u32 = code_parts.next()?.parse().ok()?;
    let _shifted = code_parts.next();
    let _base = code_parts.next();
    let (modifier, event_type) = match mod_part {
        Some(text) => {
            let mut sub = text.split(':');
            let modifier: u32 = sub.next().unwrap_or("1").parse().unwrap_or(1);
            let event: u32 = sub.next().unwrap_or("1").parse().unwrap_or(1);
            (modifier, event)
        }
        None => (1, 1),
    };
    let _ = event_type; // release/repeat 由 `is_release`/`is_repeat` 单独判断
    format_parsed_key(codepoint, modifier.saturating_sub(1))
}

/// modifyOtherKeys（`keys.ts:696`）：`ESC [ 27 ; <mod> ; <cp> ~`。
fn parse_modify_other_keys(sequence: &str) -> Option<String> {
    let body = sequence.strip_prefix("\u{1b}[")?.strip_suffix('~')?;
    let parts: Vec<&str> = body.split(';').collect();
    if parts.len() != 3 || parts[0] != "27" {
        return None;
    }
    let modifier: u32 = parts[1].parse().ok()?;
    let codepoint: u32 = parts[2].parse().ok()?;
    format_parsed_key(codepoint, modifier.saturating_sub(1))
}

/// 键名格式化（`keys.ts` 的 `formatParsedKey` 子集）。
fn format_parsed_key(codepoint: u32, modifier: u32) -> Option<String> {
    const SHIFT: u32 = 1;
    const ALT: u32 = 2;
    const CTRL: u32 = 4;
    const LOCK_MASK: u32 = 64 + 128;
    let effective = modifier & !LOCK_MASK;
    let (name, is_letter) = match codepoint {
        27 => ("escape".to_string(), false),
        9 => ("tab".to_string(), false),
        13 => ("enter".to_string(), false),
        32 => ("space".to_string(), false),
        127 => ("backspace".to_string(), false),
        other => {
            let ch = char::from_u32(other)?;
            (ch.to_string(), ch.is_ascii_alphabetic())
        }
    };
    let mut parts: Vec<String> = Vec::new();
    if effective & CTRL != 0 {
        parts.push("ctrl".to_string());
    }
    if effective & ALT != 0 {
        parts.push("alt".to_string());
    }
    if effective & SHIFT != 0 {
        parts.push("shift".to_string());
    }
    let base = if is_letter {
        name.to_ascii_lowercase()
    } else {
        name
    };
    parts.push(base);
    Some(parts.join("+"))
}

/// legacy 序列表（`keys.ts:368/394/408`，仅列夹具覆盖到的项）。
fn legacy_key(sequence: &str) -> Option<String> {
    const PLAIN: &[(&str, &[&str])] = &[
        ("up", &["\u{1b}[A", "\u{1b}OA"]),
        ("down", &["\u{1b}[B", "\u{1b}OB"]),
        ("right", &["\u{1b}[C", "\u{1b}OC"]),
        ("left", &["\u{1b}[D", "\u{1b}OD"]),
        // 只用 `parseKey` 认识的序列：oracle 夹具（生成器跑 ref 的 `parseKey`）里
        // `[1~`/`[4~` 的 key 是 null。xterm 变体在**键位层**处理（见
        // `crates/pi-cli/src/tui.rs::map_key_sequence`），这里不得越权。
        ("home", &["\u{1b}[H", "\u{1b}OH"]),
        ("end", &["\u{1b}[F", "\u{1b}OF"]),
        ("insert", &["\u{1b}[2~"]),
        ("delete", &["\u{1b}[3~"]),
        ("pageUp", &["\u{1b}[5~", "\u{1b}[[5~"]),
        ("pageDown", &["\u{1b}[6~", "\u{1b}[[6~"]),
        ("f1", &["\u{1b}OP", "\u{1b}[11~", "\u{1b}[[A"]),
        ("f2", &["\u{1b}OQ", "\u{1b}[12~", "\u{1b}[[B"]),
        ("f3", &["\u{1b}OR", "\u{1b}[13~", "\u{1b}[[C"]),
        ("f4", &["\u{1b}OS", "\u{1b}[14~", "\u{1b}[[D"]),
        ("f5", &["\u{1b}[15~", "\u{1b}[[E"]),
        ("f6", &["\u{1b}[17~"]),
        ("f7", &["\u{1b}[18~"]),
        ("f8", &["\u{1b}[19~"]),
        ("f9", &["\u{1b}[20~"]),
        ("f10", &["\u{1b}[21~"]),
        ("f11", &["\u{1b}[23~"]),
        ("f12", &["\u{1b}[24~"]),
    ];
    for (name, sequences) in PLAIN {
        if sequences.contains(&sequence) {
            return Some((*name).to_string());
        }
    }
    // CSI 修饰形式 `ESC [ 1 ; <mod> A/B/C/D`（`keys.ts` 的 CSI 分支）
    if let Some(body) = sequence
        .strip_prefix("\u{1b}[")
        .and_then(|s| s.strip_suffix(['A', 'B', 'C', 'D']))
    {
        let parts: Vec<&str> = body.split(';').collect();
        if parts.len() == 2 && parts[0] == "1" {
            let modifier: u32 = parts[1].parse().ok()?;
            let arrow = &sequence[sequence.len() - 1..];
            let name = match arrow {
                "A" => "up",
                "B" => "down",
                "C" => "right",
                "D" => "left",
                _other => return None,
            };
            let modifier = modifier.saturating_sub(1);
            return format_parsed_key(
                match name {
                    "up" => 0xFFF1,
                    "down" => 0xFFF2,
                    "right" => 0xFFF3,
                    "left" => 0xFFF4,
                    _other => return None,
                },
                modifier,
            )
            .map(|_| {
                let mut parts: Vec<String> = Vec::new();
                if modifier & 1 != 0 {
                    parts.push("shift".to_string());
                }
                if modifier & 2 != 0 {
                    parts.push("alt".to_string());
                }
                if modifier & 4 != 0 {
                    parts.push("ctrl".to_string());
                }
                parts.push(name.to_string());
                parts.join("+")
            });
        }
    }
    None
}

/// 是否 release 事件——**逐条对齐** `keys.ts:527-552`。
///
/// 粘贴内容里的 `:3F` 之类（如蓝牙 MAC `90:62:3F:A5`）不算 release；ref 靠"粘贴内容含 `\x1b[200~`"排除，
/// 这里同样处理。
#[must_use]
pub fn is_release(sequence: &str) -> bool {
    if sequence.contains("\u{1b}[200~") {
        return false;
    }
    const SUFFIXES: [&str; 8] = [":3u", ":3~", ":3A", ":3B", ":3C", ":3D", ":3H", ":3F"];
    SUFFIXES.iter().any(|suffix| sequence.contains(suffix))
}

/// 是否 repeat 事件（`keys.ts:557` 起：kitty 事件类型 `:2`）。
#[must_use]
pub fn is_repeat(sequence: &str) -> bool {
    const SUFFIXES: [&str; 2] = [":2u", ":2~"];
    SUFFIXES.iter().any(|suffix| sequence.contains(suffix))
}

#[cfg(test)]
mod flush_contract_tests {
    use super::{InputEvent, InputState};

    fn payload(events: &[InputEvent]) -> String {
        let mut out = String::new();
        for event in events {
            let text = match event {
                InputEvent::Data(text) => text,
                InputEvent::Paste(text) => text,
            };
            out.push_str(text);
        }
        out
    }

    /// 契约（B6b）：超时 flush **不得丢字节**——缓冲里剩下的原始字节必须原样发出。
    #[test]
    fn flush_returns_every_buffered_byte() {
        let mut state = InputState::new(false);
        let (_events, wake) = state.process(b"\x1b[1;2", 1000);
        assert!(wake.is_some(), "不完整序列应要求超时唤醒");
        let events = state.flush();
        assert_eq!(payload(&events), "\u{1b}[1;2", "flush 必须原样交回缓冲内容");
        assert!(state.flush().is_empty(), "再次 flush 不应重复发出");
    }

    /// 契约：闭合的粘贴整段发出，粘贴之后的普通文本在 flush 时原样交回。
    #[test]
    fn flush_after_paste_keeps_later_input_intact() {
        let mut state = InputState::new(false);
        let (events, _wake) = state.process(b"\x1b[200~hello\x1b[201~x", 1000);
        let mut all = payload(&events);
        all.push_str(&payload(&state.flush()));
        assert_eq!(all, "hellox", "粘贴与其后的字节都不能丢：{events:?}");
    }
}

#[cfg(test)]
mod event_type_tests {
    use super::parse_key;

    /// 按住方向键时终端发的是带 `:2`（repeat）的序列；ref 只丢 release，重复事件按普通按键处理。
    /// 此前 `\x1b[1;1:2B` 解析为 None ⇒ 粘着按住 ↓ 只移动一格。
    #[test]
    fn repeat_sequences_parse_like_press() {
        assert_eq!(parse_key("\u{1b}[B", false).as_deref(), Some("down"));
        assert_eq!(parse_key("\u{1b}[1;1:2B", false).as_deref(), Some("down"));
        assert_eq!(parse_key("\u{1b}[1;1:3B", false).as_deref(), Some("down"));
        assert_eq!(parse_key("\u{1b}[1;1:2A", false).as_deref(), Some("up"));
        assert_eq!(parse_key("\u{1b}[1;1:2C", false).as_deref(), Some("right"));
        assert_eq!(parse_key("\u{1b}[1;1:2D", false).as_deref(), Some("left"));
        // kitty CSI-u 的 `:` 原本已能解析，归一化后仍是同一个键。
        assert_eq!(
            parse_key("\u{1b}[97;5:2u", false).as_deref(),
            Some("ctrl+a")
        );
        // modifyOtherKeys 的三段式不能被误伤。
        assert_eq!(
            parse_key("\u{1b}[27;5;97~", false).as_deref(),
            Some("ctrl+a")
        );
    }
}
