# P3：七个选择器的文案对齐（P8e-15）

判定依据：**TS 录制的 golden 是唯一真值**。全量回归 `same=202/0`（dark）、`same=36/0`（light）
说明：凡 golden 覆盖到的帧，pi-rs 的文案与 TS **逐字节一致**。下面逐个说明覆盖情况与结论。

| 选择器 | 覆盖它的 golden | 结论 |
|---|---|---|
| settings | T29/T30/T31（settings 列表与滚动/变更） | 已覆盖，same |
| model | T26/T27（模型选择器与过滤） | 已覆盖，same |
| theme | T33（settings 列表帧；子菜单未展开） | 列表项已覆盖；子菜单文案取自 ref `settings-selector.ts:316-345`，逐条核对一致 |
| resume | 11 个 golden 含 `Resume Session` | 已覆盖，same |
| tree | 12 个 golden 含 `Session Tree` | 已覆盖，same |
| summarize | 3 个 golden 含 `Summarize` | 已覆盖，same |
| warnings | T34（warnings 子菜单） | 已覆盖，same |

## 说明
- 这些选择器的黄金帧都由 TS 录制（`golden-tui/*/run-{1,2,3}`），不是我们生成的；
- 未单独新增 golden 的原因：上述七个都已有 TS 录制的场景覆盖（列表/子菜单/滚动），
  再加同一面板的 golden 属于重复录制；**新增的只有 P8e-14/Q2.c 那四个粘贴场景**；
- 若后续改动触及这些面板文案，回归会立刻在对应场景报 different。
