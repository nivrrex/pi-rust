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
          const args = this.args && typeof this.args === "object" ? JSON.stringify(this.args) : "";
          const failed = this.result && this.result.isError === true;
          const glyph = failed ? "x" : "+";
          const color = failed ? "error" : "success";
          const text = `${glyph} ${String(this.toolName)}${args ? ` · ${args}` : ""} · ${failed ? "err" : "ok"}`;
          return [`  ${theme.fg(color, text.slice(0, Math.max(0, Number(width) - 4)))}`];
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
