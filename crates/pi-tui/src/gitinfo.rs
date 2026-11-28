//! footer 的 cwd 渲染与 git 分支读取（ref `components/footer.ts:32-44`、
//! `core/footer-data-provider.ts:14-48/239-248`）。
//!
//! 两条规则逐条照抄 ref：
//! 1. cwd 在 home 之内 ⇒ 缩写成 `~` 或 `~/…`；不在 home 内 ⇒ 原样返回绝对路径；
//! 2. 分支：向上找 `.git`（目录，或 worktree 的 `.git` 文件 + `commondir`），
//!    读 `HEAD`：`ref: refs/heads/<name>` ⇒ `<name>`；其它（分离头）⇒ `detached`；
//!    找不到仓库 ⇒ 不显示。

use std::path::{Path, PathBuf};

/// cwd 的家目录缩写（ref `footer.ts:32-44` 的 `formatCwdForFooter`）。
#[must_use]
pub fn format_cwd_for_footer(cwd: &str, home: Option<&str>) -> String {
    let Some(home) = home.filter(|value| !value.is_empty()) else {
        return cwd.to_string();
    };
    let Ok(resolved_cwd) = std::fs::canonicalize(cwd) else {
        // cwd 不存在时退回字符串比较（避免因为一个 stat 失败就丢掉缩写）
        return abbreviate(cwd, home).unwrap_or_else(|| cwd.to_string());
    };
    let resolved_home = std::fs::canonicalize(home).unwrap_or_else(|_| PathBuf::from(home));
    match resolved_cwd.strip_prefix(&resolved_home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => cwd.to_string(),
    }
}

fn abbreviate(cwd: &str, home: &str) -> Option<String> {
    let cwd = cwd.trim_end_matches('/');
    let home = home.trim_end_matches('/');
    if cwd == home {
        return Some("~".to_string());
    }
    cwd.strip_prefix(&format!("{home}/"))
        .map(|rest| format!("~/{rest}"))
}

/// git 仓库的关键路径（ref `findGitPaths`）。
struct GitPaths {
    head: PathBuf,
}

/// 向上查找 `.git`：目录直接用；worktree 的 `.git` 是文件，内容 `gitdir: <path>`。
fn find_git_paths(cwd: &Path) -> Option<GitPaths> {
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        let dot_git = current.join(".git");
        match std::fs::metadata(&dot_git) {
            Ok(meta) if meta.is_dir() => {
                let head = dot_git.join("HEAD");
                return head.exists().then_some(GitPaths { head });
            }
            Ok(meta) if meta.is_file() => {
                let content = std::fs::read_to_string(&dot_git).ok()?;
                let git_dir = content.trim().strip_prefix("gitdir: ")?;
                let mut git_dir = if Path::new(git_dir).is_absolute() {
                    PathBuf::from(git_dir)
                } else {
                    current.join(git_dir)
                };
                if let Ok(common) = std::fs::read_to_string(git_dir.join("commondir")) {
                    let common = common.trim();
                    git_dir = if Path::new(common).is_absolute() {
                        PathBuf::from(common)
                    } else {
                        git_dir.join(common)
                    };
                }
                let head = git_dir.join("HEAD");
                return head.exists().then_some(GitPaths { head });
            }
            Ok(_) | Err(_) => {}
        }
        dir = current.parent();
    }
    None
}

/// 当前分支名；不在仓库内返回 `None`，分离头返回 `Some("detached")`
/// （ref `footer-data-provider.ts:239-248`）。
#[must_use]
pub fn git_branch(cwd: &Path) -> Option<String> {
    let paths = find_git_paths(cwd)?;
    let content = std::fs::read_to_string(&paths.head).ok()?;
    let content = content.trim();
    match content.strip_prefix("ref: refs/heads/") {
        Some(branch) if branch != ".invalid" => Some(branch.to_string()),
        // `.invalid` 只有 git 自己能解释；这里不 fork 子进程，按 detached 处理
        Some(_) => Some("detached".to_string()),
        None => Some("detached".to_string()),
    }
}

/// footer 第一行的完整文本：cwd（家目录缩写）+ 可选的分支后缀。
#[must_use]
pub fn footer_cwd_line(cwd: &str, home: Option<&str>) -> String {
    let formatted = format_cwd_for_footer(cwd, home);
    match git_branch(Path::new(cwd)) {
        Some(branch) => format!("{formatted} ({branch})"),
        None => formatted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tmpdir")
    }

    #[test]
    fn abbreviates_home_like_ref() {
        let dir = temp();
        let home = dir.path();
        let sub = home.join("proj");
        std::fs::create_dir_all(&sub).expect("mkdir");
        assert_eq!(
            format_cwd_for_footer(sub.to_str().expect("utf8"), home.to_str()),
            "~/proj"
        );
        assert_eq!(
            format_cwd_for_footer(home.to_str().expect("utf8"), home.to_str()),
            "~"
        );
        assert_eq!(
            format_cwd_for_footer("/opt/other", home.to_str()),
            "/opt/other",
            "home 之外保持绝对路径"
        );
        assert_eq!(
            format_cwd_for_footer("/opt/other", None),
            "/opt/other",
            "没有 home 时原样"
        );
    }

    #[test]
    fn reads_branch_from_git_dir() {
        let dir = temp();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("mkdir");
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").expect("write");
        assert_eq!(git_branch(&repo), Some("main".to_string()));
        assert_eq!(
            footer_cwd_line(repo.to_str().expect("utf8"), dir.path().to_str()),
            {
                let base = format_cwd_for_footer(repo.to_str().expect("utf8"), dir.path().to_str());
                format!("{base} (main)")
            }
        );
    }

    #[test]
    fn worktree_git_file_is_followed() {
        let dir = temp();
        let repo = dir.path().join("wt");
        let gitdir = dir.path().join("real/.git/worktrees/wt");
        std::fs::create_dir_all(&gitdir).expect("mkdir");
        std::fs::create_dir_all(&repo).expect("mkdir");
        std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/feature-x\n").expect("write");
        std::fs::write(repo.join(".git"), format!("gitdir: {}\n", gitdir.display()))
            .expect("write");
        assert_eq!(git_branch(&repo), Some("feature-x".to_string()));
    }

    #[test]
    fn detached_head_reports_detached() {
        let dir = temp();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("mkdir");
        std::fs::write(repo.join(".git/HEAD"), "0123456789abcdef\n").expect("write");
        assert_eq!(git_branch(&repo), Some("detached".to_string()));
    }

    #[test]
    fn outside_repo_has_no_branch() {
        let dir = temp();
        assert_eq!(git_branch(dir.path()), None);
    }
}
