//! Ghast fireballs and blaze small fireballs (`AbstractHurtingProjectile`).

use crate::ext_entity::EntityExt;
use crate::persist::Input;

/// Reads a saved one (`None` until simulated).
pub fn load(type_name: &'static str, r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let _ = (type_name, r);
    None
}
