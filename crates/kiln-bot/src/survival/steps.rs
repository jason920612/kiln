//! Steps: the small jobs a survival bot's plan is made of (walk, dig, place, open a chest...).
//! Each runs over several client ticks and sends the packets the vanilla client would.

use super::{Agent, Dir, Pending, Window, add, look_at, tools};
use crate::Out;
use crate::physics::{EYE, Input};
use crate::proto;
use crate::world;
use kiln_data::block_props;

/// Result of running a step for one tick.
pub enum Res {
    Working,
    Done,
    Failed(&'static str),
    /// Replaces the step by these (and continues with them).
    Expand(Vec<Step>),
}

#[derive(Debug, Clone, Default)]
pub struct WalkState {
    since_check: u32,
    last: [f64; 2],
    stuck: u8,
    detour: f64,
    detour_ticks: u32,
    started: bool,
    waited: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DigState {
    phase: u8,
    elapsed: u32,
    need: u32,
    wait: u32,
    seq: i32,
}

#[derive(Debug, Clone, Default)]
pub struct PlaceState {
    phase: u8,
    wait: u32,
    seq: i32,
}

/// A block to place and how: which hotbar slot, what should result, how the player looks and
/// which neighbour is clicked.
#[derive(Debug, Clone)]
pub struct PlaceSpec {
    pub pos: [i32; 3],
    pub slot: u8,
    /// Block name that should be there afterwards.
    pub expect: &'static str,
    /// Where the player looks (orientation of repeaters, pistons, observers...).
    pub look: Option<Dir>,
    /// Direction from the target to the block clicked; automatic when `None`.
    pub from: Option<Dir>,
    /// The `facing` the placed block should have (checked, not enforced).
    pub facing: Option<Dir>,
}

#[derive(Debug, Clone)]
pub enum Step {
    /// Walk toward (x, z) until within `tol` blocks.
    Walk { to: [f64; 2], tol: f64, sprint: bool, st: WalkState },
    /// Make sure hotbar `slot` holds `item` (at least `count` of it) and select it.
    Equip { slot: u8, item: &'static str, count: u32 },
    Cmd(String),
    Wait(u32),
    /// Break a block with the right tool.
    Dig { pos: [i32; 3], tries: u8, st: DigState },
    /// Dig straight down until `target_y`.
    Shaft { target_y: i32 },
    /// Tunnel two blocks high in a horizontal direction, lighting and stripping ores.
    Tunnel { dir: Dir, remaining: u32, since_torch: u32 },
    Place { spec: PlaceSpec, tries: u8, st: PlaceState },
    /// Right click a block face (toggle a lever, open a chest).
    Click { pos: [i32; 3], face: Dir, st: PlaceState },
    /// Wait for a container screen after a click.
    AwaitWindow { waited: u32 },
    /// Shift-click one of the hotbar slots into the open container.
    MoveHotbar { slot: u8, waited: u32 },
    /// Shift-click a slot of the open container.
    MoveSlot { slot: i16, waited: u32 },
    CloseWindow,
    /// Eat the food in the held slot.
    Eat { slot: u8, waited: u32 },
}

impl Step {
    pub fn walk(to: [f64; 2], tol: f64, sprint: bool) -> Step {
        Step::Walk { to, tol, sprint, st: WalkState::default() }
    }

    pub fn walk_to_cell(pos: [i32; 3]) -> Step {
        Step::walk([pos[0] as f64 + 0.5, pos[2] as f64 + 0.5], 0.3, false)
    }

    pub fn dig(pos: [i32; 3]) -> Step {
        Step::Dig { pos, tries: 3, st: DigState::default() }
    }

    pub fn place(spec: PlaceSpec) -> Step {
        Step::Place { spec, tries: 2, st: PlaceState::default() }
    }

