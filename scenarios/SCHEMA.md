# scenarios/SCHEMA.md — 场景脚本格式

场景脚本是 JSON，`mock-provider` 与 `tools/difftest` 都读它。小而明确，不做可编程逻辑：
时序完全由"响应数组的顺序"与"闸门位置"决定。

## 顶层字段

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `id` | string | 是 | 场景号，如 `S01`。仅供 runner 与产物目录命名 |
| `description` | string | 否 | 一句话说明这个场景测什么 |
| `provider` | string | 是 | `openai-completions` 或 `anthropic-messages`；runner 据此生成 `models.json` |
| `model` | string | 是 | 假模型 id，默认用 `mock-model` |
| `mode` | string | 否 | `print`（默认）或 `rpc`；rpc 场景由 runner 通过 stdin 驱动 |
| `commands` | string[] | 否 | `mode: "rpc"` 时的 stdin 命令序列，见下 |
| `files` | object | 否 | 运行前在临时工作目录里预置的文件：`{"路径": "内容"}` |
| `tui` | object | 否 | **TUI 场景**：`{"cols":100,"rows":30,"steps":[...],"snapshots":["name"]}`。`cols`/`rows` 为 1..1000 的整数；`steps` 非空，动作 ∈ `waitScreen`（配 `contains`/`timeoutMs`）、`waitStable`（配 `quietMs`/`timeoutMs`）、`type`（配 `text`/可选 `delayMs`）、`snapshot`（配 `name`）、`delay`（配 `ms`：显式等待，TUI 等模型回复时无输出、`waitStable` 会过早判稳）、`waitExit`；`snapshots` 必须非空且与 `snapshot` 步骤的 `name` 一一对应。未知键、非法尺寸、未知动作、快照不匹配、缺 `steps` 均被场景校验器拒绝（与 `deny_unknown_fields` 同精神）。仅 TUI 场景需要；普通场景不得出现该键。 |
| `responses` | object[] | 是 | 按请求顺序回放；用完以后重复最后一条 |

## responses 的一���

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `file` | string | — | SSE 夹具路径，相对**仓库根**，通常引用 `fixtures/sse/**` |
| `status` | number | 200 | 非 200 时返回 JSON 错误体并断开，用于 429 等 |
| `retryAfter` | string | — | 写 `Retry-After` 响应头（秒数字符串） |
| `gateAfterChunks` | number | — | 发出第 N 块后暂停，等 `POST /control/release` 才继续 |
| `disconnectAfterChunks` | number | — | 发出第 N 块后直接断开（流中途断连） |

同一个场景里要"先报错再成功"就写两条 `responses`，靠请求顺序区分。

## `followUp` 的分阶段实现（跨实现场景 X01/X02）

`followUp` 的每个条目可带 `impl`：

| 取值 | 含义 |
|---|---|
| `same`（默认） | **与主实现相同**（对拍时主=pi-rs 就跑 pi-rs） |
| `ts` | 该阶段固定用 TS 版 pi |
| `pi-rs` | 该阶段用 Rust 版 pi-rs |
| `other` | **与主实现相反**的那个：主=TS → pi-rs；主=pi-rs → TS |

跨实现场景的**基线录制**要固定成"全 TS"：设 `DIFTEST_FORCE_TS=1`，
所有阶段都跑 TS，于是录出来的是 TS→TS→TS；对拍时（不设该变量）按场景里写的**绝对**实现跑，例如 X01 = TS（主）→ pi-rs → TS。
这样比较的就是"Rust 的续写结果与 TS 的续写结果是否无法区分"。

条目键名只允许 `prompt`/`cliArgs`/`impl`，写错会被场景校验器拒绝。

## rpc 场景的 commands

`commands` 是逐行发送的 stdin 命令（严格 JSONL）。runner 会在每条命令后等待
`gateAfterChunks` 闸门被释放，除非该场景没有闸门。支持 `abort`、`prompt`、`steer`、
`follow_up`、`get_state` 等。

## 例子

```json
{
  "id": "S01",
  "description": "纯文本回复",
  "provider": "openai-completions",
  "model": "mock-model",
  "mode": "print",
  "files": { "input.txt": "hello\n" },
  "responses": [{ "file": "fixtures/sse/openai/text-basic.sse" }]
}
```
