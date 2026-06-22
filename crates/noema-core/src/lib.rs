pub mod api;
pub mod audit;
// The LOCOMO/recall benchmark harness is a large, optional module. Gate it behind
// a feature so library consumers (e.g. zode) do not compile it; the noema CLI
// enables the feature because it owns the `bench` subcommand.
#[cfg(feature = "benchmark")]
// The harness carries prompt-builder helpers that are not all wired into the
// active pipeline yet; allow dead_code here so the experimental bench module
// does not block the `-D warnings` clippy gate.
#[allow(dead_code)]
pub mod benchmark;
pub mod capacity;
pub mod config;
pub mod crypto;
pub mod error;
pub mod extraction;
pub mod frontmatter;
pub mod hippocampus;
pub mod identity;
pub mod ids;
pub mod index;
pub mod jsonl;
pub mod lock;
pub mod memory;
pub mod memorypack;
pub mod multihop;
pub mod offload;
pub mod pageindex;
pub mod paths;
pub mod policy;
pub mod project;
pub mod recall;
pub mod review;
pub mod s3;
pub mod sensitivity;
pub mod storage;
pub mod store;
pub mod text;
pub mod vacuum;
pub mod variants;
