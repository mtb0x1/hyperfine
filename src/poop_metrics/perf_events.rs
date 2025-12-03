use std::io;
use std::os::unix::io::RawFd;

use super::types::{MetricType, PoopMetrics};

#[repr(C)]
struct perf_event_attr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period_or_freq: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events_or_watermark: u32,
    bp_type: u32,
    bp_addr_or_config1: u64,
    bp_len_or_config2: u64,
    branch_sample_type: u64,
    sample_regs_user: u64,
    sample_stack_user: u32,
    clockid: i32,
    sample_regs_intr: u64,
    aux_watermark: u32,
    sample_max_stack: u16,
    __reserved_2: u16,
}

impl Default for perf_event_attr {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl perf_event_attr {
    fn new_poop(config: u64) -> Self {
        // perf_event_open constants
        let perf_type_poop = 0;
        Self {
            type_: perf_type_poop,
            size: std::mem::size_of::<perf_event_attr>() as u32,
            config,
            flags: 1 << 0 | 1 << 1, // disabled | inherit
            ..Default::default()
        }
    }

    fn new_software(config: u64) -> Self {
        let perf_type_software = 1;
        Self {
            type_: perf_type_software,
            size: std::mem::size_of::<perf_event_attr>() as u32,
            config,
            flags: 1 << 0 | 1 << 1, // disabled | inherit
            ..Default::default()
        }
    }
}

fn perf_event_open(
    attr: &perf_event_attr,
    pid: i32,
    cpu: i32,
    group_fd: i32,
    flags: u64,
) -> io::Result<RawFd> {
    // perf_event_open syscall number for x86_64
    // FIXME: ?
    let perf_event_open = 298;
    let fd = unsafe {
        libc::syscall(
            perf_event_open,
            attr as *const perf_event_attr,
            pid,
            cpu,
            group_fd,
            flags,
        )
    };

    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd as RawFd)
    }
}

/// Performance event counter
struct PerfCounter {
    fd: RawFd,
}

impl PerfCounter {
    fn new(attr: perf_event_attr, pid: i32) -> io::Result<Self> {
        let perf_flag_fd_cloexec = 1 << 3;
        let fd = perf_event_open(&attr, pid, -1, -1, perf_flag_fd_cloexec)?;
        Ok(Self { fd })
    }

