//! `--approve` / `--no-approve`（ref `cli/args.ts:229-232` 的 `projectTrustOverride`）。
//!
//! 此前二者在 `args.rs` 里被塞进 `unsupported` ⇒ 传了就报错拒绝，
//! 而 `--help` 里**明明列着**它们（`Trust project-local files for this run`），
//! 用户按 help 用会被拒。底层 `trust::resolve_project_trusted` 的
//! `trust_override` 参数早就支持，只差命令行没接上。

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn approve_sets_a_positive_override() {
    let parsed = pi_cli::args::parse(&owned(&["--approve", "-p", "hi"]));
    assert_eq!(parsed.project_trust_override, Some(true));
}

#[test]
fn no_approve_sets_a_negative_override() {
    let parsed = pi_cli::args::parse(&owned(&["--no-approve", "-p", "hi"]));
    assert_eq!(parsed.project_trust_override, Some(false));
}

#[test]
fn short_forms_work_too() {
    let a = pi_cli::args::parse(&owned(&["-a", "-p", "hi"]));
    assert_eq!(a.project_trust_override, Some(true));
    let n = pi_cli::args::parse(&owned(&["-na", "-p", "hi"]));
    assert_eq!(n.project_trust_override, Some(false));
}

/// 不传时保持 `None`（交给 trust 的其余五级裁决）。
#[test]
fn absent_leaves_the_override_unset() {
    let parsed = pi_cli::args::parse(&owned(&["-p", "hi"]));
    assert_eq!(parsed.project_trust_override, None);
}
