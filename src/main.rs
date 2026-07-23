use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use fuselect::app::{AppContext, AppError, OutputMode, PresetInput, WorkerInput};
use fuselect::secrets::{OsKeyringStore, SecretString};
use fuselect::storage::SqliteStore;

#[derive(Parser)]
#[command(name = fuselect::APPLICATION_NAME, version, about = "本地优先的 Coding Agent Fusion 网关", disable_help_flag = false, color = clap::ColorChoice::Never)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// 初始化本机元数据和 Gateway Key（不会打印密钥）
    Init,
    /// 仅在后续阶段实现
    GatewayToken,
    /// 启动或管理本机 loopback 网关
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
    /// 管理上游 Worker 元数据与 Keyring 引用
    Worker {
        #[command(subcommand)]
        command: WorkerCommand,
    },
    /// 管理本地 Fusion 预设
    Fusion {
        #[command(subcommand)]
        command: FusionCommand,
    },
    /// 仅在后续阶段实现
    Codex {
        #[command(subcommand)]
        command: CodexCommand,
    },
    /// 启动键盘优先的终端控制台
    Tui,
    /// 诊断本机配置、密钥库和 Provider 状态
    Doctor,
    /// 显示本地数据与上游数据外发边界
    Privacy,
    /// 校验或导出本机配置
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// 创建、列出或恢复元数据备份
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
    /// 查看网关、预算和 Worker 健康状态
    Status,
    /// 查看只含元数据的运行日志
    Logs {
        #[command(subcommand)]
        command: LogsCommand,
    },
}
#[derive(Subcommand)]
enum GatewayCommand {
    /// 在前台启动仅监听 127.0.0.1 的网关
    Start {
        /// 覆盖默认监听端口
        #[arg(long)]
        port: Option<u16>,
        /// 输出元数据事件，不输出请求或响应正文
        #[arg(long)]
        verbose: bool,
    },
    /// 轮换本机 Gateway Key
    RotateKey,
}
#[derive(Subcommand)]
enum WorkerCommand {
    /// 添加一个 OpenAI-compatible Worker
    Add(Box<WorkerAdd>),
    /// 列出已配置的 Worker
    List,
    /// 查看单个 Worker 的非敏感配置
    Show { id: String },
    /// 删除 Worker 元数据及其 Keyring 密钥
    Remove {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// 探测 Worker 的流式工具兼容性（后续阶段启用）
    Test { id: String },
}
#[derive(Subcommand)]
enum CodexCommand {
    /// 创建独立 Fuselect Codex Profile
    Setup,
    /// 查看 Fuselect Codex Profile 状态
    Status,
    /// 从备份恢复 Codex 配置
    Rollback {
        backup_id: String,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Subcommand)]
enum ConfigCommand {
    /// 校验本机配置和 schema
    Validate,
    /// 导出不含密钥和原始内容的配置
    Export {
        #[arg(long)]
        redact: bool,
    },
}
#[derive(Subcommand)]
enum BackupCommand {
    /// 创建非敏感元数据备份
    Create,
    /// 列出可恢复的备份
    List,
    /// 恢复备份，并在恢复前再次备份
    Restore {
        backup_id: String,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Subcommand)]
enum LogsCommand {
    /// 列出不含 Prompt、代码或工具内容的运行元数据
    List,
}
#[derive(Args)]
struct WorkerAdd {
    /// 稳定、唯一的小写 Worker ID
    #[arg(long)]
    id: Option<String>,
    /// 人类可读名称
    #[arg(long)]
    name: Option<String>,
    /// Provider API Base URL
    #[arg(long)]
    base_url: Option<String>,
    /// Provider 模型 ID
    #[arg(long)]
    model: Option<String>,
    /// 每百万输入 Token 的整数微美元价格
    #[arg(long)]
    input_price_microusd: Option<u64>,
    /// 每百万输出 Token 的整数微美元价格
    #[arg(long)]
    output_price_microusd: Option<u64>,
    /// 每百万缓存输入 Token 的整数微美元价格
    #[arg(long)]
    cached_input_price_microusd: Option<u64>,
    /// 最大上下文 Token
    #[arg(long)]
    context_window: Option<u32>,
    /// 能力标签，可重复；必须包含 tools
    #[arg(long = "capability")]
    capabilities: Vec<String>,
    /// Provider 条款或隐私政策 URL
    #[arg(long)]
    provider_policy_url: Option<String>,
    /// 经过验证的协议兼容配置名
    #[arg(long, default_value = "openai-chat-completions")]
    compatibility_profile: Option<String>,
    /// 已存在的 Keyring 引用；API Key 本身不能作为参数传入
    #[arg(long)]
    secret_ref: Option<String>,
}
#[derive(Subcommand)]
enum FusionCommand {
    Preset {
        #[command(subcommand)]
        command: PresetCommand,
    },
}
#[derive(Subcommand)]
enum PresetCommand {
    /// 创建一个 Fusion 预设
    Add(PresetAdd),
    /// 列出已配置预设和可用的内置模板
    List,
    /// 查看预设或内置模板
    Show { name: String },
    /// 删除一个已配置的 Fusion 预设
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}
#[derive(Args)]
struct PresetAdd {
    /// 预设名称
    #[arg(long)]
    name: Option<String>,
    /// 质量档位：budget 或 high
    #[arg(long)]
    quality_tier: Option<String>,
    /// 外层 Worker 的本地选择策略
    #[arg(long)]
    outer_worker_policy: Option<String>,
    /// 顾问 Worker ID，可重复传入 1–8 次
    #[arg(long = "advisor")]
    advisors: Vec<String>,
    /// Judge Worker ID
    #[arg(long)]
    judge: Option<String>,
    /// 每个 Fusion 阶段允许的最大输出 Token
    #[arg(long)]
    max_completion_tokens: Option<u32>,
    /// 每任务硬预算（整数微美元）
    #[arg(long)]
    task_budget_microusd: Option<u64>,
    /// UTC 每日硬预算（整数微美元）
    #[arg(long)]
    daily_budget_microusd: Option<u64>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let mode = if cli.json {
        OutputMode::Json
    } else {
        OutputMode::Human
    };
    let result = run(cli, mode).await;
    match result {
        Ok(value) => print_success(mode, value),
        Err(error) => {
            print_error(mode, &error);
            std::process::exit(error.exit_code());
        }
    }
}

async fn context(mode: OutputMode) -> Result<AppContext, AppError> {
    Ok(AppContext {
        store: SqliteStore::open_default().await?,
        secrets: Arc::new(OsKeyringStore),
        output_mode: mode,
    })
}
async fn run(cli: Cli, mode: OutputMode) -> Result<serde_json::Value, AppError> {
    let mut terminal = TerminalPrompt;
    run_with_prompt(cli, mode, &mut terminal).await
}

async fn run_with_prompt(
    cli: Cli,
    mode: OutputMode,
    prompt_io: &mut dyn PromptIo,
) -> Result<serde_json::Value, AppError> {
    let Some(command) = cli.command else {
        return Err(AppError::Validation(
            "请使用 `fuselect --help` 查看可用命令。".into(),
        ));
    };
    match command {
        Command::Init => context(mode).await?.init().await,
        Command::Worker { command } => match command {
            WorkerCommand::Add(args) => {
                let (input, new_secret) = resolve_worker_add(*args, prompt_io)?;
                context(mode).await?.add_worker(input, new_secret).await
            }
            WorkerCommand::List => context(mode).await?.workers().await,
            WorkerCommand::Show { id } => context(mode).await?.worker(&id).await,
            WorkerCommand::Remove { id, yes } => {
                if !confirm_action(
                    yes,
                    &format!("将禁用并删除 Worker {id} 的元数据与密钥引用"),
                    prompt_io,
                )? {
                    return Err(AppError::ConfirmationRequired);
                }
                context(mode).await?.remove_worker(&id).await
            }
            WorkerCommand::Test { id } => context(mode).await?.worker_test(&id).await,
        },
        Command::Fusion {
            command: FusionCommand::Preset { command },
        } => match command {
            PresetCommand::Add(args) => {
                let input = resolve_preset_add(args, prompt_io)?;
                context(mode).await?.add_preset(input).await
            }
            PresetCommand::List => context(mode).await?.presets().await,
            PresetCommand::Show { name } => context(mode).await?.preset(&name).await,
            PresetCommand::Remove { name, yes } => {
                if !confirm_action(yes, &format!("将删除 Fusion 预设 {name}"), prompt_io)? {
                    return Err(AppError::ConfirmationRequired);
                }
                context(mode).await?.remove_preset(&name).await
            }
        },
        _ => Err(AppError::Validation("该命令尚未属于本阶段实现范围".into())),
    }
}

trait PromptIo {
    fn is_interactive(&self) -> bool;
    fn prompt(&mut self, label: &str) -> Result<String, AppError>;
    fn prompt_optional(&mut self, label: &str) -> Result<Option<String>, AppError>;
    fn prompt_secret(&mut self, label: &str) -> Result<SecretString, AppError>;
    fn confirm(&mut self, summary: &str) -> Result<bool, AppError>;
}

struct TerminalPrompt;

impl PromptIo for TerminalPrompt {
    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal() && io::stdout().is_terminal()
    }

