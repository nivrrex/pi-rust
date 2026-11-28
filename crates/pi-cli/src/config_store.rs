//! `auth.json` / `settings.json` 的跨进程锁与原子写。
//!
//! 锁协议与官方 `proper-lockfile` 一致：在目标路径后创建 `.lock` 目录，目录 mtime
//! 表示存活时间；过期锁可被回收。这样 pi-rs 与 TS 版共享同一配置目录时能互相看到
//! 对方的锁，而不是各自锁在不同机制上。

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use thiserror::Error;

/// 配置锁或原子写入失败。
#[derive(Debug, Error)]
pub(crate) enum ConfigStoreError {
    /// 文件系统操作失败。
    #[error("{path}: {source}")]
    Io {
        /// 相关路径。
        path: PathBuf,
        /// 底层错误。
        #[source]
        source: io::Error,
    },
    /// 等待锁超时。
    #[error("timed out waiting for config lock {path}")]
    LockTimeout {
        /// 锁目录。
        path: PathBuf,
    },
    /// 心跳线程失效，锁可能已被其它进程接管。
    #[error("config lock heartbeat failed for {path}: {message}")]
    Heartbeat {
        /// 锁目录。
        path: PathBuf,
        /// 失败原因。
        message: String,
    },
}

impl ConfigStoreError {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// 一个已持有的配置锁；Drop 会释放，显式 [`release`](Self::release) 可报告清理失败。
pub(crate) struct ConfigLockGuard {
    lock_path: PathBuf,
    stop: Arc<AtomicBool>,
    heartbeat_error: Arc<Mutex<Option<String>>>,
    lock_mtime: Arc<Mutex<std::time::SystemTime>>,
    heartbeat: Option<JoinHandle<()>>,
    released: bool,
}

impl ConfigLockGuard {
    /// 获取锁；`stale` 与官方一致，`timeout` 是本进程等待上限。
    pub(crate) fn acquire(
        path: &Path,
        stale: Duration,
        timeout: Duration,
    ) -> Result<Self, ConfigStoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| ConfigStoreError::io(parent, error))?;
        }
        let mut lock_name = path.as_os_str().to_os_string();
        lock_name.push(".lock");
        let lock_path = PathBuf::from(lock_name);
        let deadline = Instant::now() + timeout;
        loop {
            match std::fs::create_dir(&lock_path) {
                Ok(()) => return Self::start_heartbeat(lock_path, stale),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let expired = std::fs::metadata(&lock_path)
                        .and_then(|meta| meta.modified())
                        .map(|modified| modified.elapsed().unwrap_or_default() >= stale)
                        .unwrap_or(false);
                    if expired {
                        // 心跳标记只在极短窗口存在；进程被杀时可能留下，回收陈旧锁前先清掉。
                        let _ = std::fs::remove_file(lock_path.join("heartbeat"));
                        match std::fs::remove_dir(&lock_path) {
                            Ok(()) => continue,
                            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                            Err(error) => return Err(ConfigStoreError::io(&lock_path, error)),
                        }
                    }
                }
                Err(error) => return Err(ConfigStoreError::io(&lock_path, error)),
            }
            if Instant::now() >= deadline {
                return Err(ConfigStoreError::LockTimeout { path: lock_path });
            }
            std::thread::sleep(
                Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn start_heartbeat(lock_path: PathBuf, stale: Duration) -> Result<Self, ConfigStoreError> {
        let stop = Arc::new(AtomicBool::new(false));
        let heartbeat_error = Arc::new(Mutex::new(None));
        let lock_mtime = Arc::new(Mutex::new(
            std::fs::metadata(&lock_path)
                .and_then(|meta| meta.modified())
                .map_err(|error| {
                    let _ = std::fs::remove_dir(&lock_path);
                    ConfigStoreError::io(&lock_path, error)
                })?,
        ));
        let worker_stop = Arc::clone(&stop);
        let worker_path = lock_path.clone();
        let worker_error = Arc::clone(&heartbeat_error);
        let worker_mtime = Arc::clone(&lock_mtime);
        let interval = (stale / 3).max(Duration::from_millis(50));
        let heartbeat = std::thread::Builder::new()
            .name("config-lock-heartbeat".to_string())
            .spawn(move || {
                let mut next = Instant::now() + interval;
                loop {
                    let now = Instant::now();
                    if now < next {
                        std::thread::park_timeout((next - now).min(Duration::from_millis(50)));
                        if worker_stop.load(Ordering::SeqCst) {
                            return;
                        }
                        continue;
                    }
                    if worker_stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let update = (|| -> io::Result<()> {
                        let mut expected = worker_mtime
                            .lock()
                            .map_err(|_| io::Error::other("lock mtime state poisoned"))?;
                        let observed = std::fs::metadata(&worker_path)?.modified()?;
                        if observed != *expected {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "config lock was replaced by another process",
                            ));
                        }
                        let marker = worker_path.join("heartbeat");
                        let result = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&marker)
                            .and_then(|_| std::fs::remove_file(&marker));
                        if let Err(error) = result {
                            if error.kind() != io::ErrorKind::NotFound {
                                return Err(error);
                            }
                        }
                        *expected = std::fs::metadata(&worker_path)?.modified()?;
                        Ok(())
                    })();
                    if let Err(error) = update {
                        if let Ok(mut slot) = worker_error.lock() {
                            *slot = Some(error.to_string());
                        }
                        return;
                    }
                    next = Instant::now() + interval;
                }
            })
            .map_err(|error| {
                let _ = std::fs::remove_dir(&lock_path);
                ConfigStoreError::io(&lock_path, error)
            })?;
        Ok(Self {
            lock_path,
            stop,
            heartbeat_error,
            lock_mtime,
            heartbeat: Some(heartbeat),
            released: false,
        })
    }

    /// 检查心跳是否仍有效；失效后不能继续信任已写入的配置。
    pub(crate) fn check(&self) -> Result<(), ConfigStoreError> {
        let expected = self
            .lock_mtime
            .lock()
            .map_err(|_| ConfigStoreError::Heartbeat {
                path: self.lock_path.clone(),
                message: "lock mtime state poisoned".to_string(),
            })?;
        let observed = std::fs::metadata(&self.lock_path)
            .and_then(|meta| meta.modified())
            .map_err(|error| ConfigStoreError::Heartbeat {
                path: self.lock_path.clone(),
                message: error.to_string(),
            })?;
        if observed != *expected {
            return Err(ConfigStoreError::Heartbeat {
                path: self.lock_path.clone(),
                message: "config lock was replaced by another process".to_string(),
            });
        }
        let message = self
            .heartbeat_error
            .lock()
            .map_err(|_| ConfigStoreError::Heartbeat {
                path: self.lock_path.clone(),
                message: "heartbeat state poisoned".to_string(),
            })?;
        match message.as_deref() {
            Some(message) => Err(ConfigStoreError::Heartbeat {
                path: self.lock_path.clone(),
                message: message.to_string(),
            }),
            None => Ok(()),
        }
    }

    /// 释放锁并报告清理失败。
    pub(crate) fn release(mut self) -> Result<(), ConfigStoreError> {
        self.check()?;
        self.cleanup()?;
        self.released = true;
        Ok(())
    }

    fn cleanup(&mut self) -> Result<(), ConfigStoreError> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.heartbeat.take() {
            handle.join().map_err(|_| ConfigStoreError::Heartbeat {
                path: self.lock_path.clone(),
                message: "heartbeat thread panicked".to_string(),
            })?;
        }
        let marker = self.lock_path.join("heartbeat");
        match std::fs::remove_file(&marker) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(ConfigStoreError::io(&marker, error)),
        }
        match std::fs::remove_dir(&self.lock_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ConfigStoreError::io(&self.lock_path, error)),
        }
    }
}

