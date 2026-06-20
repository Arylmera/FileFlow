//! FileFlow core: pure, Tauri-free domain logic.
//!
//! Two flows live here:
//! - card ingest ([`ingest`]): copy + verify card files into a dated destination,
//!   then delete from the card only if the *entire* set verified;
//! - Photos import ([`photos`]): hand a Lightroom export folder to Apple Photos.

pub mod config;
pub mod error;
pub mod ingest;
pub mod layout;
pub mod photos;
pub mod util;

pub use error::{Error, Result};
