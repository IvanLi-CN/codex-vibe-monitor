use anyhow::{Context, Result, bail};
use axum::{
    Json,
    body::{Body, Bytes},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::TryStreamExt;
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use reqwest::{Body as ReqwestBody, Client, Url};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::time::{Instant, timeout};
use tracing::{info, warn};

#[cfg(test)]
use once_cell::sync::Lazy;
#[cfg(test)]
use std::sync::Mutex as StdMutex;

include!("oauth_bridge/part_01.rs");
include!("oauth_bridge/part_02.rs");
include!("oauth_bridge/part_03.rs");
include!("oauth_bridge/part_04.rs");
