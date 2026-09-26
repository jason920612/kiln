//! Bundles' click behavior (`BundleItem.overrideStackedOnOther` / `overrideOtherStackedOnMe`).

use crate::click::ClickAction;
use crate::menu::{Env, Menu};

/// `AbstractContainerMenu.tryItemClickBehaviourOverride`: whether the carried stack or the
/// slot's stack handled the click itself.
pub(crate) fn click_override(_menu: &mut Menu, _env: &mut Env, _slot: usize, _action: ClickAction) -> bool {
    false
}
