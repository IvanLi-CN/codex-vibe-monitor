pub(crate) fn mark_websocket_payload_transport(payload: String) -> Result<String> {
    let mut value = serde_json::from_str::<Value>(&payload)
        .context("failed to parse websocket proxy payload summary")?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "transport".to_string(),
            Value::String("websocket".to_string()),
        );
    }
    serde_json::to_string(&value).context("failed to serialize websocket proxy payload summary")
}

pub(crate) fn ws_terminal_event_failure_kind(event: &WsUsageEvent) -> Option<&'static str> {
    if event.event_type == "response.failed" {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else {
        None
    }
}

pub(crate) fn build_websocket_upstream_url(base: &Url, original_uri: &Uri) -> Result<Url> {
    let mut ws_base = base.clone();
    let ws_scheme = match base.scheme() {
        "https" | "wss" => "wss",
        "http" | "ws" => "ws",
        scheme => bail!("unsupported websocket upstream base scheme: {scheme}"),
    };
    ws_base
        .set_scheme(ws_scheme)
        .map_err(|_| anyhow!("failed to set websocket upstream scheme"))?;
    build_proxy_upstream_url(&ws_base, original_uri)
}

pub(crate) fn rewrite_websocket_upstream_url_model(url: &mut Url, target_model: &str) -> bool {
    let query_pairs = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if !query_pairs.iter().any(|(key, _)| key == "model") {
        return false;
    }
    let mut query = url.query_pairs_mut();
    query.clear();
    for (key, value) in query_pairs {
        query.append_pair(
            &key,
            if key == "model" {
                target_model
            } else {
                value.as_str()
            },
        );
    }
    true
}

pub(crate) fn build_upstream_ws_request(
    upstream_url: &Url,
    headers: &HeaderMap,
    account: &PoolResolvedAccount,
    force_responses_beta: bool,
) -> Result<TungsteniteRequest<()>> {
    let mut request = upstream_url
        .as_str()
        .into_client_request()
        .context("failed to create websocket client request")?;
    let connection_scoped = connection_scoped_header_names(headers);
    for (name, value) in headers {
        if *name == header::AUTHORIZATION || *name == header::CONTENT_LENGTH {
            continue;
        }
        if should_forward_websocket_header(name, &connection_scoped) {
            request.headers_mut().insert(name.clone(), value.clone());
        }
    }
    let authorization = match &account.auth {
        PoolResolvedAuth::ApiKey { authorization } => authorization.clone(),
        PoolResolvedAuth::Oauth {
            access_token,
            chatgpt_account_id,
        } => {
            if force_responses_beta {
                request.headers_mut().insert(
                    HeaderName::from_static("openai-beta"),
                    HeaderValue::from_static("responses=experimental"),
                );
            }
            if let Some(account_id) = chatgpt_account_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                request.headers_mut().insert(
                    HeaderName::from_static("chatgpt-account-id"),
                    HeaderValue::from_str(account_id)
                        .context("invalid ChatGPT account header value")?,
                );
            }
            format!("Bearer {access_token}")
        }
    };
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&authorization).context("invalid upstream authorization header")?,
    );
    Ok(request)
}

pub(crate) async fn connect_upstream_websocket(
    request: TungsteniteRequest<()>,
    upstream_url: &Url,
    forward_proxy_url: Option<&Url>,
    meter: UpstreamSocketByteMeter,
) -> std::result::Result<
    (UpstreamWsStream, tungstenite::handshake::client::Response),
    tungstenite::Error,
