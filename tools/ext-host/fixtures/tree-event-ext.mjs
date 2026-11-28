// F60/7：`session_tree` 事件探针。
//
// `pi-handoff-gate`（用户已装）在 `session_tree` 里从 `getBranch()` **重建 todo 清单**
// （`index.ts:305-308`），注释写明「随会话分支；压缩不影响」。此前 pi-rs 只切叶子、
// 从不派发该事件 ⇒ 换分支后清单停在旧分支。
//
// 本探针把收到的每个事件的载荷摘要写进 `PI_EXT_TEST_LOG`。
import fs from "node:fs";

function record(line) {
  const path = process.env.PI_EXT_TEST_LOG;
  if (!path) return;
  try {
    fs.appendFileSync(path, `${line}\n`);
  } catch {
    /* 记录失败不影响扩展 */
  }
}

export default function treeEventExtension(pi) {
  pi.on("session_tree", (event) => {
    record(
      `session_tree:new=${event?.newLeafId ?? "null"}:old=${event?.oldLeafId ?? "null"}`,
    );
  });
  // 便于测试主动触发：注册一个命令，调它时由外部先做导航（本探针只是记录）。
  pi.registerCommand("ext_tree_probe", {
    description: "记录当前分支（F60/7 探针）",
    handler: async () => {
      record("probe_called");
    },
  });
}
