#[derive(Debug, Clone)]
pub(crate) struct SelectedForwardProxy {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<Url>,
    pub(crate) endpoint_url_raw: Option<String>,
}

impl SelectedForwardProxy {
    pub(crate) fn from_endpoint(endpoint: &ForwardProxyEndpoint) -> Self {
        Self {
            key: endpoint.key.clone(),
            source: endpoint.source.clone(),
            display_name: endpoint.display_name.clone(),
            endpoint_url: endpoint.endpoint_url.clone(),
            endpoint_url_raw: endpoint.raw_url.clone(),
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
        Some(Value::Object(stream))
    } else {
        None
    }
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
    fn binding_keys_escalate_from_name_to_protocol_to_host_port() {
        let protocol_split_manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![
                    "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#Shared%20Node".to_string(),
                    "ss://2022-blake3-aes-128-gcm:secret@ss.example.com:8388#Shared%20Node".to_string(),
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
                    "ss://2022-blake3-aes-128-gcm:secret@jp-a.example.com:8388#Shared%20Node"
                        .to_string(),
                    "ss://2022-blake3-aes-128-gcm:secret@jp-b.example.com:8388#Shared%20Node"
                        .to_string(),
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
            manager.current_bound_group_binding_key("latam", &[node.key.clone()]),
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
}
