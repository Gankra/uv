use std::cmp::min;

use pubgrub::range::Range;
use rustc_hash::FxHashMap;
use tokio::sync::mpsc::Sender;
use tracing::{debug, trace};

use distribution_types::{DistributionMetadata, ResolvedDistRef};
use pep440_rs::Version;

use crate::candidate_selector::{CandidateDist, CandidateSelector};
use crate::pubgrub::PubGrubPackage;
use crate::resolver::Request;
use crate::{InMemoryIndex, ResolveError, VersionsResponse};

enum BatchPrefetchStrategy {
    /// Go through the next versions assuming the existing selection and its constraints
    /// remain.
    Compatible {
        compatible: Range<Version>,
        previous: Version,
    },
    /// We encounter cases (botocore) where the above doesn't work: Say we previously selected
    /// a==x.y.z, which depends on b==x.y.z. a==x.y.z is incompatible, but we don't know that
    /// yet. We just selected b==x.y.z and want to prefetch, since for all versions of a we try,
    /// we have to wait for the matching version of b. The exiting range gives us only one version
    /// of b, so the compatible strategy doesn't prefetch any version. Instead, we try the next
    /// heuristic where the next version of b will be x.y.(z-1) and so forth.
    InOrder { previous: Version },
}

/// Prefetch a large number of versions if we already unsuccessfully tried many versions.
///
/// This is an optimization specifically targeted at cold cache urllib3/boto3/botocore, where we
/// have to fetch the metadata for a lot of versions.
///
/// Note that these all heuristics that could totally prefetch lots of irrelevant versions.
#[derive(Default)]
pub(crate) struct BatchPrefetcher {
    tried_versions: FxHashMap<PubGrubPackage, usize>,
    last_prefetch: FxHashMap<PubGrubPackage, usize>,
}

impl BatchPrefetcher {
    /// Prefetch a large number of versions if we already unsuccessfully tried many versions.
    pub(crate) async fn prefetch_batches(
        &mut self,
        next: &PubGrubPackage,
        version: &Version,
        current_range: &Range<Version>,
        request_sink: &Sender<Request>,
        index: &InMemoryIndex,
        selector: &CandidateSelector,
    ) -> anyhow::Result<(), ResolveError> {
        let PubGrubPackage::Package(package_name, _, _) = &next else {
            return Ok(());
        };

        let (num_tried, do_prefetch) = self.should_prefetch(next);
        if !do_prefetch {
            return Ok(());
        }
        let total_prefetch = min(num_tried, 50);

        // This is immediate, we already fetched the version map.
        let versions_response = index
            .packages
            .wait(package_name)
            .await
            .ok_or(ResolveError::Unregistered)?;

        let VersionsResponse::Found(ref version_map) = *versions_response else {
            return Ok(());
        };

        let mut phase = BatchPrefetchStrategy::Compatible {
            compatible: current_range.clone(),
            previous: version.clone(),
        };
        let mut prefetch_count = 0;
        for _ in 0..total_prefetch {
            let candidate = match phase {
                BatchPrefetchStrategy::Compatible {
                    compatible,
                    previous,
                } => {
                    if let Some(candidate) =
                        selector.select_no_preference(package_name, &compatible, version_map)
                    {
                        let compatible = compatible.intersection(
                            &Range::singleton(candidate.version().clone()).complement(),
                        );
                        phase = BatchPrefetchStrategy::Compatible {
                            compatible,
                            previous: candidate.version().clone(),
                        };
                        candidate
                    } else {
                        // We exhausted the compatible version, switch to ignoring the existing
                        // constraints on the package and instead going through versions in order.
                        phase = BatchPrefetchStrategy::InOrder { previous };
                        continue;
                    }
                }
                BatchPrefetchStrategy::InOrder { previous } => {
                    let range = if selector.use_highest_version(package_name) {
                        Range::strictly_lower_than(previous)
                    } else {
                        Range::strictly_higher_than(previous)
                    };
                    if let Some(candidate) =
                        selector.select_no_preference(package_name, &range, version_map)
                    {
                        phase = BatchPrefetchStrategy::InOrder {
                            previous: candidate.version().clone(),
                        };
                        candidate
                    } else {
                        // Both strategies exhausted their candidates.
                        break;
                    }
                }
            };

            let CandidateDist::Compatible(dist) = candidate.dist() else {
                continue;
            };
            // Avoid building a lot of source distributions.
            if !dist.prefetchable() {
                continue;
            }
            let dist = dist.for_resolution();

            // Emit a request to fetch the metadata for this version.
            trace!(
                "Prefetching {prefetch_count} ({}) {}",
                match phase {
                    BatchPrefetchStrategy::Compatible { .. } => "compatible",
                    BatchPrefetchStrategy::InOrder { .. } => "in order",
                },
                dist
            );
            prefetch_count += 1;
            if index.distributions.register(candidate.package_id()) {
                let request = match dist {
                    ResolvedDistRef::Installable(dist) => Request::Dist(dist.clone()),
                    ResolvedDistRef::Installed(dist) => Request::Installed(dist.clone()),
                };
                request_sink.send(request).await?;
            }
        }

        debug!("Prefetching {prefetch_count} {package_name} versions");

        self.last_prefetch.insert(next.clone(), num_tried);
        Ok(())
    }

    /// Each time we tried a version for a package, we register that here.
    pub(crate) fn version_tried(&mut self, package: PubGrubPackage) {
        *self.tried_versions.entry(package).or_default() += 1;
    }

    /// After 5, 10, 20, 40 tried versions, prefetch that many versions to start early but not
    /// too aggressive. Later we schedule the prefetch of 50 versions every 20 versions, this gives
    /// us a good buffer until we see prefetch again and is high enough to saturate the task pool.
    fn should_prefetch(&self, next: &PubGrubPackage) -> (usize, bool) {
        let num_tried = self.tried_versions.get(next).copied().unwrap_or_default();
        let previous_prefetch = self.last_prefetch.get(next).copied().unwrap_or_default();
        let do_prefetch = (num_tried >= 5 && previous_prefetch < 5)
            || (num_tried >= 10 && previous_prefetch < 10)
            || (num_tried >= 20 && previous_prefetch < 20)
            || (num_tried >= 20 && num_tried - previous_prefetch >= 20);
        (num_tried, do_prefetch)
    }
}
