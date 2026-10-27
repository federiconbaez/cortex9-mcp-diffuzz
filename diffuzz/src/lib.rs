//! mcp-diffuzz: a stateful, differential fuzzer for Model Context Protocol
//! servers over stdio.
//!
//! * [`step`]: message vocabulary (valid, malformed and out-of-order).
//! * [`gen`]: stateful case generator.
//! * [`target`]: child-process runner with timeouts.
//! * [`norm`] / [`oracle`]: normalization, JSON-RPC 2.0 invariants and
//!   cross-implementation disagreement.
//! * [`minimize`]: delta debugging.
//! * [`corpus`]: regression cases and the fuzzing loop.

pub mod case;
pub mod corpus;
pub mod gen;
pub mod minimize;
pub mod norm;
pub mod oracle;
pub mod rng;
pub mod step;
pub mod target;
