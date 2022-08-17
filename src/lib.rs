mod arcs;
mod arena;
mod bitmap;
mod bucket;
mod thread_id;

mod utility;
use utility::OptionExt;

pub use arena::Arena;
pub use bucket::{ArenaArc, MAX_REFCNT};

/// `triomphe::Arc` does not support weak reference, thus it allocates one `usize` less
/// than `std::sync::Arc`.
use triomphe::Arc;
