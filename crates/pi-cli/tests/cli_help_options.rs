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
fn unsupported_options_are_explicitly_rejected_at_runtime() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .args(["--extension", "x.js", "-p", "hi"])
        .output()
        .expect("运行");
    assert_eq!(output.status.code(), Some(1), "不支持的选项应以 1 退出");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("不支持的参数") && stderr.contains("--extension"),
        "应明确指出不支持的参数：{stderr}"
    );
}
