# spec/00-index.md — 规格抽取索引

本索引只列"从上游哪些文件抽规格"，具体条目见 a–i 各文件。

## 基线（已验证）

| 项 | 值 |
|---|---|
| 仓库 | `https://github.com/earendil-works/pi.git`（monorepo，`packages/*`） |
| 位置 | `ref/pi-0.87.1/`（完整 clone，109 MB，已 `chmod -R a-w` 设为只读） |
| tag | `v0.87.1`（lightweight tag，`git cat-file -t v0.87.1` → `commit`） |
| HEAD | `f07218c4d4bbc12bef056a7058c3dd49dfe41abe`（短 `f07218c`），与 PLAN.md 一致 |
| HEAD 提交信息 | `Release v0.87.1` |
| 工作区状态 | clean（`git status --short` 无输出） |

相关包版本（`packages/*/package.json`，已验证均为 `0.87.1`）：`coding-agent`、`ai`、`agent`、`tui`、`chord`、`protocol`。

## npm 包与 tag 源码的一致性（已验证）

对照对象：`<PI_TS_INSTALL>/node_modules/@earendil-works/pi-coding-agent@0.87.1`。

| 检查项 | 方法 | 结果 |
|---|---|---|
| 版本号 | 对比两边 `package.json` | 都是 `0.87.1`，`bin` 同为 `dist/bundle/cli.js`，`engines.node >= 22.19.0` |
| CHANGELOG.md | md5 | 两边同为 `fd85cd807aef18782761e847798e4d94`，字节相同 |
| README.md | md5 | 两边同为 `12ba839839f71a58911a0a90c0ef16ad`，字节相同 |
| `docs/` | 42 个文件列表 diff + `diff -rq` 内容 diff | 文件列表与内容完全一致，无差异 |
| **dist 的实际代码** | npm 包 `dist/**/*.js.map` 共 216 个，均带 `sourcesContent`；提取其中 216 个唯一 `src/**` 文件，与 `ref/pi-0.87.1/packages/coding-agent/src/**` 逐文件 md5 比对 | **216/216 字节相同，0 不同，0 缺失** |

结论：npm 包 `@earendil-works/pi-coding-agent@0.87.1` 可作为 TS 侧行为的对照基准，其 dist 就是 tag `v0.87.1` 源码的构建产物。

已验证的边界：
- 覆盖范围是 `packages/coding-agent/src/**` 中被 tsc 编译进 `dist` 的 216 个文件。`src/test/**`、`src/bun/**` 编译进 standalone 二进制的部分、以及 `packages/{ai,agent,tui,chord,protocol}` 的源码不在此 216 个之内（它们在 npm 里是独立包，运行时依赖版本也已核对为 `0.87.1`，但源码未做逐字节比对——**待验证**）。
- 未尝试从 ref 源码重新构建并比对 bundle 产物哈希：本机无 `bun`（`which bun` → 未找到）。**待验证**

## 规格来源映射（T0.3 的 a–i）

路径均相对 `ref/pi-0.87.1/packages/`。docs 列在 `coding-agent/docs/`，是散文说明，**以 src 为准**，docs 只作交叉验证。

| 卡 | 主题 | 主要来源（src） | 辅助来源（docs / 测试） |
|---|---|---|---|
| a | 消息与内容类型、StopReason、Usage | `ai/src/`（`api/`、`types*`、`compat*`）、`agent/src/types.ts`、`coding-agent/src/core/messages.ts` | `coding-agent/docs/message-types.md` |
| b | agent loop 与事件序（steering / follow-up / finishTurn / agent_before_settle） | `agent/src/agent-loop.ts`、`agent/src/agent.ts`、`coding-agent/src/core/agent-session.ts`、`coding-agent/src/core/event-bus.ts`、`coding-agent/src/modes/json-event.ts` | `coding-agent/docs/rpc.md`、`coding-agent/test/` |
| c | session 格式（header、全部 entry 类型、parentId、0.87.0 的 `context_edit`、版本与迁移） | `coding-agent/src/core/session-manager.ts`、`coding-agent/src/migrations.ts`、`coding-agent/src/core/session-cwd.ts` | `coding-agent/docs/session-format.md`、`docs/sessions.md` |
| d | compaction（阈值、cut point、split-turn、retain-none、per-model 覆盖） | `coding-agent/src/core/compaction/{compaction,branch-summarization,utils,index}.ts` | `coding-agent/docs/compaction.md` |
| e | 四个工具（read/write/edit/bash）的入参 schema、截断常量、错误文本 | `coding-agent/src/core/tools/{read,write,edit,bash,truncate,output-accumulator,path-utils}.ts`、`core/tools/index.ts`、`core/tools/renderers/` | `coding-agent/docs/`（工具说明段落） |
| f | settings / models / auth 的字段 | `coding-agent/src/core/{settings-manager,model-config,model-registry,models-store,auth-storage,runtime-credentials,resolve-config-value}.ts`、`coding-agent/src/config.ts`、`ai/src/{models,models-store,model-catalog,env-api-keys,auth/}.ts` | `docs/settings.md`、`docs/models.md`、`docs/configuration.md` |
| g | 系统提示模板（逐字抽取） | `coding-agent/src/core/system-prompt.ts`（及 `core/defaults.ts`、`core/agent-session*.ts` 中的拼装处） | `docs/how-pi-works.md` |
| h | rpc 协议 | `coding-agent/src/modes/rpc/{rpc-mode,rpc-types,rpc-client,jsonl}.ts` | `docs/rpc.md`、`docs/rpc-commands.md`、`docs/rpc-extension-ui.md` |
| i | CLI 参数 | `coding-agent/src/cli/args.ts`、`coding-agent/src/main.ts`、`coding-agent/src/cli/{initial-message,file-processor,setup,startup-ui,auth-check,session-picker}.ts` | `docs/cli.md`、`docs/usage.md` |
| （范围外） | skills / prompt templates / AGENTS.md 加载 | `coding-agent/src/core/{skills,prompt-templates,resource-loader}.ts` | `docs/skills.md`、`docs/prompt-templates.md` |
| （范围外，PLAN 第 0 节明确排除） | codemode、MCP、tool search、virtual models、classifier | `coding-agent/src/extensions/`、`packages/cody/…` 不存在；相关代码在 `coding-agent/src/core/extensions/`、`core/tools-manager.ts`、`extensions/llama/` | 不抽 |

## 抽规格时的硬规则

1. 每条规格带 `文件:行号`，行号以本 checkout 为准（`ref/pi-0.87.1` 只读，不随上游更新）。
2. 抽不出来的标"待验证"，不猜。
3. 系统提示（卡 g）必须逐字复制，不重写、不"整理"。

## 下一步

T0.3 的 a–i 逐卡抽取；卡与卡之间不合并，一次一张。
---

