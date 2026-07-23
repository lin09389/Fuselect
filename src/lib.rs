//! Fuselect is a local, privacy-first Fusion gateway for coding agents.
//!
//! The public CLI is deliberately small at this stage. Gateway and protocol
//! behavior are added behind tested domain boundaries in later milestones.

pub mod config;
pub mod domain;
pub mod secrets;
pub mod storage;

pub const APPLICATION_NAME: &str = "fuselect";