> {
    let Some(forward_proxy_url) = forward_proxy_url else {
        let stream = CountedIo::new(connect_tcp_target(upstream_url).await?, meter);
        return client_async_tls_with_config(request, Box::new(stream) as BoxedWsIo, None, None)
            .await;
    };

    let proxy_host = forward_proxy_url.host_str().ok_or_else(|| {
        tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "forward proxy endpoint is missing host",
        ))
    })?;
    let proxy_port = forward_proxy_url.port_or_known_default().ok_or_else(|| {
        tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "forward proxy endpoint is missing port",
        ))
    })?;
    let upstream_host = upstream_url
        .host_str()
        .ok_or_else(|| tungstenite::Error::Url(tungstenite::error::UrlError::NoHostName))?;
    let upstream_port = upstream_url
        .port_or_known_default()
        .ok_or(tungstenite::Error::Url(
            tungstenite::error::UrlError::UnsupportedUrlScheme,
        ))?;
    let target_authority = if upstream_host.contains(':') {
        format!("[{upstream_host}]:{upstream_port}")
    } else {
        format!("{upstream_host}:{upstream_port}")
    };
    let proxy_scheme = forward_proxy_url.scheme();
    if matches!(proxy_scheme, "socks5" | "socks5h") {
        let socks_target_host = if proxy_scheme == "socks5" {
            resolve_socks5_local_target_host(upstream_host, upstream_port).await?
        } else {
            upstream_host.to_string()
        };
        let stream = connect_socks5_forward_proxy(
            forward_proxy_url,
            proxy_host,
            proxy_port,
            &socks_target_host,
            upstream_port,
            meter.clone(),
        )
        .await?;
        return client_async_tls_with_config(request, stream, None, None).await;
    }
    if !matches!(proxy_scheme, "http" | "https") {
        return Err(tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "websocket proxy only supports HTTP CONNECT, HTTPS CONNECT, or SOCKS5 forward proxy endpoints, got {proxy_scheme}"
            ),
        )));
    }

    let stream = connect_http_forward_proxy_tunnel(
        forward_proxy_url,
        proxy_host,
        proxy_port,
        &target_authority,
        meter,
    )
    .await?;
    client_async_tls_with_config(request, stream, None, None).await
}

async fn connect_http_forward_proxy_tunnel(
    forward_proxy_url: &Url,
    proxy_host: &str,
    proxy_port: u16,
    target_authority: &str,
    meter: UpstreamSocketByteMeter,
) -> std::result::Result<BoxedWsIo, tungstenite::Error> {
    let mut stream =
        connect_http_forward_proxy(forward_proxy_url, proxy_host, proxy_port, meter).await?;
    let connect_request = build_http_connect_request(forward_proxy_url, target_authority);
    stream
        .write_all(connect_request.as_bytes())
        .await
        .map_err(tungstenite::Error::Io)?;

    let response = read_http_connect_response(&mut stream).await?;
    let extra_read = validate_http_connect_response(&response)?;
    if extra_read.is_empty() {
        Ok(stream)
    } else {
        Ok(Box::new(PrefixedIo::new(extra_read, stream)))
    }
}

fn build_http_connect_request(forward_proxy_url: &Url, target_authority: &str) -> String {
    let mut connect_request =
        format!("CONNECT {target_authority} HTTP/1.1\r\nHost: {target_authority}\r\n");
    if let Some(credential) = forward_proxy_basic_auth_credential(forward_proxy_url) {
        let encoded = base64::engine::general_purpose::STANDARD.encode(credential);
        connect_request.push_str("Proxy-Authorization: Basic ");
        connect_request.push_str(&encoded);
        connect_request.push_str("\r\n");
    }
    connect_request.push_str("\r\n");
    connect_request
}

async fn read_http_connect_response(
    stream: &mut BoxedWsIo,
) -> std::result::Result<Vec<u8>, tungstenite::Error> {
    let mut response = Vec::with_capacity(256);
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(tungstenite::Error::Io)?;
        if read == 0 {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "forward proxy closed before CONNECT response completed",
            )));
        }
        response.extend_from_slice(&buffer[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(response);
        }
        if response.len() > 16 * 1024 {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response exceeded 16KiB",
            )));
        }
    }
}

