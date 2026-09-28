//! `summon` (vanilla `SummonCommand`).

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, SuggestionProvider, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

/// `SummonCommand.spawnEntity`: the entity at `pos`, from `nbt` (an empty compound when none);
/// mobs run `finalizeSpawn` unless NBT was given.
fn spawn<S: Host>(c: &CommandContext<S>, s: &mut S, with_pos: bool, with_nbt: bool) -> Result<i32> {
    let entity = c.identifier("entity").clone();
    let pos = if with_pos { s.stack().resolve(c.coordinates("pos")) } else { s.stack().position };
    let block = [pos[0].floor() as i32, pos[1].floor() as i32, pos[2].floor() as i32];
    if !s.is_in_spawnable_bounds(block) {
        return Err(CommandError::new(tr!("commands.summon.invalidPosition")));
    }
    let nbt = if with_nbt { Some(c.nbt("nbt").clone()) } else { None };
    let name = s.summon(&entity, pos, nbt.as_ref(), !with_nbt)?;
    s.send_success(tr!("commands.summon.success", name), true);
    Ok(1)
}

pub fn summon<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("summon").requires(LEVEL_GAMEMASTERS).then(
            argument("entity", ArgumentType::resource("minecraft:entity_type"))
                .suggests(SuggestionProvider::Named("minecraft:summonable_entities"))
                .executes(|c, s: &mut S| spawn(c, s, false, false))
                .then(
                    argument("pos", ArgumentType::vec3())
                        .executes(|c, s: &mut S| spawn(c, s, true, false))
                        .then(argument("nbt", ArgumentType::NbtCompound).executes(|c, s: &mut S| spawn(c, s, true, true))),
                ),
        ),
    );
}