    fn prompt(&mut self, label: &str) -> Result<String, AppError> {
        print!("{label}：");
        io::stdout().flush().map_err(|_| AppError::Internal)?;
        let mut value = String::new();
        io::stdin()
            .read_line(&mut value)
            .map_err(|_| AppError::Internal)?;
        let value = value.trim().to_owned();
        if value.is_empty() {
            return Err(AppError::Validation(format!("{label} 不能为空")));
        }
        Ok(value)
    }

    fn prompt_optional(&mut self, label: &str) -> Result<Option<String>, AppError> {
        print!("{label}（可选，留空跳过）：");
        io::stdout().flush().map_err(|_| AppError::Internal)?;
        let mut value = String::new();
        io::stdin()
            .read_line(&mut value)
            .map_err(|_| AppError::Internal)?;
        let value = value.trim();
        Ok((!value.is_empty()).then(|| value.to_owned()))
    }

    fn prompt_secret(&mut self, label: &str) -> Result<SecretString, AppError> {
        print!("{label}（隐藏输入）：");
        io::stdout().flush().map_err(|_| AppError::Internal)?;
        let secret = rpassword::read_password().map_err(|_| AppError::Internal)?;
        if secret.is_empty() {
            return Err(AppError::Validation(format!("{label} 不能为空")));
        }
        Ok(SecretString::from(secret))
    }

