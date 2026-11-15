//! 项目信任存储（ref `core/trust-manager.ts`）。
//!
//! 为什么需要：ref 把"`<cwd>/.pi/{settings.json,extensions,skills,prompts,themes,SYSTEM.md,APPEND_SYSTEM.md}`
//! 与 `<cwd>/.agents/skills` 是否生效"交由项目信任决定（`resource-loader.ts:1029/1043`）。
//! 没有这个存储时，pi-rs 只能**无条件跳过**项目级资源（`skills.rs` 与
//! `prompt_templates.rs` 的模块文档限制），且 `ctx.isProjectTrusted()` 恒为 `false`——
//! 已装扩展 `pi-ai-summary` 用它决定是否读项目 settings，恒 false 会让它的项目级配置静默失效。
//!
//! 存储格式（ref `trust-manager.ts:107-140`）：`<agentDir>/trust.json`，一个
//! `{ "<规范化绝对路径>": true | false }` 的对象；键**排序**后以 2 空格缩进写回，
//! 尾随换行。`decision === null` 表示删除该键。
//!
//! 查询语义（`:44-58` 的 `findNearestTrustEntry`）：从 `cwd` 逐级向上找，**第一个**
//! 命中的条目决定结果；都没有则 `None`（= 未决定）。
//!
//! 本模块只做文件 I/O 与纯转换，不打印；错误以 `TrustError` 返回，由调用方决定展示。

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// 信任存储的错误。
#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    #[error("读取信任存储失败（{path}）：{source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("信任存储 {path} 不是合法 JSON：{source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("信任存储 {path} 的内容非法：{detail}")]
    Shape { path: String, detail: String },
    #[error("写入信任存储失败（{path}）：{source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// 一条信任决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustDecision {
    Trusted,
    Untrusted,
}

/// 一次 `setMany` 里的单项更新（ref `ProjectTrustUpdate`）。
///
/// `decision == None` ⇒ 删除该路径的条目（ref `:246-252`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustUpdate {
    pub path: PathBuf,
    pub decision: Option<TrustDecision>,
}

/// 面板里的一项（ref `ProjectTrustOption`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustOption {
    pub label: String,
    pub trusted: bool,
    pub updates: Vec<TrustUpdate>,
}

/// 存储根：`<agent_dir>/trust.json`。
#[derive(Debug, Clone)]
pub struct TrustStore {
    agent_dir: PathBuf,
}

impl TrustStore {
    #[must_use]
    pub fn new(agent_dir: &Path) -> Self {
        Self {
            agent_dir: agent_dir.to_path_buf(),
        }
    }

