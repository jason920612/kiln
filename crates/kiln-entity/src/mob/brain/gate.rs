//! `GateBehavior` (`RunOne`, `TryAll`) and `TriggerGate`, with `ShufflingList`.

use super::{Control, Cx, Mem, Status};
use kiln_javamath::random::{LegacyRandom, RandomSource};

/// `GateBehavior.OrderPolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderPolicy {
    Ordered,
    Shuffled,
}

/// `GateBehavior.RunningPolicy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunningPolicy {
    /// Start the first behaviour that can, then stop looking.
    RunOne,
    /// Try to start all of them.
    TryAll,
}

/// `ShufflingList`: entries with weights; a shuffle sorts them by `-pow(random, 1 / weight)`.
#[derive(Clone, Debug)]
pub struct Shuffled<T> {
    pub entries: Vec<(T, i32, f64)>,
    random: LegacyRandom,
}

impl<T> Shuffled<T> {
    pub fn new(items: Vec<(T, i32)>) -> Shuffled<T> {
        Shuffled { entries: items.into_iter().map(|(t, w)| (t, w, 0.0)).collect(), random: LegacyRandom::new(0) }
    }

    /// `ShufflingList.shuffle`: a random weight per entry (in order), then a stable sort.
    pub fn shuffle(&mut self) {
        for e in self.entries.iter_mut() {
            let f = self.random.next_float();
            e.2 = -(f as f64).powf((1.0f32 / e.1 as f32) as f64);
        }
        self.entries.sort_by(|a, b| a.2.total_cmp(&b.2));
    }

    pub fn seed(&mut self, seed: i64) {
        self.random = LegacyRandom::new(seed);
    }
}

/// `GateBehavior`.
#[derive(Clone, Debug)]
pub struct Gate {
    name: &'static str,
    entry: Vec<(Mem, Status)>,
    exit_erased: Vec<Mem>,
    order: OrderPolicy,
    running_policy: RunningPolicy,
    list: Shuffled<Box<dyn Control>>,
    running: bool,
}

impl Gate {
    /// `new RunOne(behaviours)`.
    pub fn run_one(behaviors: Vec<(Box<dyn Control>, i32)>) -> Box<dyn Control> {
        Gate::new("RunOne", &[], &[], OrderPolicy::Shuffled, RunningPolicy::RunOne, behaviors)
    }

    /// `new RunOne(entry, behaviours)`.
    pub fn run_one_when(entry: &[(Mem, Status)], behaviors: Vec<(Box<dyn Control>, i32)>) -> Box<dyn Control> {
        Gate::new("RunOne", entry, &[], OrderPolicy::Shuffled, RunningPolicy::RunOne, behaviors)
    }

    pub fn new(
        name: &'static str,
        entry: &[(Mem, Status)],
        exit_erased: &[Mem],
        order: OrderPolicy,
        running_policy: RunningPolicy,
        behaviors: Vec<(Box<dyn Control>, i32)>,
    ) -> Box<dyn Control> {
        Box::new(Gate {
            name,
            entry: entry.to_vec(),
            exit_erased: exit_erased.to_vec(),
            order,
            running_policy,
            list: Shuffled::new(behaviors),
            running: false,
        })
    }

    pub fn seed(&mut self, seed: i64) {
        self.list.seed(seed);
    }
}

