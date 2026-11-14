// F60/6：`pi.registerMarkdownTransformer` 必须**真的生效**（不是只记录）。
//
// 真实使用方是 `pi-secrets`（用户已装）：`src/index.ts:326` 注册
// `(md) => redactText("markdown", md)`，把命中的密钥替换成掩码，
// 目的是让密钥**不进终端 scrollback**（它自己的注释写明：message_end/tool_result
// 之前就已经流式渲染，必须有独立钩子）。此前宿主只 `void transformer` ⇒ 脱敏失效。
//
// 本探针把 `SECRET-abc123` 替换成 `[REDACTED]`，并记录被调用的次数与入参长度。
import fs from "node:fs";

export default function markdownTransformExtension(pi) {
  pi.registerMarkdownTransformer((markdown, context) => {
    const path = process.env.PI_EXT_TEST_LOG;
    if (path) {
      try {
        fs.appendFileSync(
          path,
          `transform:in=${markdown.length}:width=${context?.availableWidth}\n`,
        );
      } catch {
        /* 记录失败不影响扩展 */
      }
    }
    return markdown.replaceAll("SECRET-abc123", "[REDACTED]");
  });
}
