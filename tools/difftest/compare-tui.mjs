// TUI 快照比较（P7-2 W2）。零新增依赖：自带最小 ANSI 屏幕模型。
//
// 归一化规则（照 notes/reports/P7-2-runner-pty-plan.md §3）：
//   固定尺寸、丢弃颜色/粗体（SGR）、时间戳/计数/spinner 占位化、裁剪行尾空格、折叠连续空行。
// 有效产出闸门：两侧快照都必须非空且含目标条目（--require），任一为空/缺目标 ⇒ invalid（**绝不判 same**）。
//
// 用法：
//   node tools/difftest/compare-tui.mjs --left a.bin --right b.bin --cols 100 --rows 30 --require "> " [--json]
import fs from "fs";
import path from "path";

const CSI_FINAL = /[\x40-\x7e]/;

/** 把原始 PTY 字节流渲染成 (rows x cols) 的屏幕格。 */
export function renderScreen(raw, cols, rows) {
  const text = raw.toString("utf8");
  const screen = Array.from({ length: rows }, () => Array.from({ length: cols }, () => " "));
  let r = 0;
  let c = 0;
  let i = 0;
  // 注意：**不自行为 `c>=cols` 推进行**——TS 的显式 `\r\n` 已经在推进行，
  // 模型若再推一次会与真实终端行为不符并造成列/行错位（P7-3 实测教训）。
  // 这里把落点夹到最后一列覆盖写，行推进只由 `\n`/CSI 决定。
  const writeChar = (ch) => {
    const col = c >= cols ? cols - 1 : c;
    if (r < rows && col >= 0) screen[r][col] = ch;
    c = c + 1;
  };
  while (i < text.length) {
    const ch = text[i];
    if (ch === "\x1b") {
      const next = text[i + 1];
      if (next === "[") {
        // CSI: ESC [ params final
        let j = i + 2;
        while (j < text.length && !CSI_FINAL.test(text[j])) j += 1;
        const final = text[j] ?? "";
        const params = text.slice(i + 2, j);
        const nums = params.split(";").map((p) => (p === "" ? 0 : Number.parseInt(p, 10)));
        if (final === "H" || final === "f") {
          r = Math.min(Math.max((nums[0] || 1) - 1, 0), rows - 1);
          c = Math.min(Math.max((nums[1] || 1) - 1, 0), cols - 1);
        } else if (final === "A") r = Math.max(r - (nums[0] || 1), 0); // 光标上移（此前忽略 ⇒ 重绘行错位）
        else if (final === "B") r = Math.min(r + (nums[0] || 1), rows - 1);
        else if (final === "C") c = Math.min(c + (nums[0] || 1), cols - 1);
        else if (final === "D") c = Math.max(c - (nums[0] || 1), 0);
        else if (final === "J") {
          const mode = nums[0] || 0;
          if (mode === 2 || mode === 3) for (let y = 0; y < rows; y += 1) screen[y].fill(" ");
          else if (mode === 0) {
            for (let x = c; x < cols; x += 1) screen[r][x] = " ";
            for (let y = r + 1; y < rows; y += 1) screen[y].fill(" ");
          }
        } else if (final === "K") {
          const mode = nums[0] || 0;
          if (mode === 0) for (let x = c; x < cols; x += 1) screen[r][x] = " ";
          else if (mode === 2) screen[r].fill(" ");
        }
        // m（SGR 颜色/粗体）与其余控制序列：丢弃
        i = j + 1;
        continue;
      }
      if (next === "]") {
        // OSC ... BEL 或 ST
        let j = i + 2;
        while (j < text.length && text[j] !== "\x07" && !(text[j] === "\x1b" && text[j + 1] === "\\")) j += 1;
        i = text[j] === "\x07" ? j + 1 : j + 2;
        continue;
      }
      i += 2; // 其它 ESC x
      continue;
    }
    if (ch === "\r") { c = 0; i += 1; continue; }
    if (ch === "\n") {
      // **底行换行必须整屏上滚**（P7-4g/W1）：此前只把行号 clamp 在最后一行，
      // 于是真实终端（与 vt100）会把内容上移、而本模型把最后一行覆盖掉 ——
      // 实测后果：T05 的末帧丢掉 `cwd` 行（vt100 同输入有该行）。
      if (r >= rows - 1) {
        for (let y = 1; y < rows; y += 1) screen[y - 1] = screen[y].slice();
        screen[rows - 1] = Array.from({ length: cols }, () => " ");
      } else {
        r += 1;
      }
      c = 0;
      i += 1;
      continue;
    }
    if (ch === "\b") { c = Math.max(c - 1, 0); i += 1; continue; }
    if (ch === "\x07") { i += 1; continue; }
    if (ch === "\t") { c = Math.min(c + (8 - (c % 8)), cols - 1); i += 1; continue; }
    if (ch < " ") { i += 1; continue; }
    for (const grapheme of Array.from(ch + (text[i + 1] ?? ""))) void grapheme; // 占位：按码点推进
    writeChar(ch);
    i += 1;
  }
  return screen.map((row) => row.join(""));
}

