//! Metric instruments on atomics, with no metrics SDK behind them.
//!
//! A counter is one `AtomicU64` per attribute combination, an up/down counter
//! one `AtomicI64`, a histogram a small locked tally. Recording is a read lock
//! and an atomic add on the hot path; a write lock is taken only the first time
//! a combination is seen. Nothing here knows about OTLP: [`Registry::collect`]
//! turns the current values into plain [`Metric`]s, and [`super::otlp`] encodes
//! those.
//!
//! Every value is **cumulative** since the registry was created, which is what
//! OTLP calls cumulative temporality. A push that is lost is repaired by the
//! next one, so the exporter never retries.
//!
//! The attribute sets come from [`super::labels`], which keeps them to closed
//! sets; this module trusts that and does not cap the number of combinations.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError, RwLock,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

use super::labels::Label;

/// Name, description and unit of an instrument.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Meta {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) unit: &'static str,
}

/// One value per attribute combination.
struct Cells<T>(RwLock<HashMap<Vec<Label>, Arc<T>>>);

impl<T> Cells<T> {
    fn new() -> Self {
        Self(RwLock::new(HashMap::new()))
    }

    /// The cell for `attributes`, made with `make` the first time.
    ///
    /// A poisoned lock is recovered rather than propagated: a panic elsewhere
    /// must not turn every later metric into another panic.
    fn get_or_insert(&self, attributes: &[Label], make: impl FnOnce() -> T) -> Arc<T> {
        let read = self.0.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(cell) = read.get(attributes) {
            return Arc::clone(cell);
        }
        drop(read);

        let mut write = self.0.write().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(
            write
                .entry(attributes.to_vec())
                .or_insert_with(|| Arc::new(make())),
        )
    }

    /// Every cell with its attributes, in a stable order.
    fn entries(&self) -> Vec<(Vec<Label>, Arc<T>)> {
        let read = self.0.read().unwrap_or_else(PoisonError::into_inner);
        let mut entries: Vec<_> = read
            .iter()
            .map(|(attributes, cell)| (attributes.clone(), Arc::clone(cell)))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

/// A value that only goes up.
pub(crate) struct Counter {
    meta: Meta,
    cells: Cells<AtomicU64>,
}

impl Counter {
    pub(crate) fn new(meta: Meta) -> Self {
        Self {
            meta,
            cells: Cells::new(),
        }
    }

    pub(crate) fn add(&self, value: u64, attributes: &[Label]) {
        self.cells
            .get_or_insert(attributes, || AtomicU64::new(0))
            .fetch_add(value, Ordering::Relaxed);
    }
}

/// A value that goes up and down.
pub(crate) struct UpDownCounter {
    meta: Meta,
    cells: Cells<AtomicI64>,
}

impl UpDownCounter {
    pub(crate) fn new(meta: Meta) -> Self {
        Self {
            meta,
            cells: Cells::new(),
        }
    }

    pub(crate) fn add(&self, value: i64, attributes: &[Label]) {
        self.cells
            .get_or_insert(attributes, || AtomicI64::new(0))
            .fetch_add(value, Ordering::Relaxed);
    }
}

/// The running tally of one histogram attribute combination.
struct Tally {
    /// One count per bound, plus one for everything above the last.
    buckets: Vec<u64>,
    count: u64,
    sum: f64,
}

/// A distribution over explicit bucket bounds.
///
/// Each cell is locked as a whole so that a snapshot never sees a count that
/// disagrees with its buckets; the lock is held for three additions.
pub(crate) struct Histogram {
    meta: Meta,
    bounds: Vec<f64>,
    cells: Cells<Mutex<Tally>>,
}

impl Histogram {
    pub(crate) fn new(meta: Meta, bounds: &[f64]) -> Self {
        Self {
            meta,
            bounds: bounds.to_vec(),
            cells: Cells::new(),
        }
    }

    /// Record `value`. A NaN or infinite value is dropped: it has no bucket and
    /// would poison the sum.
    pub(crate) fn record(&self, value: f64, attributes: &[Label]) {
        if !value.is_finite() {
            return;
        }
        let cell = self.cells.get_or_insert(attributes, || {
            Mutex::new(Tally {
                buckets: vec![0; self.bounds.len() + 1],
                count: 0,
                sum: 0.0,
            })
        });
        // The first bound that is >= `value`: buckets are inclusive above.
        let bucket = self.bounds.partition_point(|bound| *bound < value);
        let mut tally = cell.lock().unwrap_or_else(PoisonError::into_inner);
        tally.buckets[bucket] += 1;
        tally.count += 1;
        tally.sum += value;
    }
}

/// A whole number, as OTLP's `asInt` carries it.
pub(crate) type Int = i64;

/// A histogram data point.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistogramPoint {
    pub(crate) attributes: Vec<Label>,
    pub(crate) count: u64,
    pub(crate) sum: f64,
    pub(crate) buckets: Vec<u64>,
}

/// What an instrument holds right now.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Data {
    /// A counter (`monotonic`) or an up/down counter.
    Sum {
        monotonic: bool,
        points: Vec<(Vec<Label>, Int)>,
    },
    /// A histogram over `bounds`.
    Histogram {
        bounds: Vec<f64>,
        points: Vec<HistogramPoint>,
    },
}

