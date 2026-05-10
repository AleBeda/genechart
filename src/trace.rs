//! General-purpose structured tracing to stderr.

use std::collections::HashSet;

pub struct Tracer {
    components: HashSet<String>,
}

impl Tracer {
    /// Build a Tracer from the list of component names supplied via `--trace`.
    /// The special value `"all"` (inserted by clap when `--trace` is bare)
    /// enables every component.
    pub fn new(components: &[String]) -> Self {
        Self {
            components: components.iter().map(|s| s.to_lowercase()).collect(),
        }
    }

    /// A no-op tracer (used in tests and when `--trace` is absent).
    #[allow(dead_code)] // (not currently used, but may be useful in tests and benchmarks)
    pub fn disabled() -> Self {
        Self {
            components: HashSet::new(),
        }
    }

    /// Returns true if the named component is being traced.
    pub fn enabled(&self, component: &str) -> bool {
        !self.components.is_empty()
            && (self.components.contains("all") || self.components.contains(component))
    }

    /// Emit a trace line to stderr for `component` if that component is enabled.
    pub fn emit(&self, component: &str, msg: &str) {
        if self.enabled(component) {
            eprintln!("{msg}");
        }
    }
}
