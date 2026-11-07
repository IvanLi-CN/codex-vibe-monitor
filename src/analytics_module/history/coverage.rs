use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoverageKey {
    pub(crate) metric: String,
    pub(crate) dimension: String,
    pub(crate) granularity: String,
}

impl CoverageKey {
    pub(crate) fn new(
        metric: impl Into<String>,
        dimension: impl Into<String>,
        granularity: impl Into<String>,
    ) -> Result<Self> {
        let key = Self {
            metric: metric.into(),
            dimension: dimension.into(),
            granularity: granularity.into(),
        };
        key.validate()?;
        Ok(key)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for (value, label) in [
            (&self.metric, "coverage metric"),
            (&self.dimension, "coverage dimension"),
            (&self.granularity, "coverage granularity"),
        ] {
            ensure!(!value.trim().is_empty(), "{label} is empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoverageRange {
    pub(crate) start: String,
    pub(crate) end: String,
}

impl CoverageRange {
    pub(crate) fn new(start: impl Into<String>, end: impl Into<String>) -> Result<Self> {
        let range = Self {
            start: start.into(),
            end: end.into(),
        };
        ensure!(
            !range.start.trim().is_empty(),
            "coverage range start is empty"
        );
        ensure!(!range.end.trim().is_empty(), "coverage range end is empty");
        ensure!(
            range.start < range.end,
            "coverage range must have a positive width"
        );
        Ok(range)
    }

    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    pub(crate) fn contains(&self, other: &Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CoverageKind {
    ExactFacts,
    CertifiedSeed,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SeedCapability {
    Calls,
    Tokens,
    Cost,
    OverallTotals,
    ModelBreakdown,
    UpstreamBreakdown,
    ErrorBreakdown,
    WallTime,
    PerformanceSamples,
    InvocationDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoverageClaim {
    pub(crate) key: CoverageKey,
    pub(crate) range: CoverageRange,
    pub(crate) kind: CoverageKind,
    pub(crate) proof_id: Option<String>,
    pub(crate) capabilities: BTreeSet<SeedCapability>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CoverageLedger {
    claims: Vec<CoverageClaim>,
}

impl CoverageLedger {
    pub(crate) fn claims(&self) -> &[CoverageClaim] {
        &self.claims
    }

    pub(crate) fn merge(&mut self, other: &Self) -> Result<()> {
        for claim in &other.claims {
            self.admit(claim.clone())?;
        }
        Ok(())
    }

    pub(crate) fn admit_exact(
        &mut self,
        key: CoverageKey,
        range: CoverageRange,
        proof_id: impl Into<String>,
    ) -> Result<()> {
        self.admit(CoverageClaim {
            key,
            range,
            kind: CoverageKind::ExactFacts,
            proof_id: Some(proof_id.into()),
            capabilities: BTreeSet::new(),
        })
    }

    pub(crate) fn admit_seed(
        &mut self,
        key: CoverageKey,
        range: CoverageRange,
        proof_id: impl Into<String>,
        capabilities: BTreeSet<SeedCapability>,
    ) -> Result<()> {
        ensure!(
            !capabilities.is_empty(),
            "certified seed must declare its capabilities"
        );
        self.admit(CoverageClaim {
            key,
            range,
            kind: CoverageKind::CertifiedSeed,
            proof_id: Some(proof_id.into()),
            capabilities,
        })
    }

    pub(crate) fn mark_unavailable(
        &mut self,
        key: CoverageKey,
        range: CoverageRange,
        reason: impl Into<String>,
    ) -> Result<()> {
        let reason = reason.into();
        ensure!(
            !reason.trim().is_empty(),
            "unavailable coverage requires a reason"
        );
        let claim = CoverageClaim {
            key,
            range,
            kind: CoverageKind::Unavailable,
            proof_id: Some(reason),
            capabilities: BTreeSet::new(),
        };
        claim.key.validate()?;
        CoverageRange::new(claim.range.start.clone(), claim.range.end.clone())?;
        self.claims
            .retain(|existing| existing.key != claim.key || !existing.range.overlaps(&claim.range));
        self.claims.push(claim);
        self.claims.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| left.range.start.cmp(&right.range.start))
        });
        Ok(())
    }

    pub(crate) fn is_covered(
        &self,
        key: &CoverageKey,
        range: &CoverageRange,
        required_capability: Option<SeedCapability>,
    ) -> bool {
        self.claims.iter().any(|claim| {
            claim.key == *key
                && claim.kind != CoverageKind::Unavailable
                && claim.range.contains(range)
                && match (claim.kind, required_capability) {
                    (CoverageKind::ExactFacts, _) => true,
                    (CoverageKind::CertifiedSeed, Some(capability)) => {
                        claim.capabilities.contains(&capability)
                    }
                    (CoverageKind::CertifiedSeed, None) => true,
                    (CoverageKind::Unavailable, _) => false,
                }
        })
    }

    fn admit(&mut self, claim: CoverageClaim) -> Result<()> {
        ensure!(
            claim
                .proof_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty()),
            "coverage claim requires a proof id"
        );
        if let Some(existing) = self
            .claims
            .iter()
            .find(|existing| existing.key == claim.key && existing.range.overlaps(&claim.range))
        {
            if existing.kind == claim.kind
                && existing.range == claim.range
                && existing.proof_id == claim.proof_id
                && existing.capabilities == claim.capabilities
            {
                return Ok(());
            }
            bail!(
                "coverage overlaps an existing {:?} claim; exact and certified seed ranges must be mutually exclusive",
                existing.kind
            );
        }
        self.claims.push(claim);
        self.claims.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| left.range.start.cmp(&right.range.start))
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> CoverageKey {
        CoverageKey::new("calls", "overall", "day").unwrap()
    }

    fn range(start: &str, end: &str) -> CoverageRange {
        CoverageRange::new(start, end).unwrap()
    }

    #[test]
    fn exact_and_seed_overlap_is_blocked() {
        let mut ledger = CoverageLedger::default();
        ledger
            .admit_exact(key(), range("2026-01-01", "2026-01-02"), "facts-proof")
            .unwrap();
        let mut capabilities = BTreeSet::new();
        capabilities.insert(SeedCapability::OverallTotals);
        assert!(
            ledger
                .admit_seed(
                    key(),
                    range("2026-01-01", "2026-01-02"),
                    "seed-proof",
                    capabilities,
                )
                .is_err()
        );
    }

    #[test]
    fn seed_only_advertises_explicit_capabilities() {
        let mut ledger = CoverageLedger::default();
        let mut capabilities = BTreeSet::new();
        capabilities.insert(SeedCapability::OverallTotals);
        ledger
            .admit_seed(
                key(),
                range("2026-01-01", "2026-01-02"),
                "seed-proof",
                capabilities,
            )
            .unwrap();
        assert!(ledger.is_covered(
            &key(),
            &range("2026-01-01", "2026-01-02"),
            Some(SeedCapability::OverallTotals)
        ));
        assert!(!ledger.is_covered(
            &key(),
            &range("2026-01-01", "2026-01-02"),
            Some(SeedCapability::ModelBreakdown)
        ));
    }
}
