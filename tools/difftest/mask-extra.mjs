// P7-6p(续3)/W1.4：**统一掩码**（诊断脚本专用，避免各脚本内联掩码导致口径漂移）。
// 不改 `normalize.mjs`；其归一化规则保持原样。
//
// 覆盖：临时目录（`/tmp/tui-*`、含 difftest 的临时目录）、ISO 时间戳两种格式（含 `:`/`.` 与含 `-` 的会话文件名形态）、
// UUID、spinner（braille 区间）。
export function mask(text) {
	return String(text)
		.replace(/\/tmp\/tui-[A-Za-z0-9_-]+/g, "<TMP>")
		.replace(/\/tmp\/[A-Za-z0-9._-]*difftest[A-Za-z0-9._-]*/g, "<TMP>")
		.replace(/\d{4}-\d{2}-\d{2}T[\d:.\-]+Z?/g, "<TS>")
		.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, "<UUID>")
		.replace(/[\u2800-\u28FF]/g, "<SPINNER>");
}

/** 逐行掩码并裁掉行尾空白。 */
export function maskLines(lines) {
	return lines.map((line) => mask(String(line).replace(/\s+$/g, "")));
}