    pub fn click(pos: [i32; 3], face: Dir) -> Step {
        Step::Click { pos, face, st: PlaceState::default() }
    }
}

/// Blocks a right click opens or changes instead of placing against them.
pub fn interactive(state: u16) -> bool {
    let n = world::name(state).trim_start_matches("minecraft:");
    block_props::has_block_entity(state)
        || [
            "button", "lever", "door", "trapdoor", "repeater", "comparator", "gate", "bed", "anvil", "crafting_table", "loom", "grindstone",
            "stonecutter", "smithing", "note_block", "daylight", "cartography", "enchanting", "fletching", "composter",
        ]
        .iter()
        .any(|p| n.contains(p))
}

fn yaw_of(dx: f64, dz: f64) -> f64 {
    (-dx).atan2(dz).to_degrees()
}

impl Agent {
    pub(crate) fn exec(&mut self, step: &mut Step, out: &mut Out, input: &mut Input) -> Res {
        match step {
            Step::Walk { to, tol, sprint, st } => self.walk(*to, *tol, *sprint, st, input),
            Step::Equip { slot, item, count } => {
                let slot = *slot;
                let have = self.hot[slot as usize];
                if have.item != *item || have.count < (*count).min(64) {
                    let c = format!("item replace entity @s hotbar.{slot} with minecraft:{item} 64");
                    self.command(out, &c);
                    self.hot[slot as usize] = super::Slot { item, count: 64 };
                    self.awaiting_slot = Some((slot, self.tick_no + 40));
                }
                self.select(out, slot);
                Res::Done
            }
            Step::Cmd(c) => {
                let c = std::mem::take(c);
                self.command(out, &c);
                Res::Done
            }
            Step::Wait(n) => {
                if *n == 0 {
                    return Res::Done;
                }
                *n -= 1;
                Res::Working
            }
            Step::Dig { pos, tries, st } => self.dig(*pos, tries, st, out),
            Step::Shaft { target_y } => {
                let target = *target_y;
                if self.body.pos[1] <= target as f64 + 0.01 {
                    return Res::Done;
                }
                if !self.body.on_ground {
                    return Res::Working;
                }
                let [bx, by, bz] = self.body.block_pos();
                let below = self.world.get(bx, by - 1, bz);
                let Some(below) = below else { return Res::Working };
                let deeper = self.world.block(bx, by - 2, bz);
                if world::is_lava(below) || world::is_lava(deeper) || world::is_water(below) || world::is_water(deeper) {
                    return Res::Failed("fluid below the shaft");
                }
                if block_props::hardness(below) < 0.0 {
                    return Res::Done;
                }
                if world::is_air(below) {
                    return Res::Working;
                }
                Res::Expand(vec![Step::dig([bx, by - 1, bz]), Step::Shaft { target_y: target }])
            }
            Step::Tunnel { dir, remaining, since_torch } => self.tunnel(*dir, *remaining, *since_torch),
            Step::Place { spec, tries, st } => self.place(spec, tries, st, out),
            Step::Click { pos, face, st } => self.click(*pos, *face, st, out),
            Step::AwaitWindow { waited } => {
                if self.window.is_some_and(|w| w.slots > 0) {
                    return Res::Done;
                }
                *waited += 1;
                if *waited > 40 { Res::Failed("no container screen opened") } else { Res::Working }
            }
            Step::MoveHotbar { slot, waited } => {
                let Some(Window { id, state_id, slots }) = self.window else { return Res::Failed("no window") };
                if *waited == 0 {
                    let s = (slots - 9 + *slot as i32) as i16;
                    out.send(|b| proto::container_quick_move(b, id, state_id, s));
                    self.counts.items_moved += 1;
                }
                *waited += 1;
                if *waited > 4 { Res::Done } else { Res::Working }
            }
            Step::MoveSlot { slot, waited } => {
                let Some(Window { id, state_id, .. }) = self.window else { return Res::Failed("no window") };
                if *waited == 0 {
                    let s = *slot;
                    out.send(|b| proto::container_quick_move(b, id, state_id, s));
                    self.counts.items_moved += 1;
                }
                *waited += 1;
                if *waited > 4 { Res::Done } else { Res::Working }
            }
            Step::CloseWindow => {
                if let Some(w) = self.window.take() {
                    out.send(|b| proto::container_close(b, w.id));
                }
                Res::Done
            }
            Step::Eat { slot, waited } => {
                if *waited == 0 {
                    self.select(out, *slot);
                    let seq = self.next_seq();
                    let (yaw, pitch) = (self.body.yaw, self.body.pitch);
                    out.send(|b| proto::use_item(b, seq, yaw, pitch));
                    self.counts.eaten += 1;
                }
                *waited += 1;
                if *waited > 36 { Res::Done } else { Res::Working }
            }
        }
    }

