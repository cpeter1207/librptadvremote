//! Shared remote-base operating state and radio policy.

#![deny(warnings)]
#![cfg_attr(coverage, feature(coverage_attribute))]

mod config;
mod ffi;
mod limits;

pub use config::*;
pub use limits::*;

/// One configured radio's portable operating state; the host owns physical I/O.
pub struct RemoteBase {
    /// Validated configuration shared by command and scan operations.
    config: RemoteBaseConfig,
    /// Actual backend capabilities used to validate every operating request.
    capabilities: Capabilities,
}

impl RemoteBase {
    /// Validate a complete configuration before allocating physical resources.
    pub fn new(config: RemoteBaseConfig, capabilities: Capabilities) -> Result<Self, Error> {
        config.validate(&capabilities)?;
        Ok(Self {
            config,
            capabilities,
        })
    }

    /// Return the validated immutable configuration.
    pub fn configuration(&self) -> &RemoteBaseConfig {
        &self.config
    }

    /// Return the selected backend's declared capabilities.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
}

#[cfg(test)]
#[cfg_attr(coverage, coverage(off))]
mod tests;
