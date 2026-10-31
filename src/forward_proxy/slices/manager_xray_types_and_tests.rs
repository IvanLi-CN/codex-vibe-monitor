use super::*;

#[derive(Debug, Clone)]
pub(crate) struct SelectedForwardProxy {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<Url>,
    pub(crate) endpoint_url_raw: Option<String>,
    pub(crate) egress_ip: Option<String>,
}

impl SelectedForwardProxy {
    pub(crate) fn from_endpoint(endpoint: &ForwardProxyEndpoint) -> Self {
        Self {
            key: endpoint.key.clone(),
            source: endpoint.source.clone(),
            display_name: endpoint.display_name.clone(),
            endpoint_url: endpoint.endpoint_url.clone(),
            endpoint_url_raw: endpoint.raw_url.clone(),
            egress_ip: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct XrayInstance {
    pub(crate) local_proxy_url: Url,
    pub(crate) config_path: PathBuf,
    pub(crate) child: Child,
    pub(crate) temporary: bool,
}

const XRAY_PREFLIGHT_DIAGNOSTIC_BYTES: usize = 4096;

struct XrayPreflightResult {
    status: std::process::ExitStatus,
    diagnostic: Vec<u8>,
}

#[derive(Debug, Default)]
pub(crate) struct XraySupervisor {
    pub(crate) binary: String,
    pub(crate) runtime_dir: PathBuf,
    pub(crate) instances: HashMap<String, XrayInstance>,
}

impl XraySupervisor {
    pub(crate) fn new(binary: String, runtime_dir: PathBuf) -> Self {
        Self {
            binary,
            runtime_dir,
            instances: HashMap::new(),
        }
    }

    pub(crate) async fn sync_endpoints(
        &mut self,
        endpoints: &mut [ForwardProxyEndpoint],
        shutdown: &CancellationToken,
    ) -> Result<()> {
        fs::create_dir_all(&self.runtime_dir).with_context(|| {
            format!(
                "failed to create xray runtime directory: {}",
                self.runtime_dir.display()
            )
        })?;

        let desired_keys = endpoints
            .iter()
            .filter(|endpoint| endpoint.requires_xray())
            .map(|endpoint| endpoint.key.clone())
            .collect::<HashSet<_>>();
        let stale_keys = self
            .instances
            .keys()
            .filter(|key| {
                !desired_keys.contains(*key)
                    && self
                        .instances
                        .get(*key)
                        .is_some_and(|instance| !instance.temporary)
            })
            .cloned()
            .collect::<Vec<_>>();

        for endpoint in endpoints {
            if shutdown.is_cancelled() {
                info!("stopping xray route sync because shutdown is in progress");
                bail!("xray route sync cancelled because shutdown is in progress");
            }
            if !endpoint.requires_xray() {
                continue;
            }
            match self.ensure_instance(endpoint, shutdown).await {
                Ok(route_url) => endpoint.endpoint_url = Some(route_url),
                Err(err) => {
                    endpoint.endpoint_url = None;
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        proxy_source = endpoint.source,
                        proxy_label_ref = %forward_proxy_log_ref(&endpoint.display_name),
                        proxy_url_ref = %forward_proxy_log_ref_option(endpoint.raw_url.as_deref()),
                        error = %err,
                        "failed to prepare xray forward proxy route"
                    );
                }
            }
        }

        if shutdown.is_cancelled() {
            info!("skipping stale xray route cleanup because shutdown is in progress");
            bail!("xray route sync cancelled because shutdown is in progress");
        }
        for key in stale_keys {
            if shutdown.is_cancelled() {
                info!("skipping stale xray route cleanup because shutdown is in progress");
                bail!("xray route sync cancelled because shutdown is in progress");
            }
            self.remove_instance(&key).await;
        }

        Ok(())
    }

    pub(crate) async fn shutdown_all(&mut self) {
        let keys = self.instances.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.remove_instance(&key).await;
        }
    }

    pub(crate) async fn ensure_instance(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        shutdown: &CancellationToken,
    ) -> Result<Url> {
        self.ensure_instance_with_ready_timeout(
            endpoint,
            Duration::from_millis(XRAY_PROXY_READY_TIMEOUT_MS),
            shutdown,
        )
        .await
    }

    pub(crate) async fn ensure_instance_with_ready_timeout(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
        shutdown: &CancellationToken,
    ) -> Result<Url> {
        self.ensure_instance_with_ready_timeout_kind(endpoint, ready_timeout, shutdown, false)
            .await
    }

    pub(crate) async fn ensure_temporary_instance_with_ready_timeout(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
        shutdown: &CancellationToken,
    ) -> Result<Url> {
        self.ensure_instance_with_ready_timeout_kind(endpoint, ready_timeout, shutdown, true)
            .await
    }

    async fn ensure_instance_with_ready_timeout_kind(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
        shutdown: &CancellationToken,
        temporary: bool,
    ) -> Result<Url> {
        if let Some(instance) = self.instances.get_mut(&endpoint.key) {
            match instance.child.try_wait() {
                Ok(None) => return Ok(instance.local_proxy_url.clone()),
                Ok(Some(status)) => {
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        status = %status,
                        "xray proxy process exited unexpectedly; restarting"
                    );
                }
                Err(err) => {
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        error = %err,
                        "failed to inspect xray proxy process; restarting"
                    );
                }
            }
        }

        self.remove_instance(&endpoint.key).await;
        self.spawn_instance(endpoint, ready_timeout, shutdown, temporary)
            .await
    }

    pub(crate) async fn spawn_instance(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
        shutdown: &CancellationToken,
        temporary: bool,
    ) -> Result<Url> {
        let outbound = build_xray_outbound_for_endpoint(endpoint).map_err(|err| {
            wrap_xray_error(
                err,
                ForwardProxyValidationErrorCode::XrayConfigError,
                "failed to build xray proxy configuration",
            )
        })?;
        let local_port = pick_unused_local_port().map_err(|err| {
            wrap_xray_error(
                err,
                ForwardProxyValidationErrorCode::XrayConfigError,
                "failed to allocate xray local proxy port",
            )
        })?;
        fs::create_dir_all(&self.runtime_dir).map_err(|err| {
            wrap_xray_error(
                anyhow!(err),
                ForwardProxyValidationErrorCode::XrayConfigError,
                "failed to prepare xray runtime directory",
            )
        })?;
        set_xray_runtime_permissions(&self.runtime_dir, 0o700).map_err(|err| {
            wrap_xray_error(
                anyhow!(err),
                ForwardProxyValidationErrorCode::XrayConfigError,
                "failed to protect xray runtime directory",
            )
        })?;
        let config_path = self.runtime_dir.join(format!(
            "forward-proxy-{:016x}.json",
            stable_hash_u64(&endpoint.key)
        ));
        let startup_started = Instant::now();
        let config = build_xray_instance_config(local_port, outbound);
        let serialized = serde_json::to_vec_pretty(&config).map_err(|err| {
            wrap_xray_error(
                anyhow!(err),
                ForwardProxyValidationErrorCode::XrayConfigError,
                "failed to serialize xray configuration",
            )
        })?;
        if let Err(err) = write_xray_config_file(&config_path, &serialized) {
            let _ = fs::remove_file(&config_path);
            return Err(wrap_xray_error(
                anyhow!(err),
                ForwardProxyValidationErrorCode::XrayConfigError,
                "failed to write xray configuration",
            ));
        }

        // Ask Xray to validate the generated file before starting the long-lived route. A
        // missing or unlaunchable binary is still a startup failure; a launched binary that
        // rejects the configuration is a configuration failure. The preflight is bounded by
        // the same deadline as route startup so a broken binary cannot hold validation open.
        let preflight_timeout = match ready_timeout.checked_sub(startup_started.elapsed()) {
            Some(remaining) if !remaining.is_zero() => remaining,
            _ => {
                let _ = fs::remove_file(&config_path);
                return Err(anyhow!(ForwardProxyValidationError::new(
                    ForwardProxyValidationErrorCode::ProxyTimeout,
                    "xray proxy route startup timed out",
                )));
            }
        };
        let preflight = match run_xray_config_preflight(
            &self.binary,
            &config_path,
            preflight_timeout,
            shutdown,
        )
        .await
        {
            Ok(preflight) => preflight,
            Err(err) => {
                let _ = fs::remove_file(&config_path);
                return Err(err);
            }
        };
        if !preflight.status.success() {
            warn!(
                proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                status = %preflight.status,
                diagnostic = summarize_xray_diagnostic(&preflight.diagnostic),
                "xray rejected proxy configuration during preflight"
            );
            let _ = fs::remove_file(&config_path);
            return Err(anyhow!(ForwardProxyValidationError::new(
                ForwardProxyValidationErrorCode::XrayConfigError,
                "xray rejected proxy configuration",
            )));
        }
        let remaining_ready_timeout = match ready_timeout.checked_sub(startup_started.elapsed()) {
            Some(remaining) if !remaining.is_zero() => remaining,
            _ => {
                let _ = fs::remove_file(&config_path);
                return Err(anyhow!(ForwardProxyValidationError::new(
                    ForwardProxyValidationErrorCode::ProxyTimeout,
                    "xray proxy route startup timed out",
                )));
            }
        };

        let mut child = match Command::new(&self.binary)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(_) => {
                let _ = fs::remove_file(&config_path);
                return Err(anyhow!(ForwardProxyValidationError::new(
                    ForwardProxyValidationErrorCode::XrayStartupError,
                    "failed to start xray binary",
                )));
            }
        };
        if let Err(err) =
            wait_for_xray_proxy_ready(&mut child, local_port, remaining_ready_timeout, shutdown)
                .await
        {
            let _ = terminate_child_process(
                &mut child,
                Duration::from_secs(2),
                &forward_proxy_log_ref(&endpoint.key),
            )
            .await;
            let _ = fs::remove_file(&config_path);
            return Err(wrap_xray_error(
                err,
                ForwardProxyValidationErrorCode::XrayStartupError,
                "xray process failed to become ready",
            ));
        }

        let local_proxy_url = match Url::parse(&format!("socks5h://127.0.0.1:{local_port}")) {
            Ok(url) => url,
            Err(err) => {
                let proxy_key_ref = forward_proxy_log_ref(&endpoint.key);
                let _ = terminate_child_process(&mut child, Duration::from_secs(2), &proxy_key_ref)
                    .await;
                let _ = fs::remove_file(&config_path);
                return Err(wrap_xray_error(
                    anyhow!(err),
                    ForwardProxyValidationErrorCode::XrayStartupError,
                    "failed to build local xray proxy endpoint",
                ));
            }
        };
        self.instances.insert(
            endpoint.key.clone(),
            XrayInstance {
                local_proxy_url: local_proxy_url.clone(),
                config_path,
                child,
                temporary,
            },
        );

        Ok(local_proxy_url)
    }

    pub(crate) async fn remove_instance(&mut self, key: &str) {
        if let Some(mut instance) = self.instances.remove(key) {
            let proxy_key_ref = forward_proxy_log_ref(key);
            let _ = terminate_child_process(
                &mut instance.child,
                if instance.temporary {
                    Duration::ZERO
                } else {
                    Duration::from_secs(2)
                },
                &proxy_key_ref,
            )
            .await;
            if let Err(err) = fs::remove_file(&instance.config_path)
                && err.kind() != io::ErrorKind::NotFound
            {
                warn!(
                    proxy_key_ref = %proxy_key_ref,
                    path = %instance.config_path.display(),
                    error = %err,
                    "failed to remove xray config file"
                );
            }
        }
    }
}