    /// `trust.json` 的完整路径。
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.agent_dir.join("trust.json")
    }

    /// 查 `cwd` 的有效决定（最近的祖先条目；没有则 `None`）。
    ///
    /// # Errors
    /// `trust.json` 存在但读不出/不是合法形状时返回错误——**不**把损坏的文件当"无决定"，
    /// 否则用户会看到项目资源突然全部失效而没有任何提示。
    pub fn get(&self, cwd: &Path) -> Result<Option<TrustDecision>, TrustError> {
        let data = self.read()?;
        Ok(find_nearest(&data, cwd))
    }

    /// 写入多条更新（ref `setMany`）。返回写回后的文件内容（供测试与日志）。
    ///
    /// # Errors
    /// 读/写失败、或目录无法创建。
    pub fn set_many(&self, updates: &[TrustUpdate]) -> Result<(), TrustError> {
        let mut data = self.read()?;
        for update in updates {
            let key = normalize_path(&update.path);
            match update.decision {
                Some(decision) => {
                    data.insert(key, decision);
                }
                None => {
                    data.remove(&key);
                }
            }
        }
        self.write(&data)
    }

    fn read(&self) -> Result<BTreeMap<String, TrustDecision>, TrustError> {
        let path = self.path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(source) => {
                return Err(TrustError::Read {
                    path: display(&path),
                    source,
                });
            }
        };
        // 与 ref 一致：去 BOM（Windows 编辑器常见）。
        let text = text.strip_prefix('\u{feff}').unwrap_or(text.as_str());
        let parsed: serde_json::Value =
            serde_json::from_str(text).map_err(|source| TrustError::Parse {
                path: display(&path),
                source,
            })?;
        let Some(object) = parsed.as_object() else {
            return Err(TrustError::Shape {
                path: display(&path),
                detail: "顶层必须是对象".to_string(),
            });
        };
        let mut data = BTreeMap::new();
        for (key, value) in object {
            let decision = match value {
                serde_json::Value::Bool(true) => TrustDecision::Trusted,
                serde_json::Value::Bool(false) => TrustDecision::Untrusted,
                // ref 允许 `null`（表示"删除/未决定"），读入时视为无条目。
                serde_json::Value::Null => continue,
                _other => {
                    return Err(TrustError::Shape {
                        path: display(&path),
                        detail: format!("{key:?} 的值必须是 true/false/null"),
                    });
                }
            };
            data.insert(key.clone(), decision);
        }
        Ok(data)
    }

    fn write(&self, data: &BTreeMap<String, TrustDecision>) -> Result<(), TrustError> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| TrustError::Write {
                path: display(&path),
                source,
            })?;
        }
        // `BTreeMap` 迭代即按 key 排序，与 ref 的 `Object.keys(data).sort()` 一致。
        let mut object = serde_json::Map::new();
        for (key, decision) in data {
            object.insert(
                key.clone(),
                serde_json::Value::Bool(matches!(decision, TrustDecision::Trusted)),
            );
        }
        let mut text =
            serde_json::to_string_pretty(&serde_json::Value::Object(object)).map_err(|source| {
                TrustError::Parse {
                    path: display(&path),
                    source,
                }
            })?;
        text.push('\n');
        std::fs::write(&path, text).map_err(|source| TrustError::Write {
            path: display(&path),
            source,
        })
    }
}

/// 从 `cwd` 向上找第一个条目（ref `:44-58`）。
fn find_nearest(data: &BTreeMap<String, TrustDecision>, cwd: &Path) -> Option<TrustDecision> {
    let mut current = normalize_path(cwd);
    loop {
        if let Some(decision) = data.get(&current) {
            return Some(*decision);
        }
        let parent = current.rsplit_once('/').map(|(head, _)| {
            if head.is_empty() {
                "/".to_string()
            } else {
                head.to_string()
            }
        });
        match parent {
            Some(next) if next != current => current = next,
            _other => return None,
        }
    }
}

