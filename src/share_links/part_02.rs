#[derive(Debug, Clone)]
pub(crate) struct VmessShareLink {
    pub(crate) address: String,
    pub(crate) port: u16,
    pub(crate) id: String,
    pub(crate) alter_id: u32,
    pub(crate) security: String,
    pub(crate) network: String,
    pub(crate) host: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) tls_mode: Option<String>,
    pub(crate) sni: Option<String>,
    pub(crate) alpn: Option<Vec<String>>,
    pub(crate) fingerprint: Option<String>,
    pub(crate) header_type: Option<String>,
    pub(crate) service_name: Option<String>,
    pub(crate) authority: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) seed: Option<String>,
    pub(crate) display_name: String,
}

impl VmessShareLink {
    fn stable_identity(&self) -> String {
        let alpn = self
            .alpn
            .as_ref()
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        format!(
            "vmess://{}@{}:{}?aid={}&security={}&net={}&host={}&path={}&tls={}&sni={}&alpn={}&fp={}&type={}&serviceName={}&authority={}&mode={}&seed={}",
            self.id,
            self.address.to_ascii_lowercase(),
            self.port,
            self.alter_id,
            self.security.to_ascii_lowercase(),
            self.network.to_ascii_lowercase(),
            self.host
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.path.as_deref().unwrap_or_default(),
            self.tls_mode
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.sni.as_deref().unwrap_or_default().to_ascii_lowercase(),
            alpn,
            self.fingerprint
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.header_type
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.service_name.as_deref().unwrap_or_default(),
            self.authority.as_deref().unwrap_or_default(),
            self.mode
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.seed.as_deref().unwrap_or_default(),
        )
    }
}

