//! The players' section grid (`PlayerGrid`) and the searches built on it answer exactly what a
//! scan of every player would: the same players in the same order, the same nearest player.

use kiln_entity::Entity;
use kiln_entity::level::{EntityFilter, EntityLevel, Event, PlayerGrid, PlayerView, nearest_player_to, player_box};
use kiln_entity::math::{Aabb, BlockPos, Vec3};
use kiln_entity::memory::MemoryLevel;
use kiln_javamath::random::LegacyRandom;

/// A level whose player queries go through a `PlayerGrid`; the rest is a `MemoryLevel`.
struct GridLevel {
    inner: MemoryLevel,
    grid: PlayerGrid,
    /// Answer the player queries by scanning every player instead (the reference).
    scan: bool,
}

impl GridLevel {
    fn new(views: Vec<PlayerView>) -> GridLevel {
        let grid = PlayerGrid::build(&views);
        let mut inner = MemoryLevel::new(-64, 1);
        inner.players = views;
        GridLevel { inner, grid, scan: false }
    }

    fn scanning(views: Vec<PlayerView>) -> GridLevel {
        GridLevel { scan: true, ..GridLevel::new(views) }
    }
}

impl EntityLevel for GridLevel {
    fn block(&self, pos: BlockPos) -> u16 {
        self.inner.block(pos)
    }
    fn set_block(&mut self, pos: BlockPos, state: u16, flags: u32) -> bool {
        self.inner.set_block(pos, state, flags)
    }
    fn random(&mut self) -> &mut LegacyRandom {
        self.inner.random()
    }
    fn game_time(&self) -> i64 {
        self.inner.game_time()
    }
    fn min_y(&self) -> i32 {
        self.inner.min_y()
    }
    fn entities_in(&self, area: &Aabb, filter: EntityFilter, exclude: i32) -> Vec<i32> {
        self.inner.entities_in(area, filter, exclude)
    }
    fn entity_mut(&mut self, id: i32) -> Option<&mut Entity> {
        self.inner.entity_mut(id)
    }
    fn entity(&self, id: i32) -> Option<&Entity> {
        self.inner.entity(id)
    }
    fn add_entity(&mut self, entity: Entity) {
        self.inner.add_entity(entity)
    }
    fn next_entity_id(&mut self) -> i32 {
        self.inner.next_entity_id()
    }
    fn fresh_seed(&mut self) -> i64 {
        self.inner.fresh_seed()
    }
    fn emit(&mut self, event: Event) {
        self.inner.emit(event)
    }
    fn players(&self) -> &[PlayerView] {
        &self.inner.players
    }
    fn players_in(&self, area: &Aabb) -> Vec<PlayerView> {
        if self.scan {
            return self.inner.players.iter().filter(|p| player_box(p).intersects(area)).copied().collect();
        }
        self.grid.in_area(&self.inner.players, area)
    }
    fn player(&self, id: i32) -> Option<PlayerView> {
        if self.scan {
            return self.inner.players.iter().find(|p| p.id == id).copied();
        }
        self.grid.by_id(&self.inner.players, id)
    }
    fn player_by_uuid(&self, uuid: u128) -> Option<PlayerView> {
        if self.scan {
            return self.inner.players.iter().find(|p| p.uuid == uuid).copied();
        }
        self.grid.by_uuid(&self.inner.players, uuid)
    }
}

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    }
}

fn crowd(rng: &mut Rng, n: usize, spread: f64) -> Vec<PlayerView> {
    (0..n)
        .map(|i| {
            let mut p = PlayerView::new(100 + i as i32, Vec3::new(rng.f(-spread, spread), rng.f(-20.0, 200.0), rng.f(-spread, spread)));
            // Duplicated UUIDs on purpose: the first one wins, as a scan finds it.
            p.uuid = (rng.next() % (n as u64 / 2 + 1)) as u128;
            p.sneaking = rng.next() % 3 == 0;
            p.spectator = rng.next() % 7 == 0;
            p.alive = rng.next() % 9 != 0;
            p
        })
        .collect()
}

