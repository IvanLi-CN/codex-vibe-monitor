use super::*;
use axum::response::Redirect;
use axum::routing::get;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpListener;

#[tokio::test]
async fn models_dev_http_client_does_not_follow_redirects() {
    let redirected_requests = Arc::new(AtomicUsize::new(0));
    let target_hits = Arc::clone(&redirected_requests);
    let target_app = Router::new().route(
        "/",
        get(move || {
            let target_hits = Arc::clone(&target_hits);
            async move {
                target_hits.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        }),
    );
    let target_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect target");
    let target_addr = target_listener
        .local_addr()
        .expect("redirect target address");
    let target_handle = tokio::spawn(async move {
        axum::serve(target_listener, target_app)
            .await
            .expect("redirect target should run");
    });

    let target_url = format!("http://{target_addr}/");
    let redirect_app = Router::new().route(
        "/",
        get(move || {
            let target_url = target_url.clone();
            async move { Redirect::temporary(&target_url) }
        }),
    );
    let redirect_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect source");
    let redirect_addr = redirect_listener
        .local_addr()
        .expect("redirect source address");
    let redirect_handle = tokio::spawn(async move {
        axum::serve(redirect_listener, redirect_app)
            .await
            .expect("redirect source should run");
    });

    let client = HttpClients::build(&test_config())
        .expect("build HTTP clients")
        .models_dev;
    let response = client
        .get(format!("http://{redirect_addr}/"))
        .send()
        .await
        .expect("read redirect response");

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(redirected_requests.load(Ordering::SeqCst), 0);

    redirect_handle.abort();
    target_handle.abort();
}
