//! `CommandResponseTracker`: counts the targets a command changed and picks the single- or
//! multiple-target feedback the way 26.x commands (`tag`, `enchant`, `clear`, ...) do.

use crate::error::CommandError;
use crate::host::Host;
use crate::text::Text;

/// Which elements count for the feedback (`ElementType`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Counted {
    /// Every tracked element.
    All,
    /// Elements tracked with a non-zero value.
    NonZero,
}

pub struct Tracker<E> {
    total: i32,
    only: Option<E>,
    count: i32,
    only_non_zero: Option<E>,
    non_zero: i32,
}

impl<E: Clone> Tracker<E> {
    pub fn new() -> Self {
        Tracker { total: 0, only: None, count: 0, only_non_zero: None, non_zero: 0 }
    }

    /// `track(element, value)`.
    pub fn track(&mut self, e: &E, value: i32) {
        self.total = self.total.wrapping_add(value);
        self.count += 1;
        self.only = (self.count == 1).then(|| e.clone());
        if value != 0 {
            self.non_zero += 1;
            self.only_non_zero = (self.non_zero == 1).then(|| e.clone());
        }
    }

    pub fn track_bool(&mut self, e: &E, success: bool) {
        self.track(e, success as i32);
    }

    pub fn total(&self) -> i32 {
        self.total
    }

    pub fn count(&self, which: Counted) -> i32 {
        match which {
            Counted::All => self.count,
            Counted::NonZero => self.non_zero,
        }
    }

    /// `dispatch`: the single-element message when exactly one element counts.
    pub fn message(&self, which: Counted, single: impl FnOnce(&E, i32) -> Text, multiple: impl FnOnce(i32, i32) -> Text) -> Text {
        let first = match which {
            Counted::All => &self.only,
            Counted::NonZero => &self.only_non_zero,
        };
        match first {
            Some(e) => single(e, self.total),
            None => multiple(self.count(which), self.total),
        }
    }

    /// `sendFeedback`: fails with `error` when nothing counts, else sends the message; the
    /// command's result is the total.
    pub fn send<S: Host>(
        &self,
        s: &mut S,
        broadcast: bool,
        which: Counted,
        error: Option<CommandError>,
        single: impl FnOnce(&E, i32) -> Text,
        multiple: impl FnOnce(i32, i32) -> Text,
    ) -> Result<i32, CommandError> {
        if let Some(e) = error
            && self.count(which) == 0
        {
            return Err(e);
        }
        let text = self.message(which, single, multiple);
        s.send_success(text, broadcast);
        Ok(self.total)
    }
}
