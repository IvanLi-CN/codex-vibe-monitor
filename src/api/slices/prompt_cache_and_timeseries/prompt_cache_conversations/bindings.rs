use super::*;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::{BTreeMap, HashSet};

include!("bindings/part-01.rs");
include!("bindings/part-02.rs");
include!("bindings/part-03.rs");
include!("bindings/part-04.rs");
include!("bindings/part-05.rs");
include!("bindings/part-06.rs");
