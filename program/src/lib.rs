#![feature(destructuring_assignment)]

#[macro_use]
pub mod error;

pub mod ids;
pub mod instruction;
pub mod matching;
pub mod oracle;
pub mod processor;
pub mod queue;
pub mod state;
pub mod utils;

#[cfg(feature = "declare-id")]
pub mod mango_program;

#[cfg(not(feature = "no-entrypoint"))]
pub mod entrypoint;
