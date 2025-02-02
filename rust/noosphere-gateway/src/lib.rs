//! This crate contains substantially all of the implementation of the Noosphere Gateway
//! and provides it as a re-usable library. It is the same implementation of the gateway
//! that is used by the Noosphere CLI.

#![cfg(not(target_arch = "wasm32"))]
#![warn(missing_docs)]

#[macro_use]
extern crate tracing;

mod authority;
mod error;
mod extractor;
mod gateway;
mod handlers;
mod try_or_reset;
mod worker;

pub use gateway::*;
