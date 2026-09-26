//! Movement encodings: packed angles, 1/4096-block position deltas (`VecDeltaCodec`),
//! `LpVec3` velocities, and a per-entity tracker that picks movement packets the way
//! `ServerEntity.sendChanges` does.

use super::{Angle, PosDelta, PositionPath};
use crate::{DecodeError, Reader, WriteExt};
use bytes::{BufMut, Bytes, BytesMut};

/// Java `Math.round(double)`: the nearest integer, ties toward positive infinity;
/// saturates at the `i64` range and maps NaN to 0 like the Java cast.
pub fn java_round(v: f64) -> i64 {
    // `f64::round` breaks ties away from zero; `r - v` is exact (Sterbenz), so a negative
    // tie shows up as exactly -0.5.
    let r = v.round();
    (if r - v == -0.5 { r + 1.0 } else { r }) as i64
}

/// Fixed-point coordinate used by movement deltas: `round(v * 4096)`.
pub fn encode_coord(v: f64) -> i64 {
    java_round(v * 4096.0)
}

/// Movement packets encode positions relative to the last position sent to viewers (the base),
/// in 1/4096 blocks; both sides move the base when a packet carries a position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionCodec {
    base: [f64; 3],
}

impl PositionCodec {
    pub fn new(base: [f64; 3]) -> Self {
        Self { base }
    }

    pub fn base(&self) -> [f64; 3] {
        self.base
    }

    pub fn set_base(&mut self, base: [f64; 3]) {
        self.base = base;
    }

    fn raw_delta(&self, pos: [f64; 3]) -> Option<[i16; 3]> {
        let mut out = [0; 3];
        for i in 0..3 {
            out[i] = i16::try_from(encode_coord(pos[i]) - encode_coord(self.base[i])).ok()?;
        }
        Some(out)
    }

    /// The delta from the base to `pos`, or `None` when an axis moved 8 blocks or more and the
    /// position must be sent in full (`entity_position_sync`).
    pub fn delta(&self, pos: [f64; 3]) -> Option<PosDelta> {
        self.raw_delta(pos).map(PosDelta::Linear)
    }

    /// A stepped path: each step relative to the previous one, the first to the base.
    /// `None` if any step is out of range. The base is left unchanged.
    pub fn stepped(&self, steps: &[([f64; 3], i32)]) -> Option<PosDelta> {
        let mut codec = *self;
        let mut out = Vec::with_capacity(steps.len());
        for &(pos, ticks) in steps {
            out.push((codec.raw_delta(pos)?, ticks));
            codec.base = pos;
        }
        Some(PosDelta::Stepped(out))
    }

    /// The position a client reconstructs from `delta`: untouched axes keep the exact base.
    pub fn decode(&self, delta: [i16; 3]) -> [f64; 3] {
        let mut out = self.base;
        for i in 0..3 {
            if delta[i] != 0 {
                out[i] = (encode_coord(self.base[i]) + delta[i] as i64) as f64 / 4096.0;
            }
        }
        out
    }
}

const LP_ABS_MIN: f64 = 3.051944088384301E-5;
const LP_ABS_MAX: f64 = 1.7179869183E10;

fn lp_pack(v: f64) -> u64 {
    java_round((v * 0.5 + 0.5) * 32766.0) as u64
}

fn lp_unpack(v: u64) -> f64 {
    ((v & 0x7fff) as f64).min(32766.0) * 2.0 / 32766.0 - 1.0
}

/// `LpVec3`: a velocity as three 15-bit fractions of a shared integer scale, packed in 6 bytes
/// (one zero byte for near-zero vectors, plus a VarInt when the scale exceeds 3).
pub fn put_lp_vec3(b: &mut BytesMut, v: [f64; 3]) {
    let v = v.map(|c| if c.is_nan() { 0.0 } else { c.clamp(-LP_ABS_MAX, LP_ABS_MAX) });
    let max = v[0].abs().max(v[1].abs()).max(v[2].abs());
    if max < LP_ABS_MIN {
        b.put_u8(0);
        return;
    }
    let scale = max.ceil() as u64;
    let extended = scale & 3 != scale;
    let markers = if extended { (scale & 3) | 4 } else { scale };
    let s = scale as f64;
    let packed = markers | lp_pack(v[0] / s) << 3 | lp_pack(v[1] / s) << 18 | lp_pack(v[2] / s) << 33;
    b.put_u8(packed as u8);
    b.put_u8((packed >> 8) as u8);
    b.put_u32((packed >> 16) as u32);
    if extended {
        b.put_varint((scale >> 2) as i32);
    }
}

