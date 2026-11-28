// `after_provider_response` 夹具。
export default function afterResponseProbe(pi) {
  const seen = [];
  pi.on("after_provider_response", (event) => {
    seen.push({ status: event?.status, headers: event?.headers });
    return undefined;
  });
  pi.on("session_before_tree", () => ({ summary: JSON.stringify({ seen }) }));
}
