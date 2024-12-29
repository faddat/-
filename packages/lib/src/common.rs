// lib/src/lib.rs
// Example library crate that organizes code into modules by DEX and common logic.

pub mod common;
pub mod meteora;
pub mod raydium;

//
// If you want to expose certain items at the root of the library, you can `pub use` them here:
//
// pub use common::CHEESE_MINT;
// pub use meteora::{MeteoraPool, fetch_meteora_cheese_pools};
// pub use raydium::{RaydiumPoolDetailed, fetch_raydium_cheese_pools};
//
// Then consumers can do `use cheese_shared::fetch_meteora_cheese_pools;` directly.
//
