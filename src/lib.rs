//! Shared remote-base operating state and radio policy.

#![deny(warnings)]
#![cfg_attr(coverage, feature(coverage_attribute))]

mod config;
mod ffi;
mod ffi_session;
mod limits;
mod scan;
mod session;

pub use config::*;
pub use limits::*;
pub use scan::{ScanDirection, ScanRequest, ScanSnapshot};
pub use session::*;

#[cfg(test)]
#[cfg_attr(coverage, coverage(off))]
mod tests;
