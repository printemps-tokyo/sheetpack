//! sheetpack -- pack sprites into texture atlases, built for CI.
//!
//! The library holds everything that can be tested without touching the
//! filesystem: rectangle packing, alpha trimming, deduplication keys,
//! manifest rendering and the input fingerprint that powers `--check`.
//! The binary in `main.rs` is a thin shell that reads files, calls in here
//! and writes the results out.
//!
//! Pipeline, in order:
//!
//! 1. [`trim`] finds the opaque bounding box of each sprite (optional).
//! 2. [`pack`] places the trimmed rectangles into one or more pages with
//!    MaxRects best-short-side-fit, deterministically.
//! 3. [`formats`] renders the placements into the manifest dialect an engine
//!    actually loads.
//! 4. [`fingerprint`] hashes the inputs and settings so a later `--check`
//!    can tell whether the committed atlas is stale.

pub mod check;
pub mod fingerprint;
pub mod formats;
pub mod json;
pub mod model;
pub mod pack;
pub mod trim;