    // ---- walking ------------------------------------------------------------------------

    fn walk(&mut self, to: [f64; 2], tol: f64, sprint: bool, st: &mut WalkState, input: &mut Input) -> Res {
        let p = self.body.pos;
        let (dx, dz) = (to[0] - p[0], to[1] - p[2]);
        let dist = dx.hypot(dz);
        if dist <= tol {
            return Res::Done;
        }
        if !st.started {
            st.started = true;
            st.last = [p[0], p[2]];
        }
        let mut yaw = yaw_of(dx, dz);
        if st.detour_ticks > 0 {
            st.detour_ticks -= 1;
            yaw += st.detour;
        }
        let (sin, cos) = yaw.to_radians().sin_cos();
        let (fx, fz) = (-sin, cos);
        // Like the vanilla client, wait for terrain that has not arrived.
        let (ax, az) = ((p[0] + fx * 2.0).floor() as i32, (p[2] + fz * 2.0).floor() as i32);
        if !self.world.has_chunk(ax >> 4, az >> 4) {
            self.counts.stall_ticks += 1;
            self.body.yaw = yaw as f32;
            return Res::Working;
        }
        if self.body.on_ground && st.detour_ticks == 0 && self.hazard_ahead(p, fx, fz) {
            st.detour = if self.rng.unit() < 0.5 { 90.0 } else { -90.0 };
            st.detour_ticks = 30;
            yaw += st.detour;
        }
        input.forward = true;
        input.sprint = sprint && self.food > 6 && !self.body.in_water && !self.body.in_lava;
        input.jump = self.body.in_water || self.body.in_lava || (self.body.horizontal_collision && self.body.on_ground);
        self.body.yaw = yaw as f32;
        self.body.pitch = 0.0;
        st.since_check += 1;
        if st.since_check >= 50 {
            st.since_check = 0;
            if (p[0] - st.last[0]).hypot(p[2] - st.last[1]) < 1.5 {
                st.stuck += 1;
                if st.stuck >= 4 {
                    return Res::Failed("stuck");
                }
                st.detour = self.rng.range(60.0, 150.0) * if self.rng.unit() < 0.5 { 1.0 } else { -1.0 };
                st.detour_ticks = 25;
            } else {
                st.stuck = 0;
            }
            st.last = [p[0], p[2]];
        }
        Res::Working
    }

    /// Whether the next steps ahead lead off a cliff or into lava.
    fn hazard_ahead(&self, p: [f64; 3], fx: f64, fz: f64) -> bool {
        let y0 = p[1].floor() as i32;
        for k in [1.0, 2.0] {
            let (x, z) = ((p[0] + fx * k).floor() as i32, (p[2] + fz * k).floor() as i32);
            let mut ground = None;
            for y in (y0 - 4..=y0 + 1).rev() {
                let s = self.world.block(x, y, z);
                if world::is_lava(s) {
                    return true;
                }
                if world::is_solid(s) || world::is_water(s) {
                    ground = Some(y);
                    break;
                }
            }
            if ground.is_none() {
                return true;
            }
            let feet = self.world.block(x, y0, z);
            if world::is_lava(feet) {
                return true;
            }
        }
        false
    }

    // ---- digging ------------------------------------------------------------------------

    fn reachable(&self, pos: [i32; 3]) -> bool {
        let eye = self.body.eye();
        let d2: f64 = (0..3)
            .map(|i| {
                let (lo, hi) = (pos[i] as f64, pos[i] as f64 + 1.0);
                (lo - eye[i]).max(0.0).max(eye[i] - hi).powi(2)
            })
            .sum();
        d2 < 4.3 * 4.3
    }

    fn look_at_block(&mut self, pos: [i32; 3]) {
        let eye = [self.body.pos[0], self.body.pos[1] + EYE, self.body.pos[2]];
        let (yaw, pitch) = look_at(eye, [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5]);
        self.body.yaw = yaw;
        self.body.pitch = pitch.clamp(-90.0, 90.0);
    }