/** 归一化：裁剪行尾空格、折叠连续空行、时间戳/spinner/计数占位化。 */
export function normalizeScreen(lines) {
  const placeholderRules = [
    [/\b\d+(\.\d+)?\s*(s|ms|sec|secs|seconds)\b/gi, "<DURATION>"],
    [/\b\d+(\.\d+)?\s*(tok|tokens)\/s\b/gi, "<RATE>"],
    [/[\u2800-\u28FF]/g, "<SPINNER>"], // 仅 braille 区间（不吞 | \ / -，否则误伤 UI 文本与制表线）
    [/\b\d+\s*(tok|tokens)\b/gi, "<TOKENS>"],
    [/\[[0-9;]*m/g, ""],
    // 每次录制的工作目录不同（mkdtemp）⇒ 掩成 <TMP>，否则快照永远不同。
    // 每轮录制的工作目录与 session id 都不同 ⇒ 掩成占位符（它们不是行为差异）。
    [/\/tmp\/tui-[A-Za-z0-9_-]+/g, "<TMP>"],
    // B1（P8e-11）：录制时 TS 会把自己安装目录的绝对路径写进画面/请求里
    // （`<home>/…/node_modules/@earendil-works/pi-coding-agent/docs/*.md`）。
    // 两侧统一掩成 `<PI_PKG>`，这样 golden 里不必保留本机路径。
    [/[^\s"']*node_modules\/@earendil-works\/pi-coding-agent/g, "<PI_PKG>"],
    [/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, "<UUID>"],
    // 会话列表里的**相对年龄**（`9mo`/`3d`/`2h`）会随录制时间失效：
    // 同一份 golden 过一个月就从 `9mo` 变成 `10mo`（T38/T57 实际踩到）。
    // 与"时间戳归零"同一口径，掩成 `<AGE>`。
    [/\b\d+(?:mo|[dhw])\b/g, "<AGE>"],
    // 年龄列是**定宽右对齐**的：`9mo`→`10mo` 会让前面的空格少一格，
    // 只掩文本不掩间距仍会差一格，所以把间距一起收敛。
    [/(\s)<AGE>/g, "$1<AGE>"],
    [/ {2,}<AGE>/g, " <AGE>"],
    // session 文件名里的 ISO 时间戳（如 2026-10-16T09-51-43-492Z）每次不同 ⇒ 占位化（方案 §3）。
    [/\d{4}-\d{2}-\d{2}T[\d:.\-]+Z?/g, "<TIMESTAMP>"],
    [/\/tmp\/[A-Za-z0-9._-]*difftest[A-Za-z0-9._-]*/g, "<TMP>"],
  ];
  const mapped = lines.map((line) => {
    // 整行掩码：TUI 底部的 "resume session" 提示行含每轮新生成的 session id/命令，
    // 且 UUID 可能被换行切断（单行正则匹配不到）⇒ 整行替换为占位符（已登记为归一化规则）。
    if (/--session-dir|is session:/.test(line)) return "<SESSION-HINT>";
    // 纯十六进制行：`To resume this session: … --session <UUID>` 若在行边界处被切断，
    // 其**尾巴**（如 `8421e4fd8dc7`）会落到下一行、首段的 `<SESSION-HINT>` 掩不到 ⇒ 单独掩掉。
    if (/^[0-9a-f]{8,}$/i.test(line.trim())) return "<ID-FRAGMENT>";
    let out = line.replace(/\s+$/g, "");
    for (const [pattern, replacement] of placeholderRules) out = out.replace(pattern, replacement);
    // 含 `<AGE>` 的行（会话列表行）是**列对齐**排版：年龄文本变长会让前面的
    // 填充空格少一格。这类行把连续空格收敛成一个，只比较"有哪些字段"。
    if (out.includes("<AGE>")) out = out.replace(/ {2,}/g, " ");
    return out;
  });
  const collapsed = [];
  for (const line of mapped) {
    if (line === "" && collapsed.length > 0 && collapsed[collapsed.length - 1] === "") continue;
    collapsed.push(line);
  }
  while (collapsed.length > 0 && collapsed[collapsed.length - 1] === "") collapsed.pop();
  return collapsed;
}

/** 有效产出闸门：两侧都非空且都含 required 中的每一条。 */
/** P7-4/W0e：取 raw 同目录 `meta.json` 里最后一个 snapshot 偏移；没有则 null。 */
export function lastSnapshotOffset(metaPath) {
  try {
    const meta = JSON.parse(fs.readFileSync(metaPath, "utf8"));
    const offsets = meta.snapshotOffsets ?? [];
    if (offsets.length === 0) return null;
    return offsets[offsets.length - 1].offset;
  } catch {
    return null;
  }
}

/**
 * P7-4/W0e：把 raw 截到指定偏移。
 * 理由：退出后 pi 会打印 resume 提示，屏幕较满时会把会话中的最后两行（cwd、footer）覆盖掉
 * ⇒ 不截断则比较的"最终屏幕"并非会话进行中的屏幕（T05 的 golden 曾因此缺两行）。
 */
const SYNC_END = Buffer.from("\u001b[?2026l");

/**
 * P7-4c：把偏移**回退到它之前最后一个同步帧结束（`\u001b[?2026l` 的结尾）**。
 * 诊断依据（notes/reports/P7-4c-trim-diag.md）：三个 `snapshotOffsets` 实测都落在"最后一个 ?2026l 之后
 * 若干字节"处（T01/T04/T05/T06 均为 14 字节），即偏移点位于一个尚未完成的帧中间 ⇒ 直接按偏移截断，
 * 渲染出的画面可能停在半个帧上。回退到帧尾可把截断点固定在同一类边界上。
 * @returns 回退后的偏移；**偏移之前不存在 `?2026l` 时返回 null（判 invalid）**，调用方必须报错而不是退回全量。
 */
export function snapOffsetToFrameEnd(raw, offset) {
  if (offset === null || offset === undefined || offset < 0 || offset > raw.length) return null;
  let last = -1;
  let idx = raw.indexOf(SYNC_END);
  // 必须要求整个同步结束序列都**早于**偏移：否则偏移恰好落在该序列内部时，
  // 会把一个"尚未读完的帧边界"当成边界（首版实测：第二帧的 ?2026l 起点 36 < 偏移 40 ⇒ 误判为 44）。
  while (idx >= 0 && idx + SYNC_END.length <= offset) {
    last = idx + SYNC_END.length;
    idx = raw.indexOf(SYNC_END, idx + SYNC_END.length);
  }
  return last < 0 ? null : last;
}

export function trimRawToOffset(raw, offset) {
  if (offset === null || offset === undefined || offset < 0 || offset >= raw.length) return raw;
  return raw.subarray(0, offset);
}

export function gate(leftLines, rightLines, required = []) {
  const nonEmpty = (lines) => lines.some((line) => line.trim() !== "");
  if (!nonEmpty(leftLines) || !nonEmpty(rightLines)) {
    return { ok: false, reason: "空快照（两侧均须非空）" };
  }
  for (const needle of required) {
    if (!leftLines.some((line) => line.includes(needle))) {
      return { ok: false, reason: `左侧缺目标条目: ${JSON.stringify(needle)}` };
    }
    if (!rightLines.some((line) => line.includes(needle))) {
      return { ok: false, reason: `右侧缺目标条目: ${JSON.stringify(needle)}` };
    }
  }
  return { ok: true, reason: "" };
}

export function compareTui(leftRaw, rightRaw, { cols, rows, required = [] }) {
  const left = normalizeScreen(renderScreen(leftRaw, cols, rows));
  const right = normalizeScreen(renderScreen(rightRaw, cols, rows));
  const gateResult = gate(left, right, required);
  if (!gateResult.ok) return { status: "invalid", reason: gateResult.reason, left, right };
  const same = left.length === right.length && left.every((line, index) => line === right[index]);
  return { status: same ? "same" : "different", reason: "", left, right };
}

function main() {
  const argv = process.argv.slice(2);
  const get = (name, fallback) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : fallback;
  };
  const leftPath = get("--left");
  const rightPath = get("--right");
  const cols = Number.parseInt(get("--cols", "100"), 10);
  const rows = Number.parseInt(get("--rows", "30"), 10);
  const required = argv.reduce((acc, item, index) => {
    if (item === "--require") acc.push(argv[index + 1]);
    return acc;
  }, []);
  if (!leftPath || !rightPath) {
    console.error("用法: node compare-tui.mjs --left a.bin --right b.bin --cols 100 --rows 30 [--require 文本]");
    process.exit(2);
  }
  // P7-4/W0e：默认按各自 meta.json 的最后一个 snapshot 偏移截断（--no-trim 关闭）。
  const noTrim = argv.includes("--no-trim");
  // 两侧必须**同口径**截断：只要有一侧没有 snapshotOffsets（旧 golden、或该场景
  // 没记录 marks），就两侧都按完整流比较；否则会出现"一侧截断、一侧不截断"的
  // 假差异（T57 就是这样：golden 有 meta、本次运行没产出 meta）。
  const offsets = [leftPath, rightPath].map((rawPath) =>
    noTrim
      ? null
      : lastSnapshotOffset(path.join(path.dirname(path.resolve(rawPath)), "meta.json")),
  );
  const trimTogether = !noTrim && offsets.every((offset) => offset !== null);
  const load = (rawPath) => {
    const raw = fs.readFileSync(rawPath);
    if (!trimTogether) return raw;
    const offset = lastSnapshotOffset(path.join(path.dirname(path.resolve(rawPath)), "meta.json"));
    if (offset === null) return raw; // 没有 marks（旧 golden / 未记录）⇒ 维持旧行为，不截断
    const snapped = snapOffsetToFrameEnd(raw, offset);
    if (snapped === null) {
      throw new Error(`截断偏移之前的原始流里没有 \u001b[?2026l 帧边界（invalid，不得退回全量）: ${rawPath} offset=${offset}`);
    }
    return trimRawToOffset(raw, snapped);
  };
  const result = compareTui(load(leftPath), load(rightPath), { cols, rows, required });
  console.log(`[compare-tui] ${result.status}${result.reason ? ` (${result.reason})` : ""}`);
  if (argv.includes("--json")) console.log(JSON.stringify(result, null, 2));
  process.exit(result.status === "same" ? 0 : 1);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
