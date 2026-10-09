//! Does nothing: allows every block break at once. The cost of a trivial handler is the cost
//! of the boundary itself (the call overhead measurements use it).

use kiln_plugin_sdk::{BlockEvent, Plugin, Verdict, export_plugin};

struct Noop;

impl Plugin for Noop {
    fn on_block_break(_ev: BlockEvent) -> Verdict {
        Verdict::Allow
    }
}

export_plugin!(Noop);
