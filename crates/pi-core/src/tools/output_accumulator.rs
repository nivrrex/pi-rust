//! bash 输出的有界累加器。
//!
//! 逐条对应 ref `core/tools/output-accumulator.ts:13-203`：原始块在超过阈值前
//! 暂存，超过后改为流式写入临时文件；内存里只保留解码后的滚动尾部。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use super::bash_text::truncate_tail;
use super::read_text::{TruncatedBy, Truncation, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};

/// 累加器最终快照。
#[derive(Debug, Clone, PartialEq)]
pub struct OutputSnapshot {
    /// 保留给模型看的尾部。
    pub content: String,
    /// 截断账目。
    pub truncation: Truncation,
    /// 完整输出路径；只有需要保留完整输出时才存在。
    pub full_output_path: Option<String>,
    /// 当前末行的解码字节数。
    pub last_line_bytes: usize,
}

/// 流式收集 stdout/stderr，限制内存并保留完整输出。
#[derive(Debug)]
pub struct OutputAccumulator {
    max_lines: usize,
    max_bytes: usize,
    max_rolling_bytes: usize,
    temp_file_prefix: String,
    pending_utf8: Vec<u8>,
    raw_chunks: Vec<Vec<u8>>,
    tail_text: String,
    tail_bytes: usize,
    tail_starts_at_line_boundary: bool,
    total_raw_bytes: usize,
    total_decoded_bytes: usize,
    completed_lines: usize,
    total_lines: usize,
    current_line_bytes: usize,
    has_open_line: bool,
    finished: bool,
    temp_file: Option<File>,
    temp_file_path: Option<PathBuf>,
}

impl OutputAccumulator {
    /// 创建累加器；上限由调用方传入，默认值与官方工具一致。
    #[must_use]
    pub fn new(max_lines: usize, max_bytes: usize, temp_file_prefix: &str) -> Self {
        Self {
            max_lines,
            max_bytes,
            max_rolling_bytes: (max_bytes * 2).max(1),
            temp_file_prefix: temp_file_prefix.to_string(),
            pending_utf8: Vec::new(),
            raw_chunks: Vec::new(),
            tail_text: String::new(),
            tail_bytes: 0,
            tail_starts_at_line_boundary: true,
            total_raw_bytes: 0,
            total_decoded_bytes: 0,
            completed_lines: 0,
            total_lines: 0,
            current_line_bytes: 0,
            has_open_line: false,
            finished: false,
            temp_file: None,
            temp_file_path: None,
        }
    }

    /// 使用 bash 工具的默认上限。
    #[must_use]
    pub fn with_defaults(temp_file_prefix: &str) -> Self {
        Self::new(DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES, temp_file_prefix)
    }

