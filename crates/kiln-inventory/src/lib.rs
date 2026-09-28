//! Inventories, menus, clicks and crafting with vanilla 26.3 semantics.
//!
//! - [`Menu`]: `AbstractContainerMenu` — slots over containers, the carried stack, every click
//!   type ([`Menu::clicked`]), and the slot synchronization vanilla does with the client
//!   ([`Menu::broadcast_changes`], state ids, hashed remote slots).
//! - [`click`]: the serverbound menu packets and their handlers (`container_click` with its
//!   desync handling, creative mode slots, closing, buttons).
//! - [`PlayerInventory`], [`SimpleContainer`], the [`Container`] trait for block entities.
//! - [`recipe`]: recipes loaded at runtime from a datapack; crafting, stonecutting, smithing,
//!   cooking and brewing lookups; the `update_recipes` packet.
//! - [`persist`]: playerdata and container block entity item lists.
//!
//! Menu operations take an [`Env`] (the player's inventory, the block container, rules and
//! world hooks) and push [`Effect`]s: the clientbound packets in vanilla's order and the side
//! effects (dropped items, equip sounds...) for the simulation to carry out.

mod bundle;
pub mod click;
pub mod container;
pub mod effect;
pub mod inventory;
pub mod menu;
pub mod menus;
pub mod merchant;
pub mod persist;
pub mod recipe;
pub mod remote;
pub mod rules;
pub mod slot;
pub mod stack;
pub mod tags;

pub use click::{ContainerClick, ContainerInput, handle_container_click, handle_set_creative_slot};
pub use container::{Container, SimpleContainer};
pub use effect::Effect;
pub use inventory::PlayerInventory;
pub use menu::{ClickCrash, Env, Menu, NoWorld, PlayerFlags, World};
pub use menus::{FurnaceKind, MenuKind};
pub use recipe::RecipeManager;
pub use rules::Rules;
pub use slot::{Slot, SlotKind, Source};
