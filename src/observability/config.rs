use crate::*;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Clone, Serialize)]
pub(crate) struct ObservabilityConfig {
    pub(crate) enabled: bool,
    pub(crate) metrics_bind: SocketAddr,
    pub(crate) grafana_public_url: Option<Url>,
    #[serde(skip)]
    pub(crate) traces: super::traces::TraceConfig,
    #[serde(skip)]
    pub(crate) scrape_token: Option<Arc<str>>,
    #[serde(skip)]
    pub(crate) read_token: Option<Arc<str>>,
}

impl std::fmt::Debug for ObservabilityConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservabilityConfig")
            .field("enabled", &self.enabled)
            .field("metrics_bind", &self.metrics_bind)
            .field("grafana_public_url", &self.grafana_public_url)
            .finish_non_exhaustive()
    }
}
impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            metrics_bind: "127.0.0.1:9091".parse().expect("static bind"),
            grafana_public_url: None,
            traces: super::traces::TraceConfig::default(),
            scrape_token: None,
            read_token: None,
        }
    }
}
fn read_token(name: &str) -> Result<Option<Arc<str>>> {
    let Some(path) = env::var_os(name) else {
        return Ok(None);
    };
    read_token_path(name, std::path::Path::new(&path)).map(Some)
}

fn read_token_path(name: &str, path: &std::path::Path) -> Result<Arc<str>> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("cannot stat {name}"))?;
    if !metadata.file_type().is_file() {
        bail!("{name} must be a regular file");
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o137 != 0 {
        bail!("{name} permissions must allow owner access and optional group read only");
    }
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {name}"))?;
    if bytes.len() > 4096 {
        bail!("{name} exceeds token limit");
    }
    let value = std::str::from_utf8(&bytes)
        .context("token file is not UTF-8")?
        .trim();
    if value.len() < 16 || value.chars().any(char::is_whitespace) {
        bail!("{name} must contain a nonempty token of at least 16 characters");
    }
    Ok(Arc::from(value))
}
impl ObservabilityConfig {
    pub(crate) fn from_env() -> Result<Self> {
        for removed in ["PERFORMANCE_DATABASE_PATH", "PERFORMANCE_TELEMETRY_ENABLED"] {
            if env::var_os(removed).is_some() {
                bail!("{removed} was removed; migrate to external observability");
            }
        }
        let enabled = parse_bool_env_var("OBSERVABILITY_ENABLED", true)?;
        let metrics_bind: SocketAddr = env::var("METRICS_BIND")
            .unwrap_or_else(|_| "127.0.0.1:9091".into())
            .parse()
            .context("invalid METRICS_BIND")?;
        let grafana_public_url = env::var("GRAFANA_PUBLIC_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|v| Url::parse(&v))
            .transpose()
            .context("invalid GRAFANA_PUBLIC_URL")?;
        if let Some(url) = &grafana_public_url
            && (url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some())
        {
            bail!("GRAFANA_PUBLIC_URL must be a credential-free HTTPS URL");
        }
        let scrape_token = read_token("METRICS_TOKEN_FILE")?;
        let read_token = read_token("OBSERVABILITY_READ_TOKEN_FILE")?;
        let result = Self {
            enabled,
            metrics_bind,
            grafana_public_url,
            traces: super::traces::TraceConfig::from_env(enabled),
            scrape_token,
            read_token,
        };
        if result.enabled
            && !result.metrics_bind.ip().is_loopback()
            && result.scrape_token.is_none()
        {
            bail!("non-loopback METRICS_BIND requires METRICS_TOKEN_FILE");
        }
        if result.scrape_token.as_deref().is_some() && result.scrape_token == result.read_token {
            bail!("scrape and read tokens must differ");
        }
        Ok(result)
    }
}
pub(super) fn authorized(headers: &HeaderMap, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return false;
    };
    let Some(provided) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    // HMAC verify avoids a token-prefix timing oracle without printing credentials.
    use hmac::{Hmac, Mac};
    let Ok(mut signer) = Hmac::<Sha256>::new_from_slice(expected.as_bytes()) else {
        return false;
    };
    signer.update(b"cvm-observability-token");
    let signature = signer.finalize().into_bytes();
    let Ok(mut verifier) = Hmac::<Sha256>::new_from_slice(provided.as_bytes()) else {
        return false;
    };
    verifier.update(b"cvm-observability-token");
    verifier.verify_slice(&signature).is_ok()
}

