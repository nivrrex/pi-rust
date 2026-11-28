// `before_provider_headers` 夹具：新增一个头、改写一个、删除一个。
export default function providerHeadersProbe(pi) {
  pi.on("before_provider_headers", (event) => {
    const headers = Array.isArray(event?.headers) ? event.headers : [];
    const out = headers
      .filter(([name]) => name !== "x-drop-me")
      .map(([name, value]) => (name === "x-replace-me" ? [name, "replaced"] : [name, value]));
    out.push(["x-injected", "from-extension"]);
    return { headers: out };
  });
}
