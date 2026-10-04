//! The redstone engineer: builds small machines block by block with real placement packets and
//! then tends them. Each machine is a recipe of placements (with the look direction that gives
//! the block its facing), container clicks and lever toggles.
//!
//! * observer clock: two observers facing each other, a lamp on one's output
//! * hopper clock: two hoppers passing items back and forth
//! * hopper line: a feeder chest over three hoppers into a chest
//! * piston door: a sticky piston under a block, worked by a lever
//! * dust line: lever, dust, repeater, dust, lamp

use super::steps::{PlaceSpec, Step};
use super::{Agent, Dir, add};

/// Hotbar slots of the items machines are made of.
const DUST: u8 = 0;
const REPEATER: u8 = 1;
const OBSERVER: u8 = 2;
const STICKY: u8 = 3;
const HOPPER: u8 = 4;
const SOLID: u8 = 5;
const MISC: u8 = 6;

#[derive(Default)]
pub struct Machines {
    built: u32,
    failures: u32,
    levers: Vec<[i32; 3]>,
    tended: usize,
    kit_done: bool,
}

impl Machines {
    pub fn reset(&mut self) {
        self.kit_done = false;
    }

    pub fn failed(&mut self) {
        self.failures += 1;
        self.built += 1;
    }
}

fn eq(slot: u8, item: &'static str) -> Step {
    Step::Equip { slot, item, count: 1 }
}

#[allow(clippy::too_many_arguments)]
fn pl(pos: [i32; 3], slot: u8, expect: &'static str, look: Option<Dir>, from: Option<Dir>, facing: Option<Dir>) -> Step {
    Step::place(PlaceSpec { pos, slot, expect, look, from, facing })
}

impl Agent {
    pub(super) fn plan_redstone(&mut self) -> Vec<Step> {
        if !self.plan.machines.kit_done {
            self.plan.machines.kit_done = true;
            return vec![Step::Equip { slot: 7, item: "iron_sword", count: 1 }, Step::Equip { slot: 8, item: "cooked_beef", count: 1 }];
        }
        if self.food <= 12 {
            return vec![Step::Equip { slot: 8, item: "cooked_beef", count: 1 }, Step::Eat { slot: 8, waited: 0 }];
        }
        const MAX_MACHINES: u32 = 6;
        if self.plan.machines.built < MAX_MACHINES {
            let around = [self.body.pos[0].floor() as i32, self.body.pos[2].floor() as i32];
            let Some(site) = self.find_flat_site(around, (-3, 3), (0, 0), 4, 48) else {
                let h = self.rng.range(0.0, std::f64::consts::TAU);
                let p = self.body.pos;
                return vec![Step::walk([p[0] + h.cos() * 30.0, p[2] + h.sin() * 30.0], 3.0, false)];
            };
            let kind = self.plan.machines.built;
            self.plan.machines.built += 1;
            let (steps, levers) = machine(kind, site);
            self.plan.machines.levers.extend(levers);
            return steps;
        }
        // Tending: toggle a lever now and then, otherwise watch.
        let m = &mut self.plan.machines;
        if m.levers.is_empty() {
            return vec![Step::Wait(200)];
        }
        let lever = m.levers[m.tended % m.levers.len()];
        m.tended += 1;
        let stand = [lever[0] as f64 + 0.5, lever[2] as f64 + 2.5];
        vec![Step::walk(stand, 0.6, false), Step::click(lever, Dir::South), Step::Wait(self.rng.range(100.0, 400.0) as u32)]
    }
}

