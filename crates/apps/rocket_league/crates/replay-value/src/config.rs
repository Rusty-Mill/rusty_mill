//! Configuration for the value-model pipeline.
//!
//! Like the scoring config, this is versioned so a dataset/model is reproducible:
//! the same `(canonical match, ValueConfig)` always yields the same dataset,
//! model weights, and ΔV. Bump [`VALUE_CONFIG_VERSION`] when defaults change.

/// Stamped onto exported datasets / evaluations; bump when defaults below change.
pub const VALUE_CONFIG_VERSION: &str = "vcfg-v3";

/// Gradient-descent hyperparameters for the logistic value model. Fixed and
/// deterministic (zero-initialized weights, full-batch, no shuffle).
#[derive(Debug, Clone)]
pub struct TrainConfig {
    pub epochs: usize,
    /// Learning rate (features are z-scored, so this is in standardized space).
    pub lr: f32,
    /// L2 weight-decay strength.
    pub l2: f32,
}

impl Default for TrainConfig {
    fn default() -> Self {
        TrainConfig {
            epochs: 4000,
            lr: 0.5,
            l2: 1e-4,
        }
    }
}

/// Value-model configuration.
#[derive(Debug, Clone)]
pub struct ValueConfig {
    /// Label horizon: a state is positive if the perspective team scores the
    /// next goal within this many seconds.
    pub horizon_s: f32,
    /// Sample every Nth resampled grid frame when building the dataset (states
    /// are highly autocorrelated; striding decorrelates and shrinks the table).
    pub sample_stride: usize,
    /// ΔV "after" offset (s) past a touch when crediting the toucher.
    pub touch_post_delay_s: f32,
    pub train: TrainConfig,
    pub version: &'static str,
}

impl Default for ValueConfig {
    fn default() -> Self {
        ValueConfig {
            horizon_s: 10.0,
            sample_stride: 15, // 0.5 s at 30 Hz
            touch_post_delay_s: 0.5,
            train: TrainConfig::default(),
            version: VALUE_CONFIG_VERSION,
        }
    }
}
