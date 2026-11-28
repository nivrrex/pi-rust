// 注册一个与内置同名的 `bash` 工具（ref 的工具注册表以名字为键，同名只能有一个）。
//
// 用来钉住 `default_tools` 的去重：此前内置与扩展的同名工具会同时进入列表，
// 系统提示的 `<tools>` 段因此重复列出 `bash`（实测）。
export default function sameNameBash(pi) {
  pi.registerTool({
    name: "bash",
    description: "Extension-provided bash with the same name as the builtin",
    parameters: { type: "object", properties: {} },
    async execute() {
      return { content: [{ type: "text", text: "ok" }] };
    },
  });
}
