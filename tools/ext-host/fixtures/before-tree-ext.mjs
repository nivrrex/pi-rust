// `session_before_tree` 夹具：**取消导航**（`{cancel:true}`）。
//
// 注意：扩展宿主进程的环境变量是**白名单式**传入的（`ext_tools.rs` 只传
// `PI_EXT_CTX`/`HOME` 等固定几项），因此夹具**不能**靠自定义环境变量切换行为
// ——最初用 `process.env.PI_BEFORE_TREE` 分流，handler 拿到的值是 undefined、
// 一律走默认分支（实测三路都返回"不介入"）。改为三个独立夹具文件。
export default function beforeTreeCancel(pi) {
  pi.on("session_before_tree", () => ({ cancel: true }));
}
