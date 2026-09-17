use super::prompt_cache_and_timeseries_shared as prompt_shared;
use super::*;
use anyhow::anyhow;
use std::time::{Duration, Instant};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
use tokio::{
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tracing::{debug, warn};
include!("timeseries/part-01.rs");
include!("timeseries/part-02.rs");
include!("timeseries/part-03.rs");
include!("timeseries/part-04.rs");
include!("timeseries/part-05.rs");
include!("timeseries/part-06.rs");
include!("timeseries/part-07.rs");
include!("timeseries/part-08.rs");
#[cfg(test)]
mod tests {
    include!("timeseries/tests/part-01.rs");
    include!("timeseries/tests/part-02.rs");
}
