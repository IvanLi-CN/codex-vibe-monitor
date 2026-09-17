use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tracing::{debug, warn};

include!("system_routes_and_tasks/part-01.rs");
include!("system_routes_and_tasks/part-02.rs");
include!("system_routes_and_tasks/part-03.rs");
include!("system_routes_and_tasks/part-04.rs");
include!("system_routes_and_tasks/part-05.rs");
include!("system_routes_and_tasks/part-06.rs");
