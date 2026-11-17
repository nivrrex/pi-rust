// `project_trust` 夹具：**不裁决**（`undecided`）。
//
// ref 的语义是"第一个给出 yes/no 的胜出，undecided 继续问下一个"，
// 因此本夹具的结果必须让宿主**继续走后续规则**（`emit_project_trust` 返回 `None`），
// 而不是把 `undecided` 当成"不信任"。
export default function projectTrustUndecided(pi) {
  pi.on("project_trust", () => ({ trusted: "undecided" }));
}