pub(crate) fn parse_vmess_share_link(raw: &str) -> Result<VmessShareLink> {
    let payload = raw
        .strip_prefix("vmess://")
        .ok_or_else(|| anyhow!("invalid vmess share link"))?;
    let decoded =
        decode_base64_string(payload).ok_or_else(|| anyhow!("failed to decode vmess payload"))?;
    let value: Value = serde_json::from_str(&decoded).context("invalid vmess json payload")?;

    let address = required_vmess_string(&value, "add")?;
    let port =
        parse_port_value(value.get("port")).ok_or_else(|| anyhow!("vmess payload missing port"))?;
    let id = required_vmess_string(&value, "id")?;
    let alter_id = parse_u32_value(value.get("aid")).unwrap_or(0);
    let security = value
        .get("scy")
        .and_then(Value::as_str)
        .or_else(|| value.get("security").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("auto")
        .to_string();
    let network = value
        .get("net")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("tcp")
        .to_ascii_lowercase();
    let host = optional_vmess_string(&value, "host");
    let path = optional_vmess_string(&value, "path");
    let tls_mode = optional_vmess_string(&value, "tls").map(|value| value.to_ascii_lowercase());
    let sni = optional_vmess_string(&value, "sni");
    let alpn = value
        .get("alpn")
        .and_then(Value::as_str)
        .map(parse_alpn_csv)
        .filter(|items| !items.is_empty());
    let fingerprint = optional_vmess_string(&value, "fp");
    let header_type = optional_vmess_string(&value, "type");
    let service_name = optional_vmess_string(&value, "serviceName");
    let authority = optional_vmess_string(&value, "authority");
    let mode = optional_vmess_string(&value, "mode");
    let seed = optional_vmess_string(&value, "seed");
    let display_name =
        optional_vmess_string(&value, "ps").unwrap_or_else(|| format!("{address}:{port}"));

    Ok(VmessShareLink {
        address,
        port,
        id,
        alter_id,
        security,
        network,
        host,
        path,
        tls_mode,
        sni,
        alpn,
        fingerprint,
        header_type,
        service_name,
        authority,
        mode,
        seed,
        display_name,
    })
}

fn required_vmess_string(value: &Value, field: &str) -> Result<String> {
    optional_vmess_string(value, field).ok_or_else(|| anyhow!("vmess payload missing {field}"))
}

fn optional_vmess_string(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn parse_u32_value(value: Option<&Value>) -> Option<u32> {
    match value {
        Some(Value::Number(num)) => num.as_u64().and_then(|v| u32::try_from(v).ok()),
        Some(Value::String(raw)) => raw.trim().parse::<u32>().ok(),
        _ => None,
    }
}

pub(crate) fn parse_port_value(value: Option<&Value>) -> Option<u16> {
    match value {
        Some(Value::Number(num)) => num.as_u64().and_then(|v| u16::try_from(v).ok()),
        Some(Value::String(raw)) => raw.trim().parse::<u16>().ok(),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ShadowsocksShareLink {
    pub(crate) method: String,
    pub(crate) password: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) display_name: String,
}

impl ShadowsocksShareLink {
    fn stable_identity(&self) -> String {
        format!(
            "{}{}{}{}{}{}{}{}",
            "ss://",
            self.method.to_ascii_lowercase(),
            ":",
            self.password,
            "@",
            self.host.to_ascii_lowercase(),
            ":",
            self.port,
        )
    }
}

pub(crate) fn parse_shadowsocks_share_link(raw: &str) -> Result<ShadowsocksShareLink> {
    let normalized = raw
        .strip_prefix("ss://")
        .ok_or_else(|| anyhow!("invalid shadowsocks share link"))?;
    let (main, fragment) = split_once_first(normalized, '#');
    let (main, _) = split_once_first(main, '?');
    let display_name = fragment
        .map(percent_decode_once_lossy)
        .filter(|value| !value.trim().is_empty());

    if let Ok(url) = Url::parse(raw)
        && let Some(host) = url.host_str()
        && let Some(port) = url.port_or_known_default()
    {
        let credentials = if !url.username().is_empty() && url.password().is_some() {
            Some((
                percent_decode_once_lossy(url.username()),
                percent_decode_once_lossy(url.password().unwrap_or_default()),
            ))
        } else if !url.username().is_empty() {
            let username = percent_decode_once_lossy(url.username());
            decode_base64_string(&username).and_then(|decoded| {
                let (method, password) = decoded.split_once(':')?;
                Some((method.to_string(), password.to_string()))
            })
        } else {
            None
        };
        if let Some((method, password)) = credentials {
            return Ok(ShadowsocksShareLink {
                method,
                password,
                host: host.to_string(),
                port,
                display_name: display_name
                    .clone()
                    .unwrap_or_else(|| format!("{host}:{port}")),
            });
        }
    }

    let decoded_main = if main.contains('@') {
        main.to_string()
    } else {
        let main_for_decode = percent_decode_once_lossy(main);
        decode_base64_string(&main_for_decode)
            .ok_or_else(|| anyhow!("failed to decode shadowsocks payload"))?
    };

    let (credential, host_port) = decoded_main
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("invalid shadowsocks payload"))?;
    let (method, password) = if let Some((method, password)) = credential.split_once(':') {
        (
            percent_decode_once_lossy(method),
            percent_decode_once_lossy(password),
        )
    } else {
        let decoded_credential = decode_base64_string(credential)
            .ok_or_else(|| anyhow!("failed to decode shadowsocks credentials"))?;
        let (method, password) = decoded_credential
            .split_once(':')
            .ok_or_else(|| anyhow!("invalid shadowsocks credentials"))?;
        (
            percent_decode_once_lossy(method),
            percent_decode_once_lossy(password),
        )
    };
    let parsed_host = Url::parse(&format!("http://{host_port}"))
        .context("invalid shadowsocks server endpoint")?;
    let host = parsed_host
        .host_str()
        .ok_or_else(|| anyhow!("shadowsocks host missing"))?
        .to_string();
    let port = parsed_host
        .port_or_known_default()
        .ok_or_else(|| anyhow!("shadowsocks port missing"))?;
    let display_name = display_name.unwrap_or_else(|| format!("{host}:{port}"));
    Ok(ShadowsocksShareLink {
        method,
        password,
        host,
        port,
        display_name,
    })
}

pub(crate) fn split_once_first(raw: &str, delimiter: char) -> (&str, Option<&str>) {
    if let Some((lhs, rhs)) = raw.split_once(delimiter) {
        (lhs, Some(rhs))
    } else {
        (raw, None)
    }
}

pub(crate) fn parse_alpn_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn deterministic_unit_f64(seed: u64) -> f64 {
    let mut value = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51afd7ed558ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ceb9fe1a85ec53);
    value ^= value >> 33;
    (value as f64) / (u64::MAX as f64)
}
