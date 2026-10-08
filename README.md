# Kiln

A Minecraft: Java Edition server core in Rust, pinned to **26.3** (protocol 777). The goal is
vanilla compatibility with much better performance: region-parallel ticking (Folia-style
regions, see the design), deterministic phase-parallel work inside a region, and explicit,
documented approximations where they buy speed.

Status: early. Players can join (offline, online mode, Velocity or BungeeCord forwarding),
see and track each other, chat, run the basic vanilla commands, build and break blocks in
creative mode with correct lighting and block entities, on a superflat world or a vanilla 26.3
save (Anvil, player data and level data read and written). The simulation is region-parallel:
cells of 8×8 chunks group into regions at least 256 blocks apart, which tick in parallel on
the tick pool with results identical to a single region (tested). Item stacks with all data
components and bit-exact overworld density functions are in place for what comes next; most
gameplay (survival, mobs, redstone, full world generation) is still to come. The milestones
are in [docs/design-v2-regionized.md](docs/design-v2-regionized.md) §14.

## Layout

| crate | role |
|---|---|
| `kiln-proto` | codec, NBT, packet encoders/decoders |
| `kiln-data` | tables generated from the vanilla jar (packets, registries, blocks, entities, game rules) |
| `kiln-link` | the boundary between networking and the simulation |
| `kiln-net` | tokio networking: handshake, status, login, configuration, per-connection writer |
| `kiln-sim` | the simulation: a synchronous tick loop that never awaits, regions ticked in parallel |
| `kiln-region` | the regionizer: cells, regions, merge, lazy split, fusion |
| `kiln-sched` | the tick pool: region fork-join and phase windows without priority inversion |
| `kiln-world` | paletted sections, chunks grouped in 8×8-chunk cells, light engine, spawn finder |
| `kiln-item` | item stacks with every data component (wire, NBT, hashed) |
| `kiln-loot` | loot tables, predicates and item modifiers loaded from the datapack, vanilla-exact |
| `kiln-worldgen`, `kiln-javamath` | 26.3 density functions and noise, bit-exact with vanilla |
| `kiln-storage` | Anvil region files and chunk NBT |
| `kiln-command` | Brigadier-compatible commands and the vanilla command set |
| `kiln-server` | the `kiln` binary |
| `kiln-bot` | headless load-test bots that speak 26.3 |
| `xtask` | data fetch, extraction and code generation |

## Building and running

Needs Rust (edition 2024) and, for data tasks, JDK 25 on `PATH`.

```sh
cargo xtask fetch        # download the pinned vanilla server into work/ and run its data generator
cargo xtask extract      # per-state block facts and game rule defaults from the vanilla jar
cargo xtask codegen      # regenerate crates/kiln-data/src/gen
cargo run --release -p kiln-server
```

Mojang's jar and data live only in the gitignored `work/` directory (`KILN_WORK` overrides it);
the committed tables contain identifiers and numbers only.

Settings come from the environment until there is a config file:

| variable | meaning |
|---|---|
| `KILN_PORT` | listen port (25565) |
| `KILN_MAX_PLAYERS` | player limit (100) |
| `KILN_WORLD` | a vanilla 26.3 world directory to load; superflat when unset |
| `KILN_WORLD_FORMAT` | `native` stores a new world in Kiln's native format (an existing world keeps its format) |
| `KILN_OPS` | comma-separated operator names; `prefix*` matches every name with that prefix |
| `KILN_TICK_THREADS` | tick pool size (all cores but one, at most 7) |
| `KILN_REGIONS` | `unified` for one region per dimension (vanilla profile) |
| `KILN_ONLINE_MODE` | authenticate with Mojang (`true`/`false`) |
| `KILN_COMPACTION` | `inline` compacts native cell files on the saving thread (for comparisons); default is a background thread |
| `KILN_MEMO` | `0` turns off the reuse of block scans by entities that stand still (collisions, supporting block, in-wall, inside blocks; results are identical, for comparisons) |
| `KILN_MEMO_CHECK` | `1` checks every such reuse against a fresh scan and panics on a difference (tests, parity and sim_load runs) |
| `KILN_PROFILE_LOOKUP` | `fetchprofile` looks names and ids up through the session service (default: on in online mode); a lookup sends only the name or id asked for |
| `KILN_VIEW_DISTANCE`, `KILN_SIMULATION_DISTANCE` | `view-distance` and `simulation-distance` (10 and 10) |
| `KILN_GENERATOR`, `KILN_SEED`, `KILN_DATAPACK` | `noise`: vanilla overworld terrain from this seed; the data generator output (`work/generated`) |
| `KILN_GEN_THREADS` | chunk generation threads per level (as many as the tick pool, at background priority: `SCHED_IDLE` on Linux, below normal on Windows) |
| `KILN_BACKGROUND_STORAGE` | `0`: chunk reads and writes on the tick thread even where chunks generate in the background (by default Anvil worlds read on two loader threads, encode unloaded chunks on an encoder thread and write region files on a writer thread) |
| `KILN_TICK_TRACE` | a file that gets one line per tick: `<unix ms> <players> <tick micros>` (exact percentiles over any stretch of a run) |
| `KILN_SLOW_PRINT` | ticks slower than this many milliseconds are logged with their phases |
| `KILN_PROXY` | `none`, `velocity` or `bungeecord` |
| `KILN_VELOCITY_SECRET`, `KILN_VELOCITY_SECRET_FILE` | Velocity modern forwarding secret |
| `KILN_BUNGEEGUARD_TOKENS` | accepted BungeeGuard tokens |
| `KILN_PROXY_COMPRESSION_THRESHOLD` | compression behind a proxy (-1 = off) |
| `RUST_LOG` | log filter (`info`) |

Lines typed on the server's standard input run as console commands (`stop` saves and exits).

