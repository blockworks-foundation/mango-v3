#![feature(destructuring_assignment)]

#[macro_use]
pub mod error;

pub mod ids;
pub mod instruction;
pub mod mango_program;
pub mod matching;
pub mod oracle;
pub mod processor;
pub mod queue;
pub mod state;
pub mod utils;
pub use mango_program::Mango;

#[cfg(not(feature = "no-entrypoint"))]
pub mod entrypoint;
