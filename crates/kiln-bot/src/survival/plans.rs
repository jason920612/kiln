//! What each role does when its step queue runs dry: a planner turns the situation (the blocks
//! around, food, what was built so far) into the next few steps.

use super::steps::{PlaceSpec, Step};
use super::{Agent, Dir, Role, add};
use crate::world;
use std::collections::VecDeque;
use std::f64::consts::TAU;

#[derive(Default)]
pub struct State {
    /// Shifts the site when its surface was no place to stand.
    pub site_shift: [f64; 2],
    pub arrival_offset: [f64; 2],
    kit_done: bool,
    fails: u32,
    /// Explorer: heading in radians.
    heading: f64,
    heading_set: bool,
    /// Miner: 0 shaft, 1 tunnels.
    miner_phase: u8,
    miner_dir: Option<Dir>,
    miner_turns: u32,
    /// Builder: operations still to do, and whether they take the house down.
    build: VecDeque<Op>,
    build_phase: u8,
    build_site: Option<[i32; 3]>,
    pub(super) machines: super::redstone::Machines,
}

impl State {
    /// Forget what was in progress (after a teleport or a death).
    pub fn reset(&mut self) {
        self.build.clear();
        self.build_phase = 0;
        self.machines.reset();
        self.miner_phase = self.miner_phase.min(1);
        self.kit_done = false;
    }

    pub fn failed(&mut self) {
        self.fails += 1;
        if self.miner_phase == 0 {
            self.miner_phase = 1;
        }
        self.machines.failed();
        self.build.clear();
        self.build_phase = 0;
    }
}

#[derive(Debug, Clone)]
pub(super) enum Op {
    Place(PlaceSpec),
    Dig([i32; 3]),
    Walk([f64; 2]),
    Wait(u32),
}

impl Agent {
    pub(crate) fn plan_next(&mut self) -> Vec<Step> {
        match self.cfg.role {
            Role::Explorer => self.plan_explorer(),
            Role::Miner => self.plan_miner(),
            Role::Builder => self.plan_builder(),
            Role::Redstone => self.plan_redstone(),
        }
    }

    /// Eating comes first when hungry.
    fn eat_if_hungry(&mut self) -> Option<Vec<Step>> {
        (self.food <= 12).then(|| vec![Step::Equip { slot: 8, item: "cooked_beef", count: 1 }, Step::Eat { slot: 8, waited: 0 }])
    }

    fn plan_explorer(&mut self) -> Vec<Step> {
        if !self.plan.kit_done {
            self.plan.kit_done = true;
            return vec![
                Step::Equip { slot: 3, item: "iron_sword", count: 1 },
                Step::Equip { slot: 8, item: "cooked_beef", count: 1 },
            ];
        }
        if let Some(eat) = self.eat_if_hungry() {
            return eat;
        }
        if !self.plan.heading_set {
            self.plan.heading_set = true;
            self.plan.heading = self.rng.range(0.0, TAU);
        }
        if self.plan.fails > 0 {
            self.plan.fails = 0;
            self.plan.heading += self.rng.range(1.5, 4.5);
        } else {
            self.plan.heading += self.rng.range(-0.5, 0.5);
        }
        let dist = self.rng.range(60.0, 160.0);
        let p = self.body.pos;
        let h = self.plan.heading;
        let to = [p[0] + h.cos() * dist, p[2] + h.sin() * dist];
        let sprint = self.rng.unit() < 0.8;
        let mut v = vec![Step::walk(to, 3.0, sprint)];
        if self.rng.unit() < 0.15 {
            v.push(Step::Wait(self.rng.range(20.0, 100.0) as u32));
        }
        v
    }