async fn run_xray_config_preflight(
    binary: &str,
    config_path: &Path,
    timeout_duration: Duration,
    shutdown: &CancellationToken,
) -> Result<XrayPreflightResult> {
    let mut child = Command::new(binary)
        .arg("run")
        .arg("-test")
        .arg("-c")
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| {
            anyhow!(ForwardProxyValidationError::new(
                ForwardProxyValidationErrorCode::XrayStartupError,
                "failed to start xray binary",
            ))
        })?;
    let mut diagnostic_task = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(read_bounded_xray_diagnostic(stderr)));

    let wait_result = tokio::select! {
        _ = shutdown.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            if let Some(task) = diagnostic_task.take() {
                task.abort();
            }
            return Err(anyhow!("xray configuration preflight cancelled because shutdown is in progress"));
        }
        result = tokio::time::timeout(timeout_duration, child.wait()) => result,
    };
    let status = match wait_result {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => {
            if let Some(task) = diagnostic_task.take() {
                task.abort();
            }
            return Err(anyhow!(ForwardProxyValidationError::new(
                ForwardProxyValidationErrorCode::XrayStartupError,
                "failed to wait for xray configuration preflight",
            )));
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            if let Some(task) = diagnostic_task.take() {
                task.abort();
            }
            return Err(anyhow!(ForwardProxyValidationError::new(
                ForwardProxyValidationErrorCode::ProxyTimeout,
                "xray configuration preflight timed out",
            )));
        }
    };
    let diagnostic = match diagnostic_task.take() {
        Some(task) => task.await.unwrap_or_default(),
        None => Vec::new(),
    };
    Ok(XrayPreflightResult { status, diagnostic })
}

async fn read_bounded_xray_diagnostic(mut stderr: tokio::process::ChildStderr) -> Vec<u8> {
    use tokio::io::AsyncReadExt;

    let mut retained = Vec::with_capacity(XRAY_PREFLIGHT_DIAGNOSTIC_BYTES);
    let mut buffer = [0u8; 1024];
    while let Ok(read) = stderr.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        if retained.len() < XRAY_PREFLIGHT_DIAGNOSTIC_BYTES {
            let remaining = XRAY_PREFLIGHT_DIAGNOSTIC_BYTES - retained.len();
            retained.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    retained
}

fn summarize_xray_diagnostic(diagnostic: &[u8]) -> &'static str {
    let lower = String::from_utf8_lossy(diagnostic).to_ascii_lowercase();
    if lower.contains("unknown field") || lower.contains("unknown key") {
        "xray rejected an unknown configuration field"
    } else if lower.contains("invalid") || lower.contains("parse") {
        "xray rejected an invalid configuration value"
    } else if diagnostic.is_empty() {
        "xray rejected proxy configuration without diagnostics"
    } else {
        "xray rejected proxy configuration"
    }
}

