//! 四个工具（read / write / edit / bash）。
//!
//! 已落地：`Tool` 抽象与参数校验、read/edit/write/bash 等工具、同路径文件变更队列，
//! 以及 bash 的有界输出累加器。图片处理与 fuzzy NFKC 等剩余差异见各模块说明。

pub mod bash;
pub mod bash_text;
pub mod definitions;
pub mod diff;
pub mod edit;
pub mod edit_text;
pub mod file_mutation_queue;
pub mod find;
pub mod grep;
pub mod ls;
pub mod output_accumulator;
pub mod powershell;
pub mod read;
pub mod read_text;
pub mod schema;
pub mod schema_order;
pub mod shell_resolve;
pub mod tool;
pub mod write;
