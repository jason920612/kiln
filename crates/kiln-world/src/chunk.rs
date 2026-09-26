//! Chunk columns: sections, light, heightmaps and a cached encoded packet body.

use crate::section::{Section, bits_for, pack};
use bytes::{BufMut, Bytes, BytesMut};
use kiln_data::blocks_types::is_air;
use kiln_proto::WriteExt;

/// Light of one section.
#[derive(Clone)]
pub enum Light {
    Zero,
    Full,
    Nibbles(Box<[u8; 2048]>),
}

impl Light {
    fn get(&self, i: usize) -> u8 {
        match self {
            Light::Zero => 0,
            Light::Full => 15,
            Light::Nibbles(n) => (n[i >> 1] >> ((i & 1) * 4)) & 0xf,
        }
    }

    fn set(&mut self, i: usize, v: u8) {
        if self.get(i) == v {
            return;
        }
        if !matches!(self, Light::Nibbles(_)) {
            let fill = if matches!(self, Light::Full) { 0xff } else { 0 };
            *self = Light::Nibbles(Box::new([fill; 2048]));
        }
        let Light::Nibbles(n) = self else { unreachable!() };
        let shift = (i & 1) * 4;
        n[i >> 1] = (n[i >> 1] & !(0xf << shift)) | (v << shift);
    }
}

pub struct Chunk {
    pub sections: Vec<Section>,
    min_y: i32,
    /// Sky light for sections -1..=len (index 0 is the section below the world).
    sky: Vec<Light>,
    /// Per column: highest non-air block + 1, relative to `min_y` (0 = empty column).
    surface: Box<[u16; 256]>,
    version: u32,
    cached: Option<(u32, Bytes)>,
}

impl Chunk {
    pub fn new(sections: Vec<Section>, min_y: i32) -> Self {
        let n = sections.len();
        let mut c = Self {
            sections,
            min_y,
            sky: vec![Light::Zero; n + 2],
            surface: Box::new([0; 256]),
            version: 0,
            cached: None,
        };
        for x in 0..16 {
            for z in 0..16 {
                c.surface[(z << 4) | x] = c.column_top(x, z);
            }
        }
        for li in 0..c.sky.len() {
            c.sky[li] = c.section_sky(li);
        }
        c
    }

    /// Sky light of light section `li` (0 = below the world) from the surface heights.
    fn section_sky(&self, li: usize) -> Light {
        let base = (li as i32 - 1) * 16;
        let (lo, hi) = self.surface.iter().fold((u16::MAX, 0), |(lo, hi), &t| (lo.min(t), hi.max(t)));
        if hi as i32 <= base {
            return Light::Full;
        }
        if lo as i32 >= base + 16 {
            return Light::Zero;
        }
        let mut n = Box::new([0u8; 2048]);
        for i in 0..4096 {
            let (ly, col) = ((i >> 8) as i32, i & 0xff);
            if base + ly >= self.surface[col] as i32 {
                n[i >> 1] |= 15 << ((i & 1) * 4);
            }
        }
        Light::Nibbles(n)
    }

    fn column_top(&self, x: usize, z: usize) -> u16 {
        for rel in (0..self.height()).rev() {
            let (s, ly) = ((rel >> 4) as usize, (rel & 15) as usize);
            if !self.sections[s].is_empty() && !is_air(self.sections[s].get(x, ly, z)) {
                return rel as u16 + 1;
            }
        }
        0
    }

    pub fn height(&self) -> i32 {
        self.sections.len() as i32 * 16
    }

    fn section_of(&self, y: i32) -> Option<(usize, usize)> {
        let rel = y - self.min_y;
        (rel >= 0 && rel < self.height()).then(|| ((rel >> 4) as usize, (rel & 15) as usize))
    }

    /// `x`, `z` are within the chunk (0..16); `y` is absolute.
    pub fn get(&self, x: usize, y: i32, z: usize) -> u16 {
        match self.section_of(y) {
            Some((s, ly)) => self.sections[s].get(x, ly, z),
            None => kiln_data::blocks::default_state::VOID_AIR,
        }
    }