#[cfg(unix)]
fn set_xray_runtime_permissions(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_xray_runtime_permissions(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

fn write_xray_config_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

pub(crate) fn stable_hash_u64(raw: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn forward_proxy_log_ref(raw: &str) -> String {
    format!("fp_{:016x}", stable_hash_u64(raw))
}

pub(crate) fn forward_proxy_log_ref_option(raw: Option<&str>) -> String {
    raw.map(forward_proxy_log_ref)
        .unwrap_or_else(|| "direct".to_string())
}

fn wrap_xray_error(
    err: anyhow::Error,
    code: ForwardProxyValidationErrorCode,
    message: &'static str,
) -> anyhow::Error {
    if err.downcast_ref::<ForwardProxyValidationError>().is_some() {
        err
    } else {
        anyhow!(ForwardProxyValidationError::new(code, message))
    }
}

pub(crate) fn pick_unused_local_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("failed to bind local socket for port allocation")?;
    let port = listener
        .local_addr()
        .context("failed to read local address for allocated port")?
        .port();
    Ok(port)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildTerminationOutcome {
    AlreadyExited,
    Graceful,
    Forced,
}

pub(crate) async fn terminate_child_process(
    child: &mut Child,
    grace_period: Duration,
    process_ref: &str,
) -> ChildTerminationOutcome {
    match child.try_wait() {
        Ok(Some(status)) => {
            info!(process_ref, status = %status, "child process already exited before shutdown");
            return ChildTerminationOutcome::AlreadyExited;
        }
        Ok(None) => {}
        Err(err) => {
            warn!(process_ref, error = %err, "failed to poll child process before shutdown");
        }
    }

    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            if result == 0 {
                info!(
                    process_ref,
                    pid,
                    grace_ms = grace_period.as_millis() as u64,
                    "sent SIGTERM to child process"
                );
                if grace_period.is_zero() {
                    warn!(
                        process_ref,
                        pid,
                        "grace period is zero; falling back to force kill immediately after SIGTERM"
                    );
                } else {
                    match timeout(grace_period, child.wait()).await {
                        Ok(Ok(status)) => {
                            info!(process_ref, pid, status = %status, "child process exited after SIGTERM");
                            return ChildTerminationOutcome::Graceful;
                        }
                        Ok(Err(err)) => {
                            warn!(process_ref, pid, error = %err, "failed while waiting for child process after SIGTERM");
                        }
                        Err(_) => {
                            warn!(
                                process_ref,
                                pid,
                                grace_ms = grace_period.as_millis() as u64,
                                "child process did not exit after SIGTERM; falling back to force kill"
                            );
                        }
                    }
                }
            } else {
                let err = io::Error::last_os_error();
                warn!(process_ref, pid, error = %err, "failed to send SIGTERM to child process; falling back to force kill");
            }
        }
    }

    if let Err(err) = child.kill().await {
        warn!(process_ref, error = %err, "failed to force kill child process");
    } else {
        info!(
            process_ref,
            grace_ms = grace_period.as_millis() as u64,
            "force killed child process after graceful shutdown fallback"
        );
    }

    match timeout(grace_period, child.wait()).await {
        Ok(Ok(status)) => {
            info!(process_ref, status = %status, "child process exited after force kill");
        }
        Ok(Err(err)) => {
            warn!(process_ref, error = %err, "failed while waiting for force killed child process");
        }
        Err(_) => {
            warn!(
                process_ref,
                grace_ms = grace_period.as_millis() as u64,
                "timed out waiting for force killed child process exit"
            );
        }
    }

    ChildTerminationOutcome::Forced
}

pub(crate) async fn wait_for_xray_proxy_ready(
    child: &mut Child,
    local_port: u16,
    ready_timeout: Duration,
    shutdown: &CancellationToken,
) -> Result<()> {
    let deadline = Instant::now() + ready_timeout;
    loop {
        if shutdown.is_cancelled() {
            bail!("xray startup cancelled because shutdown is in progress");
        }
        if let Some(status) = child
            .try_wait()
            .context("failed to poll xray proxy process status")?
        {
            bail!("xray process exited before ready: {status}");
        }
        let connect_attempt = timeout(
            Duration::from_millis(250),
            TcpStream::connect(("127.0.0.1", local_port)),
        );
        tokio::select! {
            _ = shutdown.cancelled() => {
                bail!("xray startup cancelled because shutdown is in progress");
            }
            result = connect_attempt => {
                if result.is_ok_and(|connection| connection.is_ok()) {
                    return Ok(());
                }
            }
        }
        if Instant::now() >= deadline {
            bail!("xray local socks endpoint was not ready in time");
        }
        tokio::select! {
            _ = shutdown.cancelled() => {
                bail!("xray startup cancelled because shutdown is in progress");
            }
            _ = sleep(Duration::from_millis(100)) => {}
        }
    }
}

pub(crate) fn build_xray_instance_config(local_port: u16, outbound: Value) -> Value {
    json!({
        "log": {
            "loglevel": "warning"
        },
        "inbounds": [
            {
                "tag": "inbound-local-socks",
                "listen": "127.0.0.1",
                "port": local_port,
                "protocol": "socks",
                "settings": {
                    "auth": "noauth",
                    "udp": false
                }
            }
        ],
        "outbounds": [
            outbound,
            {
                "tag": "direct",
                "protocol": "freedom"
            }
        ],
        "routing": {
            "domainStrategy": "AsIs",
            "rules": [
                {
                    "type": "field",
                    "inboundTag": ["inbound-local-socks"],
                    "outboundTag": "proxy"
                }
            ]
        }
    })
}

pub(crate) fn build_xray_outbound_for_endpoint(endpoint: &ForwardProxyEndpoint) -> Result<Value> {
    let raw = endpoint
        .raw_url
        .as_deref()
        .ok_or_else(|| anyhow!("xray endpoint missing share link url"))?;
    match endpoint.protocol {
        ForwardProxyProtocol::Vmess => build_vmess_xray_outbound(raw),
        ForwardProxyProtocol::Vless => build_vless_xray_outbound(raw),
        ForwardProxyProtocol::Trojan => build_trojan_xray_outbound(raw),
        ForwardProxyProtocol::Shadowsocks => build_shadowsocks_xray_outbound(raw),
        _ => bail!("unsupported xray protocol for endpoint"),
    }
}

pub(crate) fn build_vmess_xray_outbound(raw: &str) -> Result<Value> {
    let link = parse_vmess_share_link(raw)?;
    if matches!(link.network.as_str(), "xhttp" | "splithttp") {
        return Err(anyhow!(ForwardProxyValidationError::new(
            ForwardProxyValidationErrorCode::UnsupportedTransport,
            "xhttp transport is only supported for vless nodes",
        )));
    }
    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "vmess",
        "settings": {
            "vnext": [
                {
                    "address": link.address,
                    "port": link.port,
                    "users": [
                        {
                            "id": link.id,
                            "alterId": link.alter_id,
                            "security": link.security
                        }
                    ]
                }
            ]
        }
    });
    if let Some(stream_settings) = build_vmess_stream_settings(&link)
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

pub(crate) fn build_vmess_stream_settings(link: &VmessShareLink) -> Option<Value> {
    let mut stream = serde_json::Map::new();
    stream.insert("network".to_string(), Value::String(link.network.clone()));
    let mut has_non_default_options = link.network != "tcp";

    let security = link
        .tls_mode
        .as_deref()
        .filter(|value| !value.is_empty() && *value != "none")
        .map(|value| value.to_ascii_lowercase());
    if let Some(security) = security.as_ref() {
        stream.insert("security".to_string(), Value::String(security.clone()));
        has_non_default_options = true;
    }

    match link.network.as_str() {
        "ws" => {
            let mut ws = serde_json::Map::new();
            if let Some(path) = link.path.as_ref().filter(|value| !value.trim().is_empty()) {
                ws.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(host) = link.host.as_ref().filter(|value| !value.trim().is_empty()) {
                ws.insert("headers".to_string(), json!({ "Host": host }));
            }
            if !ws.is_empty() {
                stream.insert("wsSettings".to_string(), Value::Object(ws));
                has_non_default_options = true;
            }
        }
        "grpc" => {
            let service_name = link
                .path
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .unwrap_or_default();
            stream.insert(
                "grpcSettings".to_string(),
                json!({
                    "serviceName": service_name
                }),
            );
            has_non_default_options = true;
        }
        "httpupgrade" => {
            let mut settings = serde_json::Map::new();
            if let Some(host) = link.host.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("host".to_string(), Value::String(host.clone()));
            }
            if let Some(path) = link.path.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("path".to_string(), Value::String(path.clone()));
            }
            if !settings.is_empty() {
                stream.insert("httpupgradeSettings".to_string(), Value::Object(settings));
                has_non_default_options = true;
            }
        }
        _ => {}
    }

    if let Some(security) = security {
        if security == "tls" {
            let mut tls_settings = serde_json::Map::new();
            if let Some(server_name) = link
                .sni
                .as_ref()
                .or(link.host.as_ref())
                .filter(|value| !value.trim().is_empty())
            {
                tls_settings.insert("serverName".to_string(), Value::String(server_name.clone()));
            }
            if let Some(alpn) = link.alpn.as_ref().filter(|items| !items.is_empty()) {
                tls_settings.insert("alpn".to_string(), json!(alpn));
            }
            if let Some(fingerprint) = link
                .fingerprint
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                tls_settings.insert(
                    "fingerprint".to_string(),
                    Value::String(fingerprint.clone()),
                );
            }
            if !tls_settings.is_empty() {
                stream.insert("tlsSettings".to_string(), Value::Object(tls_settings));
                has_non_default_options = true;
            }
        } else if security == "reality" {
            let mut reality_settings = serde_json::Map::new();
            if let Some(server_name) = link
                .sni
                .as_ref()
                .or(link.host.as_ref())
                .filter(|value| !value.trim().is_empty())
            {
                reality_settings
                    .insert("serverName".to_string(), Value::String(server_name.clone()));
            }
            if let Some(fingerprint) = link
                .fingerprint
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                reality_settings.insert(
                    "fingerprint".to_string(),
                    Value::String(fingerprint.clone()),
                );
            }
            if !reality_settings.is_empty() {
                stream.insert(
                    "realitySettings".to_string(),
                    Value::Object(reality_settings),
                );
                has_non_default_options = true;
            }
        }
    }

    if has_non_default_options {
        Some(Value::Object(stream))
    } else {
        None
    }
}

pub(crate) fn build_vless_xray_outbound(raw: &str) -> Result<Value> {
    let url = Url::parse(raw).context("invalid vless share link")?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("vless host missing"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("vless port missing"))?;
    let user_id = url.username();
    if user_id.trim().is_empty() {
        bail!("vless id missing");
    }

    let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
    let encryption = query
        .get("encryption")
        .cloned()
        .unwrap_or_else(|| "none".to_string());
    let mut user = serde_json::Map::new();
    user.insert("id".to_string(), Value::String(user_id.to_string()));
    user.insert("encryption".to_string(), Value::String(encryption));
    if let Some(flow) = query.get("flow").filter(|value| !value.trim().is_empty()) {
        user.insert("flow".to_string(), Value::String(flow.clone()));
    }

    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "vless",
        "settings": {
            "vnext": [
                {
                    "address": host,
                    "port": port,
                    "users": [Value::Object(user)]
                }
            ]
        }
    });
    if let Some(stream_settings) = build_stream_settings_from_url_with_xhttp(&url, None, true)?
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