Worlds are Anvil (vanilla's format) by default. The optional native format (cell files, zstd)
loads and saves faster and is smaller; conversion is lossless both ways:

```sh
kiln world convert --to native <world> <new world>   # and --to anvil to go back
kiln world compare <anvil world> <anvil world>       # chunk NBT byte for byte, other files
```

A native cell file is a log: saves append records and a new index. When stale records take
more than half of a file, it is compacted on a background thread (a copy of the live records is
written next to it and swapped in by rename, with whatever was appended meanwhile), so saving
never waits for a rewrite; a copy left by a crash is deleted when the world opens, and the
cell file itself is never modified in place.

## Testing

```sh
cargo test --workspace
python tools/e2e.py [--client]       # tests, release build, server, smoke client, vanilla-codec
                                     # decoding of captured packets; --client joins a real 26.3 client
python tools/load_test.py --count 1000 --groups 20    # server + kiln-bot, tick statistics
cargo run --release -p kiln-bot --example survival_bench -- --count 200 --world scratch/world     --datapack work/generated        # real survival play on vanilla terrain, see below
cargo run --release -p kiln-sim --example sim_load -- --players 1000 --groups 20
                                     # the simulation alone with scripted in-process players
cargo run --release -p kiln-sim --features prof --example sim_load -- --players 40 --groups 20 \
    --mobs 400 --kinds villager --village --day-time 3000
                                     # brain mobs (--kinds a,b,c; --village adds beds, workstations and
                                     # a bell per group); with the prof feature the time spent in
                                     # named scopes (kiln_entity::prof!) is sampled and printed per
                                     # tick; KILN_SLOW_PRINT=<ms> lists the ticks over that cost
                                     # (the prof feature also prints tallies, e.g. how often the memo of a
                                     # block scan hit; the state hash printed at the end is the check that
                                     # an optimisation changed no result)
python tools/wp35_view.py [--scene chestboat|donkey|parrots|knots|trader|riders|horses|breeze]
                                     # a real 26.3 client walks through recent features (menus, riders,
                                     # shoulder and dancing parrots, knots, breeze) and screenshots the
                                     # game window; `kiln interact <player> <pos> [sneak]` clicks an entity
python tools/vanilla_baseline.py     # the same bot workload against the vanilla server
KILN_PARITY=1 cargo test -p kiln-worldgen --release --test parity
                                     # worldgen bit parity (vectors from tools/worldgen_vectors.py)
KILN_PARITY=1 cargo test -p kiln-loot --test vanilla_parity
                                     # loot parity (vectors from tools/loot_vectors.py)
python tools/mob_vectors.py --filter finalize --out work/wp33/finalize.jsonl
KILN_FINALIZE_VECTORS=work/wp33/finalize.jsonl cargo test -p kiln-entity --test finalize_parity
                                     # finalizeSpawn of natural spawns (jockeys and their riders, the
                                     # level random after) against vanilla, seed by seed
python tools/mob_vectors.py --filter spear_ --out work/wp34/mob_spear.jsonl
                                     # mobs with spears (zombies, husks, zombified piglins, riders of
                                     # zombie horses and camel husks, piglins) tick by tick
python tools/combat_vectors.py --filter spear --out work/wp34/combat/vectors.jsonl
                                     # players' spears (stabs and charges, lunge); drop --filter for all
                                     # combat, enchantment, riptide, mount and spear parity
python tools/mob_vectors.py --filter "kill_villager|ench_" --out work/wp36/mob_wp36.jsonl
KILN_MOB_VECTORS=work/wp36/mob_wp36.jsonl KILN_DATAPACK=work/generated cargo test -p kiln-entity --test mob_parity
                                     # zombies that kill villagers (conversion by difficulty, villager
                                     # data kept) and mobs' enchanted spears (ench_* need the datapack)
python tools/mob_vectors.py --filter "push_|avoid_|spider_golem" --out work/wp41/mob_wp41.jsonl
                                     # mobs bumping into boats and minecarts (the vehicles' motion is traced too,
                                     # bit for bit) and the monsters that run from cats, wolves, armadillos and
                                     # creakings or hunt iron golems; replay: KILN_MOB_VECTORS=<file> cargo test -p
                                     # kiln-entity --test mob_parity
python tools/combat_vectors.py --filter spear --out work/wp41/combat/vectors.jsonl
KILN_WORK=work KILN_SPEAR_VECTORS=work/wp41/combat/spear.jsonl cargo test -p kiln-sim --lib spear_parity
                                     # (without KILN_PARITY_FILTER) spears and fists turning fireballs and wind
                                     # charges around (stab_projectile, melee_projectile) and stabbing boats and carts
python tools/mob_vectors.py --filter "spawner_|cavespider_" --out work/wp44/spawner/vectors.jsonl
KILN_MOB_VECTORS=work/wp44/spawner/vectors.jsonl cargo test -p kiln-entity --test mob_parity
                                     # mob spawner blocks tick by tick (delay, potentials and weights,
                                     # spawn range, nearby cap, light and custom spawn rules, the player
                                     # range and the spawner_blocks_work rule, spawn eggs, the level random
                                     # and the saved block entity after) and cave spiders
python tools/spawn_vectors.py        # natural spawning's structure overrides: the mob lists NaturalSpawner.mobsAt
                                     # gives in and around fortresses, bastions, swamp huts, monuments,
                                     # outposts, trial chambers, ancient cities... of a generated vanilla world
                                     # against kiln-sim's spawn table (cargo test -p kiln-sim structure_spawns)
python tools/block_vectors.py        # what blocks do on their own, tick by tick against vanilla: random and scheduled
                                     # ticks of crops, vines and kelp, grass spreading, melting, copper, turtle eggs,
                                     # corals, sponges, tripwires, the end portal frame; kiln-blocks block_parity, and
                                     # trees grown through the real worldgen features (kiln-sim --test tree_parity)
python tools/interact_vectors.py     # signs (editing, dyes, wax, locks), books, armor worn by right click and middle
                                     # click picking against vanilla (cargo test -p kiln-sim interact_parity)
python tools/combat_vectors.py --filter melee --out work/wp45/combat/vectors.jsonl
                                     # players' melee on mobs and players: sweeping, critical hits, the mace; replay:
                                     # KILN_MELEE_VECTORS=work/wp45/combat/melee.jsonl cargo test -p kiln-sim --lib melee_parity
python tools/effect_vectors.py       # also the player's falls (blocks that stop or bounce them) and hazards (cactus,
                                     # powder snow, suffocation); effect_parity lists the few scenarios it does not match
python tools/admin_check.py          # vanilla and Kiln load each other's saves: seed, game rules, command storage, ops
python tools/parity_suites.py        # every suite that replays vanilla data, with its pass counts
python tools/container_vectors.py --filter jukebox --out work/wp36/containers/vectors.jsonl
KILN_CONTAINER_VECTORS=work/wp36/containers/vectors.jsonl cargo test -p kiln-sim container_parity
                                     # jukeboxes (song end, comparator, hoppers, power) tick by tick
python tools/command_diff.py           # vanilla vs Kiln consoles, including /summon with Passengers
python tools/entity_persist_check.py   # entity chunks and riding stacks load in vanilla and back
cargo test -p kiln-sim --test region_stacks
                                     # riding stacks, leads and open cart menus through region merges and
                                     # splits, `/tp` of entities (ids, riders and menus kept across regions),
                                     # a rider that teleports, boats through portals: every tick's digest
                                     # (entities as saved, riding, menus, packets) equal on one region, one
                                     # per group and parallel workers; invariants under independent scheduling

```

### Survival benchmark

`survival_bench` (an example of `kiln-bot`) measures the server on real survival play instead of
a synthetic crowd. It starts a release `kiln` on vanilla noise terrain (`KILN_GENERATOR=noise`, a
fixed seed, a new world directory), connects `--count` survival bots in groups of `--group-size`
whose sites lie `--spacing` blocks apart, waits until 95% have arrived, and measures `--measure`
seconds after a `--warmup`. `--phase both` then copies the saved world (without the players'
positions) and runs the same bots on it, so that chunks load from disk instead of being
generated; `--format native` does both phases on native cell files.

The bots (`kiln-bot --behavior survival`) speak the real protocol and play by the 26.3 client's
rules: vanilla movement and collision (walk, sprint, jump, swim, fall) on the blocks they receive,
digging with the right tool for the vanilla break time (Player Action start and stop), placing
blocks with Use Item On, opening chests and hoppers and shift-clicking items, eating, fighting
hostile mobs, chatting. They are operators and take their tools and building blocks with
`/item replace`, which keeps the packets they send real. Roles are dealt in turn:

* explorer: walks and sprints across far terrain, steering around cliffs, lava and obstacles,
  waiting when the terrain ahead has not arrived (the vanilla client does not walk into unloaded chunks)
* miner: digs a shaft down, then tunnels with torches, stripping the ores it meets
* builder: finds flat ground, builds a house block by block from the inside, then takes it down
* redstone: builds an observer clock, a hopper clock, a feeder chest over a hopper line into a
  chest, a piston door with a lever and a dust and repeater line to a lamp, and keeps toggling the levers

It reports server MSPT (mean, p50, p99, max, share over 50 ms) from the per-tick trace and the
phases from the server's reports; chunk generation and loading from the server's `chunk totals`
lines (chunks per second, generation thread utilisation, request to delivery, disk time per chunk,
install time, chunks generated synchronously on the tick thread); and from the bots chunk arrival
latency, the time until a new view was complete (walking, teleporting, joining), time spent
waiting for terrain, corrections (server teleports the bot did not ask for), disconnects, decode
errors, digging and placing success, plus the CPU of server, bots and machine. `KILN_BOT_TRACE=1`
makes a bot print what it does; `--phase-detail` and `--slow-print <ms>` pass `KILN_PHASE_DETAIL`
and `KILN_SLOW_PRINT` to the server.

Packets Kiln encodes are checked by decoding them with vanilla's own codecs
(`tools/VanillaDecode.java`); saved worlds are checked by loading them in the vanilla server
(`tools/vanilla_check_world.py`), and native conversion by a round trip of vanilla worlds
that vanilla loads again (`tools/native_roundtrip_check.py`). Storage throughput:
`cargo run --release -p kiln-storage --example native_bench -- bench <world>`, and in the
running simulation `cargo run --release -p kiln-sim --example sim_storage -- <world>`.

## Clean room

Kiln is written from protocol documentation, observed behaviour and the vanilla game's
bytecode as a reference for formats and constants. Code from GPL/AGPL server projects
(Pumpkin, SteelMC, Paper, Folia) is not copied or ported. The project license has not been
chosen yet.