fn validate_http_connect_response(
    response: &[u8],
) -> std::result::Result<Vec<u8>, tungstenite::Error> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response missing header terminator",
            ))
        })?;
    let status_line_end = response
        .windows(2)
        .position(|window| window == b"\r\n")
        .ok_or_else(|| {
            tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "forward proxy CONNECT response missing status line",
            ))
        })?;
    let status_line = std::str::from_utf8(&response[..status_line_end]).map_err(|err| {
        tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("forward proxy CONNECT status line is not UTF-8: {err}"),
        ))
    })?;
    if !status_line.starts_with("HTTP/1.1 200") && !status_line.starts_with("HTTP/1.0 200") {
        return Err(tungstenite::Error::Io(io::Error::other(format!(
            "forward proxy CONNECT failed: {status_line}"
        ))));
    }
    Ok(response[(header_end + 4)..].to_vec())
}

pub(crate) async fn connect_tcp_target(
    upstream_url: &Url,
) -> std::result::Result<TcpStream, tungstenite::Error> {
    let host = upstream_url
        .host_str()
        .ok_or_else(|| tungstenite::Error::Url(tungstenite::error::UrlError::NoHostName))?;
    let port = upstream_url
        .port_or_known_default()
        .ok_or(tungstenite::Error::Url(
            tungstenite::error::UrlError::UnsupportedUrlScheme,
        ))?;
    TcpStream::connect((host, port))
        .await
        .map_err(tungstenite::Error::Io)
}

pub(crate) async fn connect_http_forward_proxy(
    forward_proxy_url: &Url,
    proxy_host: &str,
    proxy_port: u16,
    meter: UpstreamSocketByteMeter,
) -> std::result::Result<BoxedWsIo, tungstenite::Error> {
    let stream = CountedIo::new(
        TcpStream::connect((proxy_host, proxy_port))
            .await
            .map_err(tungstenite::Error::Io)?,
        meter,
    );
    if forward_proxy_url.scheme() != "https" {
        return Ok(Box::new(stream));
    }
    let root_store = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name =
        rustls_pki_types::ServerName::try_from(proxy_host.to_string()).map_err(|err| {
            tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid HTTPS forward proxy host for TLS SNI: {err}"),
            ))
        })?;
    let stream = connector
        .connect(server_name, stream)
        .await
        .map_err(tungstenite::Error::Io)?;
    Ok(Box::new(stream))
}

pub(crate) async fn connect_socks5_forward_proxy(
    forward_proxy_url: &Url,
    proxy_host: &str,
    proxy_port: u16,
    upstream_host: &str,
    upstream_port: u16,
    meter: UpstreamSocketByteMeter,
) -> std::result::Result<BoxedWsIo, tungstenite::Error> {
    let mut stream = CountedIo::new(
        TcpStream::connect((proxy_host, proxy_port))
            .await
            .map_err(tungstenite::Error::Io)?,
        meter,
    );
    negotiate_socks5_forward_proxy_auth(&mut stream, forward_proxy_url).await?;
    let connect_request = build_socks5_connect_request(upstream_host, upstream_port)?;
    stream
        .write_all(&connect_request)
        .await
        .map_err(tungstenite::Error::Io)?;

    consume_socks5_connect_reply(&mut stream).await?;
    Ok(Box::new(stream))
}

async fn negotiate_socks5_forward_proxy_auth<S>(
    stream: &mut S,
    forward_proxy_url: &Url,
) -> std::result::Result<(), tungstenite::Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let username = forward_proxy_username(forward_proxy_url);
    let password = forward_proxy_password(forward_proxy_url).unwrap_or_default();
    let method_request = if username.is_empty() {
        [0x05, 0x01, 0x00].as_slice()
    } else {
        [0x05, 0x02, 0x00, 0x02].as_slice()
    };
    stream
        .write_all(method_request)
        .await
        .map_err(tungstenite::Error::Io)?;
    let mut method_response = [0_u8; 2];
    stream
        .read_exact(&mut method_response)
        .await
        .map_err(tungstenite::Error::Io)?;
    validate_socks5_auth_method(method_response, &username, &password, stream).await
}

