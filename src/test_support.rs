#[cfg(test)]
use crate::*;

#[cfg(test)]
#[path = "tests/mod.rs"]
pub(crate) mod tests;

#[cfg(not(test))]
pub(crate) mod tests {}
