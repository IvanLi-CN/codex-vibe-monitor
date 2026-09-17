use super::*;
use crate::db_pressure::{DbPressureDenyReason, DbPressureGate};
use axum::http::header;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde::ser::{SerializeSeq, SerializeStruct};
use serde_json::json;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{
    Mutex as StdMutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use tokio::sync::Notify;
include!("subscriptions/part-01.rs");
include!("subscriptions/part-02.rs");
include!("subscriptions/part-03.rs");
include!("subscriptions/part-04.rs");
include!("subscriptions/part-05.rs");
include!("subscriptions/part-06.rs");
include!("subscriptions/part-07.rs");
include!("subscriptions/part-08.rs");
include!("subscriptions/part-09.rs");
include!("subscriptions/part-10.rs");
include!("subscriptions/part-11.rs");
include!("subscriptions/part-12.rs");
include!("subscriptions/part-13.rs");
include!("subscriptions/part-14.rs");
include!("subscriptions/part-15.rs");
include!("subscriptions/part-16.rs");
include!("subscriptions/part-17.rs");
include!("subscriptions/part-18.rs");
include!("subscriptions/part-19.rs");
include!("subscriptions/part-20.rs");
include!("subscriptions/part-21.rs");
#[cfg(test)]
mod tests {
    include!("subscriptions/tests/part-01.rs");
    include!("subscriptions/tests/part-02.rs");
    include!("subscriptions/tests/part-03.rs");
    include!("subscriptions/tests/part-04.rs");
    include!("subscriptions/tests/part-05.rs");
    include!("subscriptions/tests/part-06.rs");
}
