// 归一化：**录制与比对共用这一份**（`run.mjs` 与 `compare.mjs` 都 import 它）。
//
// 抽成模块的原因很实际：如果比对用另一套（哪怕只差一个规则），两边的"一致"就无从谈起
// ——比对器与被比对方的归一化不同，是差分测试里最容易自欺的一种失败。
//
// 规则逐条：
// - 临时目录/home/端口替换成占位符（每次运行必然不同）；
// - UUID 与 8 位短 id 换成 `<UUID>`／`<IDn>`（id 由实现生成，不参与语义比较）；
// - ISO 时间戳与数值 `timestamp`/`durationMs` 归零；
// - `responseId` **整键抹掉**：TS 自己的多次运行里这个键的出现与否就会变（见 notes/decisions.md）；
// - **不动** `cost` / `usage` 等数值：PLAN 第 3 节明确要求逐位比较。

// 用前后瞻而不是 \b：会话文件名里 UUID 前面紧跟 `_`，\b 不会匹配（下划线也是单词字符）。
const UUID_RE = /(?<![0-9a-f-])[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}(?![0-9a-f-])/gi;
const ISO_RE = /\d{4}-\d{2}-\d{2}T\d{2}[:-]\d{2}[:-]\d{2}(?:[.-]\d+)?Z/g;
const SHORT_ID_RE = /\b[0-9a-f]{8}\b/g;



