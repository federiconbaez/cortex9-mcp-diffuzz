//! mcp-diffuzz: a stateful, differential fuzzer for Model Context Protocol
//! servers over stdio.
//!
//! * [`step`]: message vocabulary (valid, malformed and out-of-order).
//! * [`gen`]: stateful case generator.

pub mod gen;
pub mod rng;
pub mod step;
