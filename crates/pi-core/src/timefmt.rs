//! Unix 毫秒 → ISO-8601 字符串（`Date.prototype.toISOString()` 的形状）。
//!
//! **为什么手写**：会话 entry 的 `timestamp` 与文件名都要它，而 `chrono`/`time` 属新增依赖
//! （按 AGENTS 须先确认）。用 Howard Hinnant 的 `civil_from_days` 算法，纯整数运算。
//!
//! 输出形状与 JS 的 `toISOString()` 一致：`YYYY-MM-DDTHH:MM:SS.mmmZ`
//! ——**固定 3 位毫秒、固定 `Z` 后缀**，年份补零到 4 位（Python 的 `isoformat()` 是 6 位微秒，
//! 不能拿来对齐）。

/// 把 Unix 毫秒转成 ISO-8601。负值（1970 之前）按 0 处理——会话时间戳不会是负数。
pub fn unix_millis_to_iso8601(millis: i64) -> String {
    let millis = millis.max(0);
    let seconds = millis / 1000;
    let ms = millis % 1000;
    let days = seconds / 86_400;
    let secs_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{ms:03}Z")
}

/// `days` 是自 1970-01-01 起的天数，返回 `(year, month, day)`。
///
/// Hinnant 的 `civil_from_days`：以 0000-03-01 为纪元把闰年规则摊平成线性运算。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 当前时间的 Unix 毫秒；时钟早于 1970 时返回 0。
pub fn now_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_vectors() {
        assert_eq!(unix_millis_to_iso8601(0), "1970-01-01T00:00:00.000Z");
        // 与 JS `new Date(1700000000000).toISOString()` 一致。
        assert_eq!(
            unix_millis_to_iso8601(1_700_000_000_000),
            "2023-11-14T22:13:20.000Z"
        );
        assert_eq!(
            unix_millis_to_iso8601(1_700_000_000_123),
            "2023-11-14T22:13:20.123Z"
        );
        // 闰日。
        assert_eq!(
            unix_millis_to_iso8601(951_782_400_000),
            "2000-02-29T00:00:00.000Z"
        );
        // 非闰年的 3 月 1 日。
        assert_eq!(
            unix_millis_to_iso8601(951_868_800_000),
            "2000-03-01T00:00:00.000Z"
        );
        // 年末最后一毫秒。
        assert_eq!(
            unix_millis_to_iso8601(1_735_689_599_999),
            "2024-12-31T23:59:59.999Z"
        );
    }

    #[test]
    fn milliseconds_are_zero_padded_to_three_digits() {
        assert_eq!(
            unix_millis_to_iso8601(1_700_000_000_007),
            "2023-11-14T22:13:20.007Z"
        );
        assert_eq!(
            unix_millis_to_iso8601(1_700_000_000_070),
            "2023-11-14T22:13:20.070Z"
        );
    }

    #[test]
    fn negative_input_is_clamped() {
        assert_eq!(unix_millis_to_iso8601(-1), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn now_is_after_2020() {
        assert!(
            now_unix_millis() > 1_577_836_800_000,
            "当前时间应晚于 2020-01-01"
        );
    }
}

/// 把 [`unix_millis_to_iso8601`] 产出的固定格式（`YYYY-MM-DDTHH:MM:SS.mmmZ`）解析回毫秒。
///
/// 手写而不引依赖：格式由本模块保证固定，且**纯逻辑**可单测。无法识别时返回 `None`
/// （调用方按"没有该时间戳"处理，不要猜）。阈值判定里用它比较"最后一条 compaction 的时间"
/// 与"最后一条带 usage 的助手消息的时间"，以识别**陈旧 usage**（ref `agent-session.ts:2699-2734`）。
#[must_use]
pub fn iso8601_to_unix_millis(text: &str) -> Option<i64> {
    // 期望：YYYY-MM-DDTHH:MM:SS.mmmZ（长度 24）
    let bytes = text.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
    {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> { text.get(from..to)?.parse::<i64>().ok() };
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let minute = num(14, 16)?;
    let second = num(17, 19)?;
    let millis = num(20, 23)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    // days_from_civil（Howard Hinnant 的算法）
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis)
}

#[cfg(test)]
mod iso_tests {
    use super::{iso8601_to_unix_millis, unix_millis_to_iso8601};

    #[test]
    fn iso_roundtrips_and_rejects_garbage() {
        // 往返：几组已知值
        // 只覆盖**非负**毫秒：`unix_millis_to_iso8601` 对负数按截断处理（-1 与 0 同形），
        // 这是它既有的定义域限制（会话时间戳恒为正），本解析器不为此扩大范围。
        // 反向的负值解析由下面的 `-1` 用例单独钉住。
        for millis in [
            0_i64,
            1_700_000_000_000,
            1_000,
            1_700_000_000_123,
            2_000_000_000_000,
        ] {
            let iso = unix_millis_to_iso8601(millis);
            assert_eq!(
                iso8601_to_unix_millis(&iso),
                Some(millis),
                "往返失败：{iso}"
            );
        }
        assert_eq!(iso8601_to_unix_millis("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            iso8601_to_unix_millis("2026-10-06T12:34:56.789Z"),
            Some(1_791_290_096_789)
        );
        // 1969 年的解析是对的（本解析器按 days_from_civil 算，能过 1970 往前）
        assert_eq!(iso8601_to_unix_millis("1969-12-31T23:59:59.999Z"), Some(-1));
        // 反向的既有局限：`-1` 经 `unix_millis_to_iso8601` 与 `0` 得到同一文本
        assert_eq!(unix_millis_to_iso8601(-1), unix_millis_to_iso8601(0));
        // 负向：golden 里的占位符、错格式一律 None（不得当成 0）
        for bad in [
            "<TIMESTAMP>",
            "",
            "2026-10-06",
            "2026-10-06T12:34:56Z",
            "2026-13-06T12:34:56.789Z",
        ] {
            assert_eq!(iso8601_to_unix_millis(bad), None, "不该解析：{bad}");
        }
    }
}
