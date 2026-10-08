//! Coordinate arguments: absolute, `~` relative and `^` local, as parsed by
//! `WorldCoordinate`, `WorldCoordinates` and `LocalCoordinates`.

use crate::error::CommandError;
use crate::reader::StringReader;

type Result<T> = std::result::Result<T, CommandError>;

/// One axis: `value` or `~value` (relative to the source).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldCoordinate {
    pub relative: bool,
    pub value: f64,
}

impl WorldCoordinate {
    pub fn get(&self, base: f64) -> f64 {
        if self.relative { self.value + base } else { self.value }
    }

    fn is_relative(reader: &mut StringReader) -> bool {
        let rel = reader.can_read() && reader.peek() == '~';
        if rel {
            reader.skip();
        }
        rel
    }

    /// `center_correct` adds 0.5 to absolute integers (block centers), as vanilla does for x and z.
    pub fn parse_double(reader: &mut StringReader, center_correct: bool) -> Result<Self> {
        if reader.can_read() && reader.peek() == '^' {
            return Err(CommandError::pos_mixed().at(reader));
        }
        if !reader.can_read() {
            return Err(CommandError::expected_coordinate().at(reader));
        }
        let relative = Self::is_relative(reader);
        let start = reader.cursor();
        let mut value = if reader.can_read() && reader.peek() != ' ' { reader.read_double()? } else { 0.0 };
        let text = &reader.string()[start..reader.cursor()];
        if relative && text.is_empty() {
            return Ok(WorldCoordinate { relative: true, value: 0.0 });
        }
        if !text.contains('.') && !relative && center_correct {
            value += 0.5;
        }
        Ok(WorldCoordinate { relative, value })
    }

    pub fn parse_int(reader: &mut StringReader) -> Result<Self> {
        if reader.can_read() && reader.peek() == '^' {
            return Err(CommandError::pos_mixed().at(reader));
        }
        if !reader.can_read() {
            return Err(CommandError::expected_block_position().at(reader));
        }
        let relative = Self::is_relative(reader);
        let value = if reader.can_read() && reader.peek() != ' ' {
            if relative { reader.read_double()? } else { reader.read_int()? as f64 }
        } else {
            0.0
        };
        Ok(WorldCoordinate { relative, value })
    }
}

/// A parsed position or rotation. Rotations use vanilla's layout: `x` is the pitch, `y` the
/// yaw (see [`Coordinates::rotation`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coordinates {
    World([WorldCoordinate; 3]),
    Local { left: f64, up: f64, forwards: f64 },
}

const REL0: WorldCoordinate = WorldCoordinate { relative: true, value: 0.0 };

impl Coordinates {
    /// World position relative to `origin`, facing `rotation` (`[yaw, pitch]`).
    pub fn position(&self, origin: [f64; 3], rotation: [f32; 2]) -> [f64; 3] {
        match *self {
            Coordinates::World([x, y, z]) => [x.get(origin[0]), y.get(origin[1]), z.get(origin[2])],
            Coordinates::Local { left, up, forwards } => local_to_world(origin, rotation, left, up, forwards),
        }
    }

    /// `BlockPos.containing(position)`.
    pub fn block_pos(&self, origin: [f64; 3], rotation: [f32; 2]) -> [i32; 3] {
        self.position(origin, rotation).map(|v| v.floor() as i32)
    }

    /// `[yaw, pitch]` relative to the source's `[yaw, pitch]` (`Coordinates.getRotation`).
    pub fn rotation(&self, source: [f32; 2]) -> [f32; 2] {
        match *self {
            Coordinates::World([x, y, _]) => [y.get(source[0] as f64) as f32, x.get(source[1] as f64) as f32],
            Coordinates::Local { .. } => [0.0, 0.0],
        }
    }

    /// Which axes are relative (`isXRelative` ...); local coordinates are relative on all.
    pub fn relative(&self) -> [bool; 3] {
        match self {
            Coordinates::World(c) => c.map(|c| c.relative),
            Coordinates::Local { .. } => [true; 3],
        }
    }

    /// `Vec3Argument` (`center_correct` is true for `vec3()`).
    pub fn parse_vec3(reader: &mut StringReader, center_correct: bool) -> Result<Self> {
        if reader.can_read() && reader.peek() == '^' {
            Self::parse_local(reader)
        } else {
            Self::parse_world(reader, |r, axis| WorldCoordinate::parse_double(r, center_correct && axis != 1))
        }
    }

    /// `BlockPosArgument`.
    pub fn parse_block_pos(reader: &mut StringReader) -> Result<Self> {
        if reader.can_read() && reader.peek() == '^' {
            Self::parse_local(reader)
        } else {
            Self::parse_world(reader, |r, _| WorldCoordinate::parse_int(r))
        }
    }