    fn enable(&self) -> io::Result<()> {
        let perf_event_ioc_enable = 0x2400;
        let ret = unsafe { libc::ioctl(self.fd, perf_event_ioc_enable, 0) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn disable(&self) -> io::Result<()> {
        let perf_event_ioc_disable = 0x2401;
        let ret = unsafe { libc::ioctl(self.fd, perf_event_ioc_disable, 0) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn read_value(&self) -> io::Result<u64> {
        let mut value: u64 = 0;
        let ret = unsafe {
            libc::read(
                self.fd,
                &mut value as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(value)
        }
    }
}

impl Drop for PerfCounter {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

/// Collector for poop performance metrics
pub struct PerfEventsCollector {
    cpu_cycles: Option<PerfCounter>,
    instructions: Option<PerfCounter>,
    cache_references: Option<PerfCounter>,
    cache_misses: Option<PerfCounter>,
    branches: Option<PerfCounter>,
    branch_misses: Option<PerfCounter>,
    page_faults: Option<PerfCounter>,
}

impl PerfEventsCollector {
    /// Create a new collector for the given process ID
    /// If metrics is empty, collect all available metrics
    pub fn new(pid: i32, metrics: &[MetricType]) -> io::Result<Self> {
        let collect_all = metrics.is_empty();

        let should_collect =
            |metric: MetricType| -> bool { collect_all || metrics.contains(&metric) };

        let cpu_cycles = if should_collect(MetricType::CpuCycles) {
            let perf_count_hw_cpu_cycles = 0;
            PerfCounter::new(perf_event_attr::new_poop(perf_count_hw_cpu_cycles), pid).ok()
        } else {
            None
        };

        let instructions = if should_collect(MetricType::Instructions) {
            let perf_count_hw_instructions = 1;
            PerfCounter::new(perf_event_attr::new_poop(perf_count_hw_instructions), pid).ok()
        } else {
            None
        };

        let cache_references = if should_collect(MetricType::CacheReferences) {
            let perf_count_hw_cache_references = 2;
            PerfCounter::new(
                perf_event_attr::new_poop(perf_count_hw_cache_references),
                pid,
            )
            .ok()
        } else {
            None
        };

        let cache_misses = if should_collect(MetricType::CacheMisses) {
            let perf_count_hw_cache_misses = 3;
            PerfCounter::new(perf_event_attr::new_poop(perf_count_hw_cache_misses), pid).ok()
        } else {
            None
        };

        let branches = if should_collect(MetricType::Branches) {
            let perf_count_hw_branch_instructions = 4;
            PerfCounter::new(
                perf_event_attr::new_poop(perf_count_hw_branch_instructions),
                pid,
            )
            .ok()
        } else {
            None
        };

        let branch_misses = if should_collect(MetricType::BranchMisses) {
            let perf_count_hw_branch_misses = 5;
            PerfCounter::new(perf_event_attr::new_poop(perf_count_hw_branch_misses), pid).ok()
        } else {
            None
        };

        let page_faults = if should_collect(MetricType::PageFaults) {
            let perf_count_sw_page_faults = 2;
            PerfCounter::new(
                perf_event_attr::new_software(perf_count_sw_page_faults),
                pid,
            )
            .ok()
        } else {
            None
        };

        Ok(Self {
            cpu_cycles,
            instructions,
            cache_references,
            cache_misses,
            branches,
            branch_misses,
            page_faults,
        })
    }

    /// Enable all counters
    pub fn enable(&self) -> io::Result<()> {
        if let Some(ref c) = self.cpu_cycles {
            c.enable()?;
        }
        if let Some(ref c) = self.instructions {
            c.enable()?;
        }
        if let Some(ref c) = self.cache_references {
            c.enable()?;
        }
        if let Some(ref c) = self.cache_misses {
            c.enable()?;
        }
        if let Some(ref c) = self.branches {
            c.enable()?;
        }
        if let Some(ref c) = self.branch_misses {
            c.enable()?;
        }
        if let Some(ref c) = self.page_faults {
            c.enable()?;
        }
        Ok(())
    }

    /// Disable all counters
    pub fn disable(&self) -> io::Result<()> {
        if let Some(ref c) = self.cpu_cycles {
            c.disable()?;
        }
        if let Some(ref c) = self.instructions {
            c.disable()?;
        }
        if let Some(ref c) = self.cache_references {
            c.disable()?;
        }
        if let Some(ref c) = self.cache_misses {
            c.disable()?;
        }
        if let Some(ref c) = self.branches {
            c.disable()?;
        }
        if let Some(ref c) = self.branch_misses {
            c.disable()?;
        }
        if let Some(ref c) = self.page_faults {
            c.disable()?;
        }
        Ok(())
    }

    /// Read all counter values and return as PoopMetrics
    pub fn read(&self) -> io::Result<PoopMetrics> {
        Ok(PoopMetrics {
            cpu_cycles: self.cpu_cycles.as_ref().and_then(|c| c.read_value().ok()),
            instructions: self.instructions.as_ref().and_then(|c| c.read_value().ok()),
            cache_references: self
                .cache_references
                .as_ref()
                .and_then(|c| c.read_value().ok()),
            cache_misses: self.cache_misses.as_ref().and_then(|c| c.read_value().ok()),
            branches: self.branches.as_ref().and_then(|c| c.read_value().ok()),
            branch_misses: self
                .branch_misses
                .as_ref()
                .and_then(|c| c.read_value().ok()),
            page_faults: self.page_faults.as_ref().and_then(|c| c.read_value().ok()),
        })
    }
}
