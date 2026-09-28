//! Shulker bullets (`ShulkerBullet`).

use crate::ext_entity::EntityExt;
use crate::persist::Input;

/// Reads a saved one (`None` until simulated).
pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let _ = r;
    None
}