    /// The face of the block nearest to the eyes.
    fn face_toward_eye(&self, pos: [i32; 3]) -> Dir {
        let eye = self.body.eye();
        let c = [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5];
        let d = [eye[0] - c[0], eye[1] - c[1], eye[2] - c[2]];
        let i = (0..3).max_by(|&a, &b| d[a].abs().total_cmp(&d[b].abs())).unwrap();
        match (i, d[i] >= 0.0) {
            (0, true) => Dir::East,
            (0, false) => Dir::West,
            (1, true) => Dir::Up,
            (1, false) => Dir::Down,
            (_, true) => Dir::South,
            (_, false) => Dir::North,
        }
    }

    fn dig(&mut self, pos: [i32; 3], tries: &mut u8, st: &mut DigState, out: &mut Out) -> Res {
        let Some(state) = self.world.get(pos[0], pos[1], pos[2]) else { return Res::Failed("dig: chunk unknown") };
        let breakable = !world::is_air(state) && !world::is_lava(state) && !world::is_water(state);
        match st.phase {
            0 => {
                if !breakable {
                    return Res::Done;
                }
                if block_props::hardness(state) < 0.0 {
                    return Res::Failed("dig: unbreakable");
                }
                if !self.reachable(pos) {
                    return Res::Failed("dig: out of reach");
                }
                let kind = tools::wanted(state);
                let slot = tools::slot_of(kind);
                let carried = kind != tools::Kind::Hand && self.hot[slot as usize].item == tools::item_of(kind);
                // Not carrying that tool (a kit without it): dig with the empty hand.
                self.select(out, if carried { slot } else { tools::slot_of(tools::Kind::Hand) });
                if self.dig_cooldown > 0 || self.awaiting_slot.is_some() {
                    return Res::Working;
                }
                if !self.body.on_ground && !self.body.in_water {
                    return Res::Working;
                }
                let held_kind = if carried { kind } else { tools::Kind::Hand };
                self.look_at_block(pos);
                let Some(need) = tools::break_ticks(state, held_kind, self.body.on_ground, self.body.in_water) else {
                    return Res::Failed("dig: unbreakable");
                };
                let face = self.face_toward_eye(pos);
                let seq = self.next_seq();
                out.send(|b| proto::player_action(b, proto::action::START_DESTROY_BLOCK, pos, face.face(), seq));
                self.counts.dig_started += 1;
                st.seq = seq;
                st.need = need;
                st.elapsed = 0;
                if need == 0 {
                    self.pending.push((seq, Pending::Dig { pos, was: state }));
                    out.send(proto::punch);
                    st.phase = 2;
                    st.wait = 0;
                } else {
                    st.phase = 1;
                }
                Res::Working
            }
            1 => {
                if !breakable {
                    // Someone else broke it, or the server did: nothing left to do.
                    let face = self.face_toward_eye(pos);
                    let seq = self.next_seq();
                    out.send(|b| proto::player_action(b, proto::action::ABORT_DESTROY_BLOCK, pos, face.face(), seq));
                    return Res::Done;
                }
                out.send(proto::punch);
                st.elapsed += 1;
                if st.elapsed >= st.need {
                    let face = self.face_toward_eye(pos);
                    let seq = self.next_seq();
                    out.send(|b| proto::player_action(b, proto::action::STOP_DESTROY_BLOCK, pos, face.face(), seq));
                    self.pending.push((seq, Pending::Dig { pos, was: state }));
                    self.dig_cooldown = 5;
                    st.phase = 2;
                    st.wait = 0;
                }
                Res::Working
            }
            _ => {
                if !breakable {
                    return Res::Done;
                }
                st.wait += 1;
                // A laggy server breaks the block late (its clock is behind): give it time.
                if st.wait > 40 {
                    *tries -= 1;
                    if *tries == 0 {
                        return Res::Failed("dig: the server kept the block");
                    }
                    *st = DigState::default();
                }
                Res::Working
            }
        }
    }

    // ---- tunnelling -----------------------------------------------------------------------

