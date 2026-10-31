// P7-6k/W1：ESM 解析钩子——把上游 TS 源里的裸包名解析到**已安装 pi 包自带的** node_modules。
// 目的：不复制上游源码、不改 ref 目录、不新增 npm 依赖。
// 基线目录由 TS_PI_ENTRY 推导（`<pkg>/dist/bundle/cli.js` ⇒ `<pkg>/node_modules`）。
import { pathToFileURL } from "node:url";
import path from "node:path";
import { createRequire } from "node:module";

const entry = process.env.TS_PI_ENTRY;
if (!entry) {
	throw new Error("缺少 TS_PI_ENTRY（指向 pi 包的 dist/bundle/cli.js）");
}
const pkgDir = path.resolve(path.dirname(entry), "../..");
const modulesDir = path.join(pkgDir, "node_modules");

// 打包进 pi 包 node_modules 的运行时依赖（`ref/` 检出目录没有 node_modules，
// 而 ref 源码里 `import { … } from "@earendil-works/pi-tui"` 需要能解析）。
const bundled = (name) => path.join(modulesDir, "@earendil-works", name, "dist/index.js");

const MAP = {
	"get-east-asian-width": path.join(modulesDir, "get-east-asian-width/index.js"),
	marked: path.join(modulesDir, "marked/lib/marked.esm.js"),
	"@earendil-works/pi-tui": bundled("pi-tui"),
	"@earendil-works/pi-ai": bundled("pi-ai"),
	"@earendil-works/pi-agent-core": bundled("pi-agent-core"),
};

// 兜底：ref 检出的 TS 源码会 import 一批裸包（chalk、typebox、…），`ref/` 下没有
// node_modules。这里按"已安装 pi 包的 node_modules"解析一次，解析不到再抛原错误。
// 只在直接解析失败时触发，不改变既有映射行为。
const fallbackRequire = createRequire(path.join(modulesDir, "__oracle__.js"));

export async function resolve(specifier, context, next) {
	const target = MAP[specifier];
	if (target) return { url: pathToFileURL(target).href, shortCircuit: true };
	try {
		return await next(specifier, context);
	} catch (error) {
		if (specifier.startsWith(".") || specifier.startsWith("node:") || specifier.startsWith("file:")) {
			throw error;
		}
		try {
			return { url: pathToFileURL(fallbackRequire.resolve(specifier)).href, shortCircuit: true };
		} catch {
			throw error;
		}
	}
}
