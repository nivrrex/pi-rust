// `after_provider_response` 取证探针：把收到的 status/headers 写进文件，
// 用来核对"真实 HTTP 流的状态码与响应头是否如实传达"（F111）。
import fs from "node:fs";
import path from "node:path";

export default function afterResponseProbe(pi) {
  pi.on("after_provider_response", (event) => {
    try {
      const dir = process.env.PI_CODING_AGENT_DIR ?? process.cwd();
      fs.appendFileSync(
        path.join(dir, "after-response.log"),
        `${JSON.stringify({ status: event?.status, headers: event?.headers })}\n`,
      );
    } catch {
      /* 写不进不影响判定 */
    }
  });
}
