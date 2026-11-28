//! `--help` 与解析器必须一致：help 里列出的每个选项都要能被 `args::parse` 识别
//! （不能出现"文档里有、运行时被忽略"的项）。`cli/args.ts:272-330` 是上游清单。

use pi_cli::args;

fn help_text() -> String {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--help")
        .output()
        .expect("运行 --help");
    assert!(output.status.success(), "--help 应以 0 退出");
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// 取 help 里 `Options:` 段到 `Built-in tools:` 段之间的所有 `  --xxx` 名字。
fn listed_options(help: &str) -> Vec<String> {
    let mut options = Vec::new();
    let mut in_options = false;
    for line in help.lines() {
        if line.trim() == "Options:" {
            in_options = true;
            continue;
        }
        if line.trim().starts_with("Built-in tools:") {
            break;
        }
        if !in_options {
            continue;
        }
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("--") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
            .collect();
        if !name.is_empty() {
            options.push(name);
        }
    }
    options
}

#[test]
fn every_option_in_help_is_parsed() {
    let help = help_text();
    let options = listed_options(&help);
    assert!(options.len() >= 25, "help 里的选项太少：{options:?}");
    for name in options {
        let argv = vec![format!("--{name}")];
        let parsed = args::parse(&argv);
        let recognized = !parsed.unknown_flags.contains_key(&name)
            && !parsed
                .unsupported
                .iter()
                .any(|item| item == &format!("--{name}"));
        assert!(
            recognized || name == "help" || name == "version",
            "--{name} 在 help 里列出但解析器不认识（或落进 unsupported）"
        );
    }
}

#[test]
fn unknown_options_are_explicitly_rejected_at_runtime() {
    // 原名叫 `unsupported_options_…`、用 `--theme` 当样本——但 `--theme` 在 F220
    // 实现了（追加自定义主题目录），`--help` 里的选项至此**全部可用**。
    // 这条改为验证**未知**选项仍然明确报错（不静默忽略），这是 RUNBOOK-4 的要求。
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .args(["--definitely-not-an-option", "-p", "hi"])
        .output()
        .expect("运行");
    assert_eq!(output.status.code(), Some(1), "未知选项应以 1 退出");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unknown option") && stderr.contains("--definitely-not-an-option"),
        "应明确指出未知选项：{stderr}"
    );
}