/// 规范化绝对路径（ref `normalizeCwd` = `canonicalizePath(resolvePath(cwd))`）。
///
/// 这里不 `canonicalize`（那要求路径存在，且会把符号链接解到别处，与 TS 的
/// `canonicalizePath` 语义不同——后者只做 `resolve` + 去尾斜杠）。改为纯词法归一：
/// 绝对化、去掉 `.`、处理 `..`、去尾 `/`。
#[must_use]
pub fn normalize_path(path: &Path) -> String {
    let absolute: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut parts: Vec<String> = Vec::new();
    for component in absolute.components() {
        match component {
            Component::RootDir => parts.clear(),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            Component::Prefix(_prefix) => {}
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

/// `cwd` 的父目录（ref `getProjectTrustParentPath`）；根目录返回 `None`。
#[must_use]
pub fn parent_path(cwd: &Path) -> Option<PathBuf> {
    let normalized = normalize_path(cwd);
    if normalized == "/" {
        return None;
    }
    let (head, _tail) = normalized.rsplit_once('/')?;
    Some(PathBuf::from(if head.is_empty() { "/" } else { head }))
}

/// 可选项列表（ref `getProjectTrustOptions`，`:67-95`）。
///
/// 顺序与 ref 一致：Trust → Trust parent（若有父目录）→ [session-only] →
/// Do not trust → [session-only]。`include_session_only` 对应 ref 的
/// `options.includeSessionOnly`（启动期询问用；`/trust` 命令不用）。
#[must_use]
pub fn project_trust_options(cwd: &Path, include_session_only: bool) -> Vec<TrustOption> {
    let trust_path = PathBuf::from(normalize_path(cwd));
    let mut options = vec![TrustOption {
        label: "Trust".to_string(),
        trusted: true,
        updates: vec![TrustUpdate {
            path: trust_path.clone(),
            decision: Some(TrustDecision::Trusted),
        }],
    }];
    if let Some(parent) = parent_path(cwd) {
        options.push(TrustOption {
            label: format!("Trust parent folder ({})", display(&parent)),
            trusted: true,
            updates: vec![
                TrustUpdate {
                    path: parent,
                    decision: Some(TrustDecision::Trusted),
                },
                // 父目录受信任、本目录条目清除（ref `:79-80`）。
                TrustUpdate {
                    path: trust_path.clone(),
                    decision: None,
                },
            ],
        });
    }
    if include_session_only {
        options.push(TrustOption {
            label: "Trust (this session only)".to_string(),
            trusted: true,
            updates: Vec::new(),
        });
    }
    options.push(TrustOption {
        label: "Do not trust".to_string(),
        trusted: false,
        updates: vec![TrustUpdate {
            path: trust_path,
            decision: Some(TrustDecision::Untrusted),
        }],
    });
    if include_session_only {
        options.push(TrustOption {
            label: "Do not trust (this session only)".to_string(),
            trusted: false,
            updates: Vec::new(),
        });
    }
    options
}

/// `cwd` 是否存在**必须经信任才能生效**的项目资源（ref `hasTrustRequiringProjectResources`）。
///
/// 判据（`:185-215`）：`<cwd>/.pi` 下任一 `settings.json`/`extensions`/`skills`/
/// `prompts`/`themes`/`SYSTEM.md`/`APPEND_SYSTEM.md` 存在；或从 `cwd` 到根的任一级
/// 有 `.agents/skills`（**排除** `$HOME/.agents/skills`，那是用户级、始终受信任）。
#[must_use]
pub fn has_trust_requiring_project_resources(cwd: &Path) -> bool {
    const GATED: [&str; 7] = [
        "settings.json",
        "extensions",
        "skills",
        "prompts",
        "themes",
        "SYSTEM.md",
        "APPEND_SYSTEM.md",
    ];
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let user_agents_skills = home
        .as_ref()
        .map(|home| normalize_path(&home.join(".agents/skills")));

    let cwd_normalized = PathBuf::from(normalize_path(cwd));
    let config_dir = cwd_normalized.join(".pi");
    if GATED.iter().any(|entry| config_dir.join(entry).exists()) {
        return true;
    }
    let mut current = cwd_normalized;
    loop {
        let agents_skills = normalize_path(&current.join(".agents/skills"));
        if user_agents_skills.as_deref() != Some(agents_skills.as_str())
            && Path::new(&agents_skills).exists()
        {
            return true;
        }
        match parent_path(&current) {
            Some(parent) if parent != current => current = parent,
            _other => return false,
        }
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> TrustStore {
        TrustStore::new(dir)
    }

    #[test]
    fn missing_file_means_no_decision() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let found = store(tmp.path())
            .get(Path::new("/tmp/some/project"))
            .expect("读取");
        assert_eq!(found, None);
    }

    #[test]
    fn decisions_are_queried_from_the_nearest_ancestor() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let store = store(tmp.path());
        store
            .set_many(&[TrustUpdate {
                path: PathBuf::from("/work/repo"),
                decision: Some(TrustDecision::Trusted),
            }])
            .expect("写入");
        // 子目录继承父目录的决定
        assert_eq!(
            store.get(Path::new("/work/repo/sub/dir")).expect("查"),
            Some(TrustDecision::Trusted)
        );
        // 更近的条目覆盖（ref `findNearestTrustEntry` 取第一个命中）
        store
            .set_many(&[TrustUpdate {
                path: PathBuf::from("/work/repo/sub"),
                decision: Some(TrustDecision::Untrusted),
            }])
            .expect("写入");
        assert_eq!(
            store.get(Path::new("/work/repo/sub/dir")).expect("查"),
            Some(TrustDecision::Untrusted)
        );
        // 兄弟目录不受影响
        assert_eq!(store.get(Path::new("/work/other")).expect("查"), None);
    }

    #[test]
    fn writing_sorts_keys_and_ends_with_newline() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let store = store(tmp.path());
        store
            .set_many(&[
                TrustUpdate {
                    path: PathBuf::from("/z/last"),
                    decision: Some(TrustDecision::Untrusted),
                },
                TrustUpdate {
                    path: PathBuf::from("/a/first"),
                    decision: Some(TrustDecision::Trusted),
                },
            ])
            .expect("写入");
        let text = std::fs::read_to_string(store.path()).expect("读");
        assert!(text.ends_with('\n'), "应以换行结尾：{text:?}");
        let first = text.find("/a/first").expect("有 /a/first");
        let last = text.find("/z/last").expect("有 /z/last");
        assert!(first < last, "键应按字典序：{text}");
    }

    #[test]
    fn null_decision_deletes_the_entry() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let store = store(tmp.path());
        store
            .set_many(&[TrustUpdate {
                path: PathBuf::from("/a/b"),
                decision: Some(TrustDecision::Trusted),
            }])
            .expect("写入");
        store
            .set_many(&[TrustUpdate {
                path: PathBuf::from("/a/b"),
                decision: None,
            }])
            .expect("删除");
        assert_eq!(store.get(Path::new("/a/b")).expect("查"), None);
        let text = std::fs::read_to_string(store.path()).expect("读");
        assert!(!text.contains("/a/b"), "条目应被删除：{text}");
    }

    #[test]
    fn invalid_shapes_are_rejected_not_silently_ignored() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let store = store(tmp.path());
        std::fs::write(store.path(), "{ not json").expect("写");
        assert!(matches!(
            store.get(Path::new("/a")),
            Err(TrustError::Parse { .. })
        ));
        std::fs::write(store.path(), "[]").expect("写");
        assert!(matches!(
            store.get(Path::new("/a")),
            Err(TrustError::Shape { .. })
        ));
        std::fs::write(store.path(), r#"{"/a": "yes"}"#).expect("写");
        assert!(matches!(
            store.get(Path::new("/a")),
            Err(TrustError::Shape { .. })
        ));
    }

    #[test]
    fn null_values_read_as_absent() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let store = store(tmp.path());
        std::fs::write(store.path(), r#"{"/a": null}"#).expect("写");
        assert_eq!(store.get(Path::new("/a")).expect("查"), None);
    }

    #[test]
    fn options_follow_the_ref_order() {
        let options = project_trust_options(Path::new("/work/repo"), false);
        let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Trust", "Trust parent folder (/work)", "Do not trust"]
        );
        // 父目录选项要清掉本目录条目（否则本目录的旧决定继续覆盖父目录）
        let parent = &options[1];
        assert_eq!(parent.updates.len(), 2);
        assert_eq!(parent.updates[1].decision, None);
        // 会话内选项不落盘
        let with_session = project_trust_options(Path::new("/work/repo"), true);
        assert!(
            with_session
                .iter()
                .any(|option| option.label == "Trust (this session only)"
                    && option.updates.is_empty())
        );
    }

    #[test]
    fn root_has_no_parent_option() {
        let options = project_trust_options(Path::new("/"), false);
        assert!(!options
            .iter()
            .any(|option| option.label.starts_with("Trust parent")));
    }

    #[test]
    fn normalization_is_lexical_and_absolute() {
        assert_eq!(normalize_path(Path::new("/a/b/../c/")), "/a/c");
        assert_eq!(normalize_path(Path::new("/a/./b")), "/a/b");
        assert_eq!(normalize_path(Path::new("/")), "/");
        assert_eq!(parent_path(Path::new("/a/b")), Some(PathBuf::from("/a")));
        assert_eq!(parent_path(Path::new("/")), None);
        assert_eq!(parent_path(Path::new("/a")), Some(PathBuf::from("/")));
    }

    #[test]
    fn trust_requiring_resources_are_detected() {
        let tmp = tempfile::tempdir().expect("临时目录");
        let cwd = tmp.path().join("proj");
        std::fs::create_dir_all(&cwd).expect("建目录");
        assert!(!has_trust_requiring_project_resources(&cwd));
        std::fs::create_dir_all(cwd.join(".pi/skills")).expect("建目录");
        assert!(has_trust_requiring_project_resources(&cwd));
        // 祖先的 .agents/skills 也算
        let nested = tmp.path().join("outer/inner");
        std::fs::create_dir_all(nested.join(".agents/skills")).expect("建目录");
        assert!(has_trust_requiring_project_resources(&nested));
    }
}