pub(crate) fn build_trojan_xray_outbound(raw: &str) -> Result<Value> {
    let url = Url::parse(raw).context("invalid trojan share link")?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("trojan host missing"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("trojan port missing"))?;
    let password = url.username();
    if password.trim().is_empty() {
        bail!("trojan password missing");
    }

    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "trojan",
        "settings": {
            "servers": [
                {
                    "address": host,
                    "port": port,
                    "password": password
                }
            ]
        }
    });
    if let Some(stream_settings) =
        build_stream_settings_from_url_with_xhttp(&url, Some("tls"), false)?
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

pub(crate) fn build_shadowsocks_xray_outbound(raw: &str) -> Result<Value> {
    let parsed = parse_shadowsocks_share_link(raw)?;
    Ok(json!({
        "tag": "proxy",
        "protocol": "shadowsocks",
        "settings": {
            "servers": [
                {
                    "address": parsed.host,
                    "port": parsed.port,
                    "method": parsed.method,
                    "password": parsed.password
                }
            ]
        }
    }))
}

pub(crate) fn build_stream_settings_from_url(
    url: &Url,
    default_security: Option<&str>,
) -> Result<Option<Value>> {
    build_stream_settings_from_url_with_xhttp(url, default_security, false)
}

pub(crate) fn build_stream_settings_from_url_with_xhttp(
    url: &Url,
    default_security: Option<&str>,
    allow_xhttp: bool,
) -> Result<Option<Value>> {
    let query_pairs = url.query_pairs().into_owned().collect::<Vec<_>>();
    if xhttp_query_has_ambiguous_duplicates(&query_pairs) {
        return Err(anyhow!(ForwardProxyValidationError::new(
            ForwardProxyValidationErrorCode::InvalidXhttpExtra,
            "xhttp query parameters must be unique and non-conflicting",
        )));
    }
    let query = query_pairs.into_iter().collect::<HashMap<_, _>>();
    let network = query
        .get("type")
        .or_else(|| query.get("net"))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "tcp".to_string());
    let security = query
        .get("security")
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .or_else(|| default_security.map(str::to_string))
        .unwrap_or_else(|| "none".to_string());

    if network == "splithttp" {
        return Err(anyhow!(ForwardProxyValidationError::new(
            ForwardProxyValidationErrorCode::UnsupportedTransport,
            "splithttp transport is unsupported; use vless xhttp instead",
        )));
    }

    let host = query
        .get("host")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let path = query
        .get("path")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let xhttp_host = query.get("host").map(|value| value.trim());
    let xhttp_path = query.get("path").map(|value| value.trim());
    let service_name = query
        .get("serviceName")
        .or_else(|| query.get("service_name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| path.clone());

    let mut stream = serde_json::Map::new();
    stream.insert("network".to_string(), Value::String(network.clone()));
    let mut has_non_default_options = network != "tcp";
    if security != "none" {
        stream.insert("security".to_string(), Value::String(security.clone()));
        has_non_default_options = true;
    }

    match network.as_str() {
        "ws" => {
            let mut ws = serde_json::Map::new();
            if let Some(path) = path.as_ref() {
                ws.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(host) = host.as_ref() {
                ws.insert("headers".to_string(), json!({ "Host": host }));
            }
            if !ws.is_empty() {
                stream.insert("wsSettings".to_string(), Value::Object(ws));
                has_non_default_options = true;
            }
        }
        "grpc" => {
            let service_name = service_name.unwrap_or_default();
            stream.insert(
                "grpcSettings".to_string(),
                json!({
                    "serviceName": service_name,
                    "multiMode": query_flag_true(&query, "multiMode")
                }),
            );
            has_non_default_options = true;
        }
        "httpupgrade" => {
            let mut settings = serde_json::Map::new();
            if let Some(host) = host.as_ref() {
                settings.insert("host".to_string(), Value::String(host.clone()));
            }
            if let Some(path) = path.as_ref() {
                settings.insert("path".to_string(), Value::String(path.clone()));
            }
            if !settings.is_empty() {
                stream.insert("httpupgradeSettings".to_string(), Value::Object(settings));
                has_non_default_options = true;
            }
        }
        "xhttp" => {
            if !allow_xhttp {
                return Err(anyhow!(ForwardProxyValidationError::new(
                    ForwardProxyValidationErrorCode::UnsupportedTransport,
                    "xhttp transport is only supported for vless nodes",
                )));
            }
            stream.insert(
                "xhttpSettings".to_string(),
                build_xhttp_settings(&query, xhttp_host, xhttp_path)?,
            );
            has_non_default_options = true;
        }
        _ => {}
    }

    if security == "tls" {
        let mut tls_settings = serde_json::Map::new();
        if let Some(server_name) = query
            .get("sni")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| host.clone())
            .or_else(|| url.host_str().map(str::to_string))
        {
            tls_settings.insert("serverName".to_string(), Value::String(server_name));
        }
        if query_flag_true(&query, "allowInsecure") || query_flag_true(&query, "insecure") {
            tls_settings.insert("allowInsecure".to_string(), Value::Bool(true));
        }
        if let Some(fingerprint) = query
            .get("fp")
            .or_else(|| query.get("fingerprint"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            tls_settings.insert("fingerprint".to_string(), Value::String(fingerprint));
        }
        if let Some(alpn) = query
            .get("alpn")
            .map(|value| parse_alpn_csv(value))
            .filter(|items| !items.is_empty())
        {
            tls_settings.insert("alpn".to_string(), json!(alpn));
        }
        if !tls_settings.is_empty() {
            stream.insert("tlsSettings".to_string(), Value::Object(tls_settings));
            has_non_default_options = true;
        }
    } else if security == "reality" {
        let mut reality_settings = serde_json::Map::new();
        if let Some(server_name) = query
            .get("sni")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| host.clone())
            .or_else(|| url.host_str().map(str::to_string))
        {
            reality_settings.insert("serverName".to_string(), Value::String(server_name));
        }
        if let Some(fingerprint) = query
            .get("fp")
            .or_else(|| query.get("fingerprint"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("fingerprint".to_string(), Value::String(fingerprint));
        }
        if let Some(public_key) = query
            .get("pbk")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("publicKey".to_string(), Value::String(public_key));
        }
        if let Some(short_id) = query
            .get("sid")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("shortId".to_string(), Value::String(short_id));
        }
        if let Some(spider_x) = query
            .get("spx")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("spiderX".to_string(), Value::String(spider_x));
        }
        if !reality_settings.is_empty() {
            stream.insert(
                "realitySettings".to_string(),
                Value::Object(reality_settings),
            );
            has_non_default_options = true;
        }
    }

    if has_non_default_options {
        Ok(Some(Value::Object(stream)))
    } else {
        Ok(None)
    }
}

const SUPPORTED_XHTTP_MODES: &[&str] = &["auto", "packet-up", "stream-up", "stream-one"];

fn build_xhttp_settings(
    query: &HashMap<String, String>,
    host: Option<&str>,
    path: Option<&str>,
) -> Result<Value> {
    let mut extra = match query.get("extra") {
        Some(raw) => match serde_json::from_str::<Value>(raw) {
            Ok(Value::Object(object)) => object,
            Ok(_) => {
                return Err(anyhow!(ForwardProxyValidationError::new(
                    ForwardProxyValidationErrorCode::InvalidXhttpExtra,
                    "xhttp extra must be a JSON object",
                )));
            }
            Err(_) => {
                return Err(anyhow!(ForwardProxyValidationError::new(
                    ForwardProxyValidationErrorCode::InvalidXhttpExtra,
                    "xhttp extra contains invalid JSON",
                )));
            }
        },
        None => serde_json::Map::new(),
    };

    let effective_host = take_xhttp_string_setting(&mut extra, "host", host)?;
    let effective_path = take_xhttp_string_setting(&mut extra, "path", path)?;
    let explicit_mode = query.get("mode").map(|value| value.trim());
    let effective_mode = take_xhttp_mode_setting(&mut extra, explicit_mode)?;

    let mut settings = serde_json::Map::new();
    if let Some(host) = effective_host {
        settings.insert("host".to_string(), Value::String(host));
    }
    if let Some(path) = effective_path {
        settings.insert("path".to_string(), Value::String(path));
    }
    if let Some(mode) = effective_mode {
        settings.insert("mode".to_string(), Value::String(mode));
    }

    if !extra.is_empty() {
        settings.insert("extra".to_string(), Value::Object(extra));
    }

    Ok(Value::Object(settings))
}

fn take_xhttp_string_setting(
    extra: &mut serde_json::Map<String, Value>,
    key: &str,
    explicit: Option<&str>,
) -> Result<Option<String>> {
    if let Some(explicit) = explicit {
        extra.remove(key);
        return Ok(Some(explicit.to_string()));
    }
    let Some(value) = extra.remove(key) else {
        return Ok(None);
    };
    value.as_str().map(str::to_string).map(Some).ok_or_else(|| {
        anyhow!(ForwardProxyValidationError::new(
            ForwardProxyValidationErrorCode::InvalidXhttpExtra,
            format!("xhttp extra {key} must be a string"),
        ))
    })
}

