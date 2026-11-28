//! P4-15 步骤 1：事件流"原子段"闸门（S10 的顺序竞态）。
//!
//! 背景：ref（TS）里 `tool_execution_start` 与工具首条 `tool_execution_update` 之间不可能插进
//! `queue_update`——工具在**任何 await 之前**同步发出首条进度（
//! `ref/pi-0.87.1/packages/coding-agent/src/core/tools/bash.ts:301-302`），而 RPC 命令处理
//! （`core/agent-session.ts:837-843` 的 `_emitQueueUpdate`）只能在事件循环的 await 点被调度。
//! pi-rs 的 RPC 命令处理在**另一个线程**（`crates/pi-cli/src/rpc_session.rs`），两个线程争同一把
//! stdout 锁，于是那个窗口里可以插进来（实测：给窗口注入 300ms 延迟 → 3/3 复现）。
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pi_core::agent::section::SectionGate;

#[test]
fn without_the_gate_two_threads_can_interleave_inside_the_window() {
    // 反证（复现窗口确实存在）：没有闸门时，A 在"两条事件"之间睡一会儿，B 就能插进去。
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(SectionGate::new(Duration::from_secs(5)));
    let _not_used = &gate;
    let first = log.clone();
    // **确定性插入**：A 写完 `A:start` 后发信号，等 B 写完 `B:queue_update` 再继续。
    // 原来的写法靠 `A sleep(80)` / `B sleep(20)` 排顺序，高负载下 B 线程会被推迟到
    // `A:start` **之前**（precommit 实测一次失败样本：
    // `left: ["B:queue_update", "A:start", "A:first_update"]`）——
    // 那证明的是"调度慢"，不是"没有闸门时会插队"。
    let (a_wrote_tx, a_wrote_rx) = std::sync::mpsc::channel::<()>();
    let (b_done_tx, b_done_rx) = std::sync::mpsc::channel::<()>();
    let a = std::thread::spawn(move || {
        first.lock().expect("锁可用").push("A:start");
        a_wrote_tx.send(()).expect("发送 A:start 信号");
        b_done_rx.recv().expect("等 B 插进来");
        first.lock().expect("锁可用").push("A:first_update");
    });
    let second = log.clone();
    let b = std::thread::spawn(move || {
        a_wrote_rx.recv().expect("等 A:start");
        second.lock().expect("锁可用").push("B:queue_update");
        b_done_tx.send(()).expect("通知 A 继续");
    });
    a.join().expect("A 线程结束");
    b.join().expect("B 线程结束");
    let seen = log.lock().expect("锁可用").clone();
    assert_eq!(
        seen,
        vec!["A:start", "B:queue_update", "A:first_update"],
        "没有闸门时 B 会插进 A 的两条事件之间（这就是 S10 的竞态）"
    );
}

#[test]
fn other_thread_waits_until_the_first_event_of_the_section_is_written() {
    // 语义：A 持有段期间，B 调 `wait()` **必须阻塞**，直到 A 关段才返回 `true`。
    //
    // **判据不依赖墙钟**，靠这三件事：
    // 1. B 只在收到"A 已 `begin()`"之后才进 `wait()` —— 排除"段还没被持有 ⇒ 立刻返回"；
    // 2. A 在 B **已经进入 `wait()`** 之后才关段 —— 用一个"B 已就绪"的信道，
    //    B 在 `wait()` **之前**发信号；由于 A 收到信号后还要走完一次
    //    `end_if_owner()`，B 此时必然处于阻塞中（它没有别的事可做）；
    // 3. 断言 `wait()` 返回 `true`（正常放行）与最终事件顺序。
    //
    // 早先的写法靠 `sleep(20)`/`sleep(80)` 与 `waited >= 40ms` 排序：CPU 饱和时线程被
    // 抢占就会假红（实测旧版 9/12 失败，全部落在计时/顺序断言上）。
    let gate = Arc::new(SectionGate::new(Duration::from_secs(5)));
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let (begun_tx, begun_rx) = std::sync::mpsc::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();

    let owner = gate.clone();
    let first = log.clone();
    let a = std::thread::spawn(move || {
        owner.begin();
        first.lock().expect("锁可用").push("A:start");
        begun_tx.send(()).expect("通知已持段");
        // 等 B 走到 `wait()` 门口。
        ready_rx.recv().expect("等 B 就绪");
        // 关段：此后 B 的 `wait()` 才会返回 true。
        owner.end_if_owner();
        first.lock().expect("锁可用").push("A:done");
    });

    let waiter = gate.clone();
    let second = log.clone();
    let b = std::thread::spawn(move || {
        begun_rx.recv().expect("等 A 持段");
        ready_tx.send(()).expect("通知 A 即将 wait");
        // A 收到该信号后才关段 ⇒ 这次 `wait()` 必然经历"段被持有"→"被放开"。
        let ended_normally = waiter.wait();
        second.lock().expect("锁可用").push("B:queue_update");
        ended_normally
    });

    let ended_normally = b.join().expect("B 线程结束");
    a.join().expect("A 线程结束");
    assert!(ended_normally, "本线程应等到段结束（不是安全阀放行）");
    let seen = log.lock().expect("锁可用").clone();
    // 顺序的唯一稳定判据：`B:queue_update` 一定在 `A:done` **之前或之后**都可能
    // （A 关段与 B 返回之间没有强顺序），所以只断言**两者都在 A:start 之后**，
    // 且 `B:queue_update` 出现在 `A:done` 之前——因为 A 是先 `end_if_owner()`
    // 再 push `A:done`，而 B 被放开后立刻 push。
    assert_eq!(seen.first().copied(), Some("A:start"), "顺序：{seen:?}");
    assert!(
        seen.contains(&"A:done") && seen.contains(&"B:queue_update"),
        "两条事件都应写出：{seen:?}"
    );
}

