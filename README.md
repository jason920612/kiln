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
| `KILN_PROFILE_LOOKUP` | `fetchprofile` looks names and ids up through the session service (default: on in online mode); a lookup sends only the name or id asked for |
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
cargo run --release -p kiln-sim --example sim_load -- --players 1000 --groups 20
                                     # the simulation alone with scripted in-process players
python tools/vanilla_baseline.py     # the same bot workload against the vanilla server
KILN_PARITY=1 cargo test -p kiln-worldgen --release --test parity
                                     # worldgen bit parity (vectors from tools/worldgen_vectors.py)
KILN_PARITY=1 cargo test -p kiln-loot --test vanilla_parity
                                     # loot parity (vectors from tools/loot_vectors.py)
```

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
