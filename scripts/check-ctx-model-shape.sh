#!/bin/sh
# F234：`ctx.model` 必须是**模型对象**（ref `extensions/types.ts:327`
# `model: Model<any> | undefined`），不能是模型 id 字符串。
#
# 为什么用静态检查而不是只靠用例：出问题的是**调用点选错了函数**——
# `update_ctx_model(Some("<id>"), …)` 会把 ctx.model 写成字符串，
# 于是扩展里 `ctx.model.provider`/`ctx.model.id` 全是 undefined。
# 真机症状：`Ctrl+P` 切模型后扩展页脚变成 `(undefined) no-model`。
#
# 允许传 `None`（只改档位、不动模型）；只禁 `Some(...)` 那一半。
set -eu
root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$root"

hits="$(grep -rn 'update_ctx_model(Some(' crates/*/src --include=*.rs || true)"
if [ -n "$hits" ]; then
    echo "check-ctx-model-shape: 非测试代码把模型写成了字符串（应改用 set_ctx_model_object）" >&2
    printf '%s\n' "$hits" >&2
    exit 1
fi
echo "check-ctx-model-shape: OK"
