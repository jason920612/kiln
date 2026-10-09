//! Land claims, fail-closed. Placing the configured claim block (default a gold block) claims
//! a square of `radius` blocks around it for the placer (at most `max_claims` each); nobody
//! else can break, place, use blocks, hit animals or hurt players inside it. Operators are not
//! subject to it (the manifest's `bypass-permission`), and every subscription is
//! `fail-closed`: if this plugin traps or runs out of time, the action is denied.
//!
//! The data pattern this sample shows:
//! - A claim lives in the **cell** namespace (8x8 chunks), so the decision for a block event is
//!   made from data the event's own region owns: synchronous and exact.
//! - A claim that reaches into a neighbouring cell cannot write there from this region. It
//!   leaves a note in the plugin's global namespace (`claim:<x>:<z>` holds the owner; written
//!   with a typed atomic `compare-and-set`) and schedules a position task for each
//!   neighbouring cell (`scheduler.at-position`); the task runs in the region owning that cell
//!   a tick later, reads the note from the global snapshot and records its copy of the claim.

use kiln_plugin_sdk::registry::{self, Kind};
use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{
    BlockEvent, DamageEvent, EntityEvent, GlobalValue, InitInfo, PlaceEvent, Plugin, TaskEvent, Text, Verdict, chat, config, export_plugin, level_id,
    scheduler, uuid_u128,
};
use std::sync::Mutex;

struct Settings {
    radius: i32,
    max_claims: i64,
    claim_item: Option<u32>,
}

static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);

/// One claim: owner and the inclusive square, 32 bytes.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Claim {
    owner: u128,
    x1: i32,
    z1: i32,
    x2: i32,
    z2: i32,
}

impl Claim {
    fn contains(&self, x: i32, z: i32) -> bool {
        (self.x1..=self.x2).contains(&x) && (self.z1..=self.z2).contains(&z)
    }

