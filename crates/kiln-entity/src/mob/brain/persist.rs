//! The `Brain` tag: the memories with a codec (`Brain.Packed`), each `{value, ttl?}`.

use super::memory::{GlobalPos, Mem, Val};
use super::Brain;
use crate::math::BlockPos;
use crate::persist::{uuid_from_tag, uuid_to_tag};
use kiln_proto::nbt::Tag;

/// The default dimension of a position memory the level cannot name.
pub const OVERWORLD: &str = "minecraft:overworld";

fn global_pos(g: &GlobalPos) -> Tag {
    Tag::Compound(vec![("dimension".into(), Tag::String(g.dim.to_string())), ("pos".into(), Tag::IntArray(vec![g.pos.x, g.pos.y, g.pos.z]))])
}

fn block_pos(p: BlockPos) -> Tag {
    Tag::IntArray(vec![p.x, p.y, p.z])
}

fn read_block_pos(t: &Tag) -> Option<BlockPos> {
    match t {
        Tag::IntArray(p) if p.len() == 3 => Some(BlockPos::new(p[0], p[1], p[2])),
        _ => None,
    }
}

fn read_global_pos(t: &Tag) -> Option<GlobalPos> {
    let dim = t.get("dimension").and_then(Tag::as_str).unwrap_or(OVERWORLD);
    Some(GlobalPos::new(dim, read_block_pos(t.get("pos")?)?))
}

/// `Codec` of the value for a serializable memory.
fn encode(v: &Val) -> Option<Tag> {
    Some(match v {
        Val::Unit => Tag::Compound(vec![]),
        Val::Bool(b) => Tag::Byte(*b as i8),
        Val::Int(i) => Tag::Int(*i),
        Val::Long(l) => Tag::Long(*l),
        Val::Pos(g) => global_pos(g),
        Val::Positions(l) => Tag::List(l.iter().map(global_pos).collect()),
        Val::Uuid(u) => uuid_to_tag(*u),
        Val::Block(p) => block_pos(*p),
        _ => return None,
    })
}

fn decode(m: Mem, t: &Tag) -> Option<Val> {
    use Mem::*;
    Some(match m {
        Home | JobSite | PotentialJobSite | MeetingPoint | LikedNoteblockPosition => Val::Pos(read_global_pos(t)?),
        GolemDetectedRecently | DangerDetectedRecently | HasHuntingCooldown | IsPanicking | UniversalAnger | AdmiringItem | AdmiringDisabled | HuntedRecently => {
            Val::Bool(t.as_i64()? != 0)
        }
        LastSlept | LastWoken | LastWorkedAtPoi => Val::Long(t.as_i64()?),
        PlayDeadTicks | TemptationCooldownTicks | GazeCooldownTicks | LongJumpCooldownTicks | RamCooldownTicks | ChargeCooldownTicks | AttackTargetCooldown
        | LikedNoteblockCooldownTicks | ItemPickupCooldownTicks => Val::Int(t.as_i64()? as i32),
        AngryAt | LikedPlayer => Val::Uuid(uuid_from_tag(t)?),
        VisitedBlockPositions | UnreachableTransportBlockPositions | SnifferExploredPositions => {
            Val::Positions(t.as_list()?.iter().filter_map(read_global_pos).collect())
        }
        BreezeJumpTarget => Val::Block(read_block_pos(t)?),
        m if m.serializable() => Val::Unit,
        _ => return None,
    })
}

/// `Brain.pack`: `{memories: {<type>: {value, ttl?}}}`.
pub fn save(b: &Brain) -> Tag {
    save_state(&b.st)
}

/// [`save`] for a brain's state alone.
pub fn save_state(st: &super::BrainState) -> Tag {
    let mut mems = Vec::new();
    for (m, v, ttl) in st.mem.iter() {
        if !m.serializable() {
            continue;
        }
        let Some(value) = encode(v) else { continue };
        let mut c = vec![("value".to_owned(), value)];
        if ttl != super::memory::NEVER_EXPIRE {
            c.push(("ttl".to_owned(), Tag::Long(ttl)));
        }
        mems.push((m.name().to_owned(), Tag::Compound(c)));
    }
    Tag::Compound(vec![("memories".into(), Tag::Compound(mems))])
}

/// Reads `Brain.Packed` into the brain's memories (unregistered ones are dropped, as vanilla's
/// `setMemoryInternal` does).
pub fn load(b: &mut Brain, t: &Tag) {
    load_state(&mut b.st, t);
}

/// [`load`] for a brain's state alone.
pub fn load_state(st: &mut super::BrainState, t: &Tag) {
    let Some(Tag::Compound(mems)) = t.get("memories") else { return };
    for (name, entry) in mems {
        let Some(m) = Mem::by_name(name) else { continue };
        let Some(value) = entry.get("value").and_then(|v| decode(m, v)) else { continue };
        match entry.get("ttl").and_then(Tag::as_i64) {
            Some(ttl) => st.mem.set_expiring(m, value, ttl),
            None => st.mem.set(m, value),
        }
    }
}
