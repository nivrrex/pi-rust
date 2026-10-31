//! 四个工具（read / write / edit / bash）。
//!
//! 已落地：`Tool` 抽象与参数校验（`tool.rs`/`schema.rs`，P3.1）、read 与 edit 的**纯逻辑**部分；
//! 文件 I/O、图片处理、模糊匹配与其余两个工具待做。

pub mod bash;
pub mod bash_text;
pub mod definitions;
pub mod diff;
pub mod edit;
pub mod edit_text;
pub mod find;
pub mod grep;
pub mod ls;
pub mod powershell;
pub mod read;
pub mod read_text;
pub mod schema;
pub mod schema_order;
pub mod tool;
pub mod write;