    fn encode(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[..16].copy_from_slice(&self.owner.to_le_bytes());
        for (i, v) in [self.x1, self.z1, self.x2, self.z2].iter().enumerate() {
            b[16 + i * 4..20 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    fn decode(b: &[u8]) -> Option<Claim> {
        if b.len() != 32 {
            return None;
        }
        let i = |n: usize| i32::from_le_bytes(b[16 + n * 4..20 + n * 4].try_into().unwrap());
        Some(Claim { owner: u128::from_le_bytes(b[..16].try_into().unwrap()), x1: i(0), z1: i(1), x2: i(2), z2: i(3) })
    }
}

fn claims_in(cell: u64) -> Vec<Claim> {
    state::get(Scope::Cell(cell), "claims").map(|b| b.chunks_exact(32).filter_map(Claim::decode).collect()).unwrap_or_default()
}

fn add_claim(cell: u64, c: Claim) {
    let mut bytes = state::get(Scope::Cell(cell), "claims").unwrap_or_default();
    if !bytes.chunks_exact(32).any(|x| x == c.encode()) {
        bytes.extend_from_slice(&c.encode());
        state::put(Scope::Cell(cell), "claims", Some(&bytes));
    }
}

/// The claim over a block column, if any.
fn claim_at(cell: u64, x: i32, z: i32) -> Option<Claim> {
    claims_in(cell).into_iter().find(|c| c.contains(x, z))
}

fn note_key(x: i32, z: i32) -> String {
    format!("claim:{x}:{z}")
}

fn pack(x: i32, z: i32) -> u64 {
    ((x as u32 as u64) << 32) | z as u32 as u64
}

fn unpack(v: u64) -> (i32, i32) {
    ((v >> 32) as u32 as i32, v as u32 as i32)
}

fn refusal(c: &Claim) -> Verdict {
    Verdict::deny(Text::new().color("red", "This land is claimed by ").color("gold", name_of_owner(c.owner)))
}

fn name_of_owner(owner: u128) -> String {
    kiln_plugin_sdk::event::online()
        .into_iter()
        .find(|p| uuid_u128(&p.uuid) == owner)
        .map(|p| p.name)
        .unwrap_or_else(|| "another player".to_owned())
}

/// Owners may do anything on their land; everyone else nothing.
fn guard(player: u128, cell: u64, x: i32, z: i32) -> Verdict {
    match claim_at(cell, x, z) {
        Some(c) if c.owner != player => refusal(&c),
        _ => Verdict::Allow,
    }
}

struct Claims;

impl Plugin for Claims {
    fn init_region(info: InitInfo) {
        let int = |k: &str, d: i64| config(&info, k).and_then(|v| v.parse().ok()).unwrap_or(d);
        let key = config(&info, "claim_block").unwrap_or("minecraft:gold_block");
        *SETTINGS.lock().unwrap() =
            Some(Settings { radius: int("radius", 8) as i32, max_claims: int("max_claims", 3), claim_item: registry::id(Kind::Item, key) });
        let _ = level_id(&info, "minecraft:overworld");
    }

    fn on_block_break(ev: BlockEvent) -> Verdict {
        guard(uuid_u128(&ev.player.uuid), ev.cell, ev.pos.x, ev.pos.z)
    }

    fn on_block_place(ev: PlaceEvent) -> Verdict {
        let me = uuid_u128(&ev.player.uuid);
        // Right-clicking a block in somebody's claim (a chest, a door) is refused like placing.
        let verdict = guard(me, ev.cell, ev.pos.x, ev.pos.z);
        if verdict.is_deny() || claim_at(ev.cell, ev.pos.x, ev.pos.z).is_some() {
            return verdict;
        }
        let settings = SETTINGS.lock().unwrap();
        let Some(s) = settings.as_ref() else { return Verdict::deny_silently() };
        if ev.item.is_none() || ev.item != s.claim_item {
            return Verdict::Allow;
        }
        // The claim block: a claim of this player's, unless they have used up their claims.
        let mine = Scope::Player(ev.player.handle);
        let n = state::get_i64(mine, "claims");
        if n >= s.max_claims {
            return Verdict::deny(Text::new().color("red", format!("You can have {} claims.", s.max_claims)));
        }
        state::put_i64(mine, "claims", n + 1);
        let claim = Claim { owner: me, x1: ev.pos.x - s.radius, z1: ev.pos.z - s.radius, x2: ev.pos.x + s.radius, z2: ev.pos.z + s.radius };
        add_claim(ev.cell, claim);
        // The same claim in each neighbouring cell it reaches, through that cell's own region.
        let note = state::compare_and_set(&note_key(ev.pos.x, ev.pos.z), None, GlobalValue::Bytes(me.to_le_bytes().to_vec()));
        let _ = note;
        let (own_x, own_z) = (ev.pos.x >> 7, ev.pos.z >> 7);
        for cx in (claim.x1 >> 7)..=(claim.x2 >> 7) {
            for cz in (claim.z1 >> 7)..=(claim.z2 >> 7) {
                if (cx, cz) != (own_x, own_z) {
                    scheduler::at_position(ev.level, cx * 128 + 64, cz * 128 + 64, 1, pack(ev.pos.x, ev.pos.z));
                }
            }
        }
        chat::send(&ev.player, Text::new().color("green", format!("Claimed {0}x{0} blocks.", s.radius * 2 + 1)));
        Verdict::Allow
    }

    fn on_task(t: TaskEvent) {
        // A neighbour's claim reaching into this cell: the note holds the owner.
        let Some(cell) = t.cell else { return };
        let (x, z) = unpack(t.id);
        let Some(GlobalValue::Bytes(owner)) = state::global_get(&note_key(x, z)) else { return };
        let Ok(owner) = <[u8; 16]>::try_from(owner.as_slice()) else { return };
        let r = SETTINGS.lock().unwrap().as_ref().map_or(8, |s| s.radius);
        add_claim(cell, Claim { owner: u128::from_le_bytes(owner), x1: x - r, z1: z - r, x2: x + r, z2: z + r });
    }

    fn on_entity_interact(ev: EntityEvent) -> Verdict {
        entity_guard(&ev)
    }

    fn on_entity_attack(ev: EntityEvent) -> Verdict {
        entity_guard(&ev)
    }

    /// Players do not hurt each other on claimed land: a claim is a safe place.
    fn on_player_damage(ev: DamageEvent) -> Verdict {
        if ev.attacker.is_some() && claim_at(ev.cell, ev.pos.x, ev.pos.z).is_some() {
            return Verdict::deny(Text::new().color("red", "No fighting on claimed land."));
        }
        Verdict::Allow
    }
}

/// Animals and villagers standing on somebody's land are theirs.
fn entity_guard(ev: &EntityEvent) -> Verdict {
    guard(uuid_u128(&ev.player.uuid), ev.cell, ev.pos.0.floor() as i32, ev.pos.2.floor() as i32)
}

export_plugin!(Claims);
