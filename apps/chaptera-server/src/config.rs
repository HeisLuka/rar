use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fmt, fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use chaptera_untrusted_pub_scan::PubScanPolicyV1;
use serde::Deserialize;
use url::Url;
use zeroize::Zeroizing;

use crate::{
    source_ingress_security::SourceSecurityScannerConfig, upload_admission::UploadAdmissionConfig,
};

pub const DEFAULT_LISTEN: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_SECRET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvironmentMode {
    Dev,
    Test,
    Prod,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapteraConfig {
    pub environment: EnvironmentMode,
    pub listen: SocketAddr,
    pub public_origin: Option<String>,
    pub sqlite: SqliteConfig,
    pub worker: WorkerConfig,
    pub storage: StorageConfig,
    pub limits: LimitsConfig,
    pub upload_admission: UploadAdmissionRuntimeConfig,
    pub source_validation: SourceValidationRuntimeConfig,
    #[serde(default)]
    pub edge: EdgeConfig,
    #[serde(default)]
    pub source_ingress: Option<SourceIngressHttpRuntimeConfig>,
    pub auth: Option<AuthConfig>,
    pub key_ring: Option<KeyRingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SqliteConfig {
    pub path: PathBuf,
    pub journal_mode: String,
    pub synchronous: String,
    pub busy_timeout_ms: u64,
    pub pool_max: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub heavy_concurrency: u32,
    pub light_concurrency: u32,
    pub quota_shared_capacity: i64,
    pub quota_semantic_headroom: i64,
    pub quota_export_cap: i64,
    pub quota_background_cap: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    pub provider: String,
    pub quarantine_namespace: String,
    pub private_namespace: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    pub worker_spool_bytes: u64,
    pub min_free_disk_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EdgeConfig {
    pub trusted_proxy_ips: Vec<IpAddr>,
    pub max_header_bytes: u64,
    pub max_api_body_bytes: u64,
    pub max_upload_body_bytes: u64,
    pub request_timeout_ms: u64,
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            trusted_proxy_ips: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
            max_header_bytes: 32 * 1024,
            max_api_body_bytes: 8 * 1024 * 1024,
            max_upload_body_bytes: 256 * 1024 * 1024,
            request_timeout_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadAdmissionRuntimeConfig {
    pub principal_concurrent_cap: i64,
    pub tenant_concurrent_cap: i64,
    pub principal_bytes_cap: i64,
    pub tenant_bytes_cap: i64,
    pub max_single_upload_bytes: i64,
    pub lease_seconds: u64,
    pub retention_seconds: u64,
}

impl UploadAdmissionRuntimeConfig {
    pub fn materialize(&self) -> UploadAdmissionConfig {
        UploadAdmissionConfig {
            principal_concurrent_cap: self.principal_concurrent_cap,
            tenant_concurrent_cap: self.tenant_concurrent_cap,
            principal_bytes_cap: self.principal_bytes_cap,
            tenant_bytes_cap: self.tenant_bytes_cap,
            max_single_upload_bytes: self.max_single_upload_bytes,
            lease_duration: Duration::from_secs(self.lease_seconds),
            retention: Duration::from_secs(self.retention_seconds),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceValidationRuntimeConfig {
    pub clamd_endpoint: SocketAddr,
    pub clamd_connect_timeout_ms: u64,
    pub clamd_io_timeout_ms: u64,
    pub isolation_python: PathBuf,
    pub isolation_harness: PathBuf,
    pub worker_binary: PathBuf,
    pub worker_wall_timeout_ms: u64,
    pub worker_address_space_mb: u64,
    pub worker_cpu_seconds: u64,
    pub worker_open_files: u64,
    pub worker_output_file_mb: u64,
    pub max_file_bytes: u64,
    pub max_cfb_entries: u64,
    pub max_declared_stream_bytes: u64,
    pub temp_root: PathBuf,
}

impl SourceValidationRuntimeConfig {
    pub fn materialize(&self) -> SourceSecurityScannerConfig {
        SourceSecurityScannerConfig {
            clamd_endpoint: self.clamd_endpoint,
            clamd_connect_timeout: Duration::from_millis(self.clamd_connect_timeout_ms),
            clamd_io_timeout: Duration::from_millis(self.clamd_io_timeout_ms),
            isolation_python: self.isolation_python.clone(),
            isolation_harness: self.isolation_harness.clone(),
            worker_binary: self.worker_binary.clone(),
            worker_wall_timeout: Duration::from_millis(self.worker_wall_timeout_ms),
            worker_address_space_mb: self.worker_address_space_mb,
            worker_cpu_seconds: self.worker_cpu_seconds,
            worker_open_files: self.worker_open_files,
            worker_output_file_mb: self.worker_output_file_mb,
            policy: PubScanPolicyV1 {
                max_file_bytes: self.max_file_bytes,
                max_cfb_entries: self.max_cfb_entries,
                max_declared_stream_bytes: self.max_declared_stream_bytes,
            },
            temp_root: self.temp_root.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIngressHttpRuntimeConfig {
    pub upload_ttl_seconds: u64,
    pub direct_grant_ttl_seconds: u64,
    pub baseline: SourceBaselineRuntimeConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBaselineRuntimeConfig {
    pub isolation_python: PathBuf,
    pub isolation_harness: PathBuf,
    pub worker_binary: PathBuf,
    pub worker_wall_timeout_ms: u64,
    pub worker_address_space_mb: u64,
    pub worker_cpu_seconds: u64,
    pub worker_open_files: u64,
    pub worker_output_file_mb: u64,
    pub temp_root: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    pub login_flow_ttl_seconds: u64,
    pub session_idle_ttl_seconds: u64,
    pub session_absolute_ttl_seconds: u64,
    pub oidc: OidcConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub redirect_path: String,
    pub client_secret: SecretRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRingConfig {
    pub active: String,
    #[serde(default)]
    pub previous: Vec<String>,
    pub keys: Vec<KeyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyConfig {
    pub id: String,
    pub secret: SecretRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SecretRef {
    Env { name: String },
    File { path: PathBuf },
    Systemd { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub listen: SocketAddr,
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let listen = match env::var("CHAPTERA_LISTEN") {
            Ok(raw) => raw
                .parse()
                .map_err(|_| ConfigError::new("invalid_listen", "invalid CHAPTERA_LISTEN value"))?,
            Err(env::VarError::NotPresent) => DEFAULT_LISTEN,
            Err(env::VarError::NotUnicode(_)) => {
                return Err(ConfigError::new(
                    "invalid_listen",
                    "CHAPTERA_LISTEN is not valid UTF-8",
                ));
            }
        };

        let config = Self { listen };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_listener(self.listen)
    }
}

impl ChapteraConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let metadata = fs::metadata(path).map_err(|error| {
            ConfigError::new(
                "config_read_failed",
                format!("cannot stat config {}: {error}", path.display()),
            )
        })?;

        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::new(
                "config_too_large",
                format!(
                    "config {} exceeds {} bytes",
                    path.display(),
                    MAX_CONFIG_BYTES
                ),
            ));
        }

        let source = fs::read_to_string(path).map_err(|error| {
            ConfigError::new(
                "config_read_failed",
                format!("cannot read config {}: {error}", path.display()),
            )
        })?;

        let config: Self = toml::from_str(&source).map_err(|error| {
            ConfigError::new(
                "config_parse_failed",
                format!("invalid TOML config: {error}"),
            )
        })?;

        config.validate()?;
        Ok(config)
    }

    pub fn development_from_env() -> Result<Self, ConfigError> {
        let listen = RuntimeConfig::from_env()?.listen;
        let sqlite_path = env::var_os("CHAPTERA_SQLITE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("chaptera-dev.sqlite"));

        let config = Self {
            environment: EnvironmentMode::Dev,
            listen,
            public_origin: Some("http://127.0.0.1:8080".to_owned()),
            sqlite: SqliteConfig {
                path: sqlite_path,
                journal_mode: "wal".to_owned(),
                synchronous: "full".to_owned(),
                busy_timeout_ms: 5_000,
                pool_max: 4,
            },
            worker: WorkerConfig {
                heavy_concurrency: 1,
                light_concurrency: 2,
                quota_shared_capacity: 2,
                quota_semantic_headroom: 1,
                quota_export_cap: 1,
                quota_background_cap: 1,
            },
            storage: StorageConfig {
                provider: "s3-compatible".to_owned(),
                quarantine_namespace: "chaptera-dev-quarantine".to_owned(),
                private_namespace: "chaptera-dev-private".to_owned(),
            },
            limits: LimitsConfig {
                worker_spool_bytes: 4 * 1024 * 1024 * 1024,
                min_free_disk_bytes: 1024 * 1024 * 1024,
            },
            upload_admission: UploadAdmissionRuntimeConfig {
                principal_concurrent_cap: 2,
                tenant_concurrent_cap: 8,
                principal_bytes_cap: 512 * 1024 * 1024,
                tenant_bytes_cap: 2 * 1024 * 1024 * 1024,
                max_single_upload_bytes: 256 * 1024 * 1024,
                lease_seconds: 3600,
                retention_seconds: 7 * 24 * 60 * 60,
            },
            source_validation: SourceValidationRuntimeConfig {
                clamd_endpoint: "127.0.0.1:3310".parse().expect("static clamd endpoint"),
                clamd_connect_timeout_ms: 2_000,
                clamd_io_timeout_ms: 5_000,
                isolation_python: PathBuf::from("python3"),
                isolation_harness: PathBuf::from("tools/migration_pdf_worker_isolation.py"),
                worker_binary: PathBuf::from("target/debug/chaptera-untrusted-pub-worker"),
                worker_wall_timeout_ms: 15_000,
                worker_address_space_mb: 512,
                worker_cpu_seconds: 10,
                worker_open_files: 64,
                worker_output_file_mb: 32,
                max_file_bytes: 256 * 1024 * 1024,
                max_cfb_entries: 8_192,
                max_declared_stream_bytes: 512 * 1024 * 1024,
                temp_root: env::temp_dir(),
            },
            edge: EdgeConfig::default(),
            source_ingress: None,
            auth: None,
            key_ring: None,
        };

        config.validate()?;
        Ok(config)
    }

    pub fn runtime_config(&self) -> RuntimeConfig {
        RuntimeConfig {
            listen: self.listen,
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_listener(self.listen)?;
        validate_origin(self.environment, self.public_origin.as_deref())?;
        validate_edge(self.environment, &self.edge)?;

        if self.sqlite.path.as_os_str().is_empty() {
            return Err(ConfigError::new(
                "sqlite_path_required",
                "sqlite.path must be non-empty",
            ));
        }
        if self.environment == EnvironmentMode::Prod && !self.sqlite.path.is_absolute() {
            return Err(ConfigError::new(
                "sqlite_path_not_absolute",
                "prod sqlite.path must be absolute",
            ));
        }
        if !self.sqlite.journal_mode.eq_ignore_ascii_case("wal") {
            return Err(ConfigError::new(
                "sqlite_journal_mode_invalid",
                "sqlite.journal_mode must be wal",
            ));
        }
        if !self.sqlite.synchronous.eq_ignore_ascii_case("full") {
            return Err(ConfigError::new(
                "sqlite_synchronous_invalid",
                "sqlite.synchronous must be full",
            ));
        }
        if !(1..=30_000).contains(&self.sqlite.busy_timeout_ms) {
            return Err(ConfigError::new(
                "sqlite_busy_timeout_invalid",
                "sqlite.busy_timeout_ms must be between 1 and 30000",
            ));
        }
        if !(1..=16).contains(&self.sqlite.pool_max) {
            return Err(ConfigError::new(
                "sqlite_pool_max_invalid",
                "sqlite.pool_max must be between 1 and 16",
            ));
        }

        if !(1..=4).contains(&self.worker.heavy_concurrency) {
            return Err(ConfigError::new(
                "worker_heavy_concurrency_invalid",
                "worker.heavy_concurrency must be between 1 and 4",
            ));
        }
        if !(1..=32).contains(&self.worker.light_concurrency) {
            return Err(ConfigError::new(
                "worker_light_concurrency_invalid",
                "worker.light_concurrency must be between 1 and 32",
            ));
        }
        if self.worker.quota_shared_capacity <= 0 {
            return Err(ConfigError::new(
                "worker_quota_shared_capacity_invalid",
                "worker.quota_shared_capacity must be positive",
            ));
        }
        for (field, value) in [
            (
                "worker.quota_semantic_headroom",
                self.worker.quota_semantic_headroom,
            ),
            ("worker.quota_export_cap", self.worker.quota_export_cap),
            (
                "worker.quota_background_cap",
                self.worker.quota_background_cap,
            ),
        ] {
            if value < 0 {
                return Err(ConfigError::new(
                    "worker_quota_budget_invalid",
                    format!("{field} must be non-negative"),
                ));
            }
        }

        validate_nonempty("storage.provider", &self.storage.provider)?;
        validate_nonempty(
            "storage.quarantine_namespace",
            &self.storage.quarantine_namespace,
        )?;
        validate_nonempty("storage.private_namespace", &self.storage.private_namespace)?;
        if self.storage.quarantine_namespace == self.storage.private_namespace {
            return Err(ConfigError::new(
                "storage_namespace_collision",
                "quarantine and private storage namespaces must differ",
            ));
        }

        if self.limits.worker_spool_bytes == 0 || self.limits.min_free_disk_bytes == 0 {
            return Err(ConfigError::new(
                "disk_limit_invalid",
                "worker_spool_bytes and min_free_disk_bytes must be non-zero",
            ));
        }

        self.upload_admission
            .materialize()
            .validate()
            .map_err(|error| ConfigError::new(error.code, error.message))?;
        self.source_validation
            .materialize()
            .validate()
            .map_err(|error| ConfigError::new(error.code, error.message))?;

        let admitted_max =
            u64::try_from(self.upload_admission.max_single_upload_bytes).map_err(|_| {
                ConfigError::new(
                    "source_upload_limit_invalid",
                    "upload admission max_single_upload_bytes does not fit u64",
                )
            })?;
        if admitted_max != self.source_validation.max_file_bytes {
            return Err(ConfigError::new(
                "source_upload_limit_mismatch",
                "upload admission and source validation must use one V0 max PUB byte limit",
            ));
        }
        if self.source_validation.max_declared_stream_bytes < self.source_validation.max_file_bytes
        {
            return Err(ConfigError::new(
                "source_validation_stream_limit_invalid",
                "declared stream byte limit must be >= source file byte limit",
            ));
        }
        if self.environment == EnvironmentMode::Prod {
            for (field, path) in [
                (
                    "source_validation.isolation_python",
                    &self.source_validation.isolation_python,
                ),
                (
                    "source_validation.isolation_harness",
                    &self.source_validation.isolation_harness,
                ),
                (
                    "source_validation.worker_binary",
                    &self.source_validation.worker_binary,
                ),
                (
                    "source_validation.temp_root",
                    &self.source_validation.temp_root,
                ),
            ] {
                if !path.is_absolute() {
                    return Err(ConfigError::new(
                        "source_validation_path_not_absolute",
                        format!("prod {field} must be absolute"),
                    ));
                }
            }
        }

        if let Some(source) = &self.source_ingress {
            if self.auth.is_none() {
                return Err(ConfigError::new(
                    "source_ingress_auth_required",
                    "source ingress HTTP routes require configured authentication",
                ));
            }
            validate_source_ingress_http(self.environment, &self.upload_admission, source)?;
        }

        match (&self.auth, self.environment) {
            (Some(auth), mode) => validate_auth(mode, auth)?,
            (None, EnvironmentMode::Prod) => {
                return Err(ConfigError::new(
                    "prod_auth_required",
                    "prod configuration requires auth.oidc",
                ));
            }
            (None, _) => {}
        }

        if let Some(key_ring) = &self.key_ring {
            validate_key_ring(self.environment, key_ring)?;
        }

        Ok(())
    }

    pub fn resolve_required_secrets(
        &self,
        resolver: &SecretResolver,
    ) -> Result<ResolvedSecrets, ConfigError> {
        let oidc_client_secret = self
            .auth
            .as_ref()
            .map(|auth| resolver.resolve(self.environment, &auth.oidc.client_secret))
            .transpose()?;

        let key_ring = self
            .key_ring
            .as_ref()
            .map(|ring| resolver.resolve_key_ring(self.environment, ring))
            .transpose()?;

        Ok(ResolvedSecrets {
            oidc_client_secret,
            key_ring,
        })
    }
}

fn validate_source_ingress_http(
    mode: EnvironmentMode,
    admission: &UploadAdmissionRuntimeConfig,
    source: &SourceIngressHttpRuntimeConfig,
) -> Result<(), ConfigError> {
    if source.upload_ttl_seconds == 0
        || source.direct_grant_ttl_seconds == 0
        || source.upload_ttl_seconds > admission.lease_seconds
        || source.direct_grant_ttl_seconds > source.upload_ttl_seconds
    {
        return Err(ConfigError::new(
            "source_ingress_ttl_order_invalid",
            "direct grant TTL must fit upload TTL, which must fit upload admission lease TTL",
        ));
    }

    let baseline = &source.baseline;
    if baseline.worker_wall_timeout_ms == 0
        || baseline.worker_address_space_mb < 64
        || baseline.worker_cpu_seconds == 0
        || baseline.worker_open_files < 16
        || baseline.worker_output_file_mb == 0
    {
        return Err(ConfigError::new(
            "source_ingress_baseline_limit_invalid",
            "source ingress baseline worker limits must be explicit and bounded",
        ));
    }

    for (field, path) in [
        ("source_ingress.baseline.isolation_python", &baseline.isolation_python),
        ("source_ingress.baseline.isolation_harness", &baseline.isolation_harness),
        ("source_ingress.baseline.worker_binary", &baseline.worker_binary),
        ("source_ingress.baseline.temp_root", &baseline.temp_root),
    ] {
        if path.as_os_str().is_empty() {
            return Err(ConfigError::new(
                "source_ingress_path_required",
                format!("{field} must be configured"),
            ));
        }
        if mode == EnvironmentMode::Prod && !path.is_absolute() {
            return Err(ConfigError::new(
                "source_ingress_path_not_absolute",
                format!("{field} must be absolute in prod"),
            ));
        }
    }

    Ok(())
}

fn validate_edge(mode: EnvironmentMode, edge: &EdgeConfig) -> Result<(), ConfigError> {
    if mode == EnvironmentMode::Prod && edge.trusted_proxy_ips.is_empty() {
        return Err(ConfigError::new(
            "trusted_proxy_required",
            "prod edge requires at least one configured trusted proxy peer",
        ));
    }

    let mut peers = BTreeSet::new();
    for peer in &edge.trusted_proxy_ips {
        if !is_private_listener(*peer) {
            return Err(ConfigError::new(
                "trusted_proxy_public_forbidden",
                format!("trusted proxy peer {peer} must be loopback or private"),
            ));
        }
        if !peers.insert(*peer) {
            return Err(ConfigError::new(
                "trusted_proxy_duplicate",
                format!("trusted proxy peer {peer} is duplicated"),
            ));
        }
    }

    if !(1024..=64 * 1024).contains(&edge.max_header_bytes) {
        return Err(ConfigError::new(
            "edge_header_limit_invalid",
            "edge.max_header_bytes must be between 1024 and 65536",
        ));
    }
    if !(1024..=16 * 1024 * 1024).contains(&edge.max_api_body_bytes) {
        return Err(ConfigError::new(
            "edge_api_body_limit_invalid",
            "edge.max_api_body_bytes must be between 1024 and 16777216",
        ));
    }
    if edge.max_upload_body_bytes < edge.max_api_body_bytes
        || edge.max_upload_body_bytes > 512 * 1024 * 1024
    {
        return Err(ConfigError::new(
            "edge_upload_body_limit_invalid",
            "edge.max_upload_body_bytes must be >= API limit and <= 536870912",
        ));
    }
    if !(100..=120_000).contains(&edge.request_timeout_ms) {
        return Err(ConfigError::new(
            "edge_request_timeout_invalid",
            "edge.request_timeout_ms must be between 100 and 120000",
        ));
    }

    Ok(())
}

fn validate_listener(listen: SocketAddr) -> Result<(), ConfigError> {
    if is_private_listener(listen.ip()) {
        Ok(())
    } else {
        Err(ConfigError::new(
            "public_listener_forbidden",
            format!(
                "refusing public/unspecified application listener {listen}; bind Chaptera to loopback or a private address behind the HTTPS edge"
            ),
        ))
    }
}

fn validate_origin(mode: EnvironmentMode, raw_origin: Option<&str>) -> Result<(), ConfigError> {
    let Some(raw_origin) = raw_origin else {
        return if mode == EnvironmentMode::Prod {
            Err(ConfigError::new(
                "public_origin_required",
                "prod public_origin is required",
            ))
        } else {
            Ok(())
        };
    };

    let url = Url::parse(raw_origin).map_err(|_| {
        ConfigError::new("public_origin_invalid", "public_origin is not a valid URL")
    })?;

    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(ConfigError::new(
            "public_origin_invalid",
            "public_origin must be an origin only, without credentials, path, query or fragment",
        ));
    }

    match mode {
        EnvironmentMode::Prod if url.scheme() != "https" => Err(ConfigError::new(
            "public_origin_https_required",
            "prod public_origin must use https",
        )),
        EnvironmentMode::Dev | EnvironmentMode::Test
            if url.scheme() != "http" && url.scheme() != "https" =>
        {
            Err(ConfigError::new(
                "public_origin_scheme_invalid",
                "dev/test public_origin must use http or https",
            ))
        }
        _ => Ok(()),
    }
}

fn validate_auth(mode: EnvironmentMode, auth: &AuthConfig) -> Result<(), ConfigError> {
    validate_positive_ttl(
        "auth.login_flow_ttl_seconds",
        auth.login_flow_ttl_seconds,
        "auth_login_flow_ttl_invalid",
    )?;
    validate_positive_ttl(
        "auth.session_idle_ttl_seconds",
        auth.session_idle_ttl_seconds,
        "auth_session_idle_ttl_invalid",
    )?;
    validate_positive_ttl(
        "auth.session_absolute_ttl_seconds",
        auth.session_absolute_ttl_seconds,
        "auth_session_absolute_ttl_invalid",
    )?;
    if auth.session_absolute_ttl_seconds < auth.session_idle_ttl_seconds {
        return Err(ConfigError::new(
            "auth_session_ttl_order_invalid",
            "auth.session_absolute_ttl_seconds must be >= auth.session_idle_ttl_seconds",
        ));
    }
    validate_oidc(mode, &auth.oidc)
}

fn validate_positive_ttl(field: &str, seconds: u64, code: &'static str) -> Result<(), ConfigError> {
    if seconds == 0 {
        return Err(ConfigError::new(code, format!("{field} must be positive")));
    }
    if seconds > (i64::MAX as u64) / 1000 {
        return Err(ConfigError::new(
            code,
            format!("{field} exceeds the supported millisecond timestamp range"),
        ));
    }
    Ok(())
}

fn validate_oidc(mode: EnvironmentMode, oidc: &OidcConfig) -> Result<(), ConfigError> {
    validate_nonempty("auth.oidc.client_id", &oidc.client_id)?;

    let issuer = Url::parse(&oidc.issuer)
        .map_err(|_| ConfigError::new("oidc_issuer_invalid", "OIDC issuer is not a valid URL"))?;
    if issuer.username() != ""
        || issuer.password().is_some()
        || issuer.query().is_some()
        || issuer.fragment().is_some()
    {
        return Err(ConfigError::new(
            "oidc_issuer_invalid",
            "OIDC issuer must not include credentials, query or fragment",
        ));
    }
    if mode == EnvironmentMode::Prod && issuer.scheme() != "https" {
        return Err(ConfigError::new(
            "oidc_issuer_https_required",
            "prod OIDC issuer must use https",
        ));
    }

    if !oidc.redirect_path.starts_with('/')
        || oidc.redirect_path.starts_with("//")
        || oidc.redirect_path.contains('?')
        || oidc.redirect_path.contains('#')
    {
        return Err(ConfigError::new(
            "oidc_redirect_path_invalid",
            "OIDC redirect_path must be one local absolute path",
        ));
    }

    validate_secret_ref(mode, &oidc.client_secret)
}

fn validate_key_ring(mode: EnvironmentMode, ring: &KeyRingConfig) -> Result<(), ConfigError> {
    validate_secret_name("key_ring.active", &ring.active)?;
    if ring.previous.len() > 3 {
        return Err(ConfigError::new(
            "key_ring_overlap_too_large",
            "key_ring.previous supports at most three overlap keys in V0",
        ));
    }

    let mut key_ids = BTreeSet::new();
    for key in &ring.keys {
        validate_secret_name("key_ring.keys.id", &key.id)?;
        if !key_ids.insert(key.id.as_str()) {
            return Err(ConfigError::new(
                "key_ring_duplicate_id",
                format!("duplicate key id {}", key.id),
            ));
        }
        validate_secret_ref(mode, &key.secret)?;
    }

    if !key_ids.contains(ring.active.as_str()) {
        return Err(ConfigError::new(
            "key_ring_active_missing",
            "key_ring.active must identify one configured key",
        ));
    }

    let mut previous = BTreeSet::new();
    for id in &ring.previous {
        if id == &ring.active {
            return Err(ConfigError::new(
                "key_ring_active_in_previous",
                "active key cannot also be a previous key",
            ));
        }
        if !previous.insert(id.as_str()) {
            return Err(ConfigError::new(
                "key_ring_duplicate_previous",
                "key_ring.previous contains a duplicate id",
            ));
        }
        if !key_ids.contains(id.as_str()) {
            return Err(ConfigError::new(
                "key_ring_previous_missing",
                format!("previous key id {id} is not configured"),
            ));
        }
    }

    Ok(())
}

fn validate_secret_ref(mode: EnvironmentMode, reference: &SecretRef) -> Result<(), ConfigError> {
    match reference {
        SecretRef::Env { name } => validate_secret_name("secret env name", name),
        SecretRef::File { path } => {
            if path.as_os_str().is_empty() {
                return Err(ConfigError::new(
                    "secret_path_empty",
                    "secret file path must be non-empty",
                ));
            }
            if mode == EnvironmentMode::Prod && !path.is_absolute() {
                return Err(ConfigError::new(
                    "secret_path_not_absolute",
                    "prod secret file path must be absolute",
                ));
            }
            Ok(())
        }
        SecretRef::Systemd { name } => validate_secret_name("systemd credential name", name),
    }
}

fn validate_nonempty(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        Err(ConfigError::new(
            "config_value_empty",
            format!("{field} must be non-empty"),
        ))
    } else {
        Ok(())
    }
}

fn validate_secret_name(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    {
        return Err(ConfigError::new(
            "secret_name_invalid",
            format!("{field} must contain only ASCII letters, digits, '.', '_' or '-'"),
        ));
    }
    Ok(())
}

fn is_private_listener(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || is_unique_local(ip),
    }
}

fn is_unique_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

#[derive(Clone)]
enum EnvironmentSource {
    Process,
    Fixed(BTreeMap<String, Vec<u8>>),
}

#[derive(Clone)]
pub struct SecretResolver {
    environment: EnvironmentSource,
    credentials_directory: Option<PathBuf>,
}

impl SecretResolver {
    pub fn from_process() -> Self {
        Self {
            environment: EnvironmentSource::Process,
            credentials_directory: env::var_os("CREDENTIALS_DIRECTORY").map(PathBuf::from),
        }
    }

    pub fn for_test(
        environment: BTreeMap<String, Vec<u8>>,
        credentials_directory: Option<PathBuf>,
    ) -> Self {
        Self {
            environment: EnvironmentSource::Fixed(environment),
            credentials_directory,
        }
    }

    pub fn resolve(
        &self,
        mode: EnvironmentMode,
        reference: &SecretRef,
    ) -> Result<SecretValue, ConfigError> {
        validate_secret_ref(mode, reference)?;

        match reference {
            SecretRef::Env { name } => self.resolve_environment_secret(name),
            SecretRef::File { path } => read_secret_file(mode, path),
            SecretRef::Systemd { name } => {
                let directory = self.credentials_directory.as_ref().ok_or_else(|| {
                    ConfigError::new(
                        "credentials_directory_missing",
                        "systemd secret source requires CREDENTIALS_DIRECTORY",
                    )
                })?;
                read_secret_file(mode, &directory.join(name))
            }
        }
    }

    fn resolve_environment_secret(&self, name: &str) -> Result<SecretValue, ConfigError> {
        let bytes = match &self.environment {
            EnvironmentSource::Process => env::var_os(name)
                .and_then(|value| value.into_string().ok())
                .map(String::into_bytes),
            EnvironmentSource::Fixed(environment) => environment.get(name).cloned(),
        }
        .ok_or_else(|| {
            ConfigError::new(
                "secret_env_missing",
                format!("required secret environment variable {name} is missing"),
            )
        })?;

        SecretValue::new(bytes)
    }

    pub fn resolve_key_ring(
        &self,
        mode: EnvironmentMode,
        ring: &KeyRingConfig,
    ) -> Result<ResolvedKeyRing, ConfigError> {
        validate_key_ring(mode, ring)?;

        let mut keys = BTreeMap::new();
        for key in &ring.keys {
            keys.insert(key.id.clone(), self.resolve(mode, &key.secret)?);
        }

        Ok(ResolvedKeyRing {
            active: ring.active.clone(),
            previous: ring.previous.clone(),
            keys,
        })
    }
}

fn read_secret_file(mode: EnvironmentMode, path: &Path) -> Result<SecretValue, ConfigError> {
    let metadata = fs::metadata(path).map_err(|error| {
        ConfigError::new(
            "secret_file_read_failed",
            format!("cannot stat secret file {}: {error}", path.display()),
        )
    })?;
    if metadata.len() > MAX_SECRET_BYTES as u64 {
        return Err(ConfigError::new(
            "secret_too_large",
            format!(
                "secret file {} exceeds {} bytes",
                path.display(),
                MAX_SECRET_BYTES
            ),
        ));
    }

    #[cfg(unix)]
    if mode == EnvironmentMode::Prod {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ConfigError::new(
                "secret_file_permissions_too_open",
                format!(
                    "prod secret file {} must not be group/world accessible",
                    path.display()
                ),
            ));
        }
    }

    let bytes = fs::read(path).map_err(|error| {
        ConfigError::new(
            "secret_file_read_failed",
            format!("cannot read secret file {}: {error}", path.display()),
        )
    })?;

    SecretValue::new(trim_one_line_ending(bytes))
}

fn trim_one_line_ending(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    bytes
}

pub struct SecretValue {
    bytes: Zeroizing<Vec<u8>>,
}

impl SecretValue {
    fn new(bytes: Vec<u8>) -> Result<Self, ConfigError> {
        if bytes.is_empty() {
            return Err(ConfigError::new("secret_empty", "required secret is empty"));
        }
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(ConfigError::new(
                "secret_too_large",
                format!("secret exceeds {MAX_SECRET_BYTES} bytes"),
            ));
        }

        Ok(Self {
            bytes: Zeroizing::new(bytes),
        })
    }

    pub fn expose(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue(<redacted>)")
    }
}

pub struct ResolvedKeyRing {
    pub active: String,
    pub previous: Vec<String>,
    pub keys: BTreeMap<String, SecretValue>,
}

impl fmt::Debug for ResolvedKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedKeyRing")
            .field("active", &self.active)
            .field("previous", &self.previous)
            .field("keys", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

pub struct ResolvedSecrets {
    pub oidc_client_secret: Option<SecretValue>,
    pub key_ring: Option<ResolvedKeyRing>,
}

impl fmt::Debug for ResolvedSecrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedSecrets")
            .field(
                "oidc_client_secret",
                &self.oidc_client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("key_ring", &self.key_ring)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub code: &'static str,
    pub message: String,
}

impl ConfigError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for ConfigError {}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_PATH: AtomicU64 = AtomicU64::new(1);

    fn temp_path(label: &str) -> PathBuf {
        let n = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "chaptera-config-{label}-{}-{n}",
            std::process::id()
        ))
    }

    fn prod_toml(secret_source: &str) -> String {
        let sqlite_path = if cfg!(windows) {
            "C:/chaptera/chaptera.sqlite"
        } else {
            "/var/lib/chaptera/chaptera.sqlite"
        };
        format!(
            r#"
environment = "prod"
listen = "127.0.0.1:8080"
public_origin = "https://cloud.example.invalid"

[sqlite]
path = "{sqlite_path}"
journal_mode = "wal"
synchronous = "full"
busy_timeout_ms = 5000
pool_max = 4

[worker]
heavy_concurrency = 1
light_concurrency = 2
quota_shared_capacity = 2
quota_semantic_headroom = 1
quota_export_cap = 1
quota_background_cap = 1

[storage]
provider = "s3-compatible"
quarantine_namespace = "chaptera-quarantine"
private_namespace = "chaptera-private"

[limits]
worker_spool_bytes = 4294967296
min_free_disk_bytes = 1073741824


[upload_admission]
principal_concurrent_cap = 2
tenant_concurrent_cap = 8
principal_bytes_cap = 536870912
tenant_bytes_cap = 2147483648
max_single_upload_bytes = 268435456
lease_seconds = 3600
retention_seconds = 604800

[source_validation]
clamd_endpoint = "127.0.0.1:3310"
clamd_connect_timeout_ms = 2000
clamd_io_timeout_ms = 5000
isolation_python = "/usr/bin/python3"
isolation_harness = "/opt/chaptera/current/tools/migration_pdf_worker_isolation.py"
worker_binary = "/opt/chaptera/current/chaptera-untrusted-pub-worker"
worker_wall_timeout_ms = 15000
worker_address_space_mb = 512
worker_cpu_seconds = 10
worker_open_files = 64
worker_output_file_mb = 32
max_file_bytes = 268435456
max_cfb_entries = 8192
max_declared_stream_bytes = 536870912
temp_root = "/var/lib/chaptera/source-scan-tmp"

[edge]
trusted_proxy_ips = ["127.0.0.1", "::1"]
max_header_bytes = 32768
max_api_body_bytes = 8388608
max_upload_body_bytes = 268435456
request_timeout_ms = 30000

[auth]
login_flow_ttl_seconds = 600
session_idle_ttl_seconds = 1800
session_absolute_ttl_seconds = 86400

[auth.oidc]
issuer = "https://id.example.invalid"
client_id = "chaptera-cloud"
redirect_path = "/v1/auth/callback"
client_secret = {secret_source}
"#
        )
    }

    #[test]
    fn parses_and_validates_production_shape() {
        let config: ChapteraConfig = toml::from_str(&prod_toml(
            r#"{ source = "systemd", name = "oidc_client_secret" }"#,
        ))
        .unwrap();

        config.validate().unwrap();
        assert_eq!(config.environment, EnvironmentMode::Prod);
        assert_eq!(config.runtime_config().listen, DEFAULT_LISTEN);
    }

    #[test]
    fn auth_ttls_are_explicit_positive_and_ordered() {
        let mut config: ChapteraConfig =
            toml::from_str(&prod_toml(r#"{ source = "env", name = "OIDC_SECRET" }"#)).unwrap();

        let auth = config.auth.as_mut().unwrap();
        auth.login_flow_ttl_seconds = 0;
        assert_eq!(
            config.validate().unwrap_err().code,
            "auth_login_flow_ttl_invalid"
        );

        let auth = config.auth.as_mut().unwrap();
        auth.login_flow_ttl_seconds = 600;
        auth.session_idle_ttl_seconds = 0;
        assert_eq!(
            config.validate().unwrap_err().code,
            "auth_session_idle_ttl_invalid"
        );

        let auth = config.auth.as_mut().unwrap();
        auth.session_idle_ttl_seconds = 1800;
        auth.session_absolute_ttl_seconds = 1799;
        assert_eq!(
            config.validate().unwrap_err().code,
            "auth_session_ttl_order_invalid"
        );

        let auth = config.auth.as_mut().unwrap();
        auth.session_absolute_ttl_seconds = (i64::MAX as u64) / 1000 + 1;
        assert_eq!(
            config.validate().unwrap_err().code,
            "auth_session_absolute_ttl_invalid"
        );
    }

    #[test]
    fn rejects_unknown_fields_and_insecure_prod_origin() {
        let unknown = format!(
            "{}\nunknown_field = true\n",
            prod_toml(r#"{ source = "env", name = "OIDC_SECRET" }"#)
        );
        assert!(toml::from_str::<ChapteraConfig>(&unknown).is_err());

        let insecure = prod_toml(r#"{ source = "env", name = "OIDC_SECRET" }"#).replace(
            r#"public_origin = "https://cloud.example.invalid""#,
            r#"public_origin = "http://cloud.example.invalid""#,
        );
        let config: ChapteraConfig = toml::from_str(&insecure).unwrap();
        assert_eq!(
            config.validate().unwrap_err().code,
            "public_origin_https_required"
        );
    }

    #[test]
    fn edge_policy_rejects_public_or_unbounded_proxy_configuration() {
        let mut config: ChapteraConfig =
            toml::from_str(&prod_toml(r#"{ source = "env", name = "OIDC_SECRET" }"#)).unwrap();

        config.edge.trusted_proxy_ips = vec!["8.8.8.8".parse().unwrap()];
        assert_eq!(
            config.validate().unwrap_err().code,
            "trusted_proxy_public_forbidden"
        );

        config.edge.trusted_proxy_ips = vec!["127.0.0.1".parse().unwrap()];
        config.edge.max_header_bytes = 1024 * 1024;
        assert_eq!(
            config.validate().unwrap_err().code,
            "edge_header_limit_invalid"
        );

        config.edge.max_header_bytes = 32 * 1024;
        config.edge.max_upload_body_bytes = 512;
        assert_eq!(
            config.validate().unwrap_err().code,
            "edge_upload_body_limit_invalid"
        );
    }

    #[test]
    fn resolves_env_file_and_systemd_secrets_without_debug_leak() {
        let mut environment = BTreeMap::new();
        environment.insert("OIDC_SECRET".to_owned(), b"env-secret".to_vec());

        let directory = temp_path("credentials");
        fs::create_dir_all(&directory).unwrap();
        let systemd_secret = directory.join("oidc_client_secret");
        fs::write(&systemd_secret, b"systemd-secret\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&systemd_secret, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let resolver = SecretResolver::for_test(environment, Some(directory.clone()));

        let env_secret = resolver
            .resolve(
                EnvironmentMode::Test,
                &SecretRef::Env {
                    name: "OIDC_SECRET".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(env_secret.expose(), b"env-secret");

        let systemd = resolver
            .resolve(
                EnvironmentMode::Prod,
                &SecretRef::Systemd {
                    name: "oidc_client_secret".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(systemd.expose(), b"systemd-secret");
        assert!(!format!("{systemd:?}").contains("systemd-secret"));

        let file = directory.join("file-secret");
        fs::write(&file, b"file-secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let direct = resolver
            .resolve(EnvironmentMode::Prod, &SecretRef::File { path: file })
            .unwrap();
        assert_eq!(direct.expose(), b"file-secret");

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn key_ring_requires_explicit_active_and_bounded_overlap() {
        let config: ChapteraConfig = toml::from_str(
            &(prod_toml(r#"{ source = "env", name = "OIDC_SECRET" }"#)
                + r#"
[key_ring]
active = "grant-2026-09"
previous = ["grant-2026-08"]

[[key_ring.keys]]
id = "grant-2026-09"
secret = { source = "env", name = "GRANT_KEY_NEW" }

[[key_ring.keys]]
id = "grant-2026-08"
secret = { source = "env", name = "GRANT_KEY_OLD" }
"#),
        )
        .unwrap();

        config.validate().unwrap();

        let mut environment = BTreeMap::new();
        environment.insert("OIDC_SECRET".to_owned(), b"oidc".to_vec());
        environment.insert("GRANT_KEY_NEW".to_owned(), b"new".to_vec());
        environment.insert("GRANT_KEY_OLD".to_owned(), b"old".to_vec());
        let resolved = config
            .resolve_required_secrets(&SecretResolver::for_test(environment, None))
            .unwrap();
        let ring = resolved.key_ring.unwrap();

        assert_eq!(ring.active, "grant-2026-09");
        assert_eq!(ring.previous, vec!["grant-2026-08"]);
        assert_eq!(ring.keys["grant-2026-09"].expose(), b"new");
        assert!(!format!("{ring:?}").contains("new"));
    }

    #[cfg(unix)]
    #[test]
    fn production_rejects_group_readable_secret_file() {
        use std::os::unix::fs::PermissionsExt;

        let file = temp_path("open-secret");
        fs::write(&file, b"secret").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();

        let resolver = SecretResolver::for_test(BTreeMap::new(), None);
        let error = resolver
            .resolve(
                EnvironmentMode::Prod,
                &SecretRef::File { path: file.clone() },
            )
            .unwrap_err();

        assert_eq!(error.code, "secret_file_permissions_too_open");
        let _ = fs::remove_file(file);
    }
}