impl Drop for ConfigLockGuard {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.cleanup();
        }
    }
}

/// 同目录临时文件 + `create_new` + rename；Unix 可指定 0600。
pub(crate) fn write_atomic(path: &Path, text: &str, private: bool) -> Result<(), ConfigStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| ConfigStoreError::io(parent, error))?;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let tmp = path.with_file_name(format!(".{name}.tmp-{}-{stamp}-{seq}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|error| ConfigStoreError::io(&tmp, error))?;
    let result = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| ConfigStoreError::io(&tmp, error));
    drop(file);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(ConfigStoreError::io(path, error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_excludes_another_holder_and_releases_cleanly() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("settings.json");
        let guard =
            ConfigLockGuard::acquire(&path, Duration::from_secs(10), Duration::from_millis(50))
                .expect("first lock");
        assert!(ConfigLockGuard::acquire(
            &path,
            Duration::from_secs(10),
            Duration::from_millis(50)
        )
        .is_err());
        guard.release().expect("release");
        ConfigLockGuard::acquire(&path, Duration::from_secs(10), Duration::from_millis(50))
            .expect("lock after release")
            .release()
            .expect("second release");
    }

    #[test]
    fn another_process_using_the_same_lock_directory_is_respected() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("settings.json");
        let ready = dir.path().join("ready");
        let release = dir.path().join("release");
        let mut child = std::process::Command::new("python3")
            .arg("-c")
            .arg(
                "import os, pathlib, sys, time\nlock = pathlib.Path(sys.argv[1])\nlock.mkdir()\npathlib.Path(sys.argv[2]).touch()\nwhile not pathlib.Path(sys.argv[3]).exists():\n    time.sleep(0.01)\nlock.rmdir()",
            )
            .arg(format!("{}.lock", path.display()))
            .arg(&ready)
            .arg(&release)
            .spawn()
            .expect("spawn python");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready.exists(), "外部进程未创建锁目录");
        assert!(ConfigLockGuard::acquire(
            &path,
            Duration::from_secs(10),
            Duration::from_millis(50)
        )
        .is_err());
        std::fs::write(&release, b"").expect("release child");
        assert!(child.wait().expect("wait child").success());
        ConfigLockGuard::acquire(&path, Duration::from_secs(10), Duration::from_millis(50))
            .expect("lock after external release")
            .release()
            .expect("release");
    }

    #[cfg(unix)]
    #[test]
    fn stale_lock_with_a_leftover_heartbeat_is_recovered() {
        use std::fs::FileTimes;
        use std::time::SystemTime;

        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("settings.json");
        let lock_path = PathBuf::from(format!("{}.lock", path.display()));
        std::fs::create_dir(&lock_path).expect("lock dir");
        std::fs::write(lock_path.join("heartbeat"), b"").expect("marker");
        let file = std::fs::File::open(&lock_path).expect("open dir");
        let old = SystemTime::now() - Duration::from_secs(30);
        file.set_times(FileTimes::new().set_modified(old))
            .expect("set old mtime");
        drop(file);
        ConfigLockGuard::acquire(&path, Duration::from_secs(10), Duration::from_millis(50))
            .expect("stale lock recovered")
            .release()
            .expect("release");
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_lock_directory_is_reported_as_compromised() {
        use std::fs::FileTimes;
        use std::time::SystemTime;

        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("settings.json");
        let guard =
            ConfigLockGuard::acquire(&path, Duration::from_secs(10), Duration::from_millis(50))
                .expect("lock");
        let lock_path = PathBuf::from(format!("{}.lock", path.display()));
        let file = std::fs::File::open(&lock_path).expect("open dir");
        file.set_times(FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .expect("change mtime");
        drop(file);
        assert!(matches!(
            guard.check(),
            Err(ConfigStoreError::Heartbeat { .. })
        ));
    }

    #[test]
    fn atomic_write_replaces_the_target() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "old").expect("seed");
        write_atomic(&path, "new", false).expect("write");
        assert_eq!(std::fs::read_to_string(path).expect("read"), "new");
    }
}
