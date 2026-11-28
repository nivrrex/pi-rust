# 粘贴折叠夹具（ref 真实 Editor 产出）

- 来源：`ref/pi-0.87.1/packages/tui/src/components/editor.ts` 的 `Editor` 类
  （**只读复制**到 `/tmp/ref-tui/src`，ref 目录未被修改）。
- 生成脚本：`tools/difftest/gen-paste-cases.mjs`（顶部注释有重建步骤）。
- 病例：**33 例**，覆盖阈值边界（9/10/11 行、999/1000/1001 字符、CJK 同阈值）、
  多次粘贴编号、占位符前后插字、退格整删、光标左右穿越、提交展开、ESC 字面量、控制字符、CRLF、空粘贴。
- 每例记录：`text`（编辑器文本，含占位符）、`lines`（显示行）、`cursor`（行/列）、
  `expanded`（提交展开后的原文）、`submitted`（若该例按了回车）。
- 稳定性：连跑 3 次，`sha256` 前 16 位一致 —— `9a9d91a93371077f`。
