// ctx 与 UI 单向帧测试扩展（P9-1-D / S5）：读取只读 ctx，并调用 notify/setStatus。
import fs from "node:fs";

export default function ctxExtension(pi) {
  pi.on("session_start", (event, ctx) => {
    const log = process.env.PI_EXT_TEST_LOG;
    if (log) {
      fs.appendFileSync(
        log,
        // F234：`ctx.model` 是**模型对象**（ref `types.ts:327` `Model<any> | undefined`），
        // 不是 id 字符串。这里显式取 `provider`/`id`——直接插值对象会打出
        // `[object Object]`，早先把 `ctx.model` 当标量断言，等于把错误形状固化了。
        `ctx:${ctx?.cwd}:${ctx?.mode}:${ctx?.hasUI}:${ctx?.model?.provider ?? "null"}/${ctx?.model?.id ?? "null"}:${ctx?.thinkingLevel ?? "null"}\n`,
      );
    }
    ctx?.ui?.notify?.(`hello from extension (${ctx?.mode})`);
    ctx?.ui?.setStatus?.("demo", "status-from-ext");
  });
}
