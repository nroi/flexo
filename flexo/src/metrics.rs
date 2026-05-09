use prometheus::{IntCounter, IntCounterVec, IntGauge, Registry, Opts, register_int_counter_with_registry, register_int_counter_vec_with_registry, register_int_gauge_with_registry};
use lazy_static::lazy_static;

lazy_static! {
    pub static ref REGISTRY: Registry = Registry::new();

    pub static ref CACHE_HITS: IntCounter = register_int_counter_with_registry!(
        Opts::new("flexo_cache_hits_total", "Total number of cache hits"),
        &REGISTRY
    ).unwrap();

    pub static ref CACHE_MISSES: IntCounter = register_int_counter_with_registry!(
        Opts::new("flexo_cache_misses_total", "Total number of cache misses"),
        &REGISTRY
    ).unwrap();

    pub static ref BYTES_SERVED_FROM_CACHE: IntCounter = register_int_counter_with_registry!(
        Opts::new("flexo_bytes_served_from_cache_total", "Total bytes served from local cache"),
        &REGISTRY
    ).unwrap();

    pub static ref BYTES_SERVED_FROM_MIRROR: IntCounter = register_int_counter_with_registry!(
        Opts::new("flexo_bytes_served_from_mirror_total", "Total bytes served from remote mirrors"),
        &REGISTRY
    ).unwrap();

    pub static ref MIRROR_USAGES: IntCounterVec = register_int_counter_vec_with_registry!(
        Opts::new("flexo_mirror_usages_total", "Total number of times a mirror was used"),
        &["mirror"],
        &REGISTRY
    ).unwrap();

    pub static ref MIRROR_FAILURES: IntCounterVec = register_int_counter_vec_with_registry!(
        Opts::new("flexo_mirror_failures_total", "Total number of mirror failures"),
        &["mirror", "reason"],
        &REGISTRY
    ).unwrap();

    pub static ref CONCURRENT_DOWNLOADS: IntGauge = register_int_gauge_with_registry!(
        Opts::new("flexo_concurrent_downloads_active", "Number of concurrent downloads currently active"),
        &REGISTRY
    ).unwrap();

    pub static ref REQUESTS_TOTAL: IntCounterVec = register_int_counter_vec_with_registry!(
        Opts::new("flexo_requests_total", "Total number of requests received by type"),
        &["type"],
        &REGISTRY
    ).unwrap();
}