    fn plan_miner(&mut self) -> Vec<Step> {
        if !self.plan.kit_done {
            self.plan.kit_done = true;
            return vec![
                Step::Equip { slot: 0, item: "iron_pickaxe", count: 1 },
                Step::Equip { slot: 1, item: "iron_shovel", count: 1 },
                Step::Equip { slot: 2, item: "iron_axe", count: 1 },
                Step::Equip { slot: 3, item: "iron_sword", count: 1 },
                Step::Equip { slot: 4, item: "torch", count: 8 },
                Step::Equip { slot: 5, item: "cobblestone", count: 8 },
                Step::Equip { slot: 8, item: "cooked_beef", count: 1 },
            ];
        }
        if let Some(eat) = self.eat_if_hungry() {
            return eat;
        }
        if self.plan.miner_phase == 0 {
            let p = self.body.pos;
            let y = p[1].floor() as i32;
            let target = (self.rng.range(-58.0, -12.0) as i32).min(y - 6);
            self.plan.miner_phase = 1;
            let center = [p[0].floor() + 0.5, p[2].floor() + 0.5];
            return vec![Step::walk(center, 0.12, false), Step::Shaft { target_y: target }];
        }
        // Tunnels: a long one, then a turn, a short one, another turn.
        let dir = match self.plan.miner_dir {
            None => Dir::HORIZONTAL[self.rng.next_u64() as usize % 4],
            Some(d) => {
                let i = Dir::HORIZONTAL.iter().position(|&x| x == d).unwrap();
                let turn = if self.plan.fails > 0 || self.rng.unit() < 0.5 { 1 } else { 3 };
                Dir::HORIZONTAL[(i + turn) % 4]
            }
        };
        self.plan.fails = 0;
        self.plan.miner_dir = Some(dir);
        self.plan.miner_turns += 1;
        let len = if self.plan.miner_turns % 2 == 0 { 3 } else { self.rng.range(14.0, 32.0) as u32 };
        let p = self.body.pos;
        let center = [p[0].floor() + 0.5, p[2].floor() + 0.5];
        vec![Step::walk(center, 0.15, false), Step::Tunnel { dir, remaining: len, since_torch: 4 }]
    }

    // ---- builder ------------------------------------------------------------------------------

    fn plan_builder(&mut self) -> Vec<Step> {
        if !self.plan.kit_done {
            self.plan.kit_done = true;
            return vec![
                Step::Equip { slot: 0, item: "iron_pickaxe", count: 1 },
                Step::Equip { slot: 2, item: "iron_axe", count: 1 },
                Step::Equip { slot: 3, item: "torch", count: 4 },
                Step::Equip { slot: 4, item: "oak_planks", count: 8 },
                Step::Equip { slot: 5, item: "cobblestone", count: 8 },
                Step::Equip { slot: 6, item: "glass", count: 8 },
                Step::Equip { slot: 8, item: "cooked_beef", count: 1 },
            ];
        }
        if let Some(eat) = self.eat_if_hungry() {
            return eat;
        }
        if self.plan.build.is_empty() {
            match self.plan.build_phase {
                0 => {
                    // Find a flat patch near the site and lay out a house.
                    let around = [self.body.pos[0].floor() as i32, self.body.pos[2].floor() as i32];
                    let Some(site) = self.find_flat_site(around, 3, 40) else {
                        let h = self.rng.range(0.0, TAU);
                        let p = self.body.pos;
                        return vec![Step::walk([p[0] + h.cos() * 40.0, p[2] + h.sin() * 40.0], 3.0, false)];
                    };
                    self.plan.build_site = Some(site);
                    let ops = house(site);
                    self.plan.build.extend(ops);
                    self.plan.build_phase = 1;
                }
                1 => {
                    // Stand around admiring it, then take it down.
                    self.plan.build_phase = 2;
                    let ops = self.demolition();
                    self.plan.build.extend(ops);
                    return vec![Step::Wait(self.rng.range(100.0, 400.0) as u32)];
                }
                _ => {
                    self.plan.build_phase = 0;
                    return vec![Step::Wait(40)];
                }
            }
        }
        // The next few operations.
        let mut v = Vec::new();
        for _ in 0..6 {
            let Some(op) = self.plan.build.pop_front() else { break };
            match op {
                Op::Walk(to) => v.push(Step::walk(to, 0.25, false)),
                Op::Wait(n) => v.push(Step::Wait(n)),
                Op::Dig(pos) => v.push(Step::dig(pos)),
                Op::Place(spec) => {
                    let slot = spec.slot as usize;
                    let item = spec.expect.trim_start_matches("minecraft:");
                    let item: &'static str = match item {
                        "oak_planks" => "oak_planks",
                        "cobblestone" => "cobblestone",
                        "glass" => "glass",
                        _ => "torch",
                    };
                    if self.hot[slot].item != item || self.hot[slot].count < 4 {
                        v.push(Step::Equip { slot: spec.slot, item, count: 8 });
                    }
                    v.push(Step::place(spec));
                }
            }
        }
        v
    }

    /// The blocks of the house standing at the plan's site, dug out top first.
    fn demolition(&self) -> Vec<Op> {
        let Some(site) = self.plan.build_site else { return Vec::new() };
        let mut ops: Vec<Op> = house(site)
            .into_iter()
            .filter_map(|op| match op {
                Op::Place(spec) => Some(spec.pos),
                _ => None,
            })
            .map(Op::Dig)
            .collect();
        ops.reverse();
        ops
    }

    /// A flat, open 5x5 patch (plus a margin) near `around`: the ground y of its centre.
    pub(super) fn find_flat_site(&mut self, around: [i32; 2], half: i32, radius: i32) -> Option<[i32; 3]> {
        for _ in 0..40 {
            let cx = around[0] + self.rng.range(-radius as f64, radius as f64) as i32;
            let cz = around[1] + self.rng.range(-radius as f64, radius as f64) as i32;
            let Some(gy) = self.world.top_block_y(cx, cz, 319) else { continue };
            let mut ok = true;
            'scan: for dx in -half - 1..=half + 1 {
                for dz in -half - 1..=half + 1 {
                    let (x, z) = (cx + dx, cz + dz);
                    let Some(top) = self.world.top_block_y(x, z, 319) else {
                        ok = false;
                        break 'scan;
                    };
                    let s = self.world.block(x, top, z);
                    if top != gy || !world::is_solid(s) || world::name(s).contains("leaves") || world::name(s).contains("ice") {
                        ok = false;
                        break 'scan;
                    }
                }
            }
            if ok {
                return Some([cx, gy, cz]);
            }
        }
        None
    }
}

