//! P4-15 步骤 5.1：`Stream::final_message` 的**锁中毒必须可区分**。
//!
//! 现状（改前）：`provider.rs` 的 `final_message()` 用 `.lock().ok()` 把中毒变成 `None`，
//! 调用方（`crates/pi-core/src/agent/run.rs` 两处）**无法区分**"还没产生终态"与"槽位损坏"。
//! 本测试制造真实中毒（持锁时 panic）并断言能拿到**错误**而不是 `Ok(None)`。
use std::sync::{Arc, Mutex};

#[test]
fn poisoned_slot_is_reported_instead_of_silently_none() {
    let slot: Arc<Mutex<Option<pi_ai::AssistantMessage>>> = Arc::new(Mutex::new(None));
    let poisoner = slot.clone();
    let joined = std::thread::spawn(move || {
        let _guard = poisoner.lock().expect("首次加锁应成功");
        panic!("故意在持锁时 panic，制造中毒");
    })
    .join();
    assert!(joined.is_err(), "样本自检：制造中毒的线程确实 panic 了");
    assert!(slot.lock().is_err(), "样本自检：槽位现在处于中毒状态");

    let stream =
        pi_ai::provider::Stream::with_final_message(Box::pin(futures::stream::empty()), slot);
    let result = stream.final_message();
    assert!(
        result.is_err(),
        "中毒必须报错，不能返回 Ok(None)（那正是原来的静默行为）"
    );
    let message = result
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(
        message.contains("中毒"),
        "错误文案要说清是锁中毒：{message}"
    );
}

#[test]
fn healthy_but_empty_slot_still_reads_as_none() {
    // 正向对照：没中毒、但终态还没写进来 → 仍然是 `Ok(None)`（不能把"还没产生"当错误）。
    let slot = Arc::new(Mutex::new(None));
    let stream =
        pi_ai::provider::Stream::with_final_message(Box::pin(futures::stream::empty()), slot);
    let result = stream.final_message();
    assert!(result.is_ok(), "{result:?}");
    assert!(result.ok().flatten().is_none(), "还没产生终态");
}
