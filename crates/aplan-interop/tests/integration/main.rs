//! The single integration-test harness for this crate (`A1-630`).
//!
//! Keep the source modules in `tests/`: task paths and trace references name
//! those files directly. The integration-target census checks that this list
//! and the files on disk remain equal.

#[path = "../cut.rs"]
mod cut;
#[path = "../kep.rs"]
mod kep;
#[path = "../round_trip.rs"]
mod round_trip;
#[path = "../seam.rs"]
mod seam;
