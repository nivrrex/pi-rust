// P7-6k/W1：ESM 解析钩子——把上游 TS 源里的裸包名解析到**已安装 pi 包自带的** node_modules。
// 目的：不复制上游源码、不改 ref 目录、不新增 npm 依赖。
// 基线目录由 TS_PI_ENTRY 推导（`<pkg>/dist/bundle/cli.js` ⇒ `<pkg>/node_modules`）。
import { pathToFileURL } from "node:url";
import path from "node:path";

const entry = process.env.TS_PI_ENTRY;
if (!entry) {
	throw new Error("缺少 TS_PI_ENTRY（指向 pi 包的 dist/bundle/cli.js）");
}
const pkgDir = path.resolve(path.dirname(entry), "../..");
const modulesDir = path.join(pkgDir, "node_modules");

const MAP = {
	"get-east-asian-width": path.join(modulesDir, "get-east-asian-width/index.js"),
	marked: path.join(modulesDir, "marked/lib/marked.esm.js"),
};

export async function resolve(specifier, context, next) {
	const target = MAP[specifier];
	if (target) return { url: pathToFileURL(target).href, shortCircuit: true };
	return next(specifier, context);
}
