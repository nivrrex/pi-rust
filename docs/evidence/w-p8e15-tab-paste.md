# 粘贴 tab 丢失（W-P8e-15 续）：真机"光标停在可见末尾前 4 格"的根因

测试日期 2026-10-30；被测 `commit 47ae544` / sha256 `6851cbbfafc2c150…` / `built 2026-10-30T14:30:12Z`。
测试文本均为合成内容。**Linux 侧先定位到 tab 假设，本文件是 Windows 侧的实现与三条通道复验。**

## 根因

ref 的粘贴归一化（`packages/tui/src/components/editor.ts:1146` `normalizeText`）**先展开 tab**：

```ts
return text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").replace(/\t/g, "    ");
```

之后 `:1281-1285` 才"滤掉非可打印字符（保留 `\n`）"。我们只做了 CRLF/CR → LF，
紧接着的 `filter(|ch| ch == '\n' || ch as u32 >= 32)` 于是把 **`\t`（9）整块删掉**：

- 缩进消失（截图上第三行本该有 4 个空格）；
- `state.input` 比粘贴文本少 1 个字符/个 tab，光标列随之**少 4 格** ⇒ 硬件光标/软件反显块
  停在"可见末尾前 4 格"，且反显块正好压在行内两个样式段的边界上（"该字符前后颜色不同"）。

另一条同样会让 tab 丢失的路径：**ConPTY 把粘贴文本里的 tab 还原成 `VK_TAB` 键事件**。
它既不是括号标记的一部分，也不是普通字符 ⇒
- 括号收集期被当成"其它按键"⇒ 中断整段收集；
- 键流攒批期被当成 `TuiEvent::Tab`（补全键）⇒ 正文里没有 tab。

## 修复（`47ae544`）

1. `crates/pi-tui/src/lib.rs`：归一化补 `.replace('\t', "    ")`（顺序与 ref:1146 一致）；
   既有单测 `control_chars_are_filtered` 的期望由 `"abc"` 改为 `"ab    c"`（tab 不再算"被过滤"）。
2. `crates/pi-cli/src/tui.rs`：`KeyCode::Tab` 在**括号收集期**映射为 `BracketInput::Char('\t')`；
   **键流攒批期**（`burst.is_pending()`）并入粘贴正文。空闲时仍是 `TuiEvent::Tab`（补全）。
3. 单测 5 条：tab 展开、含 tab 粘贴后 marker 落在展开行尾、多行+tab 结构、键流 tab 保留、空闲 Tab 仍是补全键。

## 三条通道复验（真实按键/真实粘贴）

| 通道 | 粘贴内容 | input | cursor | col | 期望 col |
|---|---|---|---|---|---|
| WezTerm `Shift+Insert`（键流） | `AAA⏎\tXYZ` | 11 | 11 | **7** | 7（4 空格 + XYZ） |
| conhost 右键粘贴（键流） | `AAA⏎\tXYZ` | 11 | 11 | **7** | 7 |
| WezTerm 括号粘贴（`send-text`） | `AAA⏎\tXYZ` | 11 | 11 | **7** | 7 |
| 同上（用户场景） | 横幅 3 行，第 3 行以 tab 开头 | 155 | 155 | **64** | 64（4 + 60） |

修复前同为这三条通道：`input=7 / col=3`（tab 被删，光标少 4 格）。

## 回归

- `pi-cli --lib` **129/0**、`pi-tui --lib` **79/0**、`fmt` 通过；
- RECHECK 抽查：启动/`abc`/`Ctrl+D`/`Ctrl+P`/`Shift+Tab`/footer 通过；1200 字符单行仍折叠
  `[paste #1 1242 chars]`；`Shift+Enter` 与 `Ctrl+Enter` 仍换行不提交。

## 未改的东西

- 没有按之前那个"CR 假设"改输入层：CRLF→LF 归一从首个提交就存在（`lib.rs:944` 附近），
  `paste_normalizes_cr_to_lf_and_does_not_submit` 一直钉着；本次 dump 里 `input` 比可见字符少，
  正是 tab 被删的结果，与 CR 方向相反。
- `marker_apply` 的列算法未动（Linux 侧 CJK/超宽折行用例已证明与 TS 一致）。
