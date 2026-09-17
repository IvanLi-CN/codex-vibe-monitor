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
            .filter(|key| !desired_keys.contains(*key))
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
                        proxy_label = endpoint.display_name,
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
        self.spawn_instance(endpoint, ready_timeout, shutdown).await
    }

    pub(crate) async fn spawn_instance(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
        shutdown: &CancellationToken,
    ) -> Result<Url> {
        let outbound = build_xray_outbound_for_endpoint(endpoint)?;
        let local_port = pick_unused_local_port().context("failed to allocate xray local port")?;
        fs::create_dir_all(&self.runtime_dir).with_context(|| {
            format!(
                "failed to create xray runtime directory: {}",
                self.runtime_dir.display()
            )
        })?;
        let config_path = self.runtime_dir.join(format!(
            "forward-proxy-{:016x}.json",
            stable_hash_u64(&endpoint.key)
        ));
        let config = build_xray_instance_config(local_port, outbound);
        let serialized =
            serde_json::to_vec_pretty(&config).context("failed to serialize xray config")?;
        fs::write(&config_path, serialized)
            .with_context(|| format!("failed to write xray config: {}", config_path.display()))?;

        let mut child = match Command::new(&self.binary)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let _ = fs::remove_file(&config_path);
                return Err(err)
                    .with_context(|| format!("failed to start xray binary: {}", self.binary));
            }
        };

        if let Err(err) =
            wait_for_xray_proxy_ready(&mut child, local_port, ready_timeout, shutdown).await
        {
            let _ = terminate_child_process(
                &mut child,
                Duration::from_secs(2),
                &forward_proxy_log_ref(&endpoint.key),
            )
            .await;
            let _ = fs::remove_file(&config_path);
            return Err(err);
        }

        let local_proxy_url = Url::parse(&format!("socks5h://127.0.0.1:{local_port}"))
            .context("failed to build local xray socks endpoint")?;
        self.instances.insert(
            endpoint.key.clone(),
            XrayInstance {
                local_proxy_url: local_proxy_url.clone(),
                config_path,
                child,
            },
        );

        Ok(local_proxy_url)
    }

    pub(crate) async fn remove_instance(&mut self, key: &str) {
        if let Some(mut instance) = self.instances.remove(key) {
            let proxy_key_ref = forward_proxy_log_ref(key);
            let _ = terminate_child_process(
                &mut instance.child,
                Duration::from_secs(2),
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
    let security = link
        .tls_mode
        .as_deref()
        .filter(|value| !value.is_empty() && *value != "none")
        .map(|value| value.to_ascii_lowercase());
    let network_has_options = insert_vmess_network_settings(&mut stream, link);
    let security_has_options =
        insert_vmess_security_settings(&mut stream, link, security.as_deref());
    let has_non_default_options =
        link.network != "tcp" || network_has_options || security_has_options;

    if has_non_default_options {
        Some(Value::Object(stream))
    } else {
        None
    }
}

fn insert_vmess_network_settings(
    stream: &mut serde_json::Map<String, Value>,
    link: &VmessShareLink,
) -> bool {
    match link.network.as_str() {
        "ws" => {
            let mut settings = serde_json::Map::new();
            if let Some(path) = link.path.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(host) = link.host.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("headers".to_string(), json!({ "Host": host }));
            }
            if settings.is_empty() {
                false
            } else {
                stream.insert("wsSettings".to_string(), Value::Object(settings));
                true
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
                json!({ "serviceName": service_name }),
            );
            true
        }
        "httpupgrade" => {
            let mut settings = serde_json::Map::new();
            if let Some(host) = link.host.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("host".to_string(), Value::String(host.clone()));
            }
            if let Some(path) = link.path.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("path".to_string(), Value::String(path.clone()));
            }
            if settings.is_empty() {
                false
            } else {
                stream.insert("httpupgradeSettings".to_string(), Value::Object(settings));
                true
            }
        }
        _ => false,
    }
}

