# Kiln

A Minecraft: Java Edition server core in Rust, pinned to **26.3** (protocol 777). The goal is
vanilla compatibility with much better performance: region-parallel ticking (Folia-style
regions, see the design), deterministic phase-parallel work inside a region, and explicit,
documented approximations where they buy speed.

Status: early. Players can join (offline, online mode, Velocity or BungeeCord forwarding),
see and track each other, chat, run the basic vanilla commands, build and break blocks in
creative mode with correct lighting, on a superflat world or a vanilla 26.3 save (Anvil read
and write). Most gameplay (survival, mobs, redstone, world generation) is still to come; the
milestones are in [docs/design-v2-regionized.md](docs/design-v2-regionized.md) §14.

## Layout

| crate | role |
|---|---|
| `kiln-proto` | codec, NBT, packet encoders/decoders |
| `kiln-data` | tables generated from the vanilla jar (packets, registries, blocks, entities, game rules) |
| `kiln-link` | the boundary between networking and the simulation |
| `kiln-net` | tokio networking: handshake, status, login, configuration, per-connection writer |
| `kiln-sim` | the simulation: one synchronous tick loop, never awaits |
| `kiln-world` | paletted sections, chunks grouped in 8×8-chunk cells, light engine |
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
| `KILN_OPS` | comma-separated operator names; `prefix*` matches every name with that prefix |
| `KILN_ONLINE_MODE` | authenticate with Mojang (`true`/`false`) |
| `KILN_PROXY` | `none`, `velocity` or `bungeecord` |
| `KILN_VELOCITY_SECRET`, `KILN_VELOCITY_SECRET_FILE` | Velocity modern forwarding secret |
| `KILN_BUNGEEGUARD_TOKENS` | accepted BungeeGuard tokens |
| `KILN_PROXY_COMPRESSION_THRESHOLD` | compression behind a proxy (-1 = off) |
| `RUST_LOG` | log filter (`info`) |

Lines typed on the server's standard input run as console commands (`stop` saves and exits).

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
```

Packets Kiln encodes are checked by decoding them with vanilla's own codecs
(`tools/VanillaDecode.java`); saved worlds are checked by loading them in the vanilla server
(`tools/vanilla_check_world.py`).

## Clean room

Kiln is written from protocol documentation, observed behaviour and the vanilla game's
bytecode as a reference for formats and constants. Code from GPL/AGPL server projects
(Pumpkin, SteelMC, Paper, Folia) is not copied or ported. The project license has not been
chosen yet.