/// Called only by the synchronous executable entrypoint, before any worker is started.
/// hotpath reads startup settings lazily; preparing them after Tokio starts is unsound.
pub(crate) fn prepare_hotpath(config: &ObservabilityConfig) {
    super::PROFILER_ENABLED.store(
        config.enabled && cfg!(feature = "hotpath"),
        Ordering::Relaxed,
    );
    for (name, value) in hotpath_settings(config) {
        // SAFETY: the executable calls this before constructing Tokio or a profiler guard.
        unsafe {
            env::set_var(name, value);
        }
    }
}

fn hotpath_settings(config: &ObservabilityConfig) -> [(&'static str, String); 14] {
    [
        ("HOTPATH_ENTRIES_LIMIT", "100".to_string()),
        ("HOTPATH_LOGS_LIMIT", "1".to_string()),
        // Batch delivery to the SDK workers; event timestamps and sampling are
        // unchanged. Avoid sweeping mostly empty queues 20 times per second.
        ("HOTPATH_DRAIN_INTERVAL", "250".to_string()),
        // The SDK adds 19 ASCII bytes to truncated labels. 120 Unicode scalars
        // plus that suffix fit the Prometheus 512-byte label limit even at 4 B/char.
        ("HOTPATH_MAX_LOG_LEN", "120".to_string()),
        ("HOTPATH_SQL_RAW_LOGS", "false".to_string()),
        ("HOTPATH_METRICS_PORT", "6770".to_string()),
        ("HOTPATH_METRICS_SERVER_OFF", (!config.enabled).to_string()),
        (
            "HOTPATH_PROMETHEUS_HOST",
            config.metrics_bind.ip().to_string(),
        ),
        ("HOTPATH_PROMETHEUS_PORT", "6772".to_string()),
        ("HOTPATH_FUNCTIONS_TIME_SAMPLING_RATE", "0.1".to_string()),
        ("HOTPATH_MUTEXES_TIME_SAMPLING_RATE", "1.0".to_string()),
        ("HOTPATH_RW_LOCKS_TIME_SAMPLING_RATE", "1.0".to_string()),
        ("HOTPATH_METRICS_AUTH_TOKEN", nanoid::nanoid!(48)),
        (
            "HOTPATH_PROMETHEUS_AUTH_TOKEN",
            config.scrape_token.as_deref().unwrap_or("").to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotpath_sql_label_budget_fits_prometheus_with_utf8_and_hash_suffix() {
        let settings = hotpath_settings(&ObservabilityConfig::default());
        let query_chars: usize = settings
            .iter()
            .find(|(name, _)| *name == "HOTPATH_MAX_LOG_LEN")
            .expect("startup must bound SDK query labels")
            .1
            .parse()
            .unwrap();
        let hotpath_job = include_str!("../../ops/observability/prometheus.yml")
            .split_once("- job_name: cvm-hotpath")
            .unwrap()
            .1;
        let label_bytes: usize = hotpath_job
            .lines()
            .find_map(|line| line.trim().strip_prefix("label_value_length_limit:"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // hotpath 0.28 appends three dots and the 16-digit normalized-query hash.
        let suffix_bytes = "...0000000000000000".len();
        assert!(query_chars * char::MAX.len_utf8() + suffix_bytes <= label_bytes);
    }

    #[test]
    fn credentials_are_separate_and_never_serialized() {
        let config = ObservabilityConfig {
            scrape_token: Some(Arc::from("scrape-private-secret")),
            read_token: Some(Arc::from("read-private-secret")),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("secret"));
        assert!(!format!("{config:?}").contains("secret"));
        let mut headers = HeaderMap::new();
        assert!(!authorized(&headers, config.read_token.as_deref()));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer scrape-private-secret"),
        );
        assert!(!authorized(&headers, config.read_token.as_deref()));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer read-private-secret"),
        );
        assert!(authorized(&headers, config.read_token.as_deref()));
        assert!(!authorized(&headers, None));
    }

    #[cfg(unix)]
    #[test]
    fn token_files_require_private_regular_files() {
        let directory = std::env::temp_dir().join(format!(
            "cvm-observe-token-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let token = directory.join("token");
        std::fs::write(&token, "private-observe-token\n").unwrap();

        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            read_token_path("TEST_TOKEN", &token).unwrap().as_ref(),
            "private-observe-token"
        );

        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = read_token_path("TEST_TOKEN", &token)
            .unwrap_err()
            .to_string();
        assert!(error.contains("permissions"));

        let link = directory.join("token-link");
        std::os::unix::fs::symlink(&token, &link).unwrap();
        let error = read_token_path("TEST_TOKEN", &link)
            .unwrap_err()
            .to_string();
        assert!(error.contains("regular file"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
