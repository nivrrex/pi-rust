// `project_trust` 夹具：**两个**处理器，第一个给 `no`、第二个给 `undecided`。
//
// 顺序很关键：专用分支（ref `runner.ts:292-322`）遇到第一个 `yes`/`no` 就**停**，
// 所以结果必须是 `no`。若把所有 handler 的返回值**合并**，后一个 `undecided`
// 会覆盖前一个 `no` ⇒ 宿主按"不裁决"处理，一个明确说"不信任"的扩展被静默忽略。
export default function projectTrustTwo(pi) {
  pi.on("project_trust", () => ({ trusted: "no", remember: true }));
  pi.on("project_trust", () => ({ trusted: "undecided" }));
}
