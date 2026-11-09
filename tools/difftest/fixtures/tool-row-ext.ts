// F48/X2：合成的「工具行折叠」扩展 —— 与 `pi-ai-summary` 的补丁手法同形
// （改写 `ToolExecutionComponent.prototype.render` 与 `updateDisplay`），
// 但内容全部是合成字符，不含任何真实扩展名、真实字段文案或真实报告文本。
//
// 行为（刻意保持最小）：
//   - 补丁只做「一行折叠摘要」：`  ‹glyph› ‹name› · ‹target› · ‹state›`；
//   - `expanded === true`、`hideComponent === true`、形状不认识 ⇒ 调原 render（原生块）；
//   - `isPartial !== false` ⇒ glyph `~`、色 `warning`、状态词 `run`；
//   - 结果 isError ⇒ 两行（第二行 4 列缩进 + `! ` + 首行错误），整行 error 色；
//   - 成功 ⇒ glyph `+`、色 `success`、状态词 `ok`。
// 只用合成字符串，不读取任何真实业务文本。
import { ToolExecutionComponent } from "@earendil-works/pi-coding-agent";

export default function probeToolRows(pi) {
  const PATCH = Symbol.for("probe.toolRows.v1");
  let installed = false;

  const target = (row) => {
    const args = row.args;
    if (!args || typeof args !== "object") return "";
    try {
      const json = JSON.stringify(args);
      if (!json || json === "{}") return "";
      return json.replace(/\s+/g, " ");
    } catch {
      return "";
    }
  };

  const errorText = (result) => {
    if (!result || typeof result !== "object") return "";
    const content = Array.isArray(result.content) ? result.content : [];
    for (const block of content) {
      if (block && block.type === "text" && typeof block.text === "string") {
        return block.text.split("\n")[0] ?? "";
      }
    }
    return "";
  };

  // 与 pi-ai-summary `summaries.ts:93-141` 的 `renderSegments` 同口径：整行不得超过
  // 可用宽度，超出时截断并加 `…`。**必须自己做**——TS/pt 不做裁剪，扩展返回超宽行
  // 会让 TS 直接 `uncaughtException: Rendered line N exceeds terminal width`（实测）。
  const clip = (text, budget) => {
    if (budget <= 0) return "";
    const chars = [...text];
    let used = 0;
    let out = "";
    for (const ch of chars) {
      const w = /[\u1100-\u115F\u2E80-\uA4CF\uAC00-\uD7A3\uF900-\uFAFF\uFE30-\uFE4F\uFF00-\uFF60\uFFE0-\uFFE6]/.test(ch) ? 2 : 1;
      if (used + w > budget) return out ? `${out}…` : "…";
      out += ch;
      used += w;
    }
    return out;
  };

  const oneLine = (row, theme, width) => {
    const settled = row.isPartial === false;
    const failed = settled && row.result && row.result.isError === true;
    const glyph = !settled ? "~" : failed ? "x" : "+";
    const color = !settled ? "warning" : failed ? "error" : "success";
    const word = !settled ? "run" : failed ? "err" : "ok";
    const name = typeof row.toolName === "string" ? row.toolName : "tool";
    const arg = target(row);
    // 可用宽度扣掉行首 2 列 inset（`ROW_INSET`）与不安全余量 2 列
    // （TS 侧 `render(width)` 的 width 已含容器 padding，实测差 2 列会撞 width 上限）。
    const budget = Math.max(1, width - 4);
    const tailText = settled && !failed ? " · ok" : "";
    const argBudget = Math.max(0, budget - 2 - [...name].length - 3 - [...tailText].length);
    const trimmedArg = arg ? ` · ${clip(arg, argBudget)}` : "";
    let line = `${glyph} ${name}${trimmedArg}`;
    if (settled && failed) {
      const body = errorText(row.result);
      if (body) {
        const second = `    ! ${body}`;
        return [
          `  ${theme.fg(color, line)}`,
          `  ${theme.fg(color, second.slice(0, Math.max(0, width - 2)))}`,
        ];
      }
    }
    line = clip(`${line} · ${word}`, budget);
    return [`  ${theme.fg(color, line)}`];
  };

  pi.on("session_start", (_e, ctx) => {
    if (installed) return;
    // 与 pi-ai-summary 完全同源的取法：直接从包里拿组件类。
    const proto = ToolExecutionComponent?.prototype;
    if (!proto || typeof proto.render !== "function") return;
    if (proto[PATCH]) return;
    const originalRender = proto.render;
    const originalUpdate = proto.updateDisplay;
    const state = { originalRender, originalUpdate };
    state.patchedRender = function render(width) {
      try {
        if (this.expanded === true || this.hideComponent === true) {
          return originalRender.call(this, Number(width) || 80);
        }
        const theme = ctx?.ui?.theme;
        if (!theme) return originalRender.call(this, Number(width) || 80);
        return oneLine(this, theme, Number(width) || 80);
      } catch {
        return originalRender.call(this, Number(width) || 80);
      }
    };
    state.patchedUpdate = function updateDisplay() {
      originalUpdate.call(this);
    };
    proto.render = state.patchedRender;
    proto.updateDisplay = state.patchedUpdate;
    Object.defineProperty(proto, PATCH, { configurable: true, value: state });
    installed = true;
  });
}