/// A 5x5 house, three high with a roof, a door gap, glass windows and a torch inside; `site` is
/// the ground block under the centre. The builder stands in the middle.
pub(super) fn house(site: [i32; 3]) -> Vec<Op> {
    let [cx, gy, cz] = site;
    let y0 = gy + 1;
    let mut ops = vec![Op::Walk([cx as f64 + 0.5, cz as f64 + 0.5])];
    let spec = |pos: [i32; 3], slot: u8, expect: &'static str| PlaceSpec { pos, slot, expect, look: None, from: None, facing: None };
    let ring: Vec<[i32; 2]> = {
        let mut r = Vec::new();
        for i in -2..=2 {
            r.push([i, -2]);
        }
        for i in -1..=2 {
            r.push([2, i]);
        }
        for i in (-2..2).rev() {
            r.push([i, 2]);
        }
        for i in (-1..2).rev() {
            r.push([-2, i]);
        }
        r
    };
    for dy in 0..3 {
        for &[dx, dz] in &ring {
            let door = dx == 0 && dz == -2 && dy < 2;
            if door {
                continue;
            }
            let window = dy == 1 && ((dx.abs() == 2 && dz == 0) || (dz == 2 && dx == 0));
            let (slot, name) = if window { (6, "minecraft:glass") } else { (4, "minecraft:oak_planks") };
            ops.push(Op::Place(spec([cx + dx, y0 + dy, cz + dz], slot, name)));
        }
    }
    // Roof: the ring first (it rests on the walls), then the middle.
    for &[dx, dz] in &ring {
        ops.push(Op::Place(spec([cx + dx, y0 + 3, cz + dz], 5, "minecraft:cobblestone")));
    }
    for dx in -1..=1 {
        for dz in -1..=1 {
            ops.push(Op::Place(spec([cx + dx, y0 + 3, cz + dz], 5, "minecraft:cobblestone")));
        }
    }
    ops.push(Op::Place(PlaceSpec { pos: [cx + 1, y0, cz + 1], slot: 3, expect: "minecraft:torch", look: None, from: Some(Dir::Down), facing: None }));
    ops
}

/// Offsets of the 6 neighbours, for scans.
pub(super) fn neighbours(p: [i32; 3]) -> [[i32; 3]; 6] {
    [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East].map(|d| add(p, d.vec()))
}