/// Reads an `LpVec3` (the client's side of `put_lp_vec3`).
pub fn read_lp_vec3(r: &mut Reader) -> Result<[f64; 3], DecodeError> {
    let lo = r.u8()? as u64;
    if lo == 0 {
        return Ok([0.0; 3]);
    }
    let mid = r.u8()? as u64;
    let packed = (r.i32()? as u32 as u64) << 16 | mid << 8 | lo;
    let mut scale = lo & 3;
    if lo & 4 != 0 {
        scale |= (r.varint()? as u32 as u64) << 2;
    }
    let s = scale as f64;
    Ok([lp_unpack(packed >> 3) * s, lp_unpack(packed >> 18) * s, lp_unpack(packed >> 33) * s])
}

/// Last movement state sent for one entity, shared by all its viewers (`ServerEntity`).
///
/// Call [`tick`](Self::tick) once per server tick with the entity's current state and send
/// the returned packets to every player tracking the entity. New viewers must be spawned
/// with [`spawn`](Self::spawn) so their delta base matches everyone else's.
#[derive(Debug, Clone)]
pub struct MovementTracker {
    entity_id: i32,
    update_interval: i32,
    codec: PositionCodec,
    yaw: Angle,
    pitch: Angle,
    head_yaw: Angle,
    on_ground: bool,
    tick_count: i32,
    teleport_delay: i32,
    dirty: bool,
}

/// Entity state sampled for [`MovementTracker::tick`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveState {
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub on_ground: bool,
}

/// Position changes below this squared distance are not sent (except every 60 ticks).
const MIN_MOVE_SQR: f64 = 7.62939453125E-6;
const FORCED_POS_UPDATE_PERIOD: i32 = 60;
const FORCED_TELEPORT_PERIOD: i32 = 400;

impl MovementTracker {
    /// `update_interval` is the entity type's (`EntityType::update_interval`, 2 for players).
    pub fn new(entity_id: i32, update_interval: i32, state: &MoveState) -> Self {
        Self {
            entity_id,
            update_interval: update_interval.max(1),
            codec: PositionCodec::new(state.pos),
            yaw: Angle::from_degrees(state.yaw),
            pitch: Angle::from_degrees(state.pitch),
            head_yaw: Angle::from_degrees(state.head_yaw),
            on_ground: state.on_ground,
            tick_count: 0,
            teleport_delay: 0,
            dirty: false,
        }
    }

    pub fn entity_id(&self) -> i32 {
        self.entity_id
    }

    /// Makes the next `tick` an update tick regardless of the interval (`Entity.needsSync`),
    /// e.g. after a knockback or a teleport.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Position the viewers currently have (the delta base).
    pub fn sent_position(&self) -> [f64; 3] {
        self.codec.base()
    }

    /// `add_entity` for a new viewer, at the last sent position and angles.
    pub fn spawn(&self, uuid: uuid::Uuid, kind: i32, velocity: [f64; 3], data: i32) -> Bytes {
        super::add_entity(&super::AddEntity {
            entity_id: self.entity_id,
            uuid,
            kind,
            pos: self.codec.base(),
            velocity,
            pitch: self.pitch,
            yaw: self.yaw,
            head_yaw: self.head_yaw,
            data,
        })
    }