/// One instrument's current state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Metric {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) unit: &'static str,
    pub(crate) data: Data,
}

impl Metric {
    /// Whether no data point has been recorded yet.
    pub(crate) fn is_empty(&self) -> bool {
        match &self.data {
            Data::Sum { points, .. } => points.is_empty(),
            Data::Histogram { points, .. } => points.is_empty(),
        }
    }
}

/// Anything the [`Registry`] can read.
pub(crate) trait Collect: Send + Sync {
    fn collect(&self) -> Metric;
}

impl Collect for Counter {
    fn collect(&self) -> Metric {
        let points = self
            .cells
            .entries()
            .into_iter()
            .map(|(attributes, cell)| {
                let value = cell.load(Ordering::Relaxed);
                // OTLP's `asInt` is signed; a counter that large has bigger
                // problems, so clamp rather than wrap.
                (attributes, Int::try_from(value).unwrap_or(Int::MAX))
            })
            .collect();
        self.metric(Data::Sum {
            monotonic: true,
            points,
        })
    }
}

impl Collect for UpDownCounter {
    fn collect(&self) -> Metric {
        let points = self
            .cells
            .entries()
            .into_iter()
            .map(|(attributes, cell)| (attributes, cell.load(Ordering::Relaxed)))
            .collect();
        self.metric(Data::Sum {
            monotonic: false,
            points,
        })
    }
}

impl Collect for Histogram {
    fn collect(&self) -> Metric {
        let points = self
            .cells
            .entries()
            .into_iter()
            .map(|(attributes, cell)| {
                let tally = cell.lock().unwrap_or_else(PoisonError::into_inner);
                HistogramPoint {
                    attributes,
                    count: tally.count,
                    sum: tally.sum,
                    buckets: tally.buckets.clone(),
                }
            })
            .collect();
        self.metric(Data::Histogram {
            bounds: self.bounds.clone(),
            points,
        })
    }
}

/// Instruments that carry a [`Meta`].
trait Described {
    fn meta(&self) -> Meta;

    fn metric(&self, data: Data) -> Metric {
        let Meta {
            name,
            description,
            unit,
        } = self.meta();
        Metric {
            name,
            description,
            unit,
            data,
        }
    }
}

macro_rules! described {
    ($($ty:ty),*) => {
        $(impl Described for $ty {
            fn meta(&self) -> Meta {
                self.meta
            }
        })*
    };
}
described!(Counter, UpDownCounter, Histogram);

/// The instruments a pipeline exports.
#[derive(Default)]
pub(crate) struct Registry {
    instruments: Mutex<Vec<Arc<dyn Collect>>>,
}

