use serde::Serialize;

/// poop performance metrics collected during benchmark execution
#[derive(Debug, Default, Clone, Copy, Serialize, PartialEq)]
pub struct PoopMetrics {
    /// CPU cycles consumed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_cycles: Option<u64>,

    /// Instructions retired
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<u64>,

    /// Cache references
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_references: Option<u64>,

    /// Cache misses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_misses: Option<u64>,

    /// Branch instructions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branches: Option<u64>,

    /// Branch mispredictions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_misses: Option<u64>,

    /// Page faults
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_faults: Option<u64>,
}

impl PoopMetrics {
    /// Create a new empty PoopMetrics instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if any metric has been collected
    pub fn has_data(&self) -> bool {
        self.cpu_cycles.is_some()
            || self.instructions.is_some()
            || self.cache_references.is_some()
            || self.cache_misses.is_some()
            || self.branches.is_some()
            || self.branch_misses.is_some()
            || self.page_faults.is_some()
    }

    /// Calculate cache miss rate as a percentage
    pub fn cache_miss_rate(&self) -> Option<f64> {
        match (self.cache_references, self.cache_misses) {
            (Some(refs), Some(misses)) if refs > 0 => Some((misses as f64 / refs as f64) * 100.0),
            _ => None,
        }
    }

    /// Calculate branch miss rate as a percentage
    pub fn branch_miss_rate(&self) -> Option<f64> {
        match (self.branches, self.branch_misses) {
            (Some(branches), Some(misses)) if branches > 0 => {
                Some((misses as f64 / branches as f64) * 100.0)
            }
            _ => None,
        }
    }

    /// Calculate instructions per cycle (IPC)
    pub fn instructions_per_cycle(&self) -> Option<f64> {
        match (self.instructions, self.cpu_cycles) {
            (Some(inst), Some(cycles)) if cycles > 0 => Some(inst as f64 / cycles as f64),
            _ => None,
        }
    }
}

/// Types of poop metrics that can be collected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricType {
    CpuCycles,
    Instructions,
    CacheReferences,
    CacheMisses,
    Branches,
    BranchMisses,
    PageFaults,
}

impl MetricType {
    /// Parse a metric type from a string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "cpu-cycles" | "cycles" => Some(MetricType::CpuCycles),
            "instructions" => Some(MetricType::Instructions),
            "cache-references" | "cache-refs" => Some(MetricType::CacheReferences),
            "cache-misses" => Some(MetricType::CacheMisses),
            "branches" => Some(MetricType::Branches),
            "branch-misses" => Some(MetricType::BranchMisses),
            "page-faults" | "faults" => Some(MetricType::PageFaults),
            _ => None,
        }
    }

    /// Get the display name for this metric type
    pub fn display_name(&self) -> &'static str {
        match self {
            MetricType::CpuCycles => "CPU Cycles",
            MetricType::Instructions => "Instructions",
            MetricType::CacheReferences => "Cache References",
            MetricType::CacheMisses => "Cache Misses",
            MetricType::Branches => "Branches",
            MetricType::BranchMisses => "Branch Misses",
            MetricType::PageFaults => "Page Faults",
        }
    }

    /// Get all available metric types
    pub fn all() -> Vec<Self> {
        vec![
            MetricType::CpuCycles,
            MetricType::Instructions,
            MetricType::CacheReferences,
            MetricType::CacheMisses,
            MetricType::Branches,
            MetricType::BranchMisses,
            MetricType::PageFaults,
        ]
    }
}