    /// `Vec2Argument`: `x z`.
    pub fn parse_vec2(reader: &mut StringReader, center_correct: bool) -> Result<Self> {
        let [x, z] = Self::parse_pair(reader, CommandError::pos2d_incomplete, |r| {
            WorldCoordinate::parse_double(r, center_correct)
        })?;
        Ok(Coordinates::World([x, REL0, z]))
    }

    /// `ColumnPosArgument`: `x z` as block coordinates.
    pub fn parse_column_pos(reader: &mut StringReader) -> Result<Self> {
        let [x, z] = Self::parse_pair(reader, CommandError::pos2d_incomplete, WorldCoordinate::parse_int)?;
        Ok(Coordinates::World([x, REL0, z]))
    }

    /// `RotationArgument`: `yaw pitch`.
    pub fn parse_rotation(reader: &mut StringReader) -> Result<Self> {
        let [yaw, pitch] =
            Self::parse_pair(reader, CommandError::rotation_incomplete, |r| WorldCoordinate::parse_double(r, false))?;
        Ok(Coordinates::World([pitch, yaw, REL0]))
    }

    fn parse_pair(
        reader: &mut StringReader,
        incomplete: fn() -> CommandError,
        parse: impl Fn(&mut StringReader) -> Result<WorldCoordinate>,
    ) -> Result<[WorldCoordinate; 2]> {
        let start = reader.cursor();
        if !reader.can_read() {
            return Err(incomplete().at(reader));
        }
        let a = parse(reader)?;
        if reader.can_read() && reader.peek() == ' ' {
            reader.skip();
            Ok([a, parse(reader)?])
        } else {
            reader.set_cursor(start);
            Err(incomplete().at(reader))
        }
    }

    fn parse_world(
        reader: &mut StringReader,
        parse: impl Fn(&mut StringReader, usize) -> Result<WorldCoordinate>,
    ) -> Result<Self> {
        let start = reader.cursor();
        let mut out = [REL0; 3];
        for (axis, slot) in out.iter_mut().enumerate() {
            if axis > 0 {
                if !reader.can_read() || reader.peek() != ' ' {
                    reader.set_cursor(start);
                    return Err(CommandError::pos3d_incomplete().at(reader));
                }
                reader.skip();
            }
            *slot = parse(reader, axis)?;
        }
        Ok(Coordinates::World(out))
    }

    fn parse_local(reader: &mut StringReader) -> Result<Self> {
        let start = reader.cursor();
        let mut out = [0.0; 3];
        for (axis, slot) in out.iter_mut().enumerate() {
            if axis > 0 {
                if !reader.can_read() || reader.peek() != ' ' {
                    reader.set_cursor(start);
                    return Err(CommandError::pos3d_incomplete().at(reader));
                }
                reader.skip();
            }
            if !reader.can_read() {
                return Err(CommandError::expected_coordinate().at(reader));
            }
            if reader.peek() != '^' {
                reader.set_cursor(start);
                return Err(CommandError::pos_mixed().at(reader));
            }
            reader.skip();
            *slot = if reader.can_read() && reader.peek() != ' ' { reader.read_double()? } else { 0.0 };
        }
        Ok(Coordinates::Local { left: out[0], up: out[1], forwards: out[2] })
    }
}

/// `LocalCoordinates.apply`, with `Mth`'s table-based sine for bit-exact results.
fn local_to_world(origin: [f64; 3], [yaw, pitch]: [f32; 2], left: f64, up: f64, forwards: f64) -> [f64; 3] {
    const DEG: f32 = 0.017453292;
    let f = mth_cos(((yaw + 90.0) * DEG) as f64);
    let g = mth_sin(((yaw + 90.0) * DEG) as f64);
    let h = mth_cos((-pitch * DEG) as f64);
    let i = mth_sin((-pitch * DEG) as f64);
    let j = mth_cos(((-pitch + 90.0) * DEG) as f64);
    let k = mth_sin(((-pitch + 90.0) * DEG) as f64);
    let fwd = [(f * h) as f64, i as f64, (g * h) as f64];
    let upv = [(f * j) as f64, k as f64, (g * j) as f64];
    let cross =
        [fwd[1] * upv[2] - fwd[2] * upv[1], fwd[2] * upv[0] - fwd[0] * upv[2], fwd[0] * upv[1] - fwd[1] * upv[0]];
    let lft = cross.map(|v| -v);
    std::array::from_fn(|a| origin[a] + (fwd[a] * forwards + upv[a] * up + lft[a] * left))
}

/// `Mth.sin`.
pub fn mth_sin(v: f64) -> f32 {
    kiln_javamath::mth::sin(v)
}

