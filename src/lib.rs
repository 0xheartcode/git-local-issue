//! git-local-issue (`gli`): a distributed, offline-first issue tracker stored
//! natively in Git under `refs/issues/*`.
//!
//! The crate is organised around a few small modules:
//!   * [`git`]    - the one `GitBackend` seam for all git access
//!   * [`model`]  - operations, trailers, Lamport clocks, and the CRDT fold
//!   * [`id`]     - UUIDv7 truth ids and drift-allowed display numbers
//!   * [`store`]  - write operations (turning intents into op commits)
//!   * [`cache`]  - the disposable, rebuildable in-memory query cache
//!   * [`fsck`]   - integrity checks
//!   * [`cli`]    - the command surface

pub mod cache;
pub mod cli;
pub mod error;
pub mod fsck;
pub mod git;
pub mod id;
pub mod model;
pub mod store;
pub mod trailer;
