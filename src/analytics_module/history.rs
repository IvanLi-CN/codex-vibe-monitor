//! Historical analytics build contracts.
//!
//! This module deliberately keeps historical input separate from the continuous ingest actor.
//! A build is admitted only with a source-bound snapshot, authenticated artifacts, explicit
//! coverage, and a certificate that proves the same source fence was used throughout.

pub(crate) mod certificate;
pub(crate) mod coverage;
pub(crate) mod importer;
pub(crate) mod seed;
pub(crate) mod snapshot;
