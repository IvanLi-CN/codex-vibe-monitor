//! Historical analytics build contracts.
//!
//! This module deliberately keeps historical input separate from the continuous ingest actor.
//! A build is admitted only with a source-bound snapshot, authenticated artifacts, explicit
//! coverage, and a certificate that proves the same source fence was used throughout.

mod certificate;
mod coverage;
mod importer;
mod seed;
mod snapshot;

#[allow(unused_imports)]
pub(crate) use certificate::{
    BucketEntityProof, BucketProof, CertificateBaseline, ReconciliationCertificate,
    SourceEventProof, build_reconciliation_certificate,
};
#[allow(unused_imports)]
pub(crate) use coverage::{
    CoverageClaim, CoverageKey, CoverageKind, CoverageLedger, CoverageRange, SeedCapability,
};
#[allow(unused_imports)]
pub(crate) use importer::{
    BoundedImport, HistoryArtifact, HistoryFact, HistoryImporter, ImportCursor, ImportOutcome,
    ImportSource, StableEntityIdentity,
};
#[allow(unused_imports)]
pub(crate) use seed::{
    HistoricalSeedInput, HistoricalSeedProof, SeedRow, authenticate_historical_seed,
    create_historical_seed_proof,
};
#[allow(unused_imports)]
pub(crate) use snapshot::{
    ArtifactIdentity, ArtifactKind, BuildSnapshot, ManifestIdentity, SnapshotObservation,
    SourceFence, StableArtifact, read_verified_artifact,
};
