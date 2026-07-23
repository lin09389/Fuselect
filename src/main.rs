use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = fuselect::APPLICATION_NAME,
    version,
    about = "本地优先的 Coding Agent Fusion 网关",
    long_about = "Fuselect 为 Codex 提供一个本机、经过认证的模型 Fusion 网关。\n\
                  当前构建仅包含命令契约；执行层会在后续里程碑逐步启用。"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 初始化本机状态与默认配置
    Init,
    /// 启动或管理本机网关
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
    /// 只输出 Gateway Key，供 Codex 的 auth.command 使用
    GatewayToken,
    /// 管理上游 OpenAI-compatible Worker
    Worker {
        #[command(subcommand)]
        command: WorkerCommand,
    },
    /// 管理 Fusion 预设
    Fusion {
        #[command(subcommand)]
        command: FusionCommand,
    },
    /// 配置、检查或回滚 Codex Profile
    Codex {
        #[command(subcommand)]
        command: CodexCommand,
    },
    /// 启动键盘优先的终端控制台
    Tui,
    /// 诊断本机运行环境
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
    Status {
        #[command(flatten)]
        output: JsonOutput,
    },
    /// 查看只含元数据的运行日志
    Logs {
        #[command(subcommand)]
        command: LogsCommand,
    },
}

#[derive(Debug, Subcommand)]
enum GatewayCommand {
    /// 在前台启动 loopback 网关
    Start {
        /// 覆盖默认监听端口
        #[arg(long)]
        port: Option<u16>,
        /// 输出请求元数据事件，不包含请求或响应正文
        #[arg(long)]
        verbose: bool,
    },
    /// 轮换仅供本机使用的 Gateway Key
    RotateKey,
}

#[derive(Debug, Subcommand)]
enum WorkerCommand {
    /// 添加一个上游 Worker
    Add,
    /// 列出已配置的 Worker
    List {
        #[command(flatten)]
        output: JsonOutput,
    },
    /// 查看单个 Worker 详情
    Show { id: String },
    /// 删除一个 Worker
    Remove { id: String },
    /// 探测 Worker 的 Chat Completions 能力
    Test { id: String },
}

#[derive(Debug, Subcommand)]
enum FusionCommand {
    Preset {
        #[command(subcommand)]
        command: PresetCommand,
    },
}

#[derive(Debug, Subcommand)]
enum PresetCommand {
    /// 创建 Fusion 预设
    Add,
    /// 列出 Fusion 预设
    List {
        #[command(flatten)]
        output: JsonOutput,
    },
    /// 查看单个 Fusion 预设
    Show { name: String },
    /// 删除 Fusion 预设
    Remove { name: String },
}

#[derive(Debug, Subcommand)]
enum CodexCommand {
    /// 创建 Fuselect Codex Profile 并备份现有配置
    Setup {
        #[command(flatten)]
        confirmation: Confirmation,
    },
    /// 查看 Codex Profile 配置状态
    Status {
        #[command(flatten)]
        output: JsonOutput,
    },
    /// 从备份恢复 Codex 配置
    Rollback {
        backup_id: String,
        #[command(flatten)]
        confirmation: Confirmation,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// 校验本机配置 schema 与 Worker 约束
    Validate,
    /// 导出可共享的配置快照
    Export {
        /// 导出时必须显式启用脱敏；密钥与原始请求内容始终排除
        #[arg(long)]
        redact: bool,
    },
}

#[derive(Debug, Subcommand)]
enum BackupCommand {
    /// 创建元数据备份
    Create,
    /// 列出可用备份
    List {
        #[command(flatten)]
        output: JsonOutput,
    },
    /// 从备份恢复元数据
    Restore {
        backup_id: String,
        #[command(flatten)]
        confirmation: Confirmation,
    },
}

#[derive(Debug, Subcommand)]
enum LogsCommand {
    /// 列出只含元数据的运行日志
    List {
        /// 仅包含此 RFC 3339 时间戳及之后的记录
        #[arg(long)]
        since: Option<String>,
        #[command(flatten)]
        output: JsonOutput,
    },
}

#[derive(Debug, Clone, Args)]
struct JsonOutput {
    /// Emit stable machine-readable output without ANSI colour.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Args)]
struct Confirmation {
    /// Confirm a mutating action when no interactive prompt is available.
    #[arg(long)]
    yes: bool,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(command) => {
            eprintln!("命令 {command:?} 尚未实现；请参阅项目规划继续完成对应里程碑。");
            std::process::exit(2);
        }
        None => {
            eprintln!("请使用 `fuselect --help` 查看可用命令。");
            std::process::exit(2);
        }
    }
}
