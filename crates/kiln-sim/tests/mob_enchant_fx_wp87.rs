//! wp87: the enchantment effects that run for every living entity in vanilla and not only for players: `tick` and `location_changed` of what a
//! mob wears (a drowned with Frost Walker boots, a mob with Soul Speed), `explode` wherever an effect may (a tick, a blow of a mob, a projectile
//! that hits a block), and the `periodic_tick` predicate that Soul Speed's particles and sounds ask. The enchantments are written here (a data pack
//! of the test), so no Mojang data is needed.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;

const BOOTS_SLOT: &str = r#""slots": ["feet"], "supported_items": "minecraft:golden_boots""#;
const SWORD_SLOT: &str = r#""slots": ["mainhand"], "supported_items": "minecraft:iron_sword""#;

fn enchantment(slots: &str, effects: &str) -> String {
    format!(
        r#"{{"description": {{"text": "x"}}, "weight": 1, "max_level": 1, "min_cost": {{"base": 1, "per_level_above_first": 0}}, "max_cost": {{"base": 50, "per_level_above_first": 0}}, "anvil_cost": 1, {slots}, "effects": {{{effects}}}}}"#
    )
}

fn periodic(ticks: i32) -> String {
    format!(r#"{{"type": "minecraft:entity_properties", "entity": "this", "predicate": {{"minecraft:periodic_tick": {ticks}}}}}"#)
}

fn explode(knockback: f32) -> String {
    format!(
        r#"{{"type": "minecraft:explode", "attribute_to_user": true, "block_interaction": "none", "knockback_multiplier": {knockback}, "radius": 3.0, "large_particle": {{"type": "minecraft:gust_emitter_large"}}, "small_particle": {{"type": "minecraft:gust_emitter_small"}}, "sound": "minecraft:entity.wind_charge.wind_burst"}}"#
    )
}

struct World {
    sim: Sim,
    client: Client,
    stats: std::sync::Arc<kiln_sim::testing::SinkStats>,
    dir: PathBuf,
    ground: [i32; 3],
}

impl World {
    fn new(name: &str, enchantments: &[(&str, String)]) -> World {
        let dir = std::env::temp_dir().join(format!("kiln-mob-fx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/p87");
        std::fs::create_dir_all(pack.join("data/kiln87/enchantment")).unwrap();
        for (id, json) in enchantments {
            std::fs::write(pack.join(format!("data/kiln87/enchantment/{id}.json")), json).unwrap();
        }
        std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"p87","min_format":121,"max_format":121}}"#).unwrap();
        let mut sim = Sim::new(SimConfig::new(4, 3, Some(dir.clone())));
        assert!(sim.step([]));
        sim.console_now("reload");
        sim.console_now("datapack enable \"file/p87\"");
        for _ in 0..3 {
            assert!(sim.step([]));
        }
        let (msg, stats) = join(1, "User", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        let keep = stats.clone();
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let client = Client::new(1, stats);
        let mut w = World { sim, client, stats: keep, dir, ground: [0; 3] };
        w.ticks(5);
        w.sim.console_now("gamemode creative User");
        let p = w.client.pos;
        w.ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        let g = w.ground;
        w.sim.console_now(&format!("fill {} {} {} {} {} {} minecraft:stone", g[0] - 8, g[1], g[2] - 8, g[0] + 12, g[1], g[2] + 8));
        w.ticks(2);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    /// `n` blocks east of the player, standing on the floor.
    fn spot(&self, n: f64) -> [f64; 3] {
        let p = self.client.pos;
        [p[0] + n, self.ground[1] as f64 + 1.0, p[2]]
    }

    fn summon(&mut self, kind: &str, at: [f64; 3], nbt: &str) -> i32 {
        self.sim.console_now(&format!("summon minecraft:{kind} {} {} {} {nbt}", at[0], at[1], at[2]));
        self.ticks(1);
        *self.sim.entity_ids_of(&format!("minecraft:{kind}")).last().expect("the mob")
    }

    /// The explosion packets the player was sent since the last call.
    fn explosions(&mut self) -> usize {
        let want = kiln_data::packets::play::clientbound::EXPLODE;
        let mut log = self.stats.log.lock().unwrap();
        let all = log.take().unwrap_or_default();
        *log = Some(Vec::new());
        all.iter().filter(|p| kiln_proto::codec::Reader::new(p).varint().ok() == Some(want)).count()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn worn(kind: &str, slot: &str, item: &str, enchantment: &str) -> String {
    let _ = kind;
    format!("equipment:{{{slot}:{{id:\"minecraft:{item}\",count:1,components:{{\"minecraft:enchantments\":{{\"kiln87:{enchantment}\":1}}}}}}}}")
}

#[test]
fn an_attribute_effect_of_worn_boots_holds_for_a_mob_while_they_are_worn() {
    let _x = kiln_data::live::exclusive();
    let stride = enchantment(
        BOOTS_SLOT,
        r#""minecraft:location_changed": [{"effect": {"type": "minecraft:attribute", "amount": 1.0, "attribute": "minecraft:movement_efficiency", "id": "kiln87:stride", "operation": "add_value"}}]"#,
    );
    let mut w = World::new("stride", &[("stride", stride)]);
    let at = w.spot(3.0);
    let nbt = format!("{{NoAI:1b,PersistenceRequired:1b,{}}}", worn("husk", "feet", "golden_boots", "stride"));
    let id = w.summon("husk", at, &nbt);
    w.ticks(4);
    let on = w.sim.mob_attribute(id, "minecraft:movement_efficiency").expect("the attribute");
    assert!((on - 1.0).abs() < 1e-9, "movement_efficiency with the boots on: {on}");
    // Taken off, the effect stops (`stopLocationBasedEffects`) and its modifier goes.
    w.sim.console_now("item replace entity @e[type=minecraft:husk] armor.feet with minecraft:air");
    w.ticks(3);
    let off = w.sim.mob_attribute(id, "minecraft:movement_efficiency").expect("the attribute");
    assert!(off.abs() < 1e-9, "movement_efficiency with the boots off: {off}");
}

#[test]
fn a_location_effect_replaces_the_block_a_mob_stands_on() {
    let _x = kiln_data::live::exclusive();
    let frost = enchantment(
        BOOTS_SLOT,
        r#""minecraft:location_changed": [{"effect": {"type": "minecraft:replace_block", "offset": [0, -1, 0], "block_state": {"Name": "minecraft:ice"}, "predicate": {"type": "minecraft:matching_blocks", "blocks": "minecraft:stone"}}, "requirements": {"type": "minecraft:entity_properties", "entity": "this", "predicate": {"minecraft:flags": {"is_on_ground": true}}}}]"#,
    );
    let mut w = World::new("frost", &[("frost", frost)]);
    let mut at = w.spot(3.0);
    // (A husk that is not at rest on the ground does not freeze anything: it falls 1.5 blocks, and the landing runs the effect.)
    at[1] += 1.5;
    let nbt = format!("{{PersistenceRequired:1b,attributes:[{{id:\"minecraft:movement_speed\",base:0.0d}}],{}}}", worn("husk", "feet", "golden_boots", "frost"));
    w.summon("husk", at, &nbt);
    let (x, z) = (at[0].floor() as i32, at[2].floor() as i32);
    assert_eq!(w.sim.block_at(x, w.ground[1], z), kiln_blocks::state::parse_state("minecraft:stone"), "stone before");
    w.ticks(20);
    let under = w.sim.block_at(x, w.ground[1], z).expect("loaded");
    assert_eq!(under, kiln_blocks::state::parse_state("minecraft:ice").unwrap(), "the stone under the husk turned to ice");
    // (A block the husk is not standing on is left alone.)
    let next = w.sim.block_at(x + 2, w.ground[1], z).expect("loaded");
    assert_eq!(next, kiln_blocks::state::parse_state("minecraft:stone").unwrap());
}

#[test]
fn a_tick_effect_of_a_mob_asks_periodic_tick_of_its_age() {
    let _x = kiln_data::live::exclusive();
    let pulse = enchantment(BOOTS_SLOT, &format!(r#""minecraft:tick": [{{"effect": {{"type": "minecraft:ignite", "duration": 3.0}}, "requirements": {}}}]"#, periodic(10)));
    let mut w = World::new("pulse", &[("pulse", pulse)]);
    let at = w.spot(3.0);
    let nbt = format!("{{NoAI:1b,PersistenceRequired:1b,{}}}", worn("husk", "feet", "golden_boots", "pulse"));
    let id = w.summon("husk", at, &nbt);
    // The fire starts over every 10 ticks of the husk's life (it burns for 3 seconds, so each ignition shows as a rise in its fire ticks).
    let mut rises = Vec::new();
    let mut last = 0;
    for tick in 0..45 {
        w.ticks(1);
        let fire = w.sim.mob_state(id).expect("the husk").fire_ticks;
        if fire > last {
            rises.push(tick);
        }
        last = fire;
    }
    assert!(rises.len() >= 3, "ignited every 10 ticks: {rises:?}");
    assert!(rises.windows(2).all(|p| p[1] - p[0] == 10), "ten ticks apart: {rises:?}");
}

#[test]
fn a_tick_explosion_pushes_what_is_near_and_tells_the_players() {
    let _x = kiln_data::live::exclusive();
    let burst = enchantment(BOOTS_SLOT, &format!(r#""minecraft:tick": [{{"effect": {}, "requirements": {}}}]"#, explode(3.0), periodic(20)));
    let mut w = World::new("burst", &[("burst", burst)]);
    let at = w.spot(4.0);
    let nbt = format!("{{NoAI:1b,PersistenceRequired:1b,{}}}", worn("husk", "feet", "golden_boots", "burst"));
    w.summon("husk", at, &nbt);
    let near = w.summon("husk", [at[0] + 1.5, at[1], at[2]], "{NoAI:1b,PersistenceRequired:1b}");
    let _ = w.explosions();
    let mut pushed = false;
    let mut told = 0;
    for _ in 0..45 {
        w.ticks(1);
        told += w.explosions();
        let d = w.sim.mob_state(near).expect("the other husk").delta;
        pushed |= d[0] * d[0] + d[2] * d[2] > 1e-4;
    }
    assert!(told >= 1, "the explosion packet reached the player ({told})");
    assert!(pushed, "the husk next to the one that burst was pushed");
}

#[test]
fn a_blow_of_a_mob_with_an_exploding_weapon_bursts_at_the_mob() {
    let _x = kiln_data::live::exclusive();
    let thump = enchantment(
        SWORD_SLOT,
        &format!(
            r#""minecraft:post_attack": [{{"enchanted": "attacker", "affected": "attacker", "effect": {}, "requirements": {{"type": "minecraft:entity_properties", "entity": "direct_attacker", "predicate": {{"minecraft:entity_type": "minecraft:husk"}}}}}}]"#,
            explode(1.0)
        ),
    );
    let mut w = World::new("thump", &[("thump", thump)]);
    w.sim.console_now("gamemode survival User");
    let at = w.spot(1.2);
    let nbt = format!("{{PersistenceRequired:1b,{}}}", worn("husk", "mainhand", "iron_sword", "thump"));
    w.summon("husk", at, &nbt);
    let _ = w.explosions();
    let mut told = 0;
    for _ in 0..200 {
        w.ticks(1);
        told += w.explosions();
        if told > 0 {
            break;
        }
    }
    assert!(told >= 1, "the husk hit the player with its sword and the burst was told to the player");
}

#[test]
fn an_arrow_from_an_enchanted_bow_runs_its_block_effects() {
    let _x = kiln_data::live::exclusive();
    let marker = enchantment(
        r#""slots": ["mainhand"], "supported_items": "minecraft:bow""#,
        r#""minecraft:hit_block": [{"effect": {"type": "minecraft:replace_block", "block_state": {"Name": "minecraft:gold_block"}}}]"#,
    );
    let mut w = World::new("marker", &[("marker", marker)]);
    let at = w.spot(3.0);
    w.sim.console_now(&format!(
        "summon minecraft:arrow {} {} {} {{Motion:[0.0,-1.0,0.0],weapon:{{id:\"minecraft:bow\",count:1,components:{{\"minecraft:enchantments\":{{\"kiln87:marker\":1}}}}}}}}",
        at[0],
        at[1] + 2.0,
        at[2]
    ));
    w.ticks(10);
    let hit = w.sim.block_at(at[0].floor() as i32, w.ground[1], at[2].floor() as i32).expect("loaded");
    assert_eq!(hit, kiln_blocks::state::parse_state("minecraft:gold_block").unwrap(), "the block the arrow hit turned to gold");
}


#[test]
fn a_players_tick_effect_asks_periodic_tick_too() {
    let _x = kiln_data::live::exclusive();
    let wear = enchantment(BOOTS_SLOT, &format!(r#""minecraft:tick": [{{"effect": {{"type": "minecraft:change_item_damage", "amount": 1.0}}, "requirements": {}}}]"#, periodic(10)));
    let mut w = World::new("player_wear", &[("wear", wear)]);
    w.sim.console_now("gamemode survival User");
    w.sim.console_now("item replace entity User armor.feet with minecraft:golden_boots[enchantments={\"kiln87:wear\":1}]");
    w.ticks(1);
    let before = w.sim.item_damage(1, 8).expect("the boots");
    w.ticks(100);
    let worn = w.sim.item_damage(1, 8).expect("the boots") - before;
    // Every 10th tick of the player's age: ten times in a hundred ticks (never for a predicate nobody answers).
    assert!((9..=11).contains(&worn), "the boots lost {worn} durability in 100 ticks");
}

#[test]
fn a_players_tick_explosion_pushes_a_mob_next_to_it() {
    let _x = kiln_data::live::exclusive();
    let burst = enchantment(BOOTS_SLOT, &format!(r#""minecraft:tick": [{{"effect": {}, "requirements": {}}}]"#, explode(3.0), periodic(20)));
    let mut w = World::new("player_burst", &[("burst", burst)]);
    w.sim.console_now("item replace entity User armor.feet with minecraft:golden_boots[enchantments={\"kiln87:burst\":1}]");
    let near = w.summon("husk", w.spot(1.5), "{NoAI:1b,PersistenceRequired:1b}");
    let _ = w.explosions();
    let mut pushed = false;
    let mut told = 0;
    for _ in 0..45 {
        w.ticks(1);
        told += w.explosions();
        let d = w.sim.mob_state(near).expect("the husk").delta;
        pushed |= d[0] * d[0] + d[2] * d[2] > 1e-4;
    }
    assert!(told >= 1, "the explosion packet reached the player ({told})");
    assert!(pushed, "the husk next to the player was pushed");
}

#[test]
fn boots_with_a_damage_immunity_protect_a_mob_on_magma() {
    let _x = kiln_data::live::exclusive();
    let cool = enchantment(
        BOOTS_SLOT,
        r#""minecraft:damage_immunity": [{"effect": {}, "requirements": {"type": "minecraft:damage_source_properties", "predicate": {"tags": [{"id": "minecraft:burn_from_stepping", "expected": true}]}}}]"#,
    );
    let mut w = World::new("cool", &[("cool", cool)]);
    let g = w.ground;
    w.sim.console_now(&format!("fill {} {} {} {} {} {} minecraft:magma_block", g[0] + 2, g[1], g[2] - 3, g[0] + 8, g[1], g[2] + 3));
    w.ticks(2);
    let still = "attributes:[{id:\"minecraft:movement_speed\",base:0.0d}]";
    let plain = w.summon("husk", w.spot(4.0), &format!("{{PersistenceRequired:1b,{still}}}"));
    let booted = w.summon("husk", w.spot(6.0), &format!("{{PersistenceRequired:1b,{still},{}}}", worn("husk", "feet", "golden_boots", "cool")));
    w.ticks(60);
    let health = |w: &World, id| w.sim.mob_state(id).expect("the husk").health;
    assert!(health(&w, plain) < 20.0, "the husk without boots is burned by the magma: {}", health(&w, plain));
    assert_eq!(health(&w, booted), 20.0, "the husk in boots with the immunity is not");
}