#[test]
fn grid_queries_match_a_scan() {
    let mut rng = Rng(7);
    for (n, spread) in [(0, 10.0), (1, 10.0), (5, 30.0), (200, 100.0), (1000, 2000.0), (300, 20.0)] {
        let views = crowd(&mut rng, n, spread);
        let grid = PlayerGrid::build(&views);
        for _ in 0..300 {
            let (x, y, z) = (rng.f(-spread, spread), rng.f(-30.0, 210.0), rng.f(-spread, spread));
            let (w, h, d) = (rng.f(0.1, 60.0), rng.f(0.1, 60.0), rng.f(0.1, 60.0));
            let area = Aabb::new(x, y, z, x + w, y + h, z + d);
            let want: Vec<PlayerView> = views.iter().filter(|p| player_box(p).intersects(&area)).copied().collect();
            assert_eq!(grid.in_area(&views, &area), want, "area {area:?} of {n} players");
        }
        for v in &views {
            assert_eq!(grid.by_id(&views, v.id), Some(*v));
            assert_eq!(grid.by_uuid(&views, v.uuid), views.iter().find(|p| p.uuid == v.uuid).copied());
        }
        assert_eq!(grid.by_id(&views, -5), None);
        assert_eq!(grid.by_uuid(&views, u128::MAX), None);
    }
}

/// `cargo test -p kiln-entity --test player_index -- --ignored --nocapture`: what one query
/// costs on a crowd server, scanning every player against going through the grid.
#[test]
#[ignore]
fn crowd_bench() {
    use std::time::Instant;
    let mut rng = Rng(3);
    // A target filter with some cost, as the combat conditions are (about 100 ns).
    let costly = |p: &PlayerView| {
        let mut h = p.id as u64;
        for _ in 0..50 {
            h = std::hint::black_box(h.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (h >> 7));
        }
        p.alive && !p.spectator && h != 1
    };
    for (n, spread) in [(100usize, 1000.0), (1000, 1000.0), (1000, 120.0), (5000, 1000.0), (5000, 250.0)] {
        let views = crowd(&mut rng, n, spread);
        let level = GridLevel::new(views.clone());
        let queries: Vec<Vec3> = (0..2000).map(|_| Vec3::new(rng.f(-spread, spread), rng.f(0.0, 200.0), rng.f(-spread, spread))).collect();
        let t = Instant::now();
        let mut sink = 0usize;
        for q in &queries {
            let best = views.iter().filter(|p| costly(p)).min_by(|a, b| a.pos.distance_to_sqr(*q).total_cmp(&b.pos.distance_to_sqr(*q)));
            sink += best.map_or(0, |p| p.id as usize);
        }
        let scan = t.elapsed();
        let t = Instant::now();
        for q in &queries {
            sink += nearest_player_to(&level, *q, |p| costly(p)).map_or(0, |p| p.id as usize);
        }
        let grid = t.elapsed();
        let t = Instant::now();
        for q in &queries {
            let area = Aabb::new(q.x - 10.0, q.y - 10.0, q.z - 10.0, q.x + 10.0, q.y + 10.0, q.z + 10.0);
            sink += views.iter().filter(|p| player_box(p).intersects(&area)).count();
        }
        let scan_area = t.elapsed();
        let t = Instant::now();
        for q in &queries {
            let area = Aabb::new(q.x - 10.0, q.y - 10.0, q.z - 10.0, q.x + 10.0, q.y + 10.0, q.z + 10.0);
            sink += level.players_in(&area).len();
        }
        let grid_area = t.elapsed();
        eprintln!(
            "{n:5} players over {spread}: nearest scan {:.2} us grid {:.2} us; players_in scan {:.2} us grid {:.2} us ({sink})",
            scan.as_secs_f64() * 1e6 / 2000.0,
            grid.as_secs_f64() * 1e6 / 2000.0,
            scan_area.as_secs_f64() * 1e6 / 2000.0,
            grid_area.as_secs_f64() * 1e6 / 2000.0
        );
    }
}

