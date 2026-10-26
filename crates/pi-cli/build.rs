//! 把**构建时**的提交号与时间写进二进制（P8d/B2）。
//!
//! 目的：`/tmp/pi-smoke/run-real.sh` 启动前能打印"这个二进制是什么时候、从哪个提交构建的"，
//! 避免用户拿旧二进制验证新修复（真实发生过：工具正文修复后仍在旧二进制上复现）。
//! 只用 `std::process::Command`，不新增依赖。

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| !output.stdout.is_empty())
        .unwrap_or(false);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    println!(
        "cargo:rustc-env=PI_BUILD_COMMIT={commit}{}",
        if dirty { "-dirty" } else { "" }
    );
    println!("cargo:rustc-env=PI_BUILD_EPOCH={stamp}");
}