    fn confirm(&mut self, summary: &str) -> Result<bool, AppError> {
        print!("{summary}。确认继续？[y/N] ");
        io::stdout().flush().map_err(|_| AppError::Internal)?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|_| AppError::Internal)?;
        Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "YES"))
    }
}

fn resolve_worker_add(
    mut args: WorkerAdd,
    prompt_io: &mut dyn PromptIo,
) -> Result<(WorkerInput, Option<SecretString>), AppError> {
    let interactive = prompt_io.is_interactive();
    let id = required(args.id.take(), "Worker ID", interactive, prompt_io)?;
    let name = required(args.name.take(), "显示名称", interactive, prompt_io)?;
    let base_url = required(args.base_url.take(), "Base URL", interactive, prompt_io)?;
    let model = required(args.model.take(), "模型 ID", interactive, prompt_io)?;
    let input_price_microusd = required_number(
        args.input_price_microusd.take(),
        "输入价格（微美元/百万 Token）",
        interactive,
        prompt_io,
    )?;
    let output_price_microusd = required_number(
        args.output_price_microusd.take(),
        "输出价格（微美元/百万 Token）",
        interactive,
        prompt_io,
    )?;
    let cached_input_price_microusd = match args.cached_input_price_microusd.take() {
        Some(value) => Some(value),
        None if interactive => prompt_io
            .prompt_optional("缓存输入价格（微美元/百万 Token）")?
            .map(|value| {
                value.parse().map_err(|_| {
                    AppError::Validation("缓存输入价格（微美元/百万 Token）必须是整数".into())
                })
            })
            .transpose()?,
        None => None,
    };
    let context_window = required_number(
        args.context_window.take(),
        "上下文窗口 Token",
        interactive,
        prompt_io,
    )?;
    let provider_policy_url = required(
        args.provider_policy_url.take(),
        "Provider 条款/隐私 URL",
        interactive,
        prompt_io,
    )?;
    let capabilities = if args.capabilities.is_empty() && interactive {
        parse_list(&prompt_io.prompt("能力标签（逗号分隔，必须包含 tools）")?)
    } else {
        args.capabilities
    };
    let input = WorkerInput {
        id,
        name,
        base_url,
        model,
        input_price_microusd,
        output_price_microusd,
        cached_input_price_microusd,
        context_window,
        capabilities,
        provider_policy_url,
        compatibility_profile: args
            .compatibility_profile
            .unwrap_or_else(|| "openai-chat-completions".into()),
        secret_ref: args.secret_ref,
    };
    let new_secret = if input.secret_ref.is_none() && interactive {
        Some(prompt_io.prompt_secret("API Key")?)
    } else if input.secret_ref.is_none() {
        return Err(AppError::NonInteractiveInputRequired(
            "secret-ref；非交互环境请先写入系统密钥库，再传入 --secret-ref。例如：fuselect worker add --secret-ref fuselect/worker/coder-a …".into(),
        ));
    } else {
        None
    };
    Ok((input, new_secret))
}