    /// 追加一个原始字节块；写入临时文件失败时返回错误。
    pub fn append(&mut self, data: &[u8]) -> io::Result<()> {
        if self.finished {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cannot append to a finished output accumulator",
            ));
        }

        self.total_raw_bytes += data.len();
        self.append_decoded_bytes(data);
        if self.temp_file.is_some() || self.should_use_temp_file() {
            self.ensure_temp_file()?;
            if let Some(file) = &mut self.temp_file {
                file.write_all(data)?;
            }
        } else if !data.is_empty() {
            self.raw_chunks.push(data.to_vec());
        }
        Ok(())
    }

    /// 结束输入并刷新尚未解码的 UTF-8 后缀。
    pub fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        if !self.pending_utf8.is_empty() {
            self.append_text("\u{fffd}");
            self.pending_utf8.clear();
        }
        if self.should_use_temp_file() {
            self.ensure_temp_file()?;
        }
        if let Some(file) = &mut self.temp_file {
            file.flush()?;
        }
        Ok(())
    }

    /// 生成快照；`persist_if_truncated` 为真时确保完整输出已落盘。
    pub fn snapshot(&mut self, persist_if_truncated: bool) -> io::Result<OutputSnapshot> {
        let mut tail = truncate_tail(&self.snapshot_text(), self.max_lines, self.max_bytes);
        let truncated =
            self.total_lines > self.max_lines || self.total_decoded_bytes > self.max_bytes;
        let truncated_by = if truncated {
            Some(
                tail.truncated_by
                    .unwrap_or(if self.total_decoded_bytes > self.max_bytes {
                        TruncatedBy::Bytes
                    } else {
                        TruncatedBy::Lines
                    }),
            )
        } else {
            None
        };
        if persist_if_truncated && truncated {
            self.ensure_temp_file()?;
        }
        tail.truncated = truncated;
        tail.truncated_by = truncated_by;
        let truncation = Truncation {
            content: std::mem::take(&mut tail.content),
            truncated: tail.truncated,
            truncated_by: tail.truncated_by,
            total_lines: self.total_lines,
            total_bytes: self.total_decoded_bytes,
            output_lines: tail.output_lines,
            output_bytes: tail.output_bytes,
            last_line_partial: tail.last_line_partial,
            first_line_exceeds_limit: false,
            max_lines: self.max_lines,
            max_bytes: self.max_bytes,
        };
        Ok(OutputSnapshot {
            content: truncation.content.clone(),
            truncation,
            full_output_path: self
                .temp_file_path
                .as_ref()
                .map(|path| path.display().to_string()),
            last_line_bytes: self.current_line_bytes,
        })
    }

    /// 当前末行的解码字节数，供部分行截断提示使用。
    #[must_use]
    pub fn last_line_bytes(&self) -> usize {
        self.current_line_bytes
    }

    fn should_use_temp_file(&self) -> bool {
        self.total_raw_bytes > self.max_bytes
            || self.total_decoded_bytes > self.max_bytes
            || self.total_lines > self.max_lines
    }

    fn append_decoded_bytes(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let combined;
        let mut input = if self.pending_utf8.is_empty() {
            data
        } else {
            combined = [self.pending_utf8.as_slice(), data].concat();
            combined.as_slice()
        };
        self.pending_utf8.clear();

        while !input.is_empty() {
            match std::str::from_utf8(input) {
                Ok(text) => {
                    self.append_text(text);
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid > 0 {
                        self.append_text(&String::from_utf8_lossy(&input[..valid]));
                        input = &input[valid..];
                    }
                    match error.error_len() {
                        Some(len) => {
                            self.append_text("\u{fffd}");
                            input = &input[len..];
                        }
                        None => {
                            self.pending_utf8.extend_from_slice(input);
                            break;
                        }
                    }
                }
            }
        }
    }

    fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let bytes = text.len();
        self.total_decoded_bytes += bytes;
        self.tail_text.push_str(text);
        self.tail_bytes += bytes;
        if self.tail_bytes > self.max_rolling_bytes * 2 {
            self.trim_tail();
        }

        let mut newlines = 0;
        let mut last_newline = None;
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                newlines += 1;
                last_newline = Some(index);
            }
        }
        if newlines == 0 {
            self.current_line_bytes += bytes;
            self.has_open_line = true;
        } else {
            self.completed_lines += newlines;
            let tail = &text[last_newline.unwrap_or_default() + 1..];
            self.current_line_bytes = tail.len();
            self.has_open_line = !tail.is_empty();
        }
        self.total_lines = self.completed_lines + usize::from(self.has_open_line);
    }

    fn trim_tail(&mut self) {
        let buffer = self.tail_text.as_bytes();
        if buffer.len() <= self.max_rolling_bytes {
            self.tail_bytes = buffer.len();
            return;
        }
        let mut start = buffer.len() - self.max_rolling_bytes;
        while start < buffer.len() && !self.tail_text.is_char_boundary(start) {
            start += 1;
        }
        self.tail_starts_at_line_boundary = if start == 0 {
            self.tail_starts_at_line_boundary
        } else {
            buffer[start - 1] == b'\n'
        };
        self.tail_text = String::from_utf8_lossy(&buffer[start..]).into_owned();
        self.tail_bytes = self.tail_text.len();
    }

    fn snapshot_text(&self) -> String {
        if self.tail_starts_at_line_boundary {
            return self.tail_text.clone();
        }
        self.tail_text
            .split_once('\n')
            .map_or_else(|| self.tail_text.clone(), |(_, tail)| tail.to_string())
    }

    fn ensure_temp_file(&mut self) -> io::Result<()> {
        if self.temp_file.is_some() {
            return Ok(());
        }
        for _ in 0..8 {
            let path = std::env::temp_dir().join(format!(
                "{}-{}.log",
                self.temp_file_prefix,
                uuid::Uuid::new_v4().simple()
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            let mut file = match options.open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            for chunk in &self.raw_chunks {
                file.write_all(chunk)?;
            }
            self.raw_chunks.clear();
            self.temp_file = Some(file);
            self.temp_file_path = Some(path);
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique full-output file",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_output_is_kept_in_memory_without_a_temp_file() {
        let mut output = OutputAccumulator::new(10, 1024, "pi-test");
        output.append(b"hello\n").expect("append");
        output.finish().expect("finish");
        let snapshot = output.snapshot(true).expect("snapshot");
        assert_eq!(snapshot.content, "hello\n");
        assert!(!snapshot.truncation.truncated);
        assert!(snapshot.full_output_path.is_none());
        assert_eq!(snapshot.truncation.total_bytes, 6);
    }

    #[test]
    fn large_output_streams_to_a_unique_file_and_keeps_only_a_tail() {
        let mut first = OutputAccumulator::new(10, 32, "pi-test");
        let mut second = OutputAccumulator::new(10, 32, "pi-test");
        let data = vec![b'x'; 1024];
        first.append(&data).expect("append first");
        second.append(&data).expect("append second");
        first.finish().expect("finish first");
        second.finish().expect("finish second");
        let first_snapshot = first.snapshot(true).expect("snapshot first");
        let second_snapshot = second.snapshot(true).expect("snapshot second");
        let first_path = first_snapshot.full_output_path.expect("first path");
        let second_path = second_snapshot.full_output_path.expect("second path");
        assert_ne!(first_path, second_path);
        assert_eq!(std::fs::read(&first_path).expect("read first"), data);
        assert_eq!(std::fs::read(&second_path).expect("read second"), data);
        assert_eq!(first_snapshot.content.len(), 32);
        assert!(first_snapshot.truncation.truncated_by == Some(TruncatedBy::Bytes));
        assert!(first.raw_chunks.is_empty());
        assert!(first.tail_bytes <= first.max_rolling_bytes * 2);
        let _ = std::fs::remove_file(first_path);
        let _ = std::fs::remove_file(second_path);
    }

    #[test]
    fn utf8_sequences_split_across_chunks_are_not_replaced() {
        let mut output = OutputAccumulator::new(10, 1024, "pi-test");
        let bytes = "中\n".as_bytes();
        output.append(&bytes[..1]).expect("first byte");
        output.append(&bytes[1..]).expect("remaining bytes");
        output.finish().expect("finish");
        assert_eq!(output.snapshot(false).expect("snapshot").content, "中\n");
    }

    #[test]
    fn invalid_utf8_uses_the_same_replacement_behavior_as_text_decoder() {
        let mut output = OutputAccumulator::new(10, 1024, "pi-test");
        output.append(b"a\xffb").expect("append");
        output.finish().expect("finish");
        assert_eq!(
            output.snapshot(false).expect("snapshot").content,
            "a\u{fffd}b"
        );
    }

    #[test]
    fn line_limit_preserves_the_last_lines() {
        let mut output = OutputAccumulator::new(2, 1024, "pi-test");
        output.append(b"one\ntwo\nthree\n").expect("append");
        output.finish().expect("finish");
        let snapshot = output.snapshot(true).expect("snapshot");
        assert_eq!(snapshot.content, "two\nthree");
        assert_eq!(snapshot.truncation.total_lines, 3);
        assert_eq!(snapshot.truncation.output_lines, 2);
        assert!(snapshot.truncation.truncated_by == Some(TruncatedBy::Lines));
        if let Some(path) = snapshot.full_output_path {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn file_paths_are_regular_files_not_symlinks() {
        let mut output = OutputAccumulator::new(1, 1, "pi-test");
        output.append(b"xx").expect("append");
        output.finish().expect("finish");
        let snapshot = output.snapshot(true).expect("snapshot");
        let path = snapshot.full_output_path.expect("path");
        let metadata = std::fs::symlink_metadata(std::path::Path::new(&path)).expect("metadata");
        assert!(metadata.file_type().is_file());
        let _ = std::fs::remove_file(path);
    }
}
