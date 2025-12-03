pub mod types;

#[cfg(target_os = "linux")]
pub mod perf_events;

pub use types::{MetricType, PoopMetrics};

#[cfg(target_os = "linux")]
pub use perf_events::PerfEventsCollector;

use std::io;

#[allow(unused)]
/// Trait for collecting poop metrics
pub trait MetricsCollector {
    /// Create a new collector for the given process ID
    fn new(pid: i32, metrics: &[MetricType]) -> io::Result<Self>
    where
        Self: Sized;

    /// Enable metric collection
    fn enable(&self) -> io::Result<()>;

    /// Disable metric collection
    fn disable(&self) -> io::Result<()>;

    /// Read collected metrics
    fn read(&self) -> io::Result<PoopMetrics>;
}

#[cfg(target_os = "linux")]
impl MetricsCollector for PerfEventsCollector {
    fn new(pid: i32, metrics: &[MetricType]) -> io::Result<Self> {
        PerfEventsCollector::new(pid, metrics)
    }

    fn enable(&self) -> io::Result<()> {
        PerfEventsCollector::enable(self)
    }

    fn disable(&self) -> io::Result<()> {
        PerfEventsCollector::disable(self)
    }

    fn read(&self) -> io::Result<PoopMetrics> {
        PerfEventsCollector::read(self)
    }
}

/// Create a metrics collector for the current platform
#[cfg(target_os = "linux")]
pub fn create_collector(pid: i32, metrics: &[MetricType]) -> io::Result<PerfEventsCollector> {
    PerfEventsCollector::new(pid, metrics)
}

/// Create a metrics collector for the current platform (stub for non-Linux)
#[cfg(not(target_os = "linux"))]
pub fn create_collector(_pid: i32, _metrics: &[MetricType]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "poop metrics collection is only supported on Linux",
    ))
}
