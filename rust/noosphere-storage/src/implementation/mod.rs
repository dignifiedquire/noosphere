mod memory;
mod tracking;

pub use memory::*;
pub use tracking::*;

#[cfg(all(not(feature = "sqlite"), not(target_arch = "wasm32")))]
mod native;

#[cfg(all(not(feature = "sqlite"), not(target_arch = "wasm32")))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(feature = "sqlite")]
mod channel;

#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::{
    SqliteStorage as NativeStorage, SqliteStorageInit as NativeStorageInit,
    SqliteStore as NativeStore,
};
