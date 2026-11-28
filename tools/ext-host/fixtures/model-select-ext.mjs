// `model_select` 夹具：记下每次载荷，用 `session_before_tree` 的返回值带回。
export default function modelSelectProbe(pi) {
  const seen = [];
  pi.on("model_select", (event) => {
    seen.push({ model: event?.model, previousModel: event?.previousModel, source: event?.source });
    return undefined;
  });
  pi.on("session_before_tree", () => ({ summary: JSON.stringify({ seen }) }));
}
