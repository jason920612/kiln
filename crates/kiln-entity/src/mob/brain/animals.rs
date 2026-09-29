//! Shared pieces of the camel, allay and sniffer brains (wp28 animals): anonymous subclasses of
//! the common behaviours (`SnifferAi$1`, `CamelAi$CamelPanic`, ...), which vanilla names with an
//! empty class name, and the helpers the three mobs use.

use super::{Control, Cx, Mem};

/// An anonymous subclass of a shared behaviour that adds a start condition (checked before the
/// behaviour's own, all pure) and/or something to do in `start`. Its class name is empty, so the
/// trace skips it (vanilla's `getSimpleName` of an anonymous class).
#[derive(Clone, Debug)]
pub struct Anon {
    /// The class name the trace shows: empty for an anonymous class, the simple name of a
    /// nested one (`CamelPanic`).
    name: &'static str,
    inner: Box<dyn Control>,
    /// An extra `checkExtraStartConditions` term (`super.check(..) && this`); must be pure.
    pre: Option<fn(&Cx) -> bool>,
    /// Runs when the behaviour starts (`start` override; the order against `super.start` does not
    /// matter where the two touch different things).
    on_start: Option<fn(&mut Cx)>,
}

impl Anon {
    pub fn wrap(inner: Box<dyn Control>, pre: Option<fn(&Cx) -> bool>, on_start: Option<fn(&mut Cx)>) -> Box<dyn Control> {
        Box::new(Anon { name: "", inner, pre, on_start })
    }

    /// A named subclass (`CamelAi$CamelPanic`).
    pub fn named(name: &'static str, inner: Box<dyn Control>, pre: Option<fn(&Cx) -> bool>, on_start: Option<fn(&mut Cx)>) -> Box<dyn Control> {
        Box::new(Anon { name, inner, pre, on_start })
    }

    pub fn on_start(inner: Box<dyn Control>, f: fn(&mut Cx)) -> Box<dyn Control> {
        Anon::wrap(inner, None, Some(f))
    }
}

impl Control for Anon {
    fn name(&self) -> &'static str {
        self.name
    }
    fn running(&self) -> bool {
        self.inner.running()
    }
    fn required(&self, out: &mut Vec<Mem>) {
        self.inner.required(out);
    }
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        if self.pre.is_some_and(|p| !p(cx)) {
            return false;
        }
        let started = self.inner.try_start(cx);
        if started && let Some(f) = self.on_start {
            f(cx);
        }
        started
    }
    fn tick_or_stop(&mut self, cx: &mut Cx) {
        self.inner.tick_or_stop(cx);
    }
    fn do_stop(&mut self, cx: &mut Cx) {
        self.inner.do_stop(cx);
    }
    fn running_names(&self, out: &mut Vec<String>) {
        // The wrapped behaviour's own name does not show (it is the anonymous subclass that runs).
        if self.inner.running() {
            out.push(self.name.to_owned());
        }
    }
    fn seed_gates(&mut self, base: i64, k: &mut i64) {
        self.inner.seed_gates(base, k);
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}

/// `BehaviorBuilder.triggerIf(predicate, oneShot)`: the predicate first (pure), then the one-shot.
#[derive(Clone, Debug)]
pub struct TriggerIf {
    pred: fn(&Cx) -> bool,
    inner: Box<dyn Control>,
}

impl TriggerIf {
    pub fn new(pred: fn(&Cx) -> bool, inner: Box<dyn Control>) -> Box<dyn Control> {
        Box::new(TriggerIf { pred, inner })
    }
}

impl Control for TriggerIf {
    fn name(&self) -> &'static str {
        ""
    }
    fn running(&self) -> bool {
        false
    }
    fn required(&self, out: &mut Vec<Mem>) {
        self.inner.required(out);
    }
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        (self.pred)(cx) && self.inner.try_start(cx)
    }
    fn tick_or_stop(&mut self, _cx: &mut Cx) {}
    fn do_stop(&mut self, _cx: &mut Cx) {}
    fn seed_gates(&mut self, base: i64, k: &mut i64) {
        self.inner.seed_gates(base, k);
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}