async fn validate_socks5_auth_method<S>(
    method_response: [u8; 2],
    username: &str,
    password: &str,
    stream: &mut S,
) -> std::result::Result<(), tungstenite::Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if method_response[0] != 0x05 || method_response[1] == 0xff {
        return Err(tungstenite::Error::Io(io::Error::other(
            "SOCKS5 forward proxy did not accept an authentication method",
        )));
    }
    match method_response[1] {
        0x00 => Ok(()),
        0x02 => authenticate_socks5_username_password(stream, username, password).await,
        method => Err(tungstenite::Error::Io(io::Error::other(format!(
            "SOCKS5 forward proxy selected unsupported authentication method {method}"
        )))),
    }
}

async fn authenticate_socks5_username_password<S>(
    stream: &mut S,
    username: &str,
    password: &str,
) -> std::result::Result<(), tungstenite::Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let username = username.as_bytes();
    let password = password.as_bytes();
    if username.len() > u8::MAX as usize || password.len() > u8::MAX as usize {
        return Err(tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SOCKS5 credentials exceed 255 bytes",
        )));
    }
    let mut auth_request = Vec::with_capacity(username.len() + password.len() + 3);
    auth_request.extend_from_slice(&[0x01, username.len() as u8]);
    auth_request.extend_from_slice(username);
    auth_request.push(password.len() as u8);
    auth_request.extend_from_slice(password);
    stream
        .write_all(&auth_request)
        .await
        .map_err(tungstenite::Error::Io)?;
    let mut auth_response = [0_u8; 2];
    stream
        .read_exact(&mut auth_response)
        .await
        .map_err(tungstenite::Error::Io)?;
    if auth_response == [0x01, 0x00] {
        Ok(())
    } else {
        Err(tungstenite::Error::Io(io::Error::other(
            "SOCKS5 forward proxy rejected username/password authentication",
        )))
    }
}

fn build_socks5_connect_request(
    upstream_host: &str,
    upstream_port: u16,
) -> std::result::Result<Vec<u8>, tungstenite::Error> {
    let mut connect_request = Vec::with_capacity(8 + upstream_host.len());
    connect_request.extend_from_slice(&[0x05, 0x01, 0x00]);
    if let Ok(ip) = upstream_host.parse::<IpAddr>() {
        append_socks5_ip_target(&mut connect_request, ip);
    } else {
        append_socks5_hostname_target(&mut connect_request, upstream_host)?;
    }
    connect_request.extend_from_slice(&upstream_port.to_be_bytes());
    Ok(connect_request)
}

fn append_socks5_ip_target(request: &mut Vec<u8>, ip: IpAddr) {
    match ip {
        IpAddr::V4(addr) => {
            request.push(0x01);
            request.extend_from_slice(&addr.octets());
        }
        IpAddr::V6(addr) => {
            request.push(0x04);
            request.extend_from_slice(&addr.octets());
        }
    }
}

fn append_socks5_hostname_target(
    request: &mut Vec<u8>,
    upstream_host: &str,
) -> std::result::Result<(), tungstenite::Error> {
    let host = upstream_host.as_bytes();
    if host.len() > u8::MAX as usize {
        return Err(tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SOCKS5 upstream host exceeds 255 bytes",
        )));
    }
    request.push(0x03);
    request.push(host.len() as u8);
    request.extend_from_slice(host);
    Ok(())
}