    /// Returns the previous state, or `None` if `y` is outside the world.
    pub fn set(&mut self, x: usize, y: i32, z: usize, state: u16) -> Option<u16> {
        let (s, ly) = self.section_of(y)?;
        let old = self.sections[s].set(x, ly, z, state);
        if old != state {
            self.version += 1;
            if is_air(old) != is_air(state) {
                self.recompute_column(x, z);
            }
        }
        Some(old)
    }

    /// Updates the surface height and the column's sky light: full above the surface, dark below.
    /// (Vertical-only sky light; horizontal propagation arrives with the lighting engine.)
    fn recompute_column(&mut self, x: usize, z: usize) {
        let old = self.surface[(z << 4) | x] as i32;
        let top = self.column_top(x, z);
        self.surface[(z << 4) | x] = top;
        let (lo, hi) = (old.min(top as i32), old.max(top as i32));
        for (li, light) in self.sky.iter_mut().enumerate() {
            let base = (li as i32 - 1) * 16;
            if base + 16 <= lo || base >= hi {
                continue; // this section's column values did not change
            }
            for ly in 0..16 {
                let v = if base + ly >= top as i32 { 15 } else { 0 };
                light.set(((ly as usize) << 8) | (z << 4) | x, v);
            }
            if let Light::Nibbles(n) = light {
                if n.iter().all(|&b| b == 0xff) {
                    *light = Light::Full;
                } else if n.iter().all(|&b| b == 0) {
                    *light = Light::Zero;
                }
            }
        }
    }

    /// Chunk Data body after the coordinates; re-encoded only when the chunk changed.
    pub fn packet_body(&mut self, biome_count: usize) -> Bytes {
        if let Some((v, body)) = &self.cached {
            if *v == self.version {
                return body.clone();
            }
        }
        let body = self.encode(biome_count);
        self.cached = Some((self.version, body.clone()));
        body
    }

    fn encode(&self, biome_count: usize) -> Bytes {
        let mut b = BytesMut::with_capacity(16 * 1024);

        let bits = bits_for(self.height() as usize + 1);
        let heights = pack(&self.surface.map(|h| h as u64), bits);
        const WORLD_SURFACE: i32 = 1;
        const MOTION_BLOCKING: i32 = 4;
        const MOTION_BLOCKING_NO_LEAVES: i32 = 5;
        b.put_varint(3);
        for kind in [WORLD_SURFACE, MOTION_BLOCKING, MOTION_BLOCKING_NO_LEAVES] {
            b.put_varint(kind);
            b.put_varint(heights.len() as i32);
            for l in &heights {
                b.put_u64(*l);
            }
        }

        let mut data = BytesMut::with_capacity(8 * 1024);
        for s in &self.sections {
            s.encode(&mut data, biome_count);
        }
        b.put_varint(data.len() as i32);
        b.put_slice(&data);

        b.put_varint(0); // block entities

        let mut sky_mask = 0u64;
        let mut empty_sky = 0u64;
        for (i, l) in self.sky.iter().enumerate() {
            match l {
                Light::Zero => empty_sky |= 1 << i,
                _ => sky_mask |= 1 << i,
            }
        }
        let all = (1u64 << self.sky.len()) - 1;
        b.put_bitset(&[sky_mask]);
        b.put_bitset(&[0]); // block light
        b.put_bitset(&[empty_sky]);
        b.put_bitset(&[all]); // no block light anywhere yet
        b.put_varint(sky_mask.count_ones() as i32);
        for l in &self.sky {
            match l {
                Light::Zero => {}
                Light::Full => {
                    b.put_varint(2048);
                    b.put_bytes(0xff, 2048);
                }
                Light::Nibbles(n) => {
                    b.put_varint(2048);
                    b.put_slice(&n[..]);
                }
            }
        }
        b.put_varint(0);
        b.freeze()
    }
}