    fn tunnel(&mut self, dir: Dir, remaining: u32, since_torch: u32) -> Res {
        if remaining == 0 {
            return Res::Done;
        }
        let p = self.body.block_pos();
        let ahead = add(p, dir.vec());
        let head = add(ahead, [0, 1, 0]);
        let floor = add(ahead, [0, -1, 0]);
        let w = &self.world;
        let at = |q: [i32; 3]| w.get(q[0], q[1], q[2]);
        let (Some(_), Some(_), Some(fl)) = (at(ahead), at(head), at(floor)) else { return Res::Working };
        // Fluids in the new cells turn the tunnel away; fluids next to them are plugged with
        // cobblestone first, as a careful miner does.
        let mut ores = Vec::new();
        let mut plugs: Vec<[i32; 3]> = Vec::new();
        for cell in [ahead, head] {
            let s = w.block(cell[0], cell[1], cell[2]);
            if world::is_lava(s) || world::is_water(s) {
                return Res::Failed("fluid ahead in the tunnel");
            }
            for d in [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East] {
                let n = add(cell, d.vec());
                let s = w.block(n[0], n[1], n[2]);
                if (world::is_lava(s) || world::is_water(s)) && !plugs.contains(&n) && n != p && n != add(p, [0, 1, 0]) {
                    plugs.push(n);
                }
                if world::name(s).ends_with("_ore") && !ores.contains(&n) && n != p && n != add(p, [0, 1, 0]) {
                    ores.push(n);
                }
            }
        }
        if plugs.len() > 4 {
            return Res::Failed("too much fluid next to the tunnel");
        }
        let mut steps = Vec::new();
        if !plugs.is_empty() || world::is_air(fl) {
            steps.push(Step::Equip { slot: 5, item: "cobblestone", count: 6 });
        }
        for n in &plugs {
            steps.push(Step::place(PlaceSpec { pos: *n, slot: 5, expect: "minecraft:cobblestone", look: None, from: None, facing: None }));
        }
        if world::is_lava(fl) || world::is_water(fl) {
            return Res::Failed("fluid in the floor");
        }
        if world::is_air(fl) {
            steps.push(Step::place(PlaceSpec { pos: floor, slot: 5, expect: "minecraft:cobblestone", look: None, from: None, facing: None }));
        }
        for cell in [ahead, head] {
            let s = w.block(cell[0], cell[1], cell[2]);
            if !world::is_air(s) {
                if block_props::hardness(s) < 0.0 {
                    return Res::Failed("bedrock ahead");
                }
                steps.push(Step::dig(cell));
            }
        }
        for ore in ores.into_iter().take(3) {
            steps.push(Step::dig(ore));
        }
        steps.push(Step::walk_to_cell(ahead));
        let mut torch = since_torch + 1;
        if torch >= 8 {
            torch = 0;
            steps.push(Step::Equip { slot: 4, item: "torch", count: 1 });
            steps.push(Step::place(PlaceSpec {
                pos: ahead,
                slot: 4,
                expect: "minecraft:torch",
                look: None,
                from: Some(Dir::Down),
                facing: None,
            }));
        }
        steps.push(Step::Tunnel { dir, remaining: remaining - 1, since_torch: torch });
        Res::Expand(steps)
    }

    // ---- placing and clicking -----------------------------------------------------------------