/// `Mth.cos`.
pub fn mth_cos(v: f64) -> f32 {
    kiln_javamath::mth::cos(v)
}

/// `Mth.atan2`: the table-based arctangent vanilla uses for facing rotations.
pub fn mth_atan2(y: f64, x: f64) -> f64 {
    kiln_javamath::mth::atan2(y, x)
}

/// `Mth.wrapDegrees(float)`: into `[-180, 180)`.
pub fn wrap_degrees(v: f32) -> f32 {
    let mut f = v % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec3(s: &str) -> Result<Coordinates> {
        Coordinates::parse_vec3(&mut StringReader::new(s), true)
    }

    #[test]
    fn absolute_and_relative() {
        let c = vec3("1 ~2 ~").unwrap();
        assert_eq!(c.position([10.0, 20.0, 30.0], [0.0, 0.0]), [1.5, 22.0, 30.0]);
        assert_eq!(c.relative(), [false, true, true]);
        // Decimals and y are never center-corrected.
        assert_eq!(vec3("1.0 64 -3").unwrap().position([0.0; 3], [0.0; 2]), [1.0, 64.0, -2.5]);
        let b = Coordinates::parse_block_pos(&mut StringReader::new("~-1.5 70 ~0.5")).unwrap();
        assert_eq!(b.block_pos([0.2, 0.0, 0.2], [0.0; 2]), [-2, 70, 0]);
        let e = Coordinates::parse_block_pos(&mut StringReader::new("1.5 2 3")).unwrap_err();
        assert_eq!(e.key(), Some("parsing.int.invalid"));
    }

    #[test]
    fn local_matches_vanilla() {
        // Facing south (yaw 0): forwards is +z, left is +x.
        let c = vec3("^1 ^2 ^3").unwrap();
        let p = c.position([0.0, 64.0, 0.0], [0.0, 0.0]);
        assert!((p[0] - 1.0).abs() < 1e-6 && (p[1] - 66.0).abs() < 1e-6 && (p[2] - 3.0).abs() < 1e-6, "{p:?}");
        // Facing east (yaw -90), looking 45 degrees down: vanilla values (LocalCoordinates.apply).
        let p = Coordinates::Local { left: 0.0, up: 0.0, forwards: 10.0 }.position([0.0; 3], [-90.0, 45.0]);
        let f = mth_cos(((-90.0f32 + 90.0) * 0.017453292) as f64);
        let h = mth_cos((-45.0f32 * 0.017453292) as f64);
        assert_eq!(p[0], (f * h) as f64 * 10.0);
        assert!(p[1] < -7.07 && p[1] > -7.08);
        assert_eq!(Coordinates::Local { left: 0.0, up: 0.0, forwards: 0.0 }.relative(), [true; 3]);
    }

    #[test]
    fn parse_errors() {
        let key = |r: Result<Coordinates>| {
            let e = r.unwrap_err();
            (e.key().unwrap().to_owned(), e.cursor().unwrap())
        };
        assert_eq!(key(vec3("1 2")), ("argument.pos3d.incomplete".into(), 0));
        assert_eq!(key(vec3("^1 2 3")), ("argument.pos.mixed".into(), 0));
        assert_eq!(key(vec3("1 ^2 3")), ("argument.pos.mixed".into(), 2));
        assert_eq!(key(vec3("1 2 ")), ("argument.pos.missing.double".into(), 4));
        assert_eq!(
            key(Coordinates::parse_block_pos(&mut StringReader::new("1 2 "))),
            ("argument.pos.missing.int".into(), 4)
        );
        assert_eq!(
            key(Coordinates::parse_rotation(&mut StringReader::new("10"))),
            ("argument.rotation.incomplete".into(), 0)
        );
        assert_eq!(
            key(Coordinates::parse_vec2(&mut StringReader::new(""), true)),
            ("argument.pos2d.incomplete".into(), 0)
        );
    }

    #[test]
    fn rotation_layout() {
        let r = Coordinates::parse_rotation(&mut StringReader::new("~10 -20")).unwrap();
        assert_eq!(r.rotation([90.0, 5.0]), [100.0, -20.0]);
        let Coordinates::World([pitch, yaw, _]) = r else { panic!() };
        assert!(yaw.relative && !pitch.relative);
        let c = Coordinates::parse_column_pos(&mut StringReader::new("~1 -3")).unwrap();
        assert_eq!(c.block_pos([4.5, 70.0, 0.0], [0.0; 2]), [5, 70, -3]);
    }

    #[test]
    fn mth_table() {
        assert_eq!(mth_sin(0.0), 0.0);
        assert_eq!(mth_cos(0.0), 1.0);
        assert_eq!(mth_sin(std::f64::consts::FRAC_PI_2), 1.0);
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-540.0), -180.0);
    }
}
