use super::*;
use axum::{Router, http::StatusCode, routing::any};
use serde_json::json;
use std::{
    env, fs,
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{net::TcpListener, process::Command};

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires an installed Xray binary; run explicitly with XRAY_BINARY"]
async fn vless_xhttp_validation_pipeline_reaches_local_upstream_without_direct_fallback() {
    let xray_binary = env::var("XRAY_BINARY").unwrap_or_else(|_| "/opt/homebrew/bin/xray".into());
    if !Path::new(&xray_binary).is_file() {
        eprintln!("skipping XHTTP validation smoke: Xray binary is unavailable");
        return;
    }

    let temp_root = make_temp_test_dir("xhttp-validation-pipeline");
    let runtime_dir = temp_root.join("runtime");
    let upstream_hits = Arc::new(AtomicUsize::new(0));
    let hit_counter = upstream_hits.clone();
    let upstream_app = Router::new().route(
        "/v1/models",
        any(move || {
            hit_counter.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::OK }
        }),
    );
    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind synthetic validation upstream");
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("read synthetic validation upstream address");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .await
            .expect("synthetic validation upstream should run");
    });

    let server_port = pick_unused_local_port().expect("allocate synthetic XHTTP server port");
    let server_config_path = temp_root.join("xhttp-server.json");
    let server_config = json!({
        "log": { "loglevel": "none" },
        "inbounds": [{
            "listen": "127.0.0.1",
            "port": server_port,
            "protocol": "vless",
            "settings": {
                "clients": [{ "id": "11111111-1111-1111-1111-111111111111" }],
                "decryption": "none"
            },
            "streamSettings": {
                "network": "xhttp",
                "xhttpSettings": {
                    "path": "/xp-xhttp",
                    "mode": "stream-one"
                }
            }
        }],
        "outbounds": [{ "protocol": "freedom" }]
    });
    fs::write(
        &server_config_path,
        serde_json::to_vec_pretty(&server_config).expect("serialize synthetic XHTTP server config"),
    )
    .expect("write synthetic XHTTP server config");
    let mut server_child = Command::new(&xray_binary)
        .arg("run")
        .arg("-c")
        .arg(&server_config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("start synthetic XHTTP server");

    let server_addr = format!("127.0.0.1:{server_port}");
    let mut server_ready = false;
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(&server_addr).await.is_ok() {
            server_ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(server_ready, "synthetic XHTTP server did not become ready");

    let mut config = test_config();
    config.xray_binary = xray_binary;
    config.xray_runtime_dir = runtime_dir;
    config.openai_upstream_base_url = Url::parse(&format!("http://{upstream_addr}/"))
        .expect("valid synthetic validation upstream base url");
    let state = test_state_from_config(config, false).await;
    let endpoint = ForwardProxyEndpoint {
        key: "xhttp-validation-pipeline".to_string(),
        source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
        display_name: "xhttp-validation-pipeline".to_string(),
        protocol: ForwardProxyProtocol::Vless,
        endpoint_url: None,
        raw_url: Some(format!(
            "vless://11111111-1111-1111-1111-111111111111@{server_addr}?encryption=none&security=none&type=xhttp&path=%2Fxp-xhttp&mode=stream-one"
        )),
    };

    let result =
        probe_forward_proxy_endpoint(state.as_ref(), &endpoint, Duration::from_secs(10), None)
            .await
            .expect("actual XHTTP validation route should complete");
    assert!(
        result.is_some(),
        "actual XHTTP route should reach the upstream"
    );
    let positive_hits = upstream_hits.load(Ordering::SeqCst);
    assert!(
        positive_hits > 0,
        "XHTTP server must forward the validation request"
    );
    for _ in 0..100 {
        if state.xray_supervisor.lock().await.instances.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        state.xray_supervisor.lock().await.instances.is_empty(),
        "validation should clean the temporary Xray route"
    );

    server_child
        .kill()
        .await
        .expect("stop synthetic XHTTP server");
    let _ = server_child.wait().await;

    let broken_endpoint = ForwardProxyEndpoint {
        key: "xhttp-validation-direct-fallback-control".to_string(),
        source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
        display_name: "xhttp-validation-direct-fallback-control".to_string(),
        protocol: ForwardProxyProtocol::Vless,
        endpoint_url: None,
        raw_url: Some(format!(
            "vless://11111111-1111-1111-1111-111111111111@{server_addr}?encryption=none&security=none&type=xhttp&path=%2Fxp-xhttp&mode=stream-one"
        )),
    };
    let error = probe_forward_proxy_endpoint(
        state.as_ref(),
        &broken_endpoint,
        Duration::from_secs(3),
        None,
    )
    .await
    .expect_err("a failed XHTTP route must not fall back to direct upstream access");
    assert!(
        format!("{error:#}").contains("proxy connection failed")
            || format!("{error:#}").contains("timed out"),
        "broken XHTTP route should report a proxy failure: {error:#}"
    );
    assert_eq!(
        upstream_hits.load(Ordering::SeqCst),
        positive_hits,
        "a failed XHTTP route must not reach the upstream through direct fallback"
    );
    for _ in 0..100 {
        if state.xray_supervisor.lock().await.instances.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        state.xray_supervisor.lock().await.instances.is_empty(),
        "failed validation should clean its temporary Xray route"
    );

    upstream_handle.abort();
    cleanup_temp_test_dir(&temp_root);
}