export function normalize(text, ctx) {
  let out = text;
  // B1（P8e-11）：录制时 TS 会把自己的安装路径写进请求/事件（docs 路径、bundle 路径）。
  // 统一掩成 `<PI_PKG>`，golden 里就不再保留本机检出路径。
  out = out.replace(/[^\s"']*node_modules\/@earendil-works\/pi-coding-agent/g, "<PI_PKG>");
  out = out.replaceAll(ctx.tmp, "<TMP>");
  out = out.replaceAll(ctx.home, "<HOME>");
  out = out.replace(UUID_RE, "<UUID>");
  let counter = 0;
  out = out.replace(SHORT_ID_RE, () => `<ID${++counter}>`);
  out = out.replace(ISO_RE, "<TIMESTAMP>");
  out = out.replaceAll(String(ctx.port), "<PORT>");
  out = out.replace(/,"responseId":"[^"]*"/g, "");
  out = out.replace(/"timestamp":\s*\d+/g, '"timestamp":0');
  out = out.replace(/"durationMs":\s*[\d.]+/g, '"durationMs":0');
  return out;
}

/**
 * B 层折叠：把**连续的**流式增量事件合并成一条。
 *
 * 依据 PLAN 第 3 节："B 层（事件流）允许在归一化时折叠连续的流式增量事件
 * （如连续的 message_update）后再比较"。折叠只针对**相邻同类增量**——
 * 非相邻的同类事件（例如两个 turn 各自的 message_update）不合并，
 * 否则会掩盖"事件跑错位置"这类真实缺陷。
 */
export function foldStreamingIncrements(text) {
  const lines = text.split("\n").filter((line) => line.trim() !== "");
  const out = [];
  for (const line of lines) {
    let type = null;
    try {
      type = JSON.parse(line).type;
    } catch {
      type = null;
    }
    const foldable = type === "message_update" || type === "tool_execution_update";
    const previous = out[out.length - 1];
    if (foldable && previous && previous.type === type) {
      previous.count += 1;
      continue;
    }
    out.push({ type, line, count: foldable ? 1 : 0 });
  }
  // 每组只留**第一条**，不带条数：一次响应切成几个增量取决于分块与时序，
  // 不是语义（PLAN 第 3 节允许折叠正是为了吸收这种波动）。带条数会让"3 条 vs 1 条"
  // 判为不同，那就等于没折叠——这条被自测抓过一次。
  return out.map((entry) => entry.line).join("\n");
}

/**
 * 把 JSONL 的每一行按**键序无关**的形式重写（深层排序）。
 *
 * 为什么需要：Rust 侧 serde_json 未启用 `preserve_order`，对象键按字典序输出，
 * 而 JS 的 `JSON.stringify` 按插入序——**两者内容相同、键序不同**。
 * 这一点在 Q8 已裁定："键顺序不是判据"。所以比对前统一按键序无关的形式重写。
 *
 * 注意范围：它**只重排键**，不删改任何值（`errorMessage`、`usage`、`cost` 都原样保留），
 * 与"把某字段归一化掉"是两回事。
 *
 * `requests.jsonl` 的行形如 `{"body":"<内层 JSON 字符串>", …}`，所以内层 body 也要一起重写。
 */
export function canonicalizeJsonLines(text, options = {}) {
  return text
    .split("\n")
    .map((line) => {
      const trimmed = line.trim();
      if (trimmed === "") return line;
      let value;
      try {
        value = JSON.parse(trimmed);
      } catch {
        return line;
      }
      if (value && typeof value === "object" && typeof value.body === "string") {
        try {
          const body = JSON.parse(value.body);
          // 工具调用的 `arguments` 是**字符串里再套一层 JSON**，外层排序到不了它。
          // 它同样属于"键序不是判据"（Q8）：golden 里是
          // `{"path":"out.txt","content":"hello\n"}`，我这边 serde 会输出
          // `{"content":…,"path":…}`（S03 的 C 层比对指出）。两边一起按键序规范化。
          for (const message of body.messages ?? []) {
            for (const call of message.tool_calls ?? []) {
              const raw = call?.function?.arguments;
              if (typeof raw !== "string") continue;
              try {
                call.function.arguments = JSON.stringify(sortKeysDeep(JSON.parse(raw)));
              } catch {
                // 不是 JSON（例如流式截断）：保持原样。
              }
            }
          }
          value.body = JSON.stringify(sortKeysDeep(body, options));
        } catch {
          // 内层不是 JSON：保持原样。
        }
      }
      return JSON.stringify(sortKeysDeep(value, options));
    })
    .join("\n");
}

/** 递归按键名排序（数组保持顺序）。 */
function sortKeysDeep(value, options = {}, key = null) {
  // `preserveObjectKeys`：**这些键下面的对象保持原有键序、不排序**。
  // 用于 A 层的 `toolsAdded[*].parameters`——TS 的 `declarationsEqual`
  // （`ref/.../ai/src/utils/transcript.ts:140-141`）用 `JSON.stringify` 判等，
  // 键序**参与**判等，所以它必须逐字节比，不能被归一化掉。
  if (Array.isArray(value)) return value.map((item) => sortKeysDeep(item, options, key));
  if (value && typeof value === "object") {
    const preserve =
      options?.preserveObjectKeys && key !== null && options.preserveObjectKeys.has(key);
    const keys = preserve ? Object.keys(value) : Object.keys(value).sort();
    const out = {};
    for (const name of keys) out[name] = sortKeysDeep(value[name], options, name);
    return out;
  }
  return value;
}

/**
 * Q18 的机械规则：**只忽略 `message_start` / `message_update` 的 `usage` 字段**。
 *
 * 理由（机制已查清，见 notes/decisions.md）：TS 与我们的实现同构（整块解析 + 发事件时
 * 浅拷贝共享的 partial 对象），差异只来自**网络 chunk 边界**——golden 录制时第 1 行与
 * 其余行落在不同 chunk，于是 `text_start` 的 usage 是 0、后续 delta 是终值。
 * 这是传输时序的产物，不是实现语义差异，也无法靠改代码对齐。
 *
 * **范围严格限定**：只动这两个事件类型的 `usage`。`message_end` / `turn_end` /
 * `agent_end` 的 `usage` 与 `cost` **逐位比较**——那里的用量是终态数据，必须精确。
 */
export function stripStreamingUsage(text) {
  return text
    .split("\n")
    .map((line) => {
      const trimmed = line.trim();
      if (trimmed === "") return line;
      let value;
      try {
        value = JSON.parse(trimmed);
      } catch {
        return line;
      }
      if (value && (value.type === "message_start" || value.type === "message_update")) {
        // 事件顶层的 usage，以及载荷消息里的 usage，都要抹掉。
        delete value.usage;
        if (value.message && typeof value.message === "object") delete value.message.usage;
        return JSON.stringify(value);
      }
      return line;
    })
    .join("\n");
}


/**
 * Q-S25 规则（P4-15 步骤 3）：**只**在 `message_start` 事件里，忽略 `message.content` 中
 * `type === "text"` 且 `text === ""` 的块。
 *
 * 依据（实测，`notes/p413-step1-s25-rerecord.md`）：`golden/_runs/S25-anthropic` 的三次录制里，
 * 同一个 `message_start` 的 `message.content` 有时是 `[]`、有时是 `[{type:"text",text:"",index:0}]`
 * ——TS 侧流式起点的抖动；事件的**其余**字段逐位可比（`usage` 那部分由 Q18 处理）。
 *
 * **范围严格限定**：只处理 `message_start`；只删"空串 text 块"；不碰别的字段、不碰别的块类型。
 * 由调用方按 `meta.json` 的 `provider === "anthropic-messages"` 决定是否启用
 * （见 `compare.mjs` 的 `compareScenario`）。
 */
export function stripEmptyMessageStartTextBlocks(text) {
  return text
    .split("\n")
    .map((line) => {
      const trimmed = line.trim();
      if (trimmed === "") return line;
      let value;
      try {
        value = JSON.parse(trimmed);
      } catch {
        return line;
      }
      if (value && value.type === "message_start" && Array.isArray(value.message?.content)) {
        value.message.content = value.message.content.filter(
          (block) => !(block && block.type === "text" && block.text === ""),
        );
        return JSON.stringify(value);
      }
      return line;
    })
    .join("\n");
}