fn take_xhttp_mode_setting(
    extra: &mut serde_json::Map<String, Value>,
    explicit: Option<&str>,
) -> Result<Option<String>> {
    let value = if let Some(explicit) = explicit {
        extra.remove("mode");
        explicit.to_string()
    } else if let Some(value) = extra.remove("mode") {
        value.as_str().map(str::to_string).ok_or_else(|| {
            anyhow!(ForwardProxyValidationError::new(
                ForwardProxyValidationErrorCode::InvalidXhttpExtra,
                "xhttp extra mode must be a non-empty string",
            ))
        })?
    } else {
        return Ok(None);
    };
    let mode = value.trim().to_ascii_lowercase();
    if mode.is_empty() {
        return Err(anyhow!(ForwardProxyValidationError::new(
            ForwardProxyValidationErrorCode::InvalidXhttpExtra,
            "xhttp extra mode must be a non-empty string",
        )));
    }
    if !SUPPORTED_XHTTP_MODES.contains(&mode.as_str()) {
        return Err(anyhow!(ForwardProxyValidationError::new(
            ForwardProxyValidationErrorCode::UnsupportedXhttpMode,
            "xhttp mode must be one of auto, packet-up, stream-up, or stream-one",
        )));
    }
    Ok(Some(mode))
}

