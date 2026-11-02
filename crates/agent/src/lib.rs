//! Agent integration (spec §5): the harness table, the events an agent produces, and the pure state
//! engine that turns them into what the UI shows.
mod harness;
mod state;

pub use harness::{Harness, harness, harnesses, match_process, sanitize};
pub use state::{Agent, Ctx, Signal, Source, State, Wait, became_visible, step, typed};