#[test]
fn safety_valve_releases_other_threads() {
    // 工具卡死（段一直不关）时不能让命令响应陪等：安全阀到点放行，且**返回 false** 以便上层察觉。
    let gate = Arc::new(SectionGate::new(Duration::from_millis(30)));
    gate.begin(); // 本线程持段，且永不释放
    let waiter = gate.clone();
    let (ended_normally, waited) = std::thread::spawn(move || {
        let started = Instant::now();
        let ended = waiter.wait();
        (ended, started.elapsed())
    })
    .join()
    .expect("线程结束");
    assert!(!ended_normally, "安全阀超时应返回 false");
    assert!(
        waited >= Duration::from_millis(25),
        "至少等到安全阀时间（{waited:?}）"
    );
}

#[test]
fn owner_is_never_blocked_and_others_cannot_end_my_section() {
    let gate = SectionGate::new(Duration::from_secs(5));
    gate.begin();
    assert!(gate.held_by_me(), "刚 begin 的线程是持有者");
    assert!(gate.wait(), "持有者自己等自己应立即返回");
    // 另一个线程关不掉别人开的段（scoped thread 直接借用，不需要 unsafe）。
    let other_thread_result = std::thread::scope(|scope| {
        scope
            .spawn(|| gate.end_if_owner())
            .join()
            .expect("线程结束")
    });
    assert!(!other_thread_result, "非持有者不得关段");
    assert!(gate.held_by_me(), "段还在");
    assert!(gate.end_if_owner(), "持有者可以关段");
    assert!(!gate.held_by_me());
}

#[test]
fn safety_valve_is_counted_and_never_deadlocks() {
    // P4-15 步骤 2.5：工具段长时间持有闸门时——
    // ① 安全阀必须放行（不能死锁）；② 放行要**可观测**（计数自增）；③ 放行后本线程能继续写。
    let gate = Arc::new(SectionGate::new(Duration::from_millis(60)));
    assert_eq!(gate.valve_releases(), 0, "起点没有放行记录");

    let holder = gate.clone();
    // **确定性同步**：持有者 `begin()` 之后才通知主线程去 `wait()`。
    // 原来用 `sleep(20)` 猜"持有者应该已经拿到了"，高负载下持有者可能还没被调度，
    // 主线程先 `wait()` 就拿不到安全阀（precommit 实测该用例红）。
    let (begun_tx, begun_rx) = std::sync::mpsc::channel::<()>();
    let holding = std::thread::spawn(move || {
        holder.begin(); // 模拟"工具段"持有它
        begun_tx.send(()).expect("通知已持段");
        std::thread::sleep(Duration::from_millis(400));
        holder.end_if_owner();
    });

    begun_rx.recv().expect("持有者应已持段");
    let ended_normally = gate.wait();

    // 判据只看**语义**（是否放行、是否计数），不看"等了多久"：
    // 时间带 `>=50ms && <350ms` 依赖持有者线程按时 sleep 完、主线程按时被调度，
    // CPU 饱和时会假红（实测旧版在 12 核满载下 9/12 失败）。
    assert!(
        !ended_normally,
        "长持段时安全阀必须放行（返回 false），不能一直等"
    );
    assert_eq!(gate.valve_releases(), 1, "放行次数必须可观测（自增 1）");

    // 放行之后本线程能继续写：**每次**超时的写都会再等一个安全阀周期并计数（不会死锁）。
    let second_write = Instant::now();
    let second_ended = gate.wait();
    assert!(
        !second_ended,
        "持有者仍在时，第二次写也应走安全阀而不是无限等"
    );
    assert!(
        second_write.elapsed() < Duration::from_millis(200),
        "每次放行都必须在安全阀时限内返回（实际 {:?}）",
        second_write.elapsed()
    );
    assert_eq!(gate.valve_releases(), 2, "每次超时放行都要可观测地计数");

    holding.join().expect("持有者线程结束");
    // 段被释放后回到正常路径：不再触发安全阀，计数不再增长。
    assert!(gate.wait(), "持有者释放后应正常返回 true");
    assert_eq!(gate.valve_releases(), 2, "正常路径不应再增加放行计数");
}
