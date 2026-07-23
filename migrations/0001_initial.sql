-- Fuselect metadata schema v1.
-- Secrets (API keys, gateway keys) are never stored in SQLite.

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workers (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    model_id TEXT NOT NULL,
    input_price_microusd INTEGER NOT NULL CHECK (input_price_microusd > 0),
    output_price_microusd INTEGER NOT NULL CHECK (output_price_microusd > 0),
    cached_input_price_microusd INTEGER,
    context_window_tokens INTEGER NOT NULL CHECK (context_window_tokens > 0),
    capabilities TEXT NOT NULL,
    supports_streaming INTEGER NOT NULL CHECK (supports_streaming IN (0, 1)),
    provider_policy_url TEXT NOT NULL,
    secret_ref TEXT NOT NULL,
    compatibility_profile TEXT NOT NULL DEFAULT 'openai-chat-completions',
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    health_status TEXT NOT NULL DEFAULT 'unknown',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    schema_version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS fusion_presets (
    name TEXT PRIMARY KEY NOT NULL,
    quality_tier TEXT NOT NULL,
    outer_worker_policy TEXT NOT NULL,
    advisor_worker_ids TEXT NOT NULL,
    judge_worker_id TEXT NOT NULL,
    max_completion_tokens INTEGER NOT NULL CHECK (max_completion_tokens > 0),
    task_budget_microusd INTEGER NOT NULL CHECK (task_budget_microusd > 0),
    daily_budget_microusd INTEGER NOT NULL CHECK (daily_budget_microusd > 0),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    schema_version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS gateway (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    port INTEGER NOT NULL CHECK (port > 0),
    metadata_retention_days INTEGER NOT NULL CHECK (metadata_retention_days > 0),
    gateway_key_ref TEXT NOT NULL,
    durable_session_enabled INTEGER NOT NULL CHECK (durable_session_enabled IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    schema_version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS request_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id_hash TEXT NOT NULL UNIQUE,
    public_model TEXT NOT NULL,
    route_mode TEXT NOT NULL,
    selected_outer_worker_id TEXT,
    preset_name TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    latency_ms INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    known_cost_microusd INTEGER,
    cost_unknown INTEGER NOT NULL CHECK (cost_unknown IN (0, 1)),
    outcome TEXT,
    error_category TEXT
);

CREATE TABLE IF NOT EXISTS run_participants (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES request_runs (id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('outer', 'advisor', 'judge')),
    stage_order INTEGER NOT NULL,
    UNIQUE (run_id, worker_id, role, stage_order)
);

CREATE TABLE IF NOT EXISTS run_stages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES request_runs (id) ON DELETE CASCADE,
    stage_type TEXT NOT NULL,
    worker_id TEXT,
    attempt_number INTEGER NOT NULL DEFAULT 1 CHECK (attempt_number > 0),
    started_at TEXT NOT NULL,
    completed_at TEXT,
    latency_ms INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER,
    known_cost_microusd INTEGER,
    cost_unknown INTEGER NOT NULL CHECK (cost_unknown IN (0, 1)),
    outcome TEXT,
    error_category TEXT
);

CREATE TABLE IF NOT EXISTS daily_spend (
    utc_date TEXT PRIMARY KEY NOT NULL,
    reserved_microusd INTEGER NOT NULL DEFAULT 0 CHECK (reserved_microusd >= 0),
    settled_microusd INTEGER NOT NULL DEFAULT 0 CHECK (settled_microusd >= 0),
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_request_runs_started_at ON request_runs (started_at);
CREATE INDEX IF NOT EXISTS idx_run_stages_run_id ON run_stages (run_id);
CREATE INDEX IF NOT EXISTS idx_run_participants_run_id ON run_participants (run_id);
