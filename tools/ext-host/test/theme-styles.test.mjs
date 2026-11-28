#!/usr/bin/env node
// F72：`ctx.ui.theme` 的样式函数必须**产出 ANSI**，不能把文本原样返回。
//
// 判据来自 ref `extensions/types.ts:201-215` 的 `Theme` 接口与
// `theme/theme.ts` 的实现：`bold`/`dim`/`italic`/`bg` 都是"包一层 SGR"，
// 不是恒等函数。此前这四项是空实现（`String(text)`）⇒ 样式静默丢失。
//
// 真实调用路径：`pi-interact` 在 `ctx.ui.custom` 的组件里用 `theme.bold(...)`
// 渲染标题与提示行；`pi-ai-summary` 用 `theme.bg(...)` 画底色。
import { test } from "node:test";
import assert from "node:assert/strict";

import { makeThemeStub } from "../shim/theme-colors.mjs";

const ESC = "\u001b";
const theme = makeThemeStub({ themeName: "dark", trueColor: true });

test("bold/dim/italic wrap the text in SGR instead of returning it verbatim", () => {
  for (const [name, open, close] of [
    ["bold", `${ESC}[1m`, `${ESC}[22m`],
    ["dim", `${ESC}[2m`, `${ESC}[22m`],
    ["italic", `${ESC}[3m`, `${ESC}[23m`],
  ]) {
    const out = theme[name]("X");
    assert.notEqual(out, "X", `${name} 不能是恒等函数`);
    assert.ok(out.includes(open), `${name} 应含 ${JSON.stringify(open)}，实际 ${JSON.stringify(out)}`);
    assert.ok(out.includes(close), `${name} 应含收尾 ${JSON.stringify(close)}`);
    assert.ok(out.includes("X"), `${name} 应保留正文`);
  }
});

test("bg paints with the theme's background palette and resets after", () => {
  // 背景色键与前景色键是**两套**（ref `theme.ts:92-101` 的 `ThemeBg` 只有 7 个），
  // 所以这里用 `selectedBg` 而不是 `accent`。
  const out = theme.bg("selectedBg", "G");
  assert.notEqual(out, "G", "bg 不能是恒等函数");
  assert.ok(out.includes(`${ESC}[48`), `bg 应含背景色 SGR，实际 ${JSON.stringify(out)}`);
  assert.ok(out.includes(`${ESC}[49m`), "bg 应以 RESET_BG 收尾");
  assert.ok(out.includes("G"), "bg 应保留正文");
});

test("unknown color names throw instead of silently dropping the style", () => {
  assert.throws(() => theme.fg("definitely-not-a-color", "x"), /Unknown theme color/);
  // ref 的 `bg()` 错误文案与 `fg()` **不同**（`theme.ts:331` 的
  // `Unknown theme background color`）。
  assert.throws(() => theme.bg("definitely-not-a-color", "x"), /Unknown theme background color/);
});

test("light theme uses the light palette", () => {
  const dark = makeThemeStub({ themeName: "dark", trueColor: true }).fg("text", "x");
  const light = makeThemeStub({ themeName: "light", trueColor: true }).fg("text", "x");
  assert.notEqual(dark, light, "light 与 dark 的前景色应不同");
});

// ref `theme.ts:92-101` 的 `ThemeBg` 只有 7 个键；`bg()` 用**独立**的背景色表
// （`theme.ts:329-333`），不是把前景色表加 `48` 前缀。
test("bg only accepts the seven ThemeBg keys", () => {
  for (const key of [
    "selectedBg",
    "searchMatchBg",
    "userMessageBg",
    "customMessageBg",
    "toolPendingBg",
    "toolSuccessBg",
    "toolErrorBg",
  ]) {
    const out = theme.bg(key, "x");
    assert.ok(out.includes(`${ESC}[48`), `${key} 应产出背景色 SGR，实际 ${JSON.stringify(out)}`);
  }
});

test("bg rejects foreground colour names, as ref does", () => {
  // ref 的 `bg()` 只查 `bgColors`，`accent` 这类前景色名会抛
  // `Unknown theme background color`。此前 pi-rs 把前景色表加 48 前缀自造了一套背景色。
  for (const key of ["accent", "border", "success", "error", "text"]) {
    assert.throws(() => theme.bg(key, "x"), /Unknown theme background color/, key);
  }
});

test("dark and light use their own background values", () => {
  const dark = makeThemeStub({ themeName: "dark", trueColor: true }).bg("selectedBg", "x");
  const light = makeThemeStub({ themeName: "light", trueColor: true }).bg("selectedBg", "x");
  assert.notEqual(dark, light, "light 与 dark 的 selectedBg 应不同");
  // ref dark.json 的 vars.selectedBg = #3a3a4a，light.json = #d0d0e0。
  assert.ok(dark.includes("58;58;74"), `dark selectedBg 应是 58;58;74，实际 ${JSON.stringify(dark)}`);
  assert.ok(light.includes("208;208;224"), `light selectedBg 应是 208;208;224，实际 ${JSON.stringify(light)}`);
});
