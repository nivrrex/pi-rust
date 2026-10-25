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
    let a = std::thread::spawn(move || {
        first.lock().expect("锁可用").push("A:start");
        std::thread::sleep(Duration::from_millis(80));
        first.lock().expect("锁可用").push("A:first_update");
    });
    let second = log.clone();
    let b = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        second.lock().expect("锁可用").push("B:queue_update");
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
    let gate = Arc::new(SectionGate::new(Duration::from_secs(5)));
    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let owner = gate.clone();
    let first = log.clone();
    let a = std::thread::spawn(move || {
        owner.begin();
        first.lock().expect("锁可用").push("A:start");
        std::thread::sleep(Duration::from_millis(80));
        first.lock().expect("锁可用").push("A:first_update");
        owner.end_if_owner(); // 等价于"本线程的下一条 emit 之后关段"
    });
    let waiter = gate.clone();
    let second = log.clone();
    let b = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        let ended_normally = waiter.wait();
        let waited = started.elapsed();
        second.lock().expect("锁可用").push("B:queue_update");
        (ended_normally, waited)
    });
    a.join().expect("A 线程结束");
    let (ended_normally, waited) = b.join().expect("B 线程结束");
    assert!(ended_normally, "本线程应等到段结束（不是安全阀放行）");
    assert!(
        waited >= Duration::from_millis(40),
        "B 必须真的被挡住（实际等了 {waited:?}）"
    );
    let seen = log.lock().expect("锁可用").clone();
    assert_eq!(
        seen,
        vec!["A:start", "A:first_update", "B:queue_update"],
        "有闸门时顺序必须与 golden 一致"
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
    let holding = std::thread::spawn(move || {
        holder.begin(); // 模拟"工具段"持有它
        std::thread::sleep(Duration::from_millis(400));
        holder.end_if_owner();
    });

    // 先让持有者拿到段。
    std::thread::sleep(Duration::from_millis(20));
    let started = Instant::now();
    let ended_normally = gate.wait();
    let waited = started.elapsed();

    assert!(
        !ended_normally,
        "长持段时安全阀必须放行（返回 false），不能一直等"
    );
    assert!(
        waited >= Duration::from_millis(50) && waited < Duration::from_millis(350),
        "放行应发生在安全阀时限附近（实际 {waited:?}）"
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
