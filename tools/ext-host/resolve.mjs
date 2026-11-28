// 裸包名 → 本目录 shim 的解析钩子（ref 侧对应 core/extensions/virtual-modules.ts:12-40）。
// 只映射扩展会 import 的四个虚拟模块；其余（ws/esbuild 等）交给 Node 正常解析。
import { pathToFileURL } from "node:url";
import path from "node:path";

const here = import.meta.dirname;
const MAP = {
  "@earendil-works/pi-tui": path.join(here, "shim/pi-tui.mjs"),
  "@earendil-works/pi-coding-agent": path.join(here, "shim/pi-coding-agent.mjs"),
  "@earendil-works/pi-ai": path.join(here, "shim/pi-ai.mjs"),
  "@mariozechner/pi-tui": path.join(here, "shim/pi-tui.mjs"),
  "@mariozechner/pi-coding-agent": path.join(here, "shim/pi-coding-agent.mjs"),
  "@mariozechner/pi-ai": path.join(here, "shim/pi-ai.mjs"),
  typebox: path.join(here, "shim/typebox.mjs"),
  "@sinclair/typebox": path.join(here, "shim/typebox.mjs"),
};

export async function resolve(specifier, context, next) {
  const target = MAP[specifier];
  if (target) return { url: pathToFileURL(target).href, shortCircuit: true };
  return next(specifier, context);
}
