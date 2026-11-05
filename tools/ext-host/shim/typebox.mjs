// typebox 的最小可用实现：够把 `Type.Object/String/Number/...` 变成 JSON Schema。
// 完整校验留给宿主（协议里参数是 JSON Schema）。未知构造器显式报"未实现"。
function schema(type, extra = {}) {
  return { type, ...extra };
}
const unimplemented = (name) => () => {
  throw new Error(`pi-ext-host shim: typebox.${name} 未实现（P9 后续批次）`);
};
export const Type = {
  Object: (properties = {}, options = {}) =>
    schema("object", { properties, ...options }),
  String: (options = {}) => schema("string", options),
  Number: (options = {}) => schema("number", options),
  Integer: (options = {}) => schema("integer", options),
  Boolean: (options = {}) => schema("boolean", options),
  Array: (items, options = {}) => schema("array", { items, ...options }),
  Optional: (value) => value,
  Literal: (value) => ({ const: value }),
  Union: (anyOf) => ({ anyOf }),
  Record: unimplemented("Record"),
  Tuple: unimplemented("Tuple"),
  Enum: unimplemented("Enum"),
  Any: (options = {}) => ({ ...options }),
  Null: () => ({ type: "null" }),
};
export const Static = undefined;
export default { Type };
