#![recursion_limit = "256"]
#![expect(
    dead_code,
    reason = "All-target Clippy checks production and test compilation units separately; shared internal helpers are intentionally target-dependent."
)]

use anyhow::{Context, Result, anyhow, bail};
use async_compression::{
    Level as AsyncCompressionLevel,
    tokio::bufread::{
        DeflateDecoder as AsyncDeflateDecoder, GzipDecoder as AsyncGzipDecoder,
        GzipEncoder as AsyncGzipEncoder, ZlibDecoder as AsyncZlibDecoder,
        ZlibEncoder as AsyncZlibEncoder, ZstdDecoder as AsyncZstdDecoder,
        ZstdEncoder as AsyncZstdEncoder,
    },
};
use axum::response::sse::{Event, KeepAlive};
use axum::{
    Router,
    body::{Body, Bytes, HttpBody},
    extract::ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
    extract::{
        ConnectInfo, DefaultBodyLimit, Extension, FromRequest, FromRequestParts, OriginalUri,
        Path as AxumPath, Query, State,
    },
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, Uri, uri::Authority},
    response::{Html, IntoResponse, Json, Response, Sse},
    routing::{any, delete, get, post, put},
};
use base64::Engine;
use brotli::{Decompressor as BrotliDecompressor, DecompressorWriter};
use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveDateTime,
    SecondsFormat, TimeZone, Utc,
};
use chrono_tz::{Asia::Shanghai, Tz};
use clap::{Args, Parser, Subcommand};
use crc32fast::Hasher as Crc32Hasher;
use dotenvy::dotenv;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::{
    Compression, Decompress, FlushDecompress,
    write::{GzDecoder as WriteGzipDecoder, GzEncoder},
};
use futures_util::{FutureExt, SinkExt, StreamExt, TryStreamExt, future::Shared, stream};
use once_cell::sync::Lazy;
use rand::Rng;
use regex::Regex;
use reqwest::{Client, ClientBuilder, Proxy, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    Connection, FromRow, Pool, QueryBuilder, Row, Sqlite, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::fs;
use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};
use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    convert::Infallible,
    env, fmt,
    future::Future,
    hash::{Hash, Hasher},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    sync::{Mutex, Notify, RwLock, Semaphore, broadcast, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{MissedTickBehavior, interval, sleep, timeout},
};
use tokio_rustls::TlsConnector;
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, client_async_tls_with_config};
use tokio_util::io::{ReaderStream, StreamReader};
use tokio_util::sync::CancellationToken;
use tower::{ServiceExt, service_fn};
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};
use tracing::{debug, error, info, warn};
use tungstenite::{
    Message as TungsteniteMessage, client::IntoClientRequest, http::Request as TungsteniteRequest,
};
mod api;
mod app_state;
mod config;
mod dashboard_network_speed;
mod db_pressure;
mod external_api;
mod forward_proxy;
mod http_stream_tracking;
mod long_term_stats;
mod maintenance;
mod memory_diagnostics;
#[expect(
    clippy::too_many_arguments,
    reason = "OAuth bridge adapters preserve upstream request contracts."
)]
mod oauth_bridge;
mod pricing;
mod proxy;
mod proxy_sqlite_write_coordinator;
#[expect(
    clippy::too_many_arguments,
    reason = "Runtime shutdown coordination preserves established task handles."
)]
mod runtime;
mod runtime_mutation_bus;
mod schema;
mod share_links;
mod summary_source_change;
pub(crate) use dashboard_network_speed::*;
#[expect(
    clippy::large_enum_variant,
    reason = "Batch variants preserve established channel payload ownership."
)]
mod sqlite_batch_writer;
#[expect(
    clippy::type_complexity,
    reason = "Statistics row tuples mirror persisted query shapes."
)]
mod stats;
mod terminal_journal;
mod terminal_projection;
mod test_support;
mod upstream_accounts;

use api::*;
pub(crate) use app_state::*;
pub(crate) use config::*;
use external_api::*;
use forward_proxy::*;
use http_stream_tracking::*;
pub(crate) use long_term_stats::*;
pub(crate) use maintenance::*;
pub(crate) use memory_diagnostics::*;
pub(crate) use pricing::*;
use proxy::*;
pub(crate) use runtime_mutation_bus::*;
pub(crate) use schema::*;
pub(crate) use share_links::*;
use sqlite_batch_writer::*;
use stats::*;
pub(crate) use summary_source_change::*;
pub(crate) use terminal_projection::*;
#[allow(unused_imports)]
pub(crate) use test_support::tests;
use upstream_accounts::*;
include!("main/runtime_items.rs");
