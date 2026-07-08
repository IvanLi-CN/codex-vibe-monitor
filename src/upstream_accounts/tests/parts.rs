#[allow(unused_imports)]
use super::*;
pub(crate) use base64::Engine;
pub(crate) use base64::engine::general_purpose::URL_SAFE_NO_PAD;
pub(crate) use sqlx::SqlitePool;
pub(crate) use tokio::sync::Notify;

#[path = "../tests_part_1.rs"]
mod tests_part_1;
#[path = "../tests_part_2.rs"]
mod tests_part_2;
#[path = "../tests_part_3.rs"]
mod tests_part_3;
#[path = "../tests_part_4.rs"]
mod tests_part_4;
#[path = "../tests_part_5.rs"]
mod tests_part_5;
#[path = "../tests_part_6.rs"]
mod tests_part_6;
#[path = "../tests_part_7.rs"]
mod tests_part_7;

pub(crate) use tests_part_1::*;
pub(crate) use tests_part_2::*;
pub(crate) use tests_part_3::*;
pub(crate) use tests_part_4::*;
pub(crate) use tests_part_5::*;
pub(crate) use tests_part_6::*;
pub(crate) use tests_part_7::*;
