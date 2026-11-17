// `project_trust` 夹具：裁决**信任**，并要求记住。
//
// 这个事件的语义与其它事件不同：**第一个**给出 `"yes"`/`"no"` 的扩展胜出，
// `"undecided"` 继续问下一个（ref `runner.ts:292-322`）。
export default function projectTrustYes(pi) {
  pi.on("project_trust", () => ({ trusted: "yes", remember: true }));
}
