//! Writable and written books: `ServerGamePacketListenerImpl.handleEditBook` (the book and
//! quill's pages, or signing it into a written book) and `WritableBookItem.use` /
//! `WrittenBookItem.use` (the book opens on the client).
//!
//! Not simulated: the chat filter (every text passes through) and
//! `WrittenBookItem.resolveBookComponents` (selectors and scores in the pages of a book that is
//! not marked resolved; books written through this packet are).

use crate::Player;
use kiln_inventory::Container;
use kiln_item::component::{Filterable, WritableBookContent, WrittenBookContent, ids};
use kiln_item::{ItemStack, Text, keys};
use kiln_proto::packets::world_fx;

impl Player {
    /// `handleEditBook`: `slot` is a hotbar slot or 40 (the off hand). A book and quill in it
    /// gets `pages`; with a `title` it becomes a written book by this player instead.
    pub(crate) fn edit_book(&mut self, slot: i32, pages: &[String], title: Option<&str>) {
        // `Inventory.isHotbarSlot(slot) || slot == 40`.
        if !((0..9).contains(&slot) || slot == kiln_inventory::inventory::SLOT_OFFHAND as i32) {
            return;
        }
        let slot = slot as usize;
        let stack = self.inv.item(slot).clone();
        if !stack.has(ids::WRITABLE_BOOK_CONTENT) {
            return;
        }
        match title {
            None => {
                let content = WritableBookContent { pages: pages.iter().map(|p| Filterable::pass_through(p.clone())).collect() };
                self.inv.item_mut(slot).insert(keys::WRITABLE_BOOK_CONTENT, content);
            }
            Some(title) => {
                // `ItemStack.transmuteCopy(WRITTEN_BOOK)` without the book and quill's pages.
                let Some(written) = kiln_item::registry::ITEM.id("minecraft:written_book") else { return };
                let mut book = ItemStack::from_parts(written, stack.count(), stack.patch().clone());
                book.remove(ids::WRITABLE_BOOK_CONTENT);
                book.insert(
                    keys::WRITTEN_BOOK_CONTENT,
                    WrittenBookContent {
                        title: Filterable::pass_through(title.to_owned()),
                        author: self.name.clone(),
                        generation: 0,
                        pages: pages.iter().map(|p| Filterable::pass_through(Text::literal(p.clone()))).collect(),
                        resolved: true,
                    },
                );
                *self.inv.item_mut(slot) = book;
            }
        }
        self.inv.times_changed += 1;
    }

    /// `WritableBookItem.use` / `WrittenBookItem.use` (`Player.openItemGui`): counted as a use;
    /// a written book opens in the hand it is held in (the book and quill's editor is the
    /// client's own and needs no packet).
    pub(crate) fn use_book(&mut self, off_hand: bool) {
        let held = self.in_hand(off_hand);
        let (item, written) = (held.item(), held.has(ids::WRITTEN_BOOK_CONTENT));
        self.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, item), 1);
        if written {
            self.send(world_fx::open_book(off_hand as i32));
        }
    }
}