#[test]
fn nearest_player_matches_a_scan() {
    let mut rng = Rng(11);
    for (n, spread) in [(0, 10.0), (1, 10.0), (6, 50.0), (200, 100.0), (500, 3000.0), (40, 20000.0)] {
        let level = GridLevel::new(crowd(&mut rng, n, spread));
        for round in 0..200 {
            let at = Vec3::new(rng.f(-spread, spread), rng.f(-30.0, 210.0), rng.f(-spread, spread));
            // Accept everyone, alive survivors, or a sparse subset (which pushes the search outward).
            let pick = round % 3;
            let modulo = 1 + rng.next() % 9;
            let accept = |p: &PlayerView| match pick {
                0 => true,
                1 => p.alive && !p.spectator,
                _ => p.id as u64 % modulo == 0,
            };
            let want = level.players().iter().filter(|p| accept(p)).fold(None::<PlayerView>, |best, p| match best {
                Some(b) if b.pos.distance_to_sqr(at) <= p.pos.distance_to_sqr(at) => Some(b),
                _ => Some(*p),
            });
            let mut calls = 0;
            let got = nearest_player_to(&level, at, |p| {
                calls += 1;
                accept(p)
            });
            assert_eq!(got, want, "nearest to {at:?} of {n} players (filter {pick})");
            assert!(calls <= n.max(1) * 2, "{calls} accept calls for {n} players");
        }
    }
}

#[test]
fn lightning_and_fireworks_see_the_players_a_scan_sees() {
    use kiln_item::component::{FireworkExplosion, FireworkShape, Fireworks};
    let mut rng = Rng(21);
    let (mut criteria, mut hurts) = (0, 0);
    for (n, spread) in [(0, 10.0), (3, 20.0), (40, 150.0), (300, 400.0), (600, 3000.0)] {
        for round in 0..6 {
            let views = crowd(&mut rng, n, spread);
            let at = Vec3::new(rng.f(-spread / 4.0, spread / 4.0), rng.f(0.0, 150.0), rng.f(-spread / 4.0, spread / 4.0));
            let mut grid = GridLevel::new(views.clone());
            let mut scan = GridLevel::scanning(views.clone());
            // A bolt reaches every player within 256 blocks when it ends.
            let seed = 5 + round;
            let bolt = kiln_entity::ext_entity::lightning::new(1, 0, at, true, seed);
            let (mut a, mut b) = (bolt.clone(), bolt);
            let (mut ea, mut eb) = (Vec::new(), Vec::new());
            for _ in 0..60 {
                a.common_tick();
                a.tick(&mut grid);
                ea.append(&mut grid.inner.events);
                b.common_tick();
                b.tick(&mut scan);
                eb.append(&mut scan.inner.events);
            }
            assert_eq!(ea, eb, "lightning over {n} players");
            criteria += ea.iter().filter(|e| matches!(e, Event::Criterion { .. })).count();
            // A rocket with a burst, shot at a player (or at nothing): the same hits and the same
            // damage to the same players.
            let mut stack = kiln_item::ItemStack::of("minecraft:firework_rocket", 1).expect("a rocket");
            stack.insert(
                kiln_item::keys::FIREWORKS,
                Fireworks { flight_duration: 1, explosions: vec![FireworkExplosion { shape: FireworkShape::SmallBall, colors: vec![0xff0000], fade_colors: vec![], has_trail: false, has_twinkle: false }] },
            );
            let target = views.iter().find(|v| v.alive && !v.spectator).map_or(at, |v| Vec3::new(v.pos.x, v.pos.y + 1.0, v.pos.z));
            let mut rocket = kiln_entity::ext_entity::firework::new(target + Vec3::new(-3.0, -2.0, 0.0), stack, None, None, false, seed);
            rocket.delta = Vec3::new(0.7, 0.3, 0.0);
            let (mut a, mut b) = (rocket.clone(), rocket);
            let (mut ea, mut eb) = (Vec::new(), Vec::new());
            for _ in 0..40 {
                a.common_tick();
                a.tick(&mut grid);
                ea.append(&mut grid.inner.events);
                b.common_tick();
                b.tick(&mut scan);
                eb.append(&mut scan.inner.events);
            }
            assert_eq!(ea, eb, "firework over {n} players");
            hurts += ea.iter().filter(|e| matches!(e, Event::Hurt { .. })).count();
            assert_eq!(a.position(), b.position());
        }
    }
    // (The comparison is not of two empty answers.)
    assert!(criteria > 0 && hurts > 0, "{criteria} criteria, {hurts} hurts");
}