/// The recipe for machine number `kind` at `site` (the ground block under the middle of a flat
/// area); returns the steps and the levers it has.
fn machine(kind: u32, site: [i32; 3]) -> (Vec<Step>, Vec<[i32; 3]>) {
    let [cx, gy, cz] = site;
    let y0 = gy + 1;
    let origin = [cx - 2, y0, cz];
    // Stand south of the machine, in reach of all of it.
    let stand = Step::walk([cx as f64 + 0.5, cz as f64 + 2.5], 0.3, false);
    let mut v = vec![stand];
    let mut levers = Vec::new();
    match kind % 5 {
        0 => {
            // Two observers face to face. An observer placed against a block faces into it, so
            // the first goes against a stone in the second one's place, which then makes way.
            let (o1, o2) = (origin, add(origin, [1, 0, 0]));
            v.push(eq(SOLID, "stone"));
            v.push(pl(o2, SOLID, "minecraft:stone", None, Some(Dir::Down), None));
            v.push(eq(OBSERVER, "observer"));
            v.push(pl(o1, OBSERVER, "minecraft:observer", None, Some(Dir::East), Some(Dir::East)));
            v.push(eq(0, "iron_pickaxe"));
            v.push(Step::dig(o2));
            v.push(eq(OBSERVER, "observer"));
            v.push(pl(o2, OBSERVER, "minecraft:observer", None, Some(Dir::West), Some(Dir::West)));
            // A lamp behind the first observer shows the pulses.
            v.push(eq(MISC, "redstone_lamp"));
            v.push(pl(add(origin, [-1, 0, 0]), MISC, "minecraft:redstone_lamp", None, Some(Dir::Down), None));
        }
        1 => {
            // Hopper clock: H1 faces a stone, which then makes way for H2 facing back.
            let (h1, h2) = (origin, add(origin, [1, 0, 0]));
            v.push(eq(SOLID, "stone"));
            v.push(pl(h2, SOLID, "minecraft:stone", None, Some(Dir::Down), None));
            v.push(eq(HOPPER, "hopper"));
            v.push(pl(h1, HOPPER, "minecraft:hopper", None, Some(Dir::East), Some(Dir::East)));
            v.push(eq(0, "iron_pickaxe"));
            v.push(Step::dig(h2));
            v.push(eq(HOPPER, "hopper"));
            v.push(pl(h2, HOPPER, "minecraft:hopper", None, Some(Dir::West), Some(Dir::West)));
            v.push(eq(MISC, "cobblestone"));
            v.push(Step::click(h1, Dir::Up));
            v.push(Step::AwaitWindow { waited: 0 });
            v.push(Step::MoveHotbar { slot: MISC, waited: 0 });
            v.push(Step::CloseWindow);
        }
        2 => {
            // A feeder chest over three hoppers pointing into a chest.
            let c1 = add(origin, [3, 0, 0]);
            v.push(eq(MISC, "chest"));
            v.push(pl(c1, MISC, "minecraft:chest", Some(Dir::West), Some(Dir::Down), None));
            v.push(eq(HOPPER, "hopper"));
            for i in (0..3).rev() {
                let h = add(origin, [i, 0, 0]);
                v.push(pl(h, HOPPER, "minecraft:hopper", None, Some(Dir::East), Some(Dir::East)));
            }
            v.push(eq(MISC, "chest"));
            let c0 = add(origin, [0, 1, 0]);
            v.push(pl(c0, MISC, "minecraft:chest", Some(Dir::West), Some(Dir::Down), None));
            v.push(eq(MISC, "cobblestone"));
            v.push(Step::click(c0, Dir::Up));
            v.push(Step::AwaitWindow { waited: 0 });
            v.push(Step::MoveHotbar { slot: MISC, waited: 0 });
            v.push(Step::CloseWindow);
        }
        3 => {
            // Piston door: a sticky piston facing up with a block on its head, a lever on its side.
            let p = add(origin, [1, 0, 0]);
            v.push(eq(STICKY, "sticky_piston"));
            v.push(pl(p, STICKY, "minecraft:sticky_piston", Some(Dir::Down), Some(Dir::Down), Some(Dir::Up)));
            v.push(eq(SOLID, "stone"));
            v.push(pl(add(p, [0, 1, 0]), SOLID, "minecraft:stone", None, Some(Dir::Down), None));
            v.push(eq(MISC, "lever"));
            let lever = add(p, [0, 0, 1]);
            v.push(pl(lever, MISC, "minecraft:lever", None, Some(Dir::North), None));
            levers.push(lever);
        }
        _ => {
            // Lever, dust, repeater, dust, lamp along +x.
            v.push(eq(MISC, "lever"));
            let lever = origin;
            v.push(pl(lever, MISC, "minecraft:lever", None, Some(Dir::Down), None));
            v.push(eq(DUST, "redstone"));
            v.push(pl(add(origin, [1, 0, 0]), DUST, "minecraft:redstone_wire", None, Some(Dir::Down), None));
            v.push(pl(add(origin, [2, 0, 0]), DUST, "minecraft:redstone_wire", None, Some(Dir::Down), None));
            v.push(eq(REPEATER, "repeater"));
            v.push(pl(add(origin, [3, 0, 0]), REPEATER, "minecraft:repeater", Some(Dir::East), Some(Dir::Down), Some(Dir::West)));
            v.push(eq(DUST, "redstone"));
            v.push(pl(add(origin, [4, 0, 0]), DUST, "minecraft:redstone_wire", None, Some(Dir::Down), None));
            v.push(eq(MISC, "redstone_lamp"));
            v.push(pl(add(origin, [5, 0, 0]), MISC, "minecraft:redstone_lamp", None, Some(Dir::Down), None));
            levers.push(lever);
        }
    }
    (v, levers)
}
