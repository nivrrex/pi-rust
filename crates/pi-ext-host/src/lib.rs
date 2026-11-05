//! 扩展宿主（P9-1-B 第一批）：进程管理 + 握手 + `registerTool` 往返。
//!
//! 传输与协议：`notes/p9-1/protocol.md`（S5 决策：loopback TCP + 一次性令牌）。
//! 本批**只做**注册工具的最小闭环，不碰事件分发 / UI 帧 / 渲染钩子。
//!
//! 线程安全：`ExtHost` 不是 `Sync`（内部 `&mut self` 调用），使用方用 `Arc<Mutex<..>>` 串行化；
//! `ExtTool` 是 `Send + Sync`（`Arc<Mutex<ExtHost>>`）。

pub mod actor;
pub mod client;
pub mod handshake;
pub mod protocol;
pub mod tool;

pub use actor::{ActorError, HostHandle};
pub use client::{
    discover_extensions, discover_extensions_in_order, expand_extension_path, ActionCall, ExtHost,
    HostError, HostOptions, SessionCmd, UiAsk, UiFrame,
};
pub use handshake::{Handshake, HandshakeError, ToolSpec};
pub use protocol::{Frame, FrameError, PROTOCOL_VERSION};
pub use tool::ExtTool;