pub(crate) fn query_flag_true(query: &HashMap<String, String>, key: &str) -> bool {
    query.get(key).is_some_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardProxyAttemptWindowStats {
    pub(crate) attempts: i64,
    pub(crate) success_count: i64,
    pub(crate) avg_latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyWindowStatsResponse {
    pub(crate) attempts: i64,
    pub(crate) success_rate: Option<f64>,
    pub(crate) avg_latency_ms: Option<f64>,
}

impl From<ForwardProxyAttemptWindowStats> for ForwardProxyWindowStatsResponse {
    fn from(value: ForwardProxyAttemptWindowStats) -> Self {
        let success_rate = if value.attempts > 0 {
            Some((value.success_count as f64) / (value.attempts as f64))
        } else {
            None
        };
        Self {
            attempts: value.attempts,
            success_rate,
            avg_latency_ms: value.avg_latency_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyStatsResponse {
    pub(crate) one_minute: ForwardProxyWindowStatsResponse,
    pub(crate) fifteen_minutes: ForwardProxyWindowStatsResponse,
    pub(crate) one_hour: ForwardProxyWindowStatsResponse,
    pub(crate) one_day: ForwardProxyWindowStatsResponse,
    pub(crate) seven_days: ForwardProxyWindowStatsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) stats: ForwardProxyStatsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyBindingNodeResponse {
    pub(crate) key: String,
    pub(crate) alias_keys: Vec<String>,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) protocol_label: String,
    pub(crate) egress_ip: Option<String>,
    pub(crate) egress_ip_checked_at: Option<String>,
    pub(crate) egress_ip_provider: Option<String>,
    pub(crate) egress_ip_error: Option<String>,
    pub(crate) egress_ip_error_at: Option<String>,
    pub(crate) penalized: bool,
    pub(crate) selectable: bool,
    pub(crate) last24h: Vec<ForwardProxyHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxySettingsResponse {
    pub(crate) proxy_urls: Vec<String>,
    pub(crate) subscription_urls: Vec<String>,
    pub(crate) subscription_update_interval_secs: u64,
    pub(crate) nodes: Vec<ForwardProxyNodeResponse>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardProxyHourlyStatsPoint {
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyWeightHourlyStatsPoint {
    pub(crate) sample_count: i64,
    pub(crate) min_weight: f64,
    pub(crate) max_weight: f64,
    pub(crate) avg_weight: f64,
    pub(crate) last_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyHourlyBucketResponse {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyWeightHourlyBucketResponse {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) sample_count: i64,
    pub(crate) min_weight: f64,
    pub(crate) max_weight: f64,
    pub(crate) avg_weight: f64,
    pub(crate) last_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLiveNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) stats: ForwardProxyStatsResponse,
    pub(crate) last24h: Vec<ForwardProxyHourlyBucketResponse>,
    pub(crate) weight24h: Vec<ForwardProxyWeightHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLiveStatsResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) nodes: Vec<ForwardProxyLiveNodeResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyTimeseriesNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) buckets: Vec<ForwardProxyHourlyBucketResponse>,
    pub(crate) weight_buckets: Vec<ForwardProxyWeightHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyTimeseriesResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) effective_bucket: String,
    pub(crate) available_buckets: Vec<String>,
    pub(crate) nodes: Vec<ForwardProxyTimeseriesNodeResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_with_manual_proxy() -> ForwardProxyManager {
        ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec!["http://jp-edge-01:8080".to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        )
    }

    fn current_binding_node(manager: &ForwardProxyManager) -> ForwardProxyBindingNodeResponse {
        manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .expect("missing non-direct binding node")
    }

    fn xhttp_test_link(security: &str, network_key: &str) -> String {
        format!(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?encryption=none&security={security}&{network_key}=xhttp&host=top.example.com&path=%2Ftop&mode=stream-one&extra=%7B%22host%22%3A%22extra.example.com%22%2C%22path%22%3A%22%2Fextra%22%2C%22mode%22%3A%22auto%22%2C%22xmux%22%3A%7B%22maxConnections%22%3A%221%22%7D%7D&sni=edge.example.com&alpn=h2&fp=chrome&pbk=public-key&sid=01234567"
        )
    }

    #[test]
    fn vless_xhttp_builder_emits_effective_settings_for_all_security_modes() {
        for mode in ["auto", "packet-up", "stream-up", "stream-one"] {
            for security in ["none", "tls", "reality"] {
                let raw = xhttp_test_link(security, "net")
                    .replace("mode=stream-one", &format!("mode={mode}"));
                let outbound =
                    build_vless_xray_outbound(&raw).expect("xhttp vless outbound should build");
                let stream = &outbound["streamSettings"];
                assert_eq!(stream["network"], "xhttp");
                assert_eq!(stream["xhttpSettings"]["host"], "top.example.com");
                assert_eq!(stream["xhttpSettings"]["path"], "/top");
                assert_eq!(stream["xhttpSettings"]["mode"], mode);
                assert_eq!(
                    stream["xhttpSettings"]["extra"]["xmux"]["maxConnections"],
                    "1"
                );

                match security {
                    "none" => {
                        assert!(stream.get("security").is_none());
                        assert!(stream.get("tlsSettings").is_none());
                        assert!(stream.get("realitySettings").is_none());
                    }
                    "tls" => {
                        assert_eq!(stream["security"], "tls");
                        assert_eq!(stream["tlsSettings"]["alpn"], json!(["h2"]));
                        assert!(stream.get("realitySettings").is_none());
                    }
                    "reality" => {
                        assert_eq!(stream["security"], "reality");
                        assert_eq!(stream["realitySettings"]["serverName"], "edge.example.com");
                        assert!(stream["realitySettings"].get("alpn").is_none());
                        assert!(stream.get("tlsSettings").is_none());
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn vless_xhttp_builder_promotes_extra_transport_fields() {
        let raw = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&extra=%7B%22host%22%3A%22extra.example.com%22%2C%22path%22%3A%22%2Fextra%22%2C%22mode%22%3A%22stream-up%22%2C%22xmux%22%3A%7B%22maxConnections%22%3A%221%22%7D%7D";
        let outbound = build_vless_xray_outbound(raw).expect("extra transport fields should build");
        let settings = &outbound["streamSettings"]["xhttpSettings"];

        assert_eq!(settings["host"], "extra.example.com");
        assert_eq!(settings["path"], "/extra");
        assert_eq!(settings["mode"], "stream-up");
        assert_eq!(settings["extra"]["xmux"]["maxConnections"], "1");
    }

    #[test]
    fn vless_xhttp_builder_honors_explicit_empty_top_level_fields() {
        let raw = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&host=&path=&extra=%7B%22host%22%3A%22extra.example.com%22%2C%22path%22%3A%22%2Fextra%22%7D";
        let outbound =
            build_vless_xray_outbound(raw).expect("explicit empty xhttp fields should build");
        let settings = &outbound["streamSettings"]["xhttpSettings"];

        assert_eq!(settings["host"], "");
        assert_eq!(settings["path"], "");
        assert!(settings.get("extra").is_none());
    }

    #[test]
    fn vless_xhttp_builder_rejects_invalid_settings_without_protocol_fallback() {
        let invalid_extra = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&extra=%5B%5D";
        let error = build_vless_xray_outbound(invalid_extra).expect_err("array extra must fail");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed xhttp error")
                .code,
            ForwardProxyValidationErrorCode::InvalidXhttpExtra
        );

        let malformed_extra = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&extra=%7B";
        let error =
            build_vless_xray_outbound(malformed_extra).expect_err("malformed extra must fail");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed xhttp error")
                .code,
            ForwardProxyValidationErrorCode::InvalidXhttpExtra
        );

        let invalid_extra_mode = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&extra=%7B%22mode%22%3A1%7D";
        let error = build_vless_xray_outbound(invalid_extra_mode).expect_err("invalid extra mode");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed xhttp error")
                .code,
            ForwardProxyValidationErrorCode::InvalidXhttpExtra
        );

        let invalid_mode = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&mode=legacy";
        let error =
            build_vless_xray_outbound(invalid_mode).expect_err("unsupported mode must fail");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed xhttp error")
                .code,
            ForwardProxyValidationErrorCode::UnsupportedXhttpMode
        );

        let duplicate_query = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&type=tcp";
        let error = build_vless_xray_outbound(duplicate_query)
            .expect_err("ambiguous xhttp query fields must fail");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed duplicate query error")
                .code,
            ForwardProxyValidationErrorCode::InvalidXhttpExtra
        );

        let duplicate_alpn = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=tls&type=xhttp&alpn=h2&alpn=h3";
        let error = build_vless_xray_outbound(duplicate_alpn)
            .expect_err("duplicate alpn must not drift identity from runtime configuration");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed duplicate alpn error")
                .code,
            ForwardProxyValidationErrorCode::InvalidXhttpExtra
        );

        let vless_splithttp = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=splithttp";
        let error = build_vless_xray_outbound(vless_splithttp)
            .expect_err("vless splithttp must not be treated as supported xhttp");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed splithttp error")
                .code,
            ForwardProxyValidationErrorCode::UnsupportedTransport
        );

        let trojan_xhttp =
            "trojan://password@edge.example.com:443?security=tls&type=xhttp&path=%2Ftop";
        let error = build_trojan_xray_outbound(trojan_xhttp)
            .expect_err("trojan must not inherit vless xhttp support");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed transport error")
                .code,
            ForwardProxyValidationErrorCode::UnsupportedTransport
        );

        let trojan_splithttp = "trojan://password@edge.example.com:443?security=tls&type=splithttp";
        let error = build_trojan_xray_outbound(trojan_splithttp)
            .expect_err("trojan splithttp must not be treated as supported xhttp");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed splithttp error")
                .code,
            ForwardProxyValidationErrorCode::UnsupportedTransport
        );

        let vmess_payload = json!({
            "add": "edge.example.com",
            "port": 443,
            "id": "11111111-1111-1111-1111-111111111111",
            "net": "xhttp",
            "type": "none",
            "tls": "none"
        })
        .to_string();
        let vmess_xhttp = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_payload)
        );
        let error = build_vmess_xray_outbound(&vmess_xhttp)
            .expect_err("vmess must not inherit vless xhttp support");
        assert_eq!(
            error
                .downcast_ref::<ForwardProxyValidationError>()
                .expect("typed transport error")
                .code,
            ForwardProxyValidationErrorCode::UnsupportedTransport
        );
    }

    #[test]
    fn legacy_outbound_builder_preserves_transport_shapes() {
        let tcp = build_vless_xray_outbound(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?encryption=none&type=tcp&security=none",
        )
        .expect("vless tcp outbound should build");
        assert_eq!(tcp["protocol"], "vless");
        assert!(tcp.get("streamSettings").is_none());

        let ws = build_vless_xray_outbound(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?encryption=none&type=ws&security=tls&host=cdn.example.com&path=%2Fws&sni=edge.example.com&alpn=h2",
        )
        .expect("vless websocket outbound should build");
        assert_eq!(ws["streamSettings"]["network"], "ws");
        assert_eq!(ws["streamSettings"]["wsSettings"]["path"], "/ws");
        assert_eq!(
            ws["streamSettings"]["wsSettings"]["headers"]["Host"],
            "cdn.example.com"
        );
        assert_eq!(ws["streamSettings"]["tlsSettings"]["alpn"], json!(["h2"]));

        let grpc = build_vless_xray_outbound(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?encryption=none&type=grpc&security=tls&serviceName=chat&sni=edge.example.com",
        )
        .expect("vless grpc outbound should build");
        assert_eq!(grpc["streamSettings"]["network"], "grpc");
        assert_eq!(
            grpc["streamSettings"]["grpcSettings"]["serviceName"],
            "chat"
        );

        let http_upgrade = build_vless_xray_outbound(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?encryption=none&type=httpupgrade&security=none&host=cdn.example.com&path=%2Fupgrade",
        )
        .expect("vless httpupgrade outbound should build");
        assert_eq!(http_upgrade["streamSettings"]["network"], "httpupgrade");
        assert_eq!(
            http_upgrade["streamSettings"]["httpupgradeSettings"]["host"],
            "cdn.example.com"
        );
        assert_eq!(
            http_upgrade["streamSettings"]["httpupgradeSettings"]["path"],
            "/upgrade"
        );

        let trojan = build_trojan_xray_outbound(
            "trojan://fixture-password@edge.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fws&sni=edge.example.com",
        )
        .expect("trojan websocket outbound should build");
        assert_eq!(trojan["protocol"], "trojan");
        assert_eq!(trojan["streamSettings"]["network"], "ws");
        assert_eq!(trojan["streamSettings"]["security"], "tls");

        let vmess_payload = json!({
            "add": "edge.example.com",
            "port": 443,
            "id": "11111111-1111-1111-1111-111111111111",
            "net": "ws",
            "type": "none",
            "tls": "tls",
            "host": "cdn.example.com",
            "path": "/ws"
        })
        .to_string();
        let vmess = build_vmess_xray_outbound(&format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_payload)
        ))
        .expect("vmess websocket outbound should build");
        assert_eq!(vmess["protocol"], "vmess");
        assert_eq!(vmess["streamSettings"]["network"], "ws");
        assert_eq!(vmess["streamSettings"]["wsSettings"]["path"], "/ws");
    }

    #[test]
    fn binding_nodes_include_selectable_direct_with_protocol_label() {
        let manager = manager_with_manual_proxy();

        assert!(!manager.runtime.contains_key(FORWARD_PROXY_DIRECT_KEY));

        let direct = manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
            .expect("missing direct binding node");

        assert_eq!(direct.display_name, FORWARD_PROXY_DIRECT_LABEL);
        assert_eq!(direct.protocol_label, "DIRECT");
        assert!(direct.selectable);
        assert!(!direct.penalized);
    }

    #[test]
    fn binding_nodes_use_name_driven_binding_keys_and_keep_runtime_aliases() {
        let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京节点";
        let normalized_proxy_url =
            normalize_share_link_scheme(proxy_url, "vless").expect("normalize vless url");
        let legacy_alias = {
            let parsed = Url::parse(&normalized_proxy_url).expect("parse normalized vless url");
            stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
        };
        let runtime_key = normalize_single_proxy_key(proxy_url).expect("canonical vless key");
        let binding_key = forward_proxy_binding_key_candidates(
            &forward_proxy_binding_parts_from_raw(proxy_url, None)
                .expect("binding parts from vless url"),
        )[0]
        .clone();
        assert_ne!(binding_key, runtime_key);

        let manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_url.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );

        let node = manager
            .binding_nodes()
            .into_iter()
            .find(|candidate| candidate.key == binding_key)
            .expect("vless binding node should be present");

        assert!(node.alias_keys.contains(&runtime_key));
        assert!(node.alias_keys.contains(&legacy_alias));
        assert!(node.alias_keys.iter().all(|alias| {
            is_stable_forward_proxy_key(alias) || is_stable_forward_proxy_binding_key(alias)
        }));
    }

    #[test]
    fn binding_keys_ignore_transport_identity_changes_when_name_is_unique() {
        let proxy_a = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Falpha&sni=alpha.example.com#Tokyo%20Edge";
        let proxy_b = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fbeta&sni=beta.example.com#Tokyo%20Edge";
        assert_ne!(
            normalize_single_proxy_key(proxy_a),
            normalize_single_proxy_key(proxy_b),
            "runtime keys should still reflect transport identity"
        );

        let manager_a = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_a.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let manager_b = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_b.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );

        assert_eq!(
            current_binding_node(&manager_a).key,
            current_binding_node(&manager_b).key
        );
    }

    #[test]
    fn binding_keys_change_when_display_name_changes() {
        let proxy_a = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Falpha&sni=edge.example.com#Tokyo%20Edge";
        let proxy_b = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Falpha&sni=edge.example.com#Tokyo%20Edge%20Renamed";
        assert_eq!(
            normalize_single_proxy_key(proxy_a),
            normalize_single_proxy_key(proxy_b),
            "runtime keys should ignore display name changes"
        );

        let manager_a = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_a.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let manager_b = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_b.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );

        assert_ne!(
            current_binding_node(&manager_a).key,
            current_binding_node(&manager_b).key
        );
    }

    #[test]
    fn binding_nodes_keep_legacy_unsafe_display_name_aliases() {
        let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#Tokyo%40JP";
        let manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_url.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let legacy_parts = legacy_forward_proxy_binding_parts_from_raw(proxy_url)
            .expect("legacy binding parts should parse");
        let legacy_key = forward_proxy_binding_key_candidates(&legacy_parts)[0].clone();
        let node = manager
            .binding_nodes()
            .into_iter()
            .find(|candidate| candidate.protocol_label == "VLESS")
            .expect("vless binding node should be present");

        assert_eq!(node.display_name, "vless.example.com:443");
        assert!(
            node.alias_keys.contains(&legacy_key),
            "saved binding created from the legacy display name must remain resolvable"
        );
    }

    #[test]
    fn binding_keys_escalate_from_name_to_protocol_to_host_port() {
        let shadowsocks_url = |host| {
            format!(
                "{}{}{}{}{}",
                "ss://2022-blake3-aes-128-gcm:",
                "fixture-password",
                "@",
                host,
                ":8388#Shared%20Node"
            )
        };
        let protocol_split_manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![
                    "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#Shared%20Node".to_string(),
                    shadowsocks_url("ss.example.com"),
                ],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let protocol_nodes = protocol_split_manager
            .binding_nodes()
            .into_iter()
            .filter(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .collect::<Vec<_>>();
        assert_eq!(protocol_nodes.len(), 2);
        assert_ne!(protocol_nodes[0].key, protocol_nodes[1].key);

        let host_split_manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![
                    shadowsocks_url("jp-a.example.com"),
                    shadowsocks_url("jp-b.example.com"),
                ],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let host_nodes = host_split_manager
            .binding_nodes()
            .into_iter()
            .filter(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .collect::<Vec<_>>();
        assert_eq!(host_nodes.len(), 2);
        assert_ne!(host_nodes[0].key, host_nodes[1].key);

        let collapsed_manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![
                    "vless://11111111-1111-1111-1111-111111111111@shared.example.com:443?security=tls&type=ws&path=%2Falpha&sni=alpha.example.com#Shared%20Node".to_string(),
                    "vless://11111111-1111-1111-1111-111111111111@shared.example.com:443?security=tls&type=ws&path=%2Fbeta&sni=beta.example.com#Shared%20Node".to_string(),
                ],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );
        let collapsed_nodes = collapsed_manager
            .binding_nodes()
            .into_iter()
            .filter(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .collect::<Vec<_>>();
        assert_eq!(collapsed_nodes.len(), 1);
        assert!(
            collapsed_nodes[0]
                .alias_keys
                .iter()
                .any(|key| key.starts_with("fpn_"))
        );
    }

    #[test]
    fn automatic_selection_does_not_use_direct() {
        let mut manager = ForwardProxyManager::new(ForwardProxySettings::default(), Vec::new());

        assert!(manager.select_auto_proxy().is_none());
    }

    #[test]
    fn pinned_selection_rejects_nodes_that_are_no_longer_bound_selectable() {
        let mut manager = manager_with_manual_proxy();
        let binding_key = current_binding_node(&manager).key;
        let endpoint_key = manager
            .bound_key_endpoint_keys
            .get(&binding_key)
            .cloned()
            .expect("binding key should resolve to an endpoint");
        let endpoint = manager
            .endpoints
            .iter_mut()
            .find(|endpoint| endpoint.key == endpoint_key)
            .expect("endpoint should exist");
        endpoint.endpoint_url = None;

        let err = manager
            .select_proxy_for_scope(&ForwardProxyRouteScope::pinned(binding_key))
            .expect_err("stale pinned node should be rejected");

        assert!(
            err.to_string()
                .contains("pinned forward proxy key is no longer available")
        );
    }

    #[test]
    fn current_bound_group_binding_key_normalizes_alias_backed_runtime_key() {
        let mut manager = manager_with_manual_proxy();
        let node = current_binding_node(&manager);
        let alias_key = node
            .alias_keys
            .first()
            .cloned()
            .expect("binding node should expose an alias key");
        manager.bound_group_runtime.insert(
            "latam".to_string(),
            BoundForwardProxyGroupState {
                current_binding_key: Some(alias_key),
                consecutive_network_failures: 0,
            },
        );

        assert_eq!(
            manager.current_bound_group_binding_key("latam", std::slice::from_ref(&node.key)),
            Some(node.key),
        );
    }

    #[test]
    fn bound_group_network_failures_can_switch_from_direct_to_proxy() {
        let mut manager = manager_with_manual_proxy();
        let binding_key = manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| node.key)
            .expect("missing non-direct binding node");
        let scope = ForwardProxyRouteScope::BoundGroup {
            group_name: "latam".to_string(),
            bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string(), binding_key.clone()],
        };
        manager.bound_group_runtime.insert(
            "latam".to_string(),
            BoundForwardProxyGroupState {
                current_binding_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                consecutive_network_failures: 0,
            },
        );

        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );
        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );
        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );

        let group_state = manager
            .bound_group_runtime
            .get("latam")
            .expect("missing bound group state after failures");
        assert_eq!(
            group_state.current_binding_key.as_deref(),
            Some(binding_key.as_str())
        );
        assert_eq!(group_state.consecutive_network_failures, 0);
    }

    #[tokio::test]
    async fn egress_ip_metadata_preserves_last_success_when_refresh_fails() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");
        let selected_proxy = SelectedForwardProxy {
            key: "jp-edge-01".to_string(),
            source: "manual".to_string(),
            display_name: "JP Edge 01".to_string(),
            endpoint_url: None,
            endpoint_url_raw: Some("http://jp-edge-01:8080".to_string()),
            egress_ip: None,
        };

        persist_forward_proxy_egress_ip_result(&pool, &selected_proxy, Some("203.0.113.24"), None)
            .await
            .expect("persist successful egress IP");
        persist_forward_proxy_egress_ip_result(
            &pool,
            &selected_proxy,
            None,
            Some("metadata refresh timed out"),
        )
        .await
        .expect("persist failed refresh");

        let metadata =
            load_forward_proxy_metadata_history(&pool, std::slice::from_ref(&selected_proxy.key))
                .await
                .expect("load metadata")
                .remove(&selected_proxy.key)
                .expect("metadata row");
        assert_eq!(metadata.egress_ip.as_deref(), Some("203.0.113.24"));
        assert_eq!(metadata.egress_ip_provider.as_deref(), Some("ipify"));
        assert_eq!(
            metadata.egress_ip_error.as_deref(),
            Some("metadata refresh timed out")
        );
        assert!(metadata.egress_ip_checked_at.is_some());
        assert!(metadata.egress_ip_error_at.is_some());
    }

    #[test]
    fn manual_latency_average_uses_only_successful_samples() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(101.4),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some("timeout".to_string()),
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some("timeout".to_string()),
            },
        );
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(198.6),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(250.0),
                ip: None,
                http_status: Some(200),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some("responses timeout".to_string()),
            },
        );

        assert_eq!(accumulator.completed_rounds, 2);
        assert_eq!(accumulator.success_count, 3);
        assert_eq!(accumulator.average_latency_ms(), Some(183));
    }

    #[test]
    fn manual_latency_average_is_none_when_all_rounds_fail() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        for _ in 0..forward_proxy_manual_latency_round_count() {
            accumulator.record_round(
                &ForwardProxyLatencyProbeTargetResult {
                    ok: false,
                    latency_ms: None,
                    ip: None,
                    http_status: None,
                    error: Some("egress timeout".to_string()),
                },
                &ForwardProxyLatencyProbeTargetResult {
                    ok: false,
                    latency_ms: None,
                    ip: None,
                    http_status: None,
                    error: Some("oauth timeout".to_string()),
                },
                &ForwardProxyLatencyProbeTargetResult {
                    ok: false,
                    latency_ms: None,
                    ip: None,
                    http_status: None,
                    error: Some("responses timeout".to_string()),
                },
            );
        }

        assert_eq!(accumulator.completed_rounds, 5);
        assert_eq!(accumulator.success_count, 0);
        assert_eq!(accumulator.average_latency_ms(), None);
    }

    #[test]
    fn manual_latency_requires_codex_responses_reachable() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        let codex_responses = ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some("responses timeout".to_string()),
        };
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(91.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(142.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &codex_responses,
        );

        assert_eq!(accumulator.success_count, 2);
        assert_eq!(accumulator.average_latency_ms(), Some(117));
        assert!(!accumulator.all_targets_ok());
        assert_eq!(
            accumulator.failed_targets(),
            vec![FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES]
        );
        assert_eq!(codex_responses.error.as_deref(), Some("responses timeout"));
    }

    #[test]
    fn manual_latency_codex_responses_accepts_method_not_allowed() {
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::TOO_MANY_REQUESTS,
        ] {
            assert!(is_forward_proxy_codex_responses_probe_reachable_status(
                status
            ));
        }
        assert!(!is_forward_proxy_codex_responses_probe_reachable_status(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(is_forward_proxy_latency_probe_reachable_status(
            StatusCode::METHOD_NOT_ALLOWED,
            false
        ));
        assert!(is_forward_proxy_latency_probe_reachable_status(
            StatusCode::NOT_FOUND,
            true
        ));
        assert!(!is_forward_proxy_latency_probe_reachable_status(
            StatusCode::METHOD_NOT_ALLOWED,
            true
        ));
        assert!(!is_forward_proxy_latency_probe_reachable_status(
            StatusCode::TOO_MANY_REQUESTS,
            true
        ));
    }

    #[tokio::test]
    async fn timed_codex_responses_probe_maps_http_statuses() {
        let _guard = crate::oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        struct ResetCodexUpstreamOverride;
        impl Drop for ResetCodexUpstreamOverride {
            fn drop(&mut self) {
                crate::oauth_bridge::reset_test_oauth_codex_upstream_base_url_now();
            }
        }
        let _reset_override = ResetCodexUpstreamOverride;

        for (status, expected_ok) in [
            (StatusCode::METHOD_NOT_ALLOWED, true),
            (StatusCode::INTERNAL_SERVER_ERROR, false),
        ] {
            let app = Router::new().route(
                "/backend-api/codex/responses",
                get(move || async move { status }),
            );
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind codex responses probe upstream");
            let addr = listener
                .local_addr()
                .expect("read codex responses probe upstream address");
            let handle = tokio::spawn(async move {
                axum::serve(listener, app)
                    .await
                    .expect("codex responses probe upstream should run");
            });

            crate::oauth_bridge::set_test_oauth_codex_upstream_base_url(
                Url::parse(&format!("http://{addr}/backend-api/codex"))
                    .expect("valid codex responses probe upstream url"),
            )
            .await;
            let result =
                timed_forward_proxy_codex_responses_probe(&Client::new(), Duration::from_secs(1))
                    .await;

            handle.abort();
            let _ = handle.await;
            assert_eq!(
                result.ok, expected_ok,
                "unexpected result for HTTP {status}"
            );
            assert_eq!(result.http_status, Some(status.as_u16()));
            assert_eq!(result.error.is_none(), expected_ok);
        }

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind codex responses transport-error address");
        let addr = listener
            .local_addr()
            .expect("read codex responses transport-error address");
        drop(listener);
        crate::oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("http://{addr}/backend-api/codex"))
                .expect("valid codex responses transport-error url"),
        )
        .await;
        let result =
            timed_forward_proxy_codex_responses_probe(&Client::new(), Duration::from_secs(1)).await;
        assert!(!result.ok);
        assert_eq!(result.http_status, None);
        assert!(result.error.is_some());

        let app = Router::new().route(
            "/backend-api/codex/responses",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                StatusCode::OK
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind codex responses timeout upstream");
        let addr = listener
            .local_addr()
            .expect("read codex responses timeout upstream address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("codex responses timeout upstream should run");
        });
        crate::oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("http://{addr}/backend-api/codex"))
                .expect("valid codex responses timeout upstream url"),
        )
        .await;
        let result =
            timed_forward_proxy_codex_responses_probe(&Client::new(), Duration::from_millis(10))
                .await;
        handle.abort();
        let _ = handle.await;
        assert!(!result.ok);
        assert_eq!(result.http_status, None);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("timed out"))
        );
    }

    #[test]
    fn manual_latency_xhttp_responses_405_keeps_round_healthy() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        let egress_ip = ForwardProxyLatencyProbeTargetResult {
            ok: true,
            latency_ms: Some(91.0),
            ip: Some("203.0.113.24".to_string()),
            http_status: None,
            error: None,
        };
        let oauth_upstream = ForwardProxyLatencyProbeTargetResult {
            ok: true,
            latency_ms: Some(142.0),
            ip: None,
            http_status: Some(401),
            error: None,
        };
        let codex_responses = ForwardProxyLatencyProbeTargetResult {
            ok: is_forward_proxy_codex_responses_probe_reachable_status(
                StatusCode::METHOD_NOT_ALLOWED,
            ),
            latency_ms: Some(188.0),
            ip: None,
            http_status: Some(405),
            error: None,
        };

        accumulator.record_round(&egress_ip, &oauth_upstream, &codex_responses);

        let progress =
            forward_proxy_latency_timeout_progress(&ForwardProxyEndpoint::direct(), &accumulator);
        assert!(progress.all_targets_ok);
        assert!(progress.failed_targets.is_empty());
        assert!(progress.egress_ip.ok);
        assert!(progress.oauth_upstream.ok);
        assert!(progress.codex_responses.ok);
        assert_eq!(progress.codex_responses.http_status, Some(405));
        assert_eq!(progress.average_latency_ms, Some(140));
    }

    #[test]
    fn manual_latency_timeout_progress_preserves_completed_target_results() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(91.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(142.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(188.0),
                ip: None,
                http_status: Some(405),
                error: None,
            },
        );

        let progress =
            forward_proxy_latency_timeout_progress(&ForwardProxyEndpoint::direct(), &accumulator);

        assert!(progress.all_targets_ok);
        assert!(progress.failed_targets.is_empty());
        assert!(progress.egress_ip.ok);
        assert!(progress.oauth_upstream.ok);
        assert!(progress.codex_responses.ok);
        assert!(!progress.timed_out);
        assert_eq!(progress.average_latency_ms, Some(140));
    }

    #[test]
    fn manual_latency_preserves_failed_target_detail_after_later_success() {
        let mut accumulator = ForwardProxyLatencyAccumulator::default();
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(91.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(142.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some("responses timeout".to_string()),
            },
        );
        accumulator.record_round(
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(94.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(138.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(188.0),
                ip: None,
                http_status: Some(405),
                error: None,
            },
        );

        let progress =
            forward_proxy_latency_timeout_progress(&ForwardProxyEndpoint::direct(), &accumulator);

        assert!(!progress.all_targets_ok);
        assert_eq!(
            progress.failed_targets,
            vec![FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES]
        );
        assert!(!progress.codex_responses.ok);
        assert_eq!(
            progress.codex_responses.error.as_deref(),
            Some("responses timeout")
        );

        let current_success = ForwardProxyLatencyProbeTargetResult {
            ok: true,
            latency_ms: Some(188.0),
            ip: None,
            http_status: Some(405),
            error: None,
        };
        let (_, _, displayed_codex_responses) = accumulated_forward_proxy_latency_target_results(
            &accumulator,
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(94.0),
                ip: Some("203.0.113.24".to_string()),
                http_status: None,
                error: None,
            },
            &ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(138.0),
                ip: None,
                http_status: Some(401),
                error: None,
            },
            &current_success,
        );
        assert!(!displayed_codex_responses.ok);
        assert_eq!(
            displayed_codex_responses.error.as_deref(),
            Some("responses timeout")
        );
    }

    #[test]
    fn manual_latency_oauth_codex_probe_targets_preserve_base_path() {
        assert_eq!(
            oauth_codex_latency_probe_target("models")
                .expect("models target")
                .as_str(),
            "https://chatgpt.com/backend-api/codex/models"
        );
        assert_eq!(
            oauth_codex_latency_probe_target("responses")
                .expect("responses target")
                .as_str(),
            "https://chatgpt.com/backend-api/codex/responses"
        );
    }

    #[test]
    fn manual_latency_batch_schedule_is_breadth_first() {
        assert_eq!(
            forward_proxy_latency_breadth_first_schedule(3, 2),
            vec![(1, 0), (1, 1), (1, 2), (2, 0), (2, 1), (2, 2)]
        );
    }

    #[test]
    fn manual_latency_batch_query_accepts_repeated_keys() {
        assert_eq!(
            parse_forward_proxy_nodes_latency_test_keys(
                "key=__direct__&key=fpn_a&ignored=value&key=fpn_b"
            ),
            vec![
                "__direct__".to_string(),
                "fpn_a".to_string(),
                "fpn_b".to_string()
            ]
        );
    }
}