    /// Movement packets for this tick (possibly none), in send order.
    pub fn tick(&mut self, s: &MoveState) -> Vec<Bytes> {
        let mut out = Vec::new();
        if std::mem::take(&mut self.dirty) || self.tick_count % self.update_interval == 0 {
            let (yaw, pitch) = (Angle::from_degrees(s.yaw), Angle::from_degrees(s.pitch));
            let rotated = yaw != self.yaw || pitch != self.pitch;
            self.teleport_delay += 1;
            let base = self.codec.base();
            let moved = (0..3).map(|i| (s.pos[i] - base[i]).powi(2)).sum::<f64>() >= MIN_MOVE_SQR;
            let send_pos = moved || self.tick_count % FORCED_POS_UPDATE_PERIOD == 0;
            let id = self.entity_id;

            // (packet, carries position, carries rotation)
            let packet = if self.teleport_delay > FORCED_TELEPORT_PERIOD || self.on_ground != s.on_ground {
                self.on_ground = s.on_ground;
                self.teleport_delay = 0;
                Some((self.sync(s), true, true))
            } else if send_pos {
                Some(match self.codec.delta(s.pos) {
                    None => (self.sync(s), true, true),
                    Some(d) if rotated => (super::move_entity_pos_rot(id, &d, yaw, pitch, s.on_ground), true, true),
                    Some(d) => (super::move_entity_pos(id, &d, s.on_ground), true, false),
                })
            } else if rotated {
                Some((super::move_entity_rot(id, yaw, pitch, s.on_ground), false, true))
            } else {
                None
            };
            if let Some((p, has_pos, has_rot)) = packet {
                out.push(p);
                if has_pos {
                    self.codec.set_base(s.pos);
                }
                if has_rot {
                    (self.yaw, self.pitch) = (yaw, pitch);
                }
            }

            let head_yaw = Angle::from_degrees(s.head_yaw);
            if head_yaw != self.head_yaw {
                out.push(super::rotate_head(id, head_yaw));
                self.head_yaw = head_yaw;
            }
        }
        self.tick_count = self.tick_count.wrapping_add(1);
        out
    }

