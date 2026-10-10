use super::*;

// Reserve inflight, the two CPU modes and this limiter's drop counter. The
// independently filtered hotpath scrape has a further 1,000-series allowance.
const APP_SERIES_LIMIT: usize = 3996;
#[derive(Default)]
pub(super) struct SeriesBudget {
    entries: std::sync::RwLock<(HashSet<(u8, Key)>, usize)>,
    dropped: AtomicU64,
}
impl SeriesBudget {
    pub(super) fn admit(&self, key: &Key, kind: u8) -> bool {
        let entry = (kind, key.clone());
        if self
            .entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .0
            .contains(&entry)
        {
            return true;
        }
        let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
        if entries.0.contains(&entry) {
            return true;
        }
        // All histograms have 16 finite buckets, +Inf, sum and count.
        let weight = if kind == 2 { 19 } else { 1 };
        if entries.1 + weight > APP_SERIES_LIMIT {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        entries.0.insert(entry);
        entries.1 += weight;
        true
    }
    pub(super) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}
