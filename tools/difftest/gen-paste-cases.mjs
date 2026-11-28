// 用 ref 真实 Editor 产出粘贴折叠夹具（P7-9/P）。
// 每例记录：显示行（getLines）、光标（line/col）、提交文本（getExpandedText）。
import crypto from "node:crypto";
import fs from "node:fs";

// 生成前置：把 ref 的 tui 源码**只读复制**到 /tmp（不改 ref 目录），并软链两个 npm 依赖：
//   cp -r ref/pi-0.87.1/packages/tui/src /tmp/ref-tui/src
//   mkdir -p /tmp/ref-tui/node_modules && ln -s <get-east-asian-width> /tmp/ref-tui/node_modules/
//   ln -s <marked> /tmp/ref-tui/node_modules/
// 运行：node --experimental-strip-types tools/difftest/gen-paste-cases.mjs fixtures/paste/ref-cases.json
const { Editor } = await import(process.env.REF_TUI_SRC ?? "/tmp/ref-tui/src/components/editor.ts");
const PASTE = (text) => "\x1b[200~" + text + "\x1b[201~";
const stub = () =>
  new Proxy({}, {
    get(_t, prop) {
      if (prop === "requestRender") return () => {};
      if (prop === "terminal") return { rows: 30, cols: 100 };
      return () => {};
    },
  });

const lines = (n) => Array.from({ length: n }, (_, i) => `L${i + 1}`).join("\n");
const chars = (n) => "x".repeat(n);

// `steps.*` 返回的函数带 `.spec`，便于把动作原样写进夹具（供 Rust 侧重放）。
const wrap = (kind) => (text) => {
  const fn = (editor) => {
    if (kind === "paste") editor.handleInput(PASTE(text));
    else if (kind === "type") for (const ch of text) editor.handleInput(ch);
    else editor.handleInput(text);
  };
  fn.spec = { kind, text };
  return fn;
};
const steps = { paste: wrap("paste"), type: wrap("type"), key: wrap("key") };
// 组合键要**逐个序列**喂（ref 的 `handleInput` 对多字符串按"粘贴"处理）。
const keys = (...seqs) => seqs.map((seq) => steps.key(seq));

const cases = [];
const add = (name, actions) => cases.push({ name, actions });

// 阈值边界
add("lines-9", [steps.paste(lines(9))]);
add("lines-10", [steps.paste(lines(10))]);
add("lines-11", [steps.paste(lines(11))]);
add("chars-999", [steps.paste(chars(999))]);
add("chars-1000", [steps.paste(chars(1000))]);
add("chars-1001", [steps.paste(chars(1001))]);
add("lines-11-trailing-newline", [steps.paste(lines(10) + "\n")]);
add("cjk-1001-chars", [steps.paste("中".repeat(1001))]);
add("cjk-10-lines", [steps.paste(Array.from({ length: 10 }, () => "中文行").join("\n"))]);
add("cjk-11-lines", [steps.paste(Array.from({ length: 11 }, () => "中文行").join("\n"))]);
add("esc-literal", [steps.paste("\x1b[31mred\x1b[0m\n" + lines(11))]);
add("control-chars", [steps.paste("a\u0001b\u0007" + lines(11))]);
// 多次粘贴编号
add("two-pastes", [steps.paste(lines(12)), steps.paste(lines(12))]);
add("three-pastes-with-text", [steps.type("前缀"), steps.paste(lines(12)), steps.type(" 中间"), steps.paste(lines(13)), steps.type(" 后缀")]);
// 占位符前后插字
add("text-before-marker", [steps.paste(lines(12)), ...keys("\x1b[D", "\x1b[D"), steps.type("XY")]);
add("text-after-marker", [steps.paste(lines(12)), steps.type("尾")]);
add("marker-then-newline", [steps.paste(lines(12)), steps.key("\r"), steps.type("下一行")]);
// 退格整删
add("backspace-marker", [steps.paste(lines(12)), steps.key("\x7f")]);
add("backspace-marker-then-type", [steps.paste(lines(12)), steps.key("\x7f"), steps.type("after")]);
add("backspace-twice", [steps.paste(lines(12)), ...keys("\x7f", "\x7f")]);
// 光标左右穿越
add("cursor-left-through-marker", [steps.paste(lines(12)), ...keys("\x1b[D", "\x1b[D", "\x1b[C")]);
add("cursor-home", [steps.paste(lines(12)), steps.key("\x1b[H"), steps.type("HEAD")]);
add("cursor-end", [steps.paste(lines(12)), ...keys("\x1b[H", "\x1b[F"), steps.type("TAIL")]);
// 提交展开
add("submit-expand", [steps.paste(lines(12)), steps.key("\r")]);
add("submit-expand-mixed", [steps.type("说明："), steps.paste(lines(11)), steps.type("结束"), steps.key("\r")]);
// 混排
add("paste-then-small-paste", [steps.paste(lines(12)), steps.paste("小段")]);
add("small-then-large", [steps.type("abc"), steps.paste(lines(12))]);
add("two-pastes-delete-first", [steps.paste(lines(12)), steps.paste(lines(12)), ...keys("\x1b[H", "\x1b[C", "\x7f")]);
add("paste-with-crlf", [steps.paste(Array.from({ length: 11 }, (_, i) => `r${i}`).join("\r\n"))]);
add("paste-999-chars-cjk", [steps.paste("中".repeat(999))]);
add("paste-1000-chars-cjk", [steps.paste("中".repeat(1000))]);
add("empty-paste", [steps.paste("")]);
add("single-newline-paste", [steps.paste("\n")]);

const results = [];
for (const item of cases) {
  const editor = new Editor(stub(), stub(), {});
  const submitted = [];
  editor.onSubmit = (text) => submitted.push(text);
  for (const action of item.actions) action(editor);
  item.kinds = item.actions.map((action) => action.spec);
  results.push({
    name: item.name,
    actions: item.actions.map((action, index) => ({ kind: item.kinds[index].kind, text: item.kinds[index].text })),
    text: editor.getText(),
    lines: editor.getLines(),
    cursor: editor.getCursor(),
    expanded: editor.getExpandedText(),
    submitted,
  });
}
const payload = JSON.stringify(results, null, 2) + "\n";
const sha = crypto.createHash("sha256").update(payload).digest("hex").slice(0, 16);
fs.writeFileSync(process.argv[2] ?? "/tmp/paste-cases.json", payload);
console.log(JSON.stringify({ cases: results.length, sha }));