    fn sync(&self, s: &MoveState) -> Bytes {
        let path = PositionPath::Linear(s.pos);
        super::entity_position_sync(self.entity_id, &path, s.yaw, s.pitch, s.on_ground)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_round_ties_toward_positive_infinity() {
        let cases = [
            (0.5, 1),
            (-0.5, 0),
            (1.5, 2),
            (-1.5, -1),
            (2.4999, 2),
            (-2.5000001, -3),
            (0.49999999999999994, 0),
            (-0.49999999999999994, 0),
            (f64::NAN, 0),
            (1e300, i64::MAX),
            (-1e300, i64::MIN),
        ];
        for (v, want) in cases {
            assert_eq!(java_round(v), want, "round({v})");
        }
    }

    #[test]
    fn delta_is_fixed_point_difference() {
        let codec = PositionCodec::new([10.0, 64.0, -3.5]);
        let d = codec.delta([10.5, 64.0, -3.25]).unwrap();
        assert_eq!(d, PosDelta::Linear([2048, 0, 1024]));
        assert_eq!(codec.decode([2048, 0, 1024]), [10.5, 64.0, -3.25]);
    }

    #[test]
    fn delta_rounds_each_endpoint_not_the_difference() {
        // encode(new) - encode(base), as VecDeltaCodec does: 0.00012 * 4096 = 0.49 -> 0, but
        // the base 0.0001 (0.41 -> 0) and 0.00022 (0.90 -> 1) give 1.
        let codec = PositionCodec::new([0.0001, 0.0, 0.0]);
        assert_eq!(codec.delta([0.00022, 0.0, 0.0]), Some(PosDelta::Linear([1, 0, 0])));
    }

    #[test]
    fn delta_range_limits() {
        let codec = PositionCodec::new([0.0; 3]);
        // i16 covers [-32768, 32767] / 4096 blocks: just under +8 and exactly -8.
        assert_eq!(codec.delta([32767.0 / 4096.0, -8.0, 0.0]), Some(PosDelta::Linear([32767, -32768, 0])));
        assert_eq!(codec.delta([8.0, 0.0, 0.0]), None);
        assert_eq!(codec.delta([0.0, 0.0, -8.0 - 1.0 / 4096.0]), None);
    }

    #[test]
    fn decode_keeps_exact_base_on_unmoved_axes() {
        let codec = PositionCodec::new([0.1, 70.3, 0.7]);
        let out = codec.decode([4096, 0, 0]);
        assert_eq!(out[1], 70.3);
        assert_eq!(out[2], 0.7);
        assert_eq!(out[0], (encode_coord(0.1) + 4096) as f64 / 4096.0);
    }

    #[test]
    fn stepped_chains_from_the_base() {
        let codec = PositionCodec::new([0.0; 3]);
        let d = codec.stepped(&[([1.0, 0.0, 0.0], 1), ([1.0, 2.0, 0.0], 2)]).unwrap();
        assert_eq!(d, PosDelta::Stepped(vec![([4096, 0, 0], 1), ([0, 8192, 0], 2)]));
        assert!(codec.stepped(&[([1.0, 0.0, 0.0], 1), ([10.0, 0.0, 0.0], 1)]).is_none());
        assert_eq!(codec.base(), [0.0; 3]);
    }

    fn lp_roundtrip(v: [f64; 3]) -> ([f64; 3], usize) {
        let mut b = BytesMut::new();
        put_lp_vec3(&mut b, v);
        let mut r = Reader::new(&b);
        let out = read_lp_vec3(&mut r).unwrap();
        r.finish().unwrap();
        (out, b.len())
    }

    #[test]
    fn lp_vec3_zero_is_one_byte() {
        assert_eq!(lp_roundtrip([0.0; 3]), ([0.0; 3], 1));
        assert_eq!(lp_roundtrip([1e-5, -2e-5, 0.0]), ([0.0; 3], 1));
        assert_eq!(lp_roundtrip([f64::NAN, 0.0, 0.0]), ([0.0; 3], 1));
    }

    #[test]
    fn lp_vec3_precision_and_scale() {
        for v in [[0.1, -0.0784, 0.25], [1.0, 0.0, -1.0], [3.9, -2.0, 0.001], [-4.2, 0.5, 100.0], [70000.0, 0.0, -1.0]]
        {
            let (out, len) = lp_roundtrip(v);
            let max = v.iter().fold(0f64, |m, c| m.max(c.abs()));
            assert_eq!(len, if max.ceil() > 3.0 { 6 + crate::codec::varint_len((max.ceil() as i32) >> 2) } else { 6 });
            for i in 0..3 {
                assert!((out[i] - v[i]).abs() <= max.ceil() / 32766.0, "{v:?} -> {out:?}");
            }
        }
    }

    #[test]
    fn lp_vec3_known_bytes() {
        // (1, 0, -1): scale 1, x -> 32766, y -> 16383, z -> 0.
        let mut b = BytesMut::new();
        put_lp_vec3(&mut b, [1.0, 0.0, -1.0]);
        let packed: u64 = 1 | 32766 << 3 | 16383 << 18;
        let mut want = vec![packed as u8, (packed >> 8) as u8];
        want.extend_from_slice(&((packed >> 16) as u32).to_be_bytes());
        assert_eq!(&b[..], &want[..]);
    }

    fn state(pos: [f64; 3], yaw: f32, on_ground: bool) -> MoveState {
        MoveState { pos, yaw, pitch: 0.0, head_yaw: yaw, on_ground }
    }

    fn ids(packets: &[Bytes]) -> Vec<i32> {
        packets.iter().map(|p| Reader::new(p).varint().unwrap()).collect()
    }

    use kiln_data::packets::play::clientbound as cb;

    #[test]
    fn tracker_sends_on_update_interval_only() {
        let mut t = MovementTracker::new(7, 2, &state([0.0, 64.0, 0.0], 0.0, true));
        // Tick 0 is an update tick and forces a position update (tick % 60 == 0).
        assert_eq!(ids(&t.tick(&state([0.0, 64.0, 0.0], 0.0, true))), [cb::MOVE_ENTITY_POS]);
        // Tick 1 is skipped even though the entity moved.
        assert!(t.tick(&state([1.0, 64.0, 0.0], 0.0, true)).is_empty());
        assert_eq!(ids(&t.tick(&state([1.0, 64.0, 0.0], 0.0, true))), [cb::MOVE_ENTITY_POS]);
        assert_eq!(t.sent_position(), [1.0, 64.0, 0.0]);
        // No change: nothing to send.
        t.tick(&state([1.0, 64.0, 0.0], 0.0, true));
        assert!(t.tick(&state([1.0, 64.0, 0.0], 0.0, true)).is_empty());
        // Tick 5 is off-interval, but a dirty tracker updates anyway; tick 6 is regular, 7 is not.
        t.mark_dirty();
        assert_eq!(ids(&t.tick(&state([2.0, 64.0, 0.0], 0.0, true))), [cb::MOVE_ENTITY_POS]);
        assert_eq!(t.tick(&state([3.0, 64.0, 0.0], 0.0, true)).len(), 1);
        assert!(t.tick(&state([4.0, 64.0, 0.0], 0.0, true)).is_empty());
    }

    #[test]
    fn tracker_picks_packet_kinds() {
        let mut t = MovementTracker::new(7, 1, &state([0.0, 64.0, 0.0], 0.0, true));
        t.tick(&state([0.0, 64.0, 0.0], 0.0, true));
        // Rotation only (yaw and head yaw).
        assert_eq!(ids(&t.tick(&state([0.0, 64.0, 0.0], 90.0, true))), [cb::MOVE_ENTITY_ROT, cb::ROTATE_HEAD]);
        // Position and rotation.
        assert_eq!(ids(&t.tick(&state([0.5, 64.0, 0.0], 180.0, true))), [cb::MOVE_ENTITY_POS_ROT, cb::ROTATE_HEAD]);
        // Out of delta range: full sync, and the base follows.
        assert_eq!(ids(&t.tick(&state([20.0, 64.0, 0.0], 180.0, true))), [cb::ENTITY_POSITION_SYNC]);
        assert_eq!(t.sent_position(), [20.0, 64.0, 0.0]);
        // Leaving the ground forces a sync even for a small move.
        assert_eq!(ids(&t.tick(&state([20.0, 64.4, 0.0], 180.0, false))), [cb::ENTITY_POSITION_SYNC]);
        // Sub-threshold jitter is not sent.
        assert!(t.tick(&state([20.001, 64.4, 0.0], 180.0, false)).is_empty());
        assert_eq!(t.sent_position(), [20.0, 64.4, 0.0]);
    }

    #[test]
    fn tracker_move_packet_carries_delta_from_base() {
        let mut t = MovementTracker::new(3, 1, &state([0.0, 64.0, 0.0], 0.0, true));
        t.tick(&state([0.0, 64.0, 0.0], 0.0, true));
        let p = t.tick(&state([0.25, 64.0, -0.5], 0.0, true));
        let mut r = Reader::new(&p[0]);
        assert_eq!(r.varint().unwrap(), cb::MOVE_ENTITY_POS);
        assert_eq!(r.varint().unwrap(), 3);
        assert_eq!(r.varint().unwrap(), 1); // on ground, 0 steps
        assert_eq!([r.i16().unwrap(), r.i16().unwrap(), r.i16().unwrap()], [1024, 0, -2048]);
        r.finish().unwrap();
    }

    #[test]
    fn tracker_forces_sync_every_400_updates() {
        let s = state([0.0, 64.0, 0.0], 0.0, true);
        let mut t = MovementTracker::new(1, 1, &s);
        let mut syncs = 0;
        // teleport_delay reaches 401 on updates 400 and 801 (it restarts from 0 after a sync).
        for i in 0..802 {
            let p = t.tick(&state([(i % 2) as f64 * 0.5, 64.0, 0.0], 0.0, true));
            syncs += ids(&p).iter().filter(|&&id| id == cb::ENTITY_POSITION_SYNC).count();
        }
        assert_eq!(syncs, 2);
    }
}