fn resolve_preset_add(
    mut args: PresetAdd,
    prompt_io: &mut dyn PromptIo,
) -> Result<PresetInput, AppError> {
    let interactive = prompt_io.is_interactive();
    let name = required(args.name.take(), "预设名称", interactive, prompt_io)?;
    let quality_tier = required(
        args.quality_tier.take(),
        "质量档位（budget/high）",
        interactive,
        prompt_io,
    )?;
    let outer_worker_policy = required(
        args.outer_worker_policy.take(),
        "外层 Worker 策略",
        interactive,
        prompt_io,
    )?;
    let advisors = if args.advisors.is_empty() && interactive {
        parse_list(&prompt_io.prompt("顾问 Worker ID（逗号分隔，1–8 个）")?)
    } else {
        args.advisors
    };
    let judge = required(args.judge.take(), "Judge Worker ID", interactive, prompt_io)?;
    Ok(PresetInput {
        name,
        quality_tier,
        outer_worker_policy,
        advisors,
        judge,
        max_completion_tokens: required_number(
            args.max_completion_tokens.take(),
            "最大输出 Token",
            interactive,
            prompt_io,
        )?,
        task_budget_microusd: required_number(
            args.task_budget_microusd.take(),
            "任务预算（微美元）",
            interactive,
            prompt_io,
        )?,
        daily_budget_microusd: required_number(
            args.daily_budget_microusd.take(),
            "每日预算（微美元）",
            interactive,
            prompt_io,
        )?,
    })
}

fn parse_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn confirm_action(
    yes: bool,
    summary: &str,
    prompt_io: &mut dyn PromptIo,
) -> Result<bool, AppError> {
    if yes {
        return Ok(true);
    }
    if !prompt_io.is_interactive() {
        return Ok(false);
    }
    prompt_io.confirm(summary)
}

