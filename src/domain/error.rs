use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    DuplicateWorkerId(String),
    DuplicateAdvisor,
    EmptyField(&'static str),
    InvalidAdvisorCount {
        minimum: usize,
        maximum: usize,
        found: usize,
    },
    InvalidGatewayPort,
    InvalidIdentifier(String),
    InvalidPrice,
    InvalidUrl(String),
    JudgeIsAdvisor(String),
    MissingCapability {
        worker_id: String,
        capability: &'static str,
    },
    MissingWorker(String),
    TooManyWorkers {
        maximum: usize,
        found: usize,
    },
    UnsupportedSchemaVersion {
        found: u32,
        supported: u32,
    },
    ZeroContextWindow(String),
}

impl Display for ConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateWorkerId(id) => write!(formatter, "Worker ID 重复：{id}"),
            Self::DuplicateAdvisor => formatter.write_str("Fusion 顾问 ID 不能重复"),
            Self::EmptyField(field) => write!(formatter, "字段不能为空：{field}"),
            Self::InvalidAdvisorCount {
                minimum,
                maximum,
                found,
            } => write!(
                formatter,
                "Fusion 顾问数量必须在 {minimum}–{maximum} 之间，当前为 {found}"
            ),
            Self::InvalidGatewayPort => formatter.write_str("网关端口不能为 0"),
            Self::InvalidIdentifier(value) => write!(formatter, "Worker ID 非法：{value}"),
            Self::InvalidPrice => formatter.write_str("价格必须是非零的整数微美元"),
            Self::InvalidUrl(value) => write!(formatter, "上游地址必须使用 HTTPS：{value}"),
            Self::JudgeIsAdvisor(id) => write!(formatter, "Judge 不能同时作为顾问：{id}"),
            Self::MissingCapability {
                worker_id,
                capability,
            } => write!(formatter, "Worker {worker_id} 缺少必需能力：{capability}"),
            Self::MissingWorker(id) => write!(formatter, "Fusion 策略引用了不存在的 Worker：{id}"),
            Self::TooManyWorkers { maximum, found } => {
                write!(formatter, "Worker 最多允许 {maximum} 个，当前为 {found} 个")
            }
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                formatter,
                "不支持的配置版本：{found}（当前版本支持 {supported}）"
            ),
            Self::ZeroContextWindow(id) => write!(formatter, "Worker {id} 的上下文窗口必须大于 0"),
        }
    }
}

impl std::error::Error for ConfigError {}
