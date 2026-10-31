// P7-6k/W1：注册 ESM 解析钩子（`node --import ./register.mjs …`）。
import { register } from "node:module";
register("./resolve-hook.mjs", import.meta.url);