fn insert_vmess_security_settings(
    stream: &mut serde_json::Map<String, Value>,
    link: &VmessShareLink,
    security: Option<&str>,
) -> bool {
    let Some(security) = security else {
        return false;
    };
    stream.insert("security".to_string(), Value::String(security.to_string()));
    let mut settings = serde_json::Map::new();
    if let Some(server_name) = link
        .sni
        .as_ref()
        .or(link.host.as_ref())
        .filter(|value| !value.trim().is_empty())
    {
        settings.insert("serverName".to_string(), Value::String(server_name.clone()));
    }
    if security == "tls" {
        if let Some(alpn) = link.alpn.as_ref().filter(|items| !items.is_empty()) {
            settings.insert("alpn".to_string(), json!(alpn));
        }
        if let Some(fingerprint) = link
            .fingerprint
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            settings.insert(
                "fingerprint".to_string(),
                Value::String(fingerprint.clone()),
            );
        }
        if !settings.is_empty() {
            stream.insert("tlsSettings".to_string(), Value::Object(settings));
        }
    } else if security == "reality" {
        if let Some(fingerprint) = link
            .fingerprint
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            settings.insert(
                "fingerprint".to_string(),
                Value::String(fingerprint.clone()),
            );
        }
        if !settings.is_empty() {
            stream.insert("realitySettings".to_string(), Value::Object(settings));
        }
    }
    true
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
    if let Some(stream_settings) = build_stream_settings_from_url(&url, None)
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
    if let Some(stream_settings) = build_stream_settings_from_url(&url, Some("tls"))
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
) -> Option<Value> {
    let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
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

    let host = query
        .get("host")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let path = query
        .get("path")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let service_name = query
        .get("serviceName")
        .or_else(|| query.get("service_name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| path.clone());

    let mut stream = serde_json::Map::new();
    stream.insert("network".to_string(), Value::String(network.clone()));
    let network_has_options = insert_url_network_settings(
        &mut stream,
        &network,
        &query,
        host.as_deref(),
        path.as_deref(),
        service_name.as_deref(),
    );
    let security_has_options =
        insert_url_security_settings(&mut stream, &security, url, &query, host.as_deref());
    let has_non_default_options =
        network != "tcp" || security != "none" || network_has_options || security_has_options;

    if has_non_default_options {
        Some(Value::Object(stream))
    } else {
        None
    }
}

fn insert_url_network_settings(
    stream: &mut serde_json::Map<String, Value>,
    network: &str,
    query: &HashMap<String, String>,
    host: Option<&str>,
    path: Option<&str>,
    service_name: Option<&str>,
) -> bool {
    match network {
        "ws" => {
            let mut settings = serde_json::Map::new();
            if let Some(path) = path {
                settings.insert("path".to_string(), Value::String(path.to_string()));
            }
            if let Some(host) = host {
                settings.insert("headers".to_string(), json!({ "Host": host }));
            }
            if settings.is_empty() {
                false
            } else {
                stream.insert("wsSettings".to_string(), Value::Object(settings));
                true
            }
        }
        "grpc" => {
            stream.insert(
                "grpcSettings".to_string(),
                json!({
                    "serviceName": service_name.unwrap_or_default(),
                    "multiMode": query_flag_true(query, "multiMode")
                }),
            );
            true
        }
        "httpupgrade" => {
            let mut settings = serde_json::Map::new();
            if let Some(host) = host {
                settings.insert("host".to_string(), Value::String(host.to_string()));
            }
            if let Some(path) = path {
                settings.insert("path".to_string(), Value::String(path.to_string()));
            }
            if settings.is_empty() {
                false
            } else {
                stream.insert("httpupgradeSettings".to_string(), Value::Object(settings));
                true
            }
        }
        _ => false,
    }
}

fn insert_url_security_settings(
    stream: &mut serde_json::Map<String, Value>,
    security: &str,
    url: &Url,
    query: &HashMap<String, String>,
    host: Option<&str>,
) -> bool {
    let mut settings = serde_json::Map::new();
    let server_name = query
        .get("sni")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| host.map(str::to_string))
        .or_else(|| url.host_str().map(str::to_string));
    if let Some(server_name) = server_name {
        settings.insert("serverName".to_string(), Value::String(server_name));
    }
    match security {
        "tls" => {
            if query_flag_true(query, "allowInsecure") || query_flag_true(query, "insecure") {
                settings.insert("allowInsecure".to_string(), Value::Bool(true));
            }
            insert_query_string_setting(&mut settings, query, "fp", "fingerprint");
            if let Some(alpn) = query
                .get("alpn")
                .map(|value| parse_alpn_csv(value))
                .filter(|items| !items.is_empty())
            {
                settings.insert("alpn".to_string(), json!(alpn));
            }
            insert_stream_security_object(stream, "tlsSettings", settings)
        }
        "reality" => {
            insert_query_string_setting(&mut settings, query, "fp", "fingerprint");
            insert_query_string_setting(&mut settings, query, "pbk", "publicKey");
            insert_query_string_setting(&mut settings, query, "sid", "shortId");
            insert_query_string_setting(&mut settings, query, "spx", "spiderX");
            insert_stream_security_object(stream, "realitySettings", settings)
        }
        _ => false,
    }
}

fn insert_query_string_setting(
    settings: &mut serde_json::Map<String, Value>,
    query: &HashMap<String, String>,
    short_key: &str,
    output_key: &str,
) {
    let value = query
        .get(short_key)
        .or_else(|| query.get(output_key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(value) = value {
        settings.insert(output_key.to_string(), Value::String(value));
    }
}

fn insert_stream_security_object(
    stream: &mut serde_json::Map<String, Value>,
    key: &str,
    settings: serde_json::Map<String, Value>,
) -> bool {
    if settings.is_empty() {
        false
    } else {
        stream.insert(key.to_string(), Value::Object(settings));
        true
    }
}
