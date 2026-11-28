// 测试扩展：`tool_call` 的**就地修改**语义（F1/S5）。
//
// ref 依据：`agent-loop.ts:727` 把 `args`（同一对象）交给 `beforeToolCall`，`:770-775` 用同一个
// `validatedArgs` 执行；`agent-session.ts:538-543` 把 `args` 原样作为 `input` 传给扩展
// ⇒ 扩展对 `event.input` 的**就地修改**会作用到真正的工具调用。
// 而 `emitToolCall`（`runner.ts:1134-1152`）只使用返回值的 `block/reason/terminate`
// ⇒ 返回 `{ input }` **无效**。
export default function inputMutateExtension(pi) {
  pi.registerTool({
    name: "ext_echo_args",
    description: "把收到的参数原样回显成 JSON（验证参数是否被就地修改）",
    parameters: {
      type: "object",
      properties: {
        mutateInPlace: { type: "boolean" },
        returnInput: { type: "boolean" },
      },
    },
    async execute(_toolCallId, args) {
      return { content: [{ type: "text", text: JSON.stringify(args ?? null) }] };
    },
  });

  pi.on("tool_call", (event) => {
    if (event?.input?.mutateInPlace) {
      // 就地修改：与 ref 的 `args` 是同一个对象，应回传到宿主。
      event.input.injected = "in-place";
      delete event.input.mutateInPlace;
    }
    if (event?.input?.returnInput) {
      // 返回 { input } 在 ref 里**无效**，宿主必须忽略。
      return { input: { injected: "via-return" } };
    }
    return undefined;
  });
}
