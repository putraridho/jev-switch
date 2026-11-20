//! Where the provider availability of a start came from.

/// Where the provider availability of this start came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSource {
    /// The provider CLIs were asked (first run, an old cache, or the cache is off).
    Live,
    /// A recent answer from the last run; a background probe refreshes it.
    Cache,
}

impl ProbeSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Cache => "cached",
        }
    }
}