fn required(
    value: Option<String>,
    label: &str,
    interactive: bool,
    prompt_io: &mut dyn PromptIo,
) -> Result<String, AppError> {
    match value {
        Some(value) => Ok(value),
        None if interactive => prompt_io.prompt(label),
        None => Err(AppError::NonInteractiveInputRequired(format!(
            "{label}；请提供完整参数或在终端运行"
        ))),
    }
}
fn required_number<T: std::str::FromStr + std::fmt::Display>(
    value: Option<T>,
    label: &str,
    interactive: bool,
    prompt_io: &mut dyn PromptIo,
) -> Result<T, AppError> {
    match value {
        Some(value) => Ok(value),
        None if interactive => prompt_io
            .prompt(label)?
            .parse()
            .map_err(|_| AppError::Validation(format!("{label} 必须是整数"))),
        None => Err(AppError::NonInteractiveInputRequired(format!(
            "{label}；请提供完整参数或在终端运行"
        ))),
    }
}
fn print_success(mode: OutputMode, value: serde_json::Value) {
    match mode {
        OutputMode::Json => println!("{}", serde_json::json!({"status":"success","data":value})),
        OutputMode::Human => match value.get("status").and_then(|v| v.as_str()) {
            Some("initialized") => println!(
                "Fuselect 已初始化\n数据库版本：{}\nGateway Key：已安全保存\n下一步：fuselect worker add",
                value["database_version"]
            ),
            Some("added") => println!(
                "已添加：{}\nAPI Key：已配置",
                value
                    .get("id")
                    .or_else(|| value.get("name"))
                    .unwrap_or(&value)
            ),
            Some("removed") => println!(
                "已删除：{}",
                value
                    .get("id")
                    .or_else(|| value.get("name"))
                    .unwrap_or(&value)
            ),
            _ => println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
            ),
        },
    }
}
fn print_error(mode: OutputMode, error: &AppError) {
    match mode {
        OutputMode::Json => eprintln!(
            "{}",
            serde_json::json!({"status":"error","error":{"type":error.kind(),"message":error.to_string()}})
        ),
        OutputMode::Human => eprintln!("错误：{error}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct FakePrompt {
        interactive: bool,
        answers: VecDeque<String>,
        secrets: VecDeque<String>,
        confirmations: VecDeque<bool>,
        labels: Vec<String>,
        secret_labels: Vec<String>,
        confirmation_count: usize,
    }

    impl FakePrompt {
        fn interactive(answers: &[&str], secrets: &[&str]) -> Self {
            Self {
                interactive: true,
                answers: answers.iter().map(|value| (*value).to_owned()).collect(),
                secrets: secrets.iter().map(|value| (*value).to_owned()).collect(),
                confirmations: VecDeque::new(),
                labels: Vec::new(),
                secret_labels: Vec::new(),
                confirmation_count: 0,
            }
        }

        fn non_interactive() -> Self {
            Self {
                interactive: false,
                answers: VecDeque::new(),
                secrets: VecDeque::new(),
                confirmations: VecDeque::new(),
                labels: Vec::new(),
                secret_labels: Vec::new(),
                confirmation_count: 0,
            }
        }
    }

    impl PromptIo for FakePrompt {
        fn is_interactive(&self) -> bool {
            self.interactive
        }

        fn prompt(&mut self, label: &str) -> Result<String, AppError> {
            self.labels.push(label.to_owned());
            self.answers.pop_front().ok_or(AppError::Internal)
        }

        fn prompt_optional(&mut self, label: &str) -> Result<Option<String>, AppError> {
            self.labels.push(label.to_owned());
            self.answers
                .pop_front()
                .map(|value| (!value.is_empty()).then_some(value))
                .ok_or(AppError::Internal)
        }

        fn prompt_secret(&mut self, label: &str) -> Result<SecretString, AppError> {
            self.secret_labels.push(label.to_owned());
            self.secrets
                .pop_front()
                .map(SecretString::from)
                .ok_or(AppError::Internal)
        }

        fn confirm(&mut self, _summary: &str) -> Result<bool, AppError> {
            self.confirmation_count += 1;
            self.confirmations.pop_front().ok_or(AppError::Internal)
        }
    }

    #[test]
    fn worker_wizard_uses_a_separate_hidden_secret_prompt() {
        let mut prompt = FakePrompt::interactive(
            &[
                "coder-a",
                "Coder A",
                "https://example.test",
                "model-a",
                "10",
                "20",
                "",
                "128000",
                "https://example.test/privacy",
                "coding, tools",
            ],
            &["TOP_SECRET_API_KEY"],
        );
        let args = WorkerAdd {
            id: None,
            name: None,
            base_url: None,
            model: None,
            input_price_microusd: None,
            output_price_microusd: None,
            cached_input_price_microusd: None,
            context_window: None,
            capabilities: Vec::new(),
            provider_policy_url: None,
            compatibility_profile: None,
            secret_ref: None,
        };

        let (input, secret) = resolve_worker_add(args, &mut prompt).unwrap();

        assert_eq!(input.id, "coder-a");
        assert_eq!(input.capabilities, ["coding", "tools"]);
        assert_eq!(input.cached_input_price_microusd, None);
        assert_eq!(secret.unwrap().expose(), "TOP_SECRET_API_KEY");
        assert_eq!(prompt.secret_labels, ["API Key"]);
        assert!(prompt.labels.iter().all(|label| !label.contains("API Key")));
    }

    #[test]
    fn preset_wizard_collects_roles_limits_and_budgets_in_order() {
        let mut prompt = FakePrompt::interactive(
            &[
                "coding-high",
                "high",
                "quality-first",
                "advisor-a, advisor-b",
                "judge-a",
                "4096",
                "500000",
                "5000000",
            ],
            &[],
        );
        let args = PresetAdd {
            name: None,
            quality_tier: None,
            outer_worker_policy: None,
            advisors: Vec::new(),
            judge: None,
            max_completion_tokens: None,
            task_budget_microusd: None,
            daily_budget_microusd: None,
        };

        let input = resolve_preset_add(args, &mut prompt).unwrap();

        assert_eq!(input.name, "coding-high");
        assert_eq!(input.advisors, ["advisor-a", "advisor-b"]);
        assert_eq!(input.judge, "judge-a");
        assert_eq!(input.max_completion_tokens, 4096);
        assert_eq!(input.task_budget_microusd, 500_000);
        assert_eq!(input.daily_budget_microusd, 5_000_000);
        assert_eq!(
            prompt.labels,
            [
                "预设名称",
                "质量档位（budget/high）",
                "外层 Worker 策略",
                "顾问 Worker ID（逗号分隔，1–8 个）",
                "Judge Worker ID",
                "最大输出 Token",
                "任务预算（微美元）",
                "每日预算（微美元）",
            ]
        );
    }

    #[test]
    fn confirmation_obeys_tty_and_yes_boundaries() {
        let mut non_interactive = FakePrompt::non_interactive();
        assert!(!confirm_action(false, "删除", &mut non_interactive).unwrap());
        assert!(confirm_action(true, "删除", &mut non_interactive).unwrap());
        assert_eq!(non_interactive.confirmation_count, 0);

        let mut interactive = FakePrompt::interactive(&[], &[]);
        interactive.confirmations.push_back(true);
        assert!(confirm_action(false, "删除", &mut interactive).unwrap());
        assert_eq!(interactive.confirmation_count, 1);
    }
}