impl Registry {
    pub(crate) fn register<T: Collect + 'static>(&self, instrument: &Arc<T>) {
        let mut instruments = self
            .instruments
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        instruments.push(Arc::clone(instrument) as Arc<dyn Collect>);
    }

    /// The current state of every registered instrument, in registration order.
    pub(crate) fn collect(&self) -> Vec<Metric> {
        let instruments = self
            .instruments
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        instruments.iter().map(|i| i.collect()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: Meta = Meta {
        name: "test.metric",
        description: "A test instrument.",
        unit: "1",
    };

    fn label(key: &'static str, value: &str) -> Label {
        (key, value.to_owned())
    }

    fn sum_points(metric: &Metric) -> &[(Vec<Label>, Int)] {
        match &metric.data {
            Data::Sum { points, .. } => points,
            other => panic!("not a sum: {other:?}"),
        }
    }

    fn histogram_points(metric: &Metric) -> &[HistogramPoint] {
        match &metric.data {
            Data::Histogram { points, .. } => points,
            other => panic!("not a histogram: {other:?}"),
        }
    }

    #[test]
    fn a_counter_adds_per_attribute_combination() {
        let counter = Counter::new(META);
        let a = [label("k", "a")];
        let b = [label("k", "b")];
        counter.add(2, &a);
        counter.add(3, &a);
        counter.add(5, &b);

        let metric = counter.collect();
        assert_eq!(sum_points(&metric), [(a.to_vec(), 5), (b.to_vec(), 5)]);
        assert!(matches!(
            metric.data,
            Data::Sum {
                monotonic: true,
                ..
            }
        ));
    }

    #[test]
    fn an_up_down_counter_goes_down_and_is_not_monotonic() {
        let gauge = UpDownCounter::new(META);
        let attributes = [label("k", "a")];
        gauge.add(1, &attributes);
        gauge.add(1, &attributes);
        gauge.add(-1, &attributes);

        let metric = gauge.collect();
        assert_eq!(sum_points(&metric), [(attributes.to_vec(), 1)]);
        assert!(matches!(
            metric.data,
            Data::Sum {
                monotonic: false,
                ..
            }
        ));
    }

    #[test]
    fn values_are_cumulative_across_collections() {
        let counter = Counter::new(META);
        counter.add(1, &[]);
        assert_eq!(sum_points(&counter.collect())[0].1, 1);
        counter.add(1, &[]);
        assert_eq!(sum_points(&counter.collect())[0].1, 2);
    }

    #[test]
    fn points_come_out_in_a_stable_order() {
        let counter = Counter::new(META);
        for key in ["c", "a", "b"] {
            counter.add(1, &[label("k", key)]);
        }
        let order: Vec<_> = sum_points(&counter.collect())
            .iter()
            .map(|(attributes, _)| attributes[0].1.clone())
            .collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn nothing_recorded_means_no_points() {
        assert!(sum_points(&Counter::new(META).collect()).is_empty());
    }

    #[test]
    fn a_counter_too_large_for_an_int_is_clamped_not_wrapped() {
        let counter = Counter::new(META);
        counter.add(u64::MAX, &[]);
        assert_eq!(sum_points(&counter.collect())[0].1, Int::MAX);
    }

    #[test]
    fn a_histogram_buckets_inclusively_above() {
        let histogram = Histogram::new(META, &[1.0, 5.0]);
        for value in [0.5, 1.0, 1.1, 5.0, 5.1, 100.0] {
            histogram.record(value, &[]);
        }
        let metric = histogram.collect();
        let point = &histogram_points(&metric)[0];
        // (-inf, 1], (1, 5], (5, +inf)
        assert_eq!(point.buckets, [2, 2, 2]);
        assert_eq!(point.count, 6);
        assert!((point.sum - 112.7).abs() < 1e-9, "{}", point.sum);
    }

    #[test]
    fn a_histogram_drops_values_that_have_no_bucket() {
        let histogram = Histogram::new(META, &[1.0]);
        histogram.record(f64::NAN, &[]);
        histogram.record(f64::INFINITY, &[]);
        histogram.record(f64::NEG_INFINITY, &[]);
        assert!(histogram_points(&histogram.collect()).is_empty());
    }

    #[test]
    fn a_histogram_with_no_bounds_has_one_bucket() {
        let histogram = Histogram::new(META, &[]);
        histogram.record(3.0, &[]);
        assert_eq!(histogram_points(&histogram.collect())[0].buckets, [1]);
    }

    #[test]
    fn concurrent_recording_loses_nothing() {
        const THREADS: u64 = 8;
        const PER_THREAD: u64 = 1_000;
        let counter = Arc::new(Counter::new(META));
        let histogram = Arc::new(Histogram::new(META, &[10.0]));

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let (counter, histogram) = (Arc::clone(&counter), Arc::clone(&histogram));
                std::thread::spawn(move || {
                    for _ in 0..PER_THREAD {
                        counter.add(1, &[label("k", "a")]);
                        histogram.record(1.0, &[label("k", "a")]);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("join");
        }

        let total = (THREADS * PER_THREAD) as i64;
        assert_eq!(sum_points(&counter.collect())[0].1, total);
        let metric = histogram.collect();
        let point = &histogram_points(&metric)[0];
        assert_eq!(point.count, total as u64);
        assert_eq!(point.buckets, [total as u64, 0]);
    }

    #[test]
    fn the_registry_collects_what_was_registered_in_order() {
        let registry = Registry::default();
        let counter = Arc::new(Counter::new(Meta {
            name: "first",
            ..META
        }));
        let gauge = Arc::new(UpDownCounter::new(Meta {
            name: "second",
            ..META
        }));
        registry.register(&counter);
        registry.register(&gauge);

        counter.add(4, &[]);
        let names: Vec<_> = registry.collect().iter().map(|m| m.name).collect();
        assert_eq!(names, ["first", "second"]);
        assert_eq!(sum_points(&registry.collect()[0])[0].1, 4);
    }
}
