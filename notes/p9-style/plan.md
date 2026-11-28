# compare-style 的 124 个"差异"是退出尾部字节造成的假阳性

## 现象
`STYLE_CHECK=1 sh scripts/regress-tui.sh` 汇总为 `style=370`；
逐场景用 `compare-style.mjs` 复核：**124 diff / 1 same**。
首个差异示例（T01）：`r1 c0 字段=dim`，TS `dim:true`、rs `dim:false`。

## 取证（已验证）
1. `\x1b[2m`（SGR dim）在**两侧 raw 里都存在**——不是 pi-rs 漏发。
2. 它出现在**退出 TUI 之后**那一行 `To resume this session: …`：
   - TS：`…\x1b[?25h\x1b[?2004l\x1b]0;π - work\x07\x1b[2mTo resume this session:\x1b[22m …`
   - ours：`…\x1b[?2004l\x1b[<u\x1b[>4;0m\x1b[?25h\x1b[2mTo resume this session:\x1b[22m …`
   即**退出序列的顺序不同**（`?25h`/`?2004l`/`OSC 0` 的先后），`2m` 本身的语义一致。
3. `compare-style.mjs` 把整段 raw 按屏幕单元格解析，不区分"TUI 期间的画面"与
   "退出后打到 stdout 的尾巴" ⇒ 这条尾行被当成屏幕内容，于是每个场景都报 `dim` 差异。

## 结论
**不是样式回归**。`compare-tui` 的 125/125 same 是屏幕文本层的权威结果；
`compare-style` 在**未做退出尾部剥离**的情况下不适合做全量门禁。

## 未做的
没有去改 `compare-style.mjs`（红线文件，且改动会影响既有 STYLE_CHECK 语义）。
推翻条件：若需要把样式也纳入门禁，应先在 `compare-style` 里剥离退出后的尾部字节，
再用它复核；在那之前以 `compare-tui` 为准。
