// F52 合成扩展夹具：**仿 pi-ai-summary 的两处原型补丁**，全部合成文本。
//   ① ToolExecutionComponent.prototype.render → 折叠成一行 `+ <name> · <args> · ok`；
//      `expanded === true` 时回落原生（与 pi-ai-summary 同语义）。
//   ② AssistantMessageComponent.prototype.updateContent → 给思考块一个
//      `+ Thought: <预览>` 标签（`hiddenThinkingLabel`），供 `ctrl+t` 隐藏态使用。
import { ToolExecutionComponent, AssistantMessageComponent } from "@earendil-works/pi-coding-agent";

export default function collapseExt(pi) {
  let installed = false;
  pi.on("session_start", (_e, ctx) => {
    if (installed) return;
    const tool = ToolExecutionComponent?.prototype;
    if (tool && typeof tool.render === "function") {
      const original = tool.render;
      tool.render = function render(width) {
        try {
          if (this.expanded === true || this.hideComponent === true) return original.call(this, width);
          const theme = ctx?.ui?.theme;
          if (!theme) return original.call(this, width);
          // 键序**按字典序**排序后再 stringify：pi-rs 的 `serde_json` 未开 `preserve_order`
          // （`notes/questions.md` P5-1e E2 的用户裁定：不引入该 feature），参数键序与 TS
          // 不一致是**既有已知限制**，不是 F52 渲染行为。此夹具只验证折叠行的展开/回折，
          // 排序后两侧键序一致，避免把既有差异混进渲染对拍。
          const sortedArgs =
            this.args && typeof this.args === "object"
              ? Object.fromEntries(Object.keys(this.args).sort().map((k) => [k, this.args[k]]))
              : this.args;
          const args = sortedArgs && typeof sortedArgs === "object" ? JSON.stringify(sortedArgs) : "";
          const failed = this.result && this.result.isError === true;
          const glyph = failed ? "x" : "+";
          const color = failed ? "error" : "success";
          const text = `${glyph} ${String(this.toolName)}${args ? ` · ${args}` : ""} · ${failed ? "err" : "ok"}`;
          // 按**可见宽度**裁剪（CJK 占 2 列），否则 CJK 参数会让行宽超终端列数，
          // TS 会 `uncaughtException: Rendered line N exceeds terminal width`。
          // `width` 可能是 `undefined`/非有限值（首帧布局前实测会这样，
          // `Math.max(1, NaN)` 仍是 NaN ⇒ 比较恒 false ⇒ 完全不裁剪 ⇒ TS 崩溃）；
          // 因此这里取"有限值则用之、否则 80"，并再压到 80 以内，两种实现都安全。
          const raw = Number(width);
          const safe = Number.isFinite(raw) && raw > 0 ? Math.min(raw, 80) : 80;
          const budget = Math.max(1, safe - 4);
          let used = 0;
          let out = "";
          for (const ch of text) {
            const w = /[\u1100-\u115F\u2E80-\uA4CF\uAC00-\uD7A3\uF900-\uFAFF\uFE30-\uFE4F\uFF00-\uFF60\uFFE0-\uFFE6]/.test(ch) ? 2 : 1;
            if (used + w > budget) break;
            out += ch;
            used += w;
          }
          return [`  ${theme.fg(color, out)}`];
        } catch {
          return original.call(this, width);
        }
      };
    }
    const assistant = AssistantMessageComponent?.prototype;
    if (assistant && typeof assistant.updateContent === "function" && !assistant.__f52) {
      assistant.__f52 = true;
      Object.defineProperty(assistant, "hiddenThinkingLabel", { configurable: true, writable: true, value: "+ Thought: 合成预览" });
    }
    installed = true;
  });
}
