use std::fs;
use std::process::Command;

fn fuselect<I, S>(args: I) -> std::process::Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let home = std::env::temp_dir().join(format!("fuselect-cli-test-{}", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_fuselect"))
        .env("FUSELECT_HOME", &home)
        .args(args)
        .output()
        .expect("Fuselect binary should run");
    let _ = fs::remove_dir_all(home);
    output
}

#[test]
fn help_lists_gateway_subcommand() {
    let output = fuselect(["--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for command in [
        "init",
        "gateway",
        "gateway-token",
        "worker",
        "fusion",
        "codex",
        "tui",
        "doctor",
        "privacy",
        "config",
        "backup",
        "status",
        "logs",
    ] {
        assert!(
            stdout.contains(command),
            "missing command in help: {command}"
        );
    }
}

#[test]
fn help_lists_nested_command_entries() {
    let cases: &[(&[&str], &str)] = &[
        (&["gateway", "--help"], "start"),
        (&["gateway", "--help"], "rotate-key"),
        (&["worker", "--help"], "add"),
        (&["worker", "--help"], "list"),
        (&["worker", "--help"], "show"),
        (&["worker", "--help"], "remove"),
        (&["worker", "--help"], "test"),
        (&["fusion", "preset", "--help"], "add"),
        (&["fusion", "preset", "--help"], "list"),
        (&["fusion", "preset", "--help"], "show"),
        (&["fusion", "preset", "--help"], "remove"),
        (&["codex", "--help"], "setup"),
        (&["codex", "--help"], "status"),
        (&["codex", "--help"], "rollback"),
        (&["config", "--help"], "validate"),
        (&["config", "--help"], "export"),
        (&["backup", "--help"], "create"),
        (&["backup", "--help"], "list"),
        (&["backup", "--help"], "restore"),
        (&["logs", "--help"], "list"),
    ];

    for (args, needle) in cases {
        let output = fuselect(*args);
        assert!(output.status.success(), "help failed for args: {args:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(needle),
            "missing nested command in help: {needle} (args: {args:?})"
        );
    }
}

#[test]
fn help_keeps_chinese_descriptions_for_top_level_and_nested_commands() {
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["--help"],
            &[
                "启动或管理本机 loopback 网关",
                "管理上游 Worker 元数据与 Keyring 引用",
                "启动键盘优先的终端控制台",
                "显示本地数据与上游数据外发边界",
            ],
        ),
        (
            &["worker", "--help"],
            &[
                "添加一个 OpenAI-compatible Worker",
                "列出已配置的 Worker",
                "删除 Worker 元数据及其 Keyring 密钥",
            ],
        ),
        (
            &["fusion", "preset", "--help"],
            &[
                "创建一个 Fusion 预设",
                "列出已配置预设和可用的内置模板",
                "查看预设或内置模板",
            ],
        ),
    ];
    for (args, expected) in cases {
        let output = fuselect(*args);
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        for description in *expected {
            assert!(
                stdout.contains(description),
                "missing Chinese help `{description}` for {args:?}"
            );
        }
    }
}

#[test]
fn missing_command_exits_with_code_2() {
    let output = fuselect([] as [&str; 0]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn clap_syntax_errors_are_plain_stderr_without_ansi() {
    let output = fuselect([
        "--json",
        "worker",
        "add",
        "--input-price-microusd",
        "not-an-integer",
    ]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid value"));
    assert!(!stderr.contains('\u{1b}'));
}

#[test]
fn out_of_scope_commands_exit_with_code_2() {
    let cases: &[&[&str]] = &[
        &["gateway", "start"],
        &["gateway", "rotate-key"],
        &["gateway-token"],
        &["worker", "add"],
        &["codex", "setup"],
        &["tui"],
        &["doctor"],
        &["privacy"],
        &["config", "validate"],
        &["backup", "create"],
        &["status"],
        &["logs", "list"],
    ];

    for args in cases {
        let output = fuselect(*args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "expected exit code 2 for args: {args:?}"
        );
    }
}

#[test]
fn out_of_scope_commands_do_not_create_files_in_cwd() {
    let base = std::env::temp_dir().join(format!("fuselect-phase0-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("temp directory should be creatable");

    let cases: &[&[&str]] = &[
        &["gateway", "start"],
        &["codex", "setup"],
        &["backup", "create"],
    ];

    for args in cases {
        let status = Command::new(env!("CARGO_BIN_EXE_fuselect"))
            .current_dir(&base)
            .env("FUSELECT_HOME", base.join("isolated-home"))
            .args(*args)
            .status()
            .expect("Fuselect binary should run");
        assert_eq!(
            status.code(),
            Some(2),
            "expected exit code 2 for args: {args:?}"
        );
    }

    assert!(
        fs::read_dir(&base)
            .expect("temp directory should be readable")
            .next()
            .is_none(),
        "unimplemented commands must not create files in the working directory"
    );

    let _ = fs::remove_dir_all(&base);
}
