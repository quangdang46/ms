//! Graph engine for native graph analysis.
//!
//! Provides DiGraph, graph building from issues, reachability queries,
//! and what-if simulation for cascade impact analysis.

pub mod builder;
pub mod graph;
pub mod reachability;
pub mod whatif;