    fn place(&mut self, spec: &PlaceSpec, tries: &mut u8, st: &mut PlaceState, out: &mut Out) -> Res {
        let pos = spec.pos;
        let Some(cur) = self.world.get(pos[0], pos[1], pos[2]) else { return Res::Failed("place: chunk unknown") };
        if st.phase == 0 {
            if world::name(cur) == spec.expect {
                return Res::Done;
            }
            if !(world::is_air(cur) || block_props::replaceable(cur) || world::is_water(cur) || world::is_lava(cur)) {
                return Res::Failed("place: the cell is taken");
            }
            if self.tick_no < self.last_place_tick + 3 || self.dig_cooldown > 0 {
                return Res::Working;
            }
            if self.hot[spec.slot as usize].count == 0 {
                return Res::Failed("place: out of blocks");
            }
            if self.overlaps_body(pos, spec.expect) {
                return Res::Failed("place: the bot is in the way");
            }
            let support = self.pick_support(pos, spec.from);
            let Some(from) = support else { return Res::Failed("place: nothing to click") };
            let s = add(pos, from.vec());
            if !self.reachable(s) {
                return Res::Failed("place: out of reach");
            }
            let ss = self.world.block(s[0], s[1], s[2]);
            self.select(out, spec.slot);
            self.sneaking = interactive(ss);
            match spec.look {
                Some(d) => {
                    let (yaw, pitch) = d.look();
                    self.body.yaw = yaw;
                    self.body.pitch = pitch;
                }
                None => {
                    let n = from.opposite().vec();
                    let c = [s[0] as f64 + 0.5 + 0.5 * n[0] as f64, s[1] as f64 + 0.5 + 0.5 * n[1] as f64, s[2] as f64 + 0.5 + 0.5 * n[2] as f64];
                    let (yaw, pitch) = look_at(self.body.eye(), c);
                    self.body.yaw = yaw;
                    self.body.pitch = pitch.clamp(-90.0, 90.0);
                }
            }
            st.phase = 1;
            st.wait = 0;
            return Res::Working;
        }
        if st.phase == 1 {
            // The rotation and sneak flag went out with the last tick's packets.
            let Some(from) = self.pick_support(pos, spec.from) else { return Res::Failed("place: nothing to click") };
            let s = add(pos, from.vec());
            let n = from.opposite();
            let nv = n.vec();
            let cursor = [0.5 + 0.5 * nv[0] as f32, 0.5 + 0.5 * nv[1] as f32, 0.5 + 0.5 * nv[2] as f32];
            let seq = self.next_seq();
            out.send(|b| proto::use_item_on(b, s, n.face(), cursor, seq));
            out.send(proto::punch);
            self.pending.push((seq, Pending::Place { pos, expect: spec.expect, facing: spec.facing, slot: spec.slot }));
            self.last_place_tick = self.tick_no;
            st.seq = seq;
            st.phase = 2;
            st.wait = 0;
            return Res::Working;
        }
        st.wait += 1;
        let acked = !self.pending.iter().any(|(s, _)| *s == st.seq);
        if acked || st.wait > 30 {
            self.sneaking = false;
            let now = self.world.get(pos[0], pos[1], pos[2]).unwrap_or(0);
            if world::name(now) == spec.expect || *tries <= 1 {
                return Res::Done;
            }
            *tries -= 1;
            *st = PlaceState::default();
        }
        Res::Working
    }

    /// Whether the block to be placed (by name) would collide with the bot.
    fn overlaps_body(&self, pos: [i32; 3], expect: &str) -> bool {
        let solid = kiln_data::blocks_types::block_by_name(expect).is_none_or(|b| !block_props::collision(b.default).is_empty());
        if !solid {
            return false;
        }
        let p = self.body.pos;
        let (x0, x1, z0, z1) = (p[0] - 0.3, p[0] + 0.3, p[2] - 0.3, p[2] + 0.3);
        let (y0, y1) = (p[1], p[1] + 1.8);
        (pos[0] as f64) < x1
            && (pos[0] + 1) as f64 > x0
            && (pos[2] as f64) < z1
            && (pos[2] + 1) as f64 > z0
            && (pos[1] as f64) < y1
            && (pos[1] + 1) as f64 > y0
    }

    fn pick_support(&self, pos: [i32; 3], prefer: Option<Dir>) -> Option<Dir> {
        let order = [Dir::Down, Dir::North, Dir::East, Dir::South, Dir::West, Dir::Up];
        let ok = |d: Dir| {
            let s = add(pos, d.vec());
            let st = self.world.block(s[0], s[1], s[2]);
            !(world::is_air(st) || world::is_water(st) || world::is_lava(st) || block_props::replaceable(st))
        };
        match prefer {
            Some(d) => ok(d).then_some(d),
            None => order.into_iter().find(|&d| ok(d)),
        }
    }

    fn click(&mut self, pos: [i32; 3], face: Dir, st: &mut PlaceState, out: &mut Out) -> Res {
        match st.phase {
            0 => {
                if !self.reachable(pos) {
                    return Res::Failed("click: out of reach");
                }
                self.sneaking = false;
                self.look_at_block(pos);
                st.phase = 1;
                Res::Working
            }
            1 => {
                let nv = face.vec();
                let cursor = [0.5 + 0.5 * nv[0] as f32, 0.5 + 0.5 * nv[1] as f32, 0.5 + 0.5 * nv[2] as f32];
                let seq = self.next_seq();
                out.send(|b| proto::use_item_on(b, pos, face.face(), cursor, seq));
                out.send(proto::punch);
                self.counts.levers += 1;
                st.phase = 2;
                st.wait = 0;
                Res::Working
            }
            _ => {
                st.wait += 1;
                if st.wait >= 3 { Res::Done } else { Res::Working }
            }
        }
    }
}