async fn consume_socks5_connect_reply<S>(
    stream: &mut S,
) -> std::result::Result<(), tungstenite::Error>
where
    S: AsyncRead + Unpin,
{
    let mut reply_head = [0_u8; 4];
    stream
        .read_exact(&mut reply_head)
        .await
        .map_err(tungstenite::Error::Io)?;
    if reply_head[0] != 0x05 || reply_head[1] != 0x00 {
        return Err(tungstenite::Error::Io(io::Error::other(format!(
            "SOCKS5 forward proxy connect failed with status {}",
            reply_head[1]
        ))));
    }
    let address_len = match reply_head[3] {
        0x01 => 4,
        0x03 => {
            let mut len = [0_u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(tungstenite::Error::Io)?;
            len[0] as usize
        }
        0x04 => 16,
        atyp => {
            return Err(tungstenite::Error::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("SOCKS5 forward proxy returned unsupported address type {atyp}"),
            )));
        }
    };
    let mut discard = vec![0_u8; address_len + 2];
    stream
        .read_exact(&mut discard)
        .await
        .map_err(tungstenite::Error::Io)?;
    Ok(())
}

pub(crate) async fn resolve_socks5_local_target_host(
    upstream_host: &str,
    upstream_port: u16,
) -> std::result::Result<String, tungstenite::Error> {
    if upstream_host.parse::<IpAddr>().is_ok() {
        return Ok(upstream_host.to_string());
    }
    let mut addresses = tokio::net::lookup_host((upstream_host, upstream_port))
        .await
        .map_err(tungstenite::Error::Io)?;
    let Some(address) = addresses.next() else {
        return Err(tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no local DNS address resolved for SOCKS5 target {upstream_host}"),
        )));
    };
    Ok(address.ip().to_string())
}

pub(crate) fn forward_proxy_basic_auth_credential(forward_proxy_url: &Url) -> Option<String> {
    let username = forward_proxy_username(forward_proxy_url);
    if username.is_empty() {
        return None;
    }
    Some(match forward_proxy_password(forward_proxy_url) {
        Some(password) => format!("{username}:{password}"),
        None => username,
    })
}

pub(crate) fn forward_proxy_username(forward_proxy_url: &Url) -> String {
    percent_decode_once_lossy(forward_proxy_url.username())
}

pub(crate) fn forward_proxy_password(forward_proxy_url: &Url) -> Option<String> {
    forward_proxy_url.password().map(percent_decode_once_lossy)
}

pub(crate) fn should_forward_websocket_header(
    name: &HeaderName,
    connection_scoped: &HashSet<HeaderName>,
) -> bool {
    should_forward_proxy_header(name, connection_scoped)
        && !matches!(
            name.as_str(),
            "sec-websocket-accept"
                | "sec-websocket-extensions"
                | "sec-websocket-key"
                | "sec-websocket-version"
        )
}

pub(crate) fn axum_to_tungstenite_message(message: AxumWsMessage) -> Option<TungsteniteMessage> {
    match message {
        AxumWsMessage::Text(value) => Some(TungsteniteMessage::Text(value.into())),
        AxumWsMessage::Binary(value) => Some(TungsteniteMessage::Binary(value.into())),
        AxumWsMessage::Ping(value) => Some(TungsteniteMessage::Ping(value.into())),
        AxumWsMessage::Pong(value) => Some(TungsteniteMessage::Pong(value.into())),
        AxumWsMessage::Close(frame) => Some(TungsteniteMessage::Close(frame.map(|frame| {
            tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::from(frame.code),
                reason: frame.reason.to_string().into(),
            }
        }))),
    }
}

pub(crate) fn tungstenite_to_axum_message(message: TungsteniteMessage) -> Option<AxumWsMessage> {
    match message {
        TungsteniteMessage::Text(value) => Some(AxumWsMessage::Text(value.to_string())),
        TungsteniteMessage::Binary(value) => Some(AxumWsMessage::Binary(value.to_vec())),
        TungsteniteMessage::Ping(value) => Some(AxumWsMessage::Ping(value.to_vec())),
        TungsteniteMessage::Pong(value) => Some(AxumWsMessage::Pong(value.to_vec())),
        TungsteniteMessage::Close(frame) => Some(AxumWsMessage::Close(frame.map(|frame| {
            axum::extract::ws::CloseFrame {
                code: u16::from(frame.code),
                reason: frame.reason.to_string().into(),
            }
        }))),
        TungsteniteMessage::Frame(_) => None,
    }
}