impl Control for Gate {
    fn name(&self) -> &'static str {
        self.name
    }
    fn running(&self) -> bool {
        self.running
    }
    fn required(&self, out: &mut Vec<Mem>) {
        out.extend(self.entry.iter().map(|&(m, _)| m));
        for (b, _, _) in &self.list.entries {
            b.required(out);
        }
    }
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        if !self.entry.iter().all(|&(m, s)| cx.b.mem.check(m, s)) {
            return false;
        }
        self.running = true;
        if self.order == OrderPolicy::Shuffled {
            self.list.shuffle();
        }
        match self.running_policy {
            RunningPolicy::RunOne => {
                let debug = super::debug_on();
                for (b, w, _) in self.list.entries.iter_mut() {
                    if !b.running() && b.try_start(cx) {
                        if debug {
                            eprintln!("gate t={} {} picked {}/{}", cx.time, self.name, b.name(), w);
                        }
                        break;
                    }
                }
            }
            RunningPolicy::TryAll => {
                for (b, _, _) in self.list.entries.iter_mut() {
                    if !b.running() {
                        b.try_start(cx);
                    }
                }
            }
        }
        true
    }
    fn tick_or_stop(&mut self, cx: &mut Cx) {
        for (b, _, _) in self.list.entries.iter_mut() {
            if b.running() {
                b.tick_or_stop(cx);
            }
        }
        if !self.list.entries.iter().any(|(b, _, _)| b.running()) {
            self.do_stop(cx);
        }
    }
    fn do_stop(&mut self, cx: &mut Cx) {
        self.running = false;
        for (b, _, _) in self.list.entries.iter_mut() {
            if b.running() {
                b.do_stop(cx);
            }
        }
        for i in 0..self.exit_erased.len() {
            cx.b.mem.erase(self.exit_erased[i]);
        }
    }
    fn running_names(&self, out: &mut Vec<String>) {
        if self.running {
            out.push(self.name.to_owned());
        }
    }
    fn seed_gates(&mut self, base: i64, k: &mut i64) {
        if std::env::var_os("KILN_GATE_DEBUG").is_some() {
            eprintln!("GATE {}: {}", *k, self.list.entries.iter().map(|(b, w, _)| format!("{}/{}", b.name(), w)).collect::<Vec<_>>().join(" "));
        }
        self.list.seed(base + *k);
        *k += 1;
        for (b, _, _) in self.list.entries.iter_mut() {
            b.seed_gates(base, k);
        }
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}

/// `TriggerGate.triggerGate`: a one-shot running declarative triggers in (shuffled) order; it
/// always succeeds.
#[derive(Clone, Debug)]
pub struct TriggerGate {
    order: OrderPolicy,
    running_policy: RunningPolicy,
    list: Shuffled<Box<dyn Control>>,
}

impl TriggerGate {
    /// `TriggerGate.triggerOneShuffled`.
    pub fn one_shuffled(triggers: Vec<(Box<dyn Control>, i32)>) -> Box<dyn Control> {
        TriggerGate::new(triggers, OrderPolicy::Shuffled, RunningPolicy::RunOne)
    }

    pub fn new(triggers: Vec<(Box<dyn Control>, i32)>, order: OrderPolicy, running_policy: RunningPolicy) -> Box<dyn Control> {
        Box::new(TriggerGate { order, running_policy, list: Shuffled::new(triggers) })
    }
}

impl Control for TriggerGate {
    fn name(&self) -> &'static str {
        "TriggerGate"
    }
    fn running(&self) -> bool {
        false
    }
    fn required(&self, out: &mut Vec<Mem>) {
        for (b, _, _) in &self.list.entries {
            b.required(out);
        }
    }
    fn try_start(&mut self, cx: &mut Cx) -> bool {
        if self.order == OrderPolicy::Shuffled {
            self.list.shuffle();
        }
        for (b, _, _) in self.list.entries.iter_mut() {
            if b.try_start(cx) && self.running_policy == RunningPolicy::RunOne {
                break;
            }
        }
        true
    }
    fn tick_or_stop(&mut self, _cx: &mut Cx) {}
    fn do_stop(&mut self, _cx: &mut Cx) {}
    fn seed_gates(&mut self, base: i64, k: &mut i64) {
        if std::env::var_os("KILN_GATE_DEBUG").is_some() {
            eprintln!("GATE {} (trigger): {}", *k, self.list.entries.iter().map(|(b, w, _)| format!("{}/{}", b.name(), w)).collect::<Vec<_>>().join(" "));
        }
        self.list.seed(base + *k);
        *k += 1;
        for (b, _, _) in self.list.entries.iter_mut() {
            b.seed_gates(base, k);
        }
    }
    fn box_clone(&self) -> Box<dyn Control> {
        Box::new(self.clone())
    }
}
