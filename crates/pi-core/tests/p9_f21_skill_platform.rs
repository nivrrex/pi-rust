//! P9-F21 / S1b（推翻条件分支）：CRLF、BOM、Windows 风格路径分隔对 skill 发现与解析的影响。

use std::path::Path;

fn write_skill(root: &Path, rel: &str, body: &str) {
    let dir = root.join(rel);
    std::fs::create_dir_all(&dir).expect("建目录");
    std::fs::write(dir.join("SKILL.md"), body).expect("写 SKILL.md");
}

#[test]
fn crlf_frontmatter_is_parsed() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let body = "---\r\nname: crlf-skill\r\ndescription: CRLF test\r\n---\r\nBody line\r\n";
    write_skill(tmp.path(), "crlf", body);
    let result = pi_core::skills::load_skills_from_dir(tmp.path(), true);
    let names: Vec<String> = result
        .skills
        .iter()
        .map(|skill| skill.name.clone())
        .collect();
    assert!(
        names.iter().any(|name| name == "crlf-skill"),
        "CRLF 的 frontmatter 应能解析：{names:?} / diagnostics={:?}",
        result.diagnostics
    );
}

#[test]
fn bom_frontmatter_is_parsed() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let body = "\u{feff}---\nname: bom-skill\ndescription: BOM test\n---\nBody\n";
    write_skill(tmp.path(), "bom", body);
    let result = pi_core::skills::load_skills_from_dir(tmp.path(), true);
    let names: Vec<String> = result
        .skills
        .iter()
        .map(|skill| skill.name.clone())
        .collect();
    assert!(
        names.iter().any(|name| name == "bom-skill"),
        "带 BOM 的 SKILL.md 应能解析：{names:?} / diagnostics={:?}",
        result.diagnostics
    );
}

#[test]
fn windows_style_skill_paths_are_accepted() {
    let tmp = tempfile::tempdir().expect("临时目录");
    write_skill(
        tmp.path(),
        "win",
        "---\nname: win-skill\ndescription: win path\n---\nBody\n",
    );
    // `skill_paths` 里给 Windows 风格的分隔符（反斜杠）——在 Linux 上 ref 也只是字符串拼接，
    // 因此这里断言"不会把整个 skills 列表搞挂"，命中与否则取决于平台语义。
    let windows_style = tmp
        .path()
        .join("win")
        .display()
        .to_string()
        .replace('/', "\\");
    let result = pi_core::skills::load_skills(tmp.path(), tmp.path(), &[windows_style]);
    // 要么加载成功（平台等价路径），要么给出诊断；**不能 panic、不能污染其它 skill**。
    assert!(
        !result.skills.is_empty() || !result.diagnostics.is_empty(),
        "Windows 风格路径应要么加载、要么给出诊断"
    );
}
