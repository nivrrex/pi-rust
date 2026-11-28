//! 事件流的「原子段」闸门（P4-15 步骤 1）。
//!
//! ## 为什么需要它
//!
//! 场景 S10 在工具执行期间插 steering：runner 一看到 `tool_execution_start` 就发 `steer` 命令。
//! ref（TS）里这个顺序是**确定**的（golden/S10：`tool_execution_start` → `tool_execution_update`
//! → `queue_update`），原因是单线程事件循环：
//! - 工具的入口进度上报是**同步**发出的（`ref/pi-0.87.1/packages/coding-agent/src/core/tools/bash.ts:301-302`
//!   在 `execute` 一开始就 `onUpdate({ content: [], details: undefined })`），
//!   此时还没有任何 `await`；
//! - RPC 侧的命令处理（`core/agent-session.ts:837-843` 的 `_emitQueueUpdate`）只能在事件循环的
//!   **await 点**被调度 → 不可能挤在这两条事件之间。
//!
//! pi-rs 里 RPC 命令处理跑在**另一个线程**（`crates/pi-cli/src/rpc_session.rs:503` 把 `run_prompt`
//! 丢到新线程，主线程继续读 stdin），事件行由两边的线程争同一把 stdout 锁（`RpcShared::write_line`），
//! 于是"start 已刷出、工具首次上报还没刷出"这个窗口里 `queue_update` 可以插进来——
//! 实测：给该窗口注入 300ms 延迟后 3/3 复现（顺序变成 `start, queue_update, update`），
//! 而不加延迟时 26 次里出现 1 次。
//!
//! ## 语义
//!
//! `begin()` → 本线程"持有"闸门；其他线程调 [`SectionGate::wait`] 会等到闸门释放
//! （或超过**安全阀** `timeout`，避免工具卡死时把命令响应一起拖死）。
//! 持有者自己的 `wait` 立即返回，所以 `emit` 可以在段内继续写。
//!
//! 使用方式（见 `run.rs`）：工具调用前 `sink.emit_opening_section(ToolExecutionStart…)` 打开闸门，
//! 本线程的**下一次** `emit` 之后自动关闭；工具返回时再 `sink.end_section()` 兜底。
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// 事件流的原子段闸门（线程安全）。
#[derive(Debug)]
pub struct SectionGate {
    owner: Mutex<Option<ThreadId>>,
    timeout: Duration,
    /// **安全阀放行的累计次数**（P4-15 步骤 2.5）：超时放行时自增，测试与产品层都能读到它，
    /// 不再"静默降级"。`pi-cli` 的 `write_line` 每次放行会往 stderr 记一行。
    valve_releases: AtomicUsize,
}

impl SectionGate {
    /// `timeout` 是安全阀：其他线程最多等这么久，之后放行（不返回错误，只降级为"可能交错"）。
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            owner: Mutex::new(None),
            timeout,
            valve_releases: AtomicUsize::new(0),
        }
    }

    /// 中毒不放大：这把锁只保护一个 `Option<ThreadId>`，中毒时继续用里面的一致性数据。
    fn lock(&self) -> MutexGuard<'_, Option<ThreadId>> {
        self.owner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// 本线程进入原子段（覆盖之前的持有者：同一时刻只可能有一个 agent 线程）。
    pub fn begin(&self) {
        *self.lock() = Some(std::thread::current().id());
    }

    /// 释放原子段（任何线程都可以释放；通常由持有者在写完配对事件后调用）。
    pub fn end(&self) {
        *self.lock() = None;
    }

    /// 仅当**本线程**是持有者时释放；返回是否真的释放了。
    pub fn end_if_owner(&self) -> bool {
        let me = std::thread::current().id();
        let mut owner = self.lock();
        if owner.is_some_and(|id| id == me) {
            *owner = None;
            true
        } else {
            false
        }
    }

    /// 安全阀累计放行次数（可观测；见 `wait`）。
    #[must_use]
    pub fn valve_releases(&self) -> usize {
        self.valve_releases.load(Ordering::SeqCst)
    }

    /// 本线程是否是当前持有者。
    #[must_use]
    pub fn held_by_me(&self) -> bool {
        self.lock()
            .is_some_and(|id| id == std::thread::current().id())
    }

    /// 非持有者阻塞等待段结束；返回 `true` 表示"段已结束（正常）"，
    /// `false` 表示**安全阀超时放行**（此时调用方仍可写，只是不再保证顺序）。
    pub fn wait(&self) -> bool {
        let me = std::thread::current().id();
        let deadline = Instant::now() + self.timeout;
        loop {
            match *self.lock() {
                None => return true,
                Some(id) if id == me => return true,
                Some(_other_thread) => {}
            }
            if Instant::now() >= deadline {
                // 安全阀放行：**计数可观测**，调用方（`pi-cli` 的 write_line）会往 stderr 记一行。
                self.valve_releases.fetch_add(1, Ordering::SeqCst);
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
