//! The text of a text display, resolved the way the game does it when the entity is read (`TextDisplay.readAdditionalSaveData`:
//! `ComponentUtils.resolve` with the display as the source, at permission level 2): selectors become the names of
//! what they find, scores their values. The entity asks on its first tick (`Event::ResolveText`); the answer replaces
//! its text, and what it saves from then on is the resolved text.

use crate::commands::{CommandSource, PlayerRef};
use crate::{Sim, entities};
use kiln_command::host::SourceStack;
use kiln_proto::nbt::Tag;

impl Sim {
    /// Resolves the texts the regions' displays asked for, and gives them back.
    pub(crate) fn resolve_display_texts(&mut self) {
        let mut requests: Vec<(usize, u128, Tag)> = Vec::new();
        for (dim, d) in self.dims.iter_mut().enumerate() {
            for region in d.regions.iter_mut() {
                for (uuid, text) in std::mem::take(&mut region.part_mut().1.text_requests) {
                    requests.push((dim, uuid, text));
                }
            }
        }
        for (dim, uuid, text) in requests {
            let Some((me, pos)) = self.display_source(dim, uuid) else { continue };
            // A failure leaves the text empty (`Failed to parse display entity text`).
            let resolved = self.resolve_text_as(dim, pos, &me, &text).unwrap_or_else(|| Tag::String(String::new()));
            self.set_display_text(dim, uuid, resolved);
        }
    }

    /// The source of display `uuid`: the entity as a selector sees it, and its block position.
    fn display_source(&self, dim: usize, uuid: u128) -> Option<(PlayerRef, [i32; 3])> {
        for region in self.dims[dim].regions.iter() {
            if let Some(e) = region.part().0.list.iter().find(|e| e.uuid.as_u128() == uuid && !e.removed) {
                let pos = [e.pos[0].floor() as i32, e.pos[1].floor() as i32, e.pos[2].floor() as i32];
                return Some((PlayerRef::of_entity(dim, e, &self.commands.scoreboard), pos));
            }
        }
        None
    }

    /// `ComponentUtils.resolve(context(source), component)`, back to a text tag.
    fn resolve_text_as(&mut self, dim: usize, pos: [i32; 3], me: &PlayerRef, text: &Tag) -> Option<Tag> {
        let component = kiln_command::component::decode(text).ok()?;
        let source = CommandSource::Block { dim, pos };
        let mut stack = SourceStack::of_entity(me.clone());
        stack.max_permission = 2;
        let previous_source = std::mem::replace(&mut self.commands.source, source);
        let previous_stack = std::mem::replace(&mut self.commands.stack, stack);
        let resolved = component.resolve(self, Some(me));
        self.commands.source = previous_source;
        self.commands.stack = previous_stack;
        resolved.ok().map(|c| c.to_nbt())
    }

    fn set_display_text(&mut self, dim: usize, uuid: u128, text: Tag) {
        for region in self.dims[dim].regions.iter_mut() {
            if let Some(e) = region.part_mut().0.list.iter_mut().find(|e: &&mut entities::Entity| e.uuid.as_u128() == uuid && !e.removed)
                && let Some(phys) = e.phys.as_deref_mut()
            {
                kiln_entity::ext_entity::display::set_text(phys, text);
                return;
            }
        }
    }
}
