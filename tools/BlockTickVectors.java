// Differential test vectors for what blocks do on their own: random ticks (crops, grass, vines,
// ice, copper, ...), scheduled ticks and what block changes set off, recorded in a real vanilla
// 26.3 dedicated server started in-process.
//
// Each scenario builds blocks in a 32x32 window (x and z from -8 to 23, scenarios use 0..15 and
// leave the margin for what spreads; floor of stone at y 99, open sky, daytime, clear weather,
// plains), seeds the level random and then runs ops, in order:
//   rt       every randomly ticking block of the area (in y, z, x order, found at the start of
//            the op) gets BlockState.randomTick, then FluidState.randomTick, with the level
//            random (what ServerLevel.tickChunk does for the positions it picked)
//   tick N   N whole level ticks (ServerLevel.tick: scheduled block and fluid ticks, block
//            events) with random ticking off
//   set ...  Level.setBlock(pos, state, 3) of one block
// After every op the file records the area's blocks (non-air above the floor, anything but
// stone in the floor), the pending scheduled block and fluid ticks of the area (position, type,
// delay, priority), the brightness of every position above the floor that is not 15 (the light
// engine is not Kiln's to replay, the replay feeds it these numbers), and a draw of the level
// random (so every random call is accounted for).
//
// Scenario families are separate methods (scenariosFarming, scenariosGrowth, scenariosSpread,
// scenariosMisc) so that they can be extended independently.
//
// usage (cwd = a scratch server directory, e.g. work/wp44/server):
//   KILN_HARNESS_PORT=25581 java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/BlockTickVectors.java <out.jsonl> [name-filter-regex]
// (tools/block_vectors.py sets this up)

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReference;
import java.util.regex.Pattern;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.storage.ServerLevelData;

public class BlockTickVectors {
    static ServerLevel level;
    static MinecraftServer server;
    static final int Y0 = 100, HEIGHT = 12, FLOOR = Y0 - 1;
    // The window that is built, recorded and ticked: x and z from LO to HI (scenarios build in 0..15
    // and leave the margin for what spreads: water, fire, vines).
    static final int LO = -8, HI = 23;
    static final long START_TIME = 1000L;

    // ---------------------------------------------------------------- scenarios

    static class Sc {
        final String name;
        long seed;
        int difficulty = 2;
        final List<String> setup = new ArrayList<>();
        final List<Object[]> ops = new ArrayList<>();

        Sc(String name, long seed) {
            this.name = name;
            this.seed = seed;
        }

        /** A setup command, run at the area's corner (x0, 100, z0): `~` coordinates. */
        Sc cmd(String... commands) {
            for (String c : commands) setup.add(c);
            return this;
        }

        Sc difficulty(int d) {
            difficulty = d;
            return this;
        }

        Sc rt(int n) {
            for (int i = 0; i < n; i++) ops.add(new Object[] {"rt"});
            return this;
        }

        Sc tick(int n) {
            ops.add(new Object[] {"tick", n});
            return this;
        }

        /** `Level.setBlock(pos, state, 3)` at area-relative x, y (absolute), z. */
        Sc set(int x, int y, int z, String state) {
            ops.add(new Object[] {"set", x, y, z, state});
            return this;
        }

        /** Alternating rounds: `n` times (rt, then `ticks` level ticks). */
        Sc rtTick(int n, int ticks) {
            for (int i = 0; i < n; i++) {
                ops.add(new Object[] {"rt"});
                ops.add(new Object[] {"tick", ticks});
            }
            return this;
        }
    }

    static void scenarios(List<Sc> out) {
        scenariosHarness(out);
        scenariosFarming(out);
        scenariosGrowth(out);
        scenariosSpread(out);
        scenariosMisc(out);
    }

    // ================================================================ checks of the harness itself
    // Behaviour Kiln had before this harness: leaves that decay, water that spreads.
    static void scenariosHarness(List<Sc> out) {
        out.add(new Sc("harness_leaves", 2).cmd(
                "fill ~0 ~ ~0 ~8 ~ ~8 minecraft:oak_leaves[distance=7,persistent=false]").rt(12));
        out.add(new Sc("harness_water", 3).cmd(
                "fill ~0 ~ ~0 ~8 ~ ~8 minecraft:air",
                "setblock ~4 ~ ~4 minecraft:water").tick(30).set(4, 100, 4, "minecraft:air").tick(30));
    }

    // ================================================================ family: farming
    // farmland, crops, stems, nether wart, cocoa, sweet berries, sugar cane, cactus, bamboo
    static void scenariosFarming(List<Sc> out) {
        // A smoke scenario that proves the harness: wheat on farmland beside water.
        out.add(new Sc("harness_wheat", 1).cmd(
                "fill ~0 ~-1 ~0 ~8 ~-1 ~8 minecraft:farmland[moisture=7]",
                "fill ~0 ~ ~0 ~8 ~ ~8 minecraft:wheat[age=0]").rt(30));
    }

    // ================================================================ family: growth
    // kelp, vines, weeping and twisting vines, cave vines, chorus, mushrooms, propagules
    static void scenariosGrowth(List<Sc> out) {
    }

    // ================================================================ family: spread
    // grass, mycelium, nylium, saplings (trees), azalea, snow and ice melting
    static void scenariosSpread(List<Sc> out) {
    }

    // ================================================================ family: misc
    // copper oxidation, turtle eggs, redstone ore, budding amethyst, dripstone, lightning rods
    static void scenariosMisc(List<Sc> out) {
        miscCopper(out);
    }

    /** A setup command placing `block` at area-relative x, y (absolute), z. */
    static Sc miscPut(Sc sc, int x, int y, int z, String block) {
        return sc.cmd(String.format("setblock ~%d ~%d ~%d %s", x, y - Y0, z, block));
    }

    // Copper: every shape class of the weathering family, alone (5 apart: nothing in each other's
    // scan), in clusters of stages, and waxed ones among them.
    static void miscCopper(List<Sc> out) {
        String[][] isoA = {
            {"copper_block", "cut_copper", "chiseled_copper"},
            {"cut_copper_stairs[facing=east,half=top,shape=straight]", "cut_copper_slab[type=double]", "cut_copper_slab[type=top]"},
            {"exposed_copper", "weathered_cut_copper", "exposed_chiseled_copper"},
        };
        String[][] isoB = {
            {"copper_grate", "copper_grate", "copper_bulb[lit=true]"},
            {"copper_bulb[lit=false,powered=true]", "copper_trapdoor[open=true,facing=north,half=top]", "copper_trapdoor[open=false,facing=west,half=bottom]"},
            {"exposed_copper_bulb[lit=true,powered=true]", "weathered_copper_grate", "exposed_copper_trapdoor[open=true,half=top]"},
        };
        String[][] isoC = {
            {"copper_chain[axis=x]", "copper_bars", "copper_lantern[hanging=false]"},
            {"lightning_rod[facing=up]", "lightning_rod[facing=east,powered=true]", "exposed_lightning_rod[facing=down]"},
            {"copper_chest[facing=south]", "copper_golem_statue[facing=east,copper_golem_pose=sitting]", "weathered_copper_chain[axis=z]"},
        };
        String[][][] iso = {isoA, isoB, isoC};
        String[] isoNames = {"a", "b", "c"};
        for (int k = 0; k < 3; k++) {
            Sc sc = new Sc("misc_copper_iso_" + isoNames[k], 100 + k);
            for (int i = 0; i < 3; i++)
                for (int j = 0; j < 3; j++) miscPut(sc, 1 + i * 5, 100, 1 + j * 5, iso[k][i][j]);
            sc.rtTick(120, 2);
            out.add(sc);
        }
        // Doors (both halves), alone.
        {
            Sc sc = new Sc("misc_copper_doors", 110);
            String[] doors = {"copper_door", "exposed_copper_door", "weathered_copper_door"};
            for (int i = 0; i < 3; i++)
                for (int j = 0; j < 3; j++) {
                    String props = String.format("facing=%s,hinge=%s,open=%s,powered=false", new String[] {"north", "east", "south"}[i], j == 1 ? "right" : "left", j == 2 ? "true" : "false");
                    miscPut(sc, 1 + i * 5, 100, 1 + j * 5, "minecraft:" + doors[j] + "[half=lower," + props + "]");
                    miscPut(sc, 1 + i * 5, 101, 1 + j * 5, "minecraft:" + doors[j] + "[half=upper," + props + "]");
                }
            sc.rtTick(150, 2);
            out.add(sc);
        }
        // Chests: single, double (left and right halves), trapped-like facing variants.
        {
            Sc sc = new Sc("misc_copper_chests", 111);
            miscPut(sc, 1, 100, 1, "copper_chest[facing=north,type=single]");
            miscPut(sc, 6, 100, 1, "copper_chest[facing=north,type=left]");
            miscPut(sc, 7, 100, 1, "copper_chest[facing=north,type=right]");
            miscPut(sc, 1, 100, 8, "copper_chest[facing=east,type=left]");
            miscPut(sc, 1, 100, 9, "copper_chest[facing=east,type=right]");
            miscPut(sc, 12, 100, 1, "exposed_copper_chest[facing=south,type=single]");
            miscPut(sc, 12, 100, 8, "weathered_copper_chest[facing=west,type=single]");
            miscPut(sc, 6, 100, 12, "copper_chest[facing=south,type=left]");
            miscPut(sc, 5, 100, 12, "copper_chest[facing=south,type=right]");
            sc.rtTick(200, 2);
            out.add(sc);
        }
        // Clusters: a 6x6 slab of copper blocks (the chance is small while the neighbours are
        // alike), a 5x5x3 cube, layers of stages.
        {
            Sc sc = new Sc("misc_copper_slab_cluster", 120);
            sc.cmd("fill ~2 ~ ~2 ~7 ~ ~7 minecraft:copper_block");
            sc.rt(400);
            out.add(sc);
        }
        {
            Sc sc = new Sc("misc_copper_cube", 121);
            sc.cmd("fill ~2 ~ ~2 ~6 ~2 ~6 minecraft:cut_copper");
            sc.rt(500);
            out.add(sc);
        }
        {
            Sc sc = new Sc("misc_copper_stages", 122);
            sc.cmd("fill ~1 ~ ~1 ~9 ~ ~9 minecraft:copper_block",
                    "fill ~1 ~ ~1 ~5 ~ ~5 minecraft:weathered_copper",
                    "fill ~3 ~ ~3 ~4 ~ ~4 minecraft:oxidized_copper",
                    "fill ~7 ~ ~7 ~9 ~ ~9 minecraft:exposed_copper",
                    "setblock ~12 ~ ~2 minecraft:copper_block");
            sc.rt(400);
            out.add(sc);
        }
        // Waxed blocks never change and do not count as neighbours.
        {
            Sc sc = new Sc("misc_copper_waxed", 123);
            sc.cmd("fill ~1 ~ ~1 ~8 ~ ~8 minecraft:waxed_copper_block",
                    "fill ~4 ~ ~4 ~5 ~ ~5 minecraft:copper_block",
                    "fill ~11 ~ ~1 ~13 ~ ~3 minecraft:waxed_cut_copper_stairs[facing=north]",
                    "setblock ~12 ~ ~2 minecraft:cut_copper_stairs[facing=north]",
                    "setblock ~12 ~ ~8 minecraft:waxed_lightning_rod",
                    "setblock ~10 ~ ~8 minecraft:lightning_rod");
            sc.rt(400);
            out.add(sc);
        }
        // Stairs in shape-changing arrangements (the shape is recomputed when a neighbour
        // changes stage).
        {
            Sc sc = new Sc("misc_copper_stairs", 124);
            sc.cmd("fill ~1 ~ ~1 ~4 ~ ~1 minecraft:cut_copper_stairs[facing=east]",
                    "fill ~1 ~ ~2 ~4 ~ ~2 minecraft:cut_copper_stairs[facing=south]",
                    "fill ~7 ~ ~1 ~7 ~ ~4 minecraft:cut_copper_stairs[facing=west,half=top]",
                    "fill ~8 ~ ~1 ~8 ~ ~4 minecraft:cut_copper_slab[type=top]",
                    "fill ~1 ~ ~7 ~6 ~ ~7 minecraft:cut_copper_slab");
            sc.rtTick(200, 2);
            out.add(sc);
        }
        // Stairs in L shapes: pairs that age slowly and reshape their neighbours.
        {
            Sc sc = new Sc("misc_copper_stairs_pairs", 126);
            sc.cmd("setblock ~1 ~ ~1 minecraft:cut_copper_stairs[facing=east]",
                    "setblock ~2 ~ ~1 minecraft:cut_copper_stairs[facing=north]",
                    "setblock ~7 ~ ~1 minecraft:cut_copper_stairs[facing=west,half=top]",
                    "setblock ~7 ~ ~2 minecraft:cut_copper_stairs[facing=south,half=top]",
                    "setblock ~1 ~ ~8 minecraft:cut_copper_stairs[facing=south]",
                    "setblock ~2 ~ ~8 minecraft:cut_copper_stairs[facing=south]",
                    "setblock ~2 ~ ~9 minecraft:cut_copper_stairs[facing=east]",
                    "setblock ~12 ~ ~1 minecraft:cut_copper_stairs[facing=north]",
                    "setblock ~12 ~ ~12 minecraft:cut_copper_slab[type=double]",
                    "setblock ~13 ~ ~12 minecraft:cut_copper_slab[type=bottom]");
            sc.rtTick(500, 2);
            out.add(sc);
        }
        // Double chests of copper next to waxed ones: a half takes the stage of the half it joins.
        {
            Sc sc = new Sc("misc_copper_waxed_chests", 127);
            miscPut(sc, 1, 100, 1, "waxed_copper_chest[facing=south,type=left]");
            miscPut(sc, 0, 100, 1, "copper_chest[facing=south,type=right]");
            miscPut(sc, 7, 100, 1, "copper_chest[facing=south,type=left]");
            miscPut(sc, 6, 100, 1, "waxed_exposed_copper_chest[facing=south,type=right]");
            miscPut(sc, 1, 100, 8, "exposed_copper_chest[facing=north,type=right]");
            miscPut(sc, 2, 100, 8, "waxed_copper_chest[facing=north,type=left]");
            miscPut(sc, 8, 100, 8, "copper_chest[facing=east,type=right]");
            miscPut(sc, 8, 100, 9, "copper_chest[facing=east,type=left]");
            sc.rtTick(400, 2);
            out.add(sc);
        }
        // A powered, a lit and an open mixture of everything in one cluster.
        {
            Sc sc = new Sc("misc_copper_mixed", 125);
            sc.cmd("fill ~1 ~ ~1 ~3 ~ ~3 minecraft:copper_bulb[lit=true]",
                    "fill ~5 ~ ~1 ~7 ~ ~3 minecraft:copper_grate",
                    "fill ~1 ~ ~5 ~3 ~ ~5 minecraft:copper_trapdoor[open=true]",
                    "fill ~5 ~ ~5 ~5 ~3 ~5 minecraft:lightning_rod[facing=up]",
                    "fill ~7 ~ ~5 ~9 ~ ~5 minecraft:copper_chain[axis=x]",
                    "fill ~1 ~ ~8 ~1 ~ ~12 minecraft:copper_bars",
                    "setblock ~7 ~ ~8 minecraft:copper_golem_statue[facing=north]",
                    "setblock ~9 ~ ~8 minecraft:copper_lantern",
                    "setblock ~11 ~ ~8 minecraft:copper_chest[facing=north]");
            sc.rtTick(250, 2);
            out.add(sc);
        }
    }

    // ---------------------------------------------------------------- runner

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    static void run(String[] args) throws Exception {
        Path outPath = Path.of(args[0]).toAbsolutePath();
        Pattern filter = args.length > 1 ? Pattern.compile(args[1]) : null;
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "BlockTickVectors main");
        main.start();
        server = awaitServer();
        server.submit(() -> {
            level = server.overworld();
            level.tickRateManager().setFrozen(true);
            for (int cx = -2; cx <= 2; cx++)
                for (int cz = -2; cz <= 2; cz++) {
                    level.setChunkForced(cx, cz, true);
                    level.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(2000);
        List<Sc> all = new ArrayList<>();
        scenarios(all);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            try {
                command("gamerule random_tick_speed 0");
                command("gamerule advance_weather false");
                command("gamerule advance_time false");
                command("gamerule spawn_mobs false");
                command("gamerule block_drops false");
                command("time set noon");
                command("weather clear");
                for (Sc sc : all) {
                    int x0 = 0, z0 = 0;
                    if (filter != null && !filter.matcher(sc.name).find()) continue;
                    try {
                        lines.add(runScenario(sc, x0, z0));
                    } catch (Throwable t) {
                        t.printStackTrace();
                        lines.add("{\"name\":\"" + sc.name + "\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
                    }
                }
            } catch (Throwable t) {
                t.printStackTrace();
                lines.add("{\"name\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("BlockTickVectors: wrote " + lines.size() + " scenarios to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static String runScenario(Sc sc, int x0, int z0) throws Exception {
        command("difficulty " + new String[] {"peaceful", "easy", "normal", "hard"}[sc.difficulty]);
        command(String.format("fill %d %d %d %d %d %d minecraft:air", x0 + LO, FLOOR, z0 + LO, x0 + HI, Y0 + HEIGHT - 1, z0 + HI));
        command(String.format("fill %d %d %d %d %d %d minecraft:stone", x0 + LO, FLOOR, z0 + LO, x0 + HI, FLOOR, z0 + HI));
        // The clock is reset before the setup so that what the setup schedules (water, ...) is due
        // relative to START_TIME, not to wherever the previous scenario left the clock.
        setGameTime(START_TIME);
        for (String c : sc.setup) command(String.format("execute positioned %d %d %d run %s", x0, Y0, z0, c));
        // Whatever the setup scheduled runs out first, so every scenario starts without pending ticks
        // (the replay places the starting blocks without updates).
        awaitLight();
        for (int i = 0; i < 400 && pendingCount(x0, z0) > 0; i++) {
            tickLevel();
        }
        if (pendingCount(x0, z0) > 0) throw new IllegalStateException("setup of " + sc.name + " never settles");
        awaitLight();
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", sc.name);
        m.put("difficulty", sc.difficulty);
        m.put("seed", sc.seed);
        m.put("x0", x0);
        m.put("z0", z0);
        m.put("game_time", level.getGameTime());
        m.put("initial", snapshot(x0, z0));
        m.put("initial_light", light(x0, z0));
        level.getRandom().setSeed(sc.seed);
        List<Object> results = new ArrayList<>();
        List<Object> ops = new ArrayList<>();
        for (Object[] op : sc.ops) {
            switch ((String) op[0]) {
                case "rt" -> randomTickArea(x0, z0);
                case "tick" -> {
                    for (int i = 0; i < (Integer) op[1]; i++) tickLevel();
                }
                case "set" -> {
                    BlockPos p = new BlockPos(x0 + (Integer) op[1], (Integer) op[2], z0 + (Integer) op[3]);
                    var parsed = BlockStateParser.parseForBlock(level.registryAccess().lookupOrThrow(net.minecraft.core.registries.Registries.BLOCK), (String) op[4], false);
                    level.setBlock(p, parsed.blockState(), 3);
                }
                default -> throw new IllegalStateException((String) op[0]);
            }
            awaitLight();
            List<Object> opJson = new ArrayList<>();
            for (Object o : op) opJson.add(o);
            ops.add(opJson);
            long probe = level.getRandom().nextLong();
            results.add(List.of(snapshot(x0, z0), pending(x0, z0), light(x0, z0), probe));
        }
        m.put("ops", ops);
        m.put("results", results);
        return toJson(m);
    }

    static void randomTickArea(int x0, int z0) {
        List<BlockPos> ticking = new ArrayList<>();
        for (int y = FLOOR; y < Y0 + HEIGHT; y++)
            for (int z = z0 + LO; z <= z0 + HI; z++)
                for (int x = x0 + LO; x <= x0 + HI; x++) {
                    BlockPos p = new BlockPos(x, y, z);
                    BlockState s = level.getBlockState(p);
                    if (s.isRandomlyTicking() || s.getFluidState().isRandomlyTicking()) ticking.add(p);
                }
        for (BlockPos p : ticking) {
            // `ServerLevel.tickChunk`: the block's random tick, then its fluid's, each on the state
            // read before the block's tick.
            BlockState s = level.getBlockState(p);
            var fluid = s.getFluidState();
            if (s.isRandomlyTicking()) s.randomTick(level, p, level.getRandom());
            if (fluid.isRandomlyTicking()) fluid.randomTick(level, p, level.getRandom());
        }
    }

    static void tickLevel() throws Exception {
        var f = net.minecraft.world.TickRateManager.class.getDeclaredField("runGameElements");
        f.setAccessible(true);
        f.set(level.tickRateManager(), true);
        try {
            level.tick(() -> true);
        } finally {
            f.set(level.tickRateManager(), false);
        }
    }

    static void setGameTime(long t) {
        ((ServerLevelData) level.getLevelData()).setGameTime(t);
    }

    /// The light engine runs on its own thread: what blocks placed or removed do to the light has to
    /// be settled before it is read, or a vector would depend on how fast that thread ran.
    static void awaitLight() {
        var engine = level.getChunkSource().getLightEngine();
        for (int i = 0; i < 2; i++) {
            engine.tryScheduleUpdate();
            var done = engine.waitForPendingTasks(0, 0);
            long deadline = System.nanoTime() + 60_000_000_000L;
            level.getServer().managedBlock(() -> done.isDone() || System.nanoTime() > deadline);
            if (!done.isDone()) throw new IllegalStateException("light engine did not settle");
        }
    }

    // Blocks from the floor up: [dx, dy, dz, state], without air above the floor and stone in it.
    static List<Object> snapshot(int x0, int z0) {
        List<Object> out = new ArrayList<>();
        for (int y = FLOOR; y < Y0 + HEIGHT; y++)
            for (int z = z0 + LO; z <= z0 + HI; z++)
                for (int x = x0 + LO; x <= x0 + HI; x++) {
                    BlockState s = level.getBlockState(new BlockPos(x, y, z));
                    if (y == FLOOR ? s.is(Blocks.STONE) : s.isAir()) continue;
                    out.add(List.of(x - x0, y, z - z0, BlockStateParser.serialize(s)));
                }
        return out;
    }

    // [dx, dy, dz, brightness] of every position above the floor whose raw brightness is not 15.
    static List<Object> light(int x0, int z0) {
        List<Object> out = new ArrayList<>();
        for (int y = Y0; y < Y0 + HEIGHT; y++)
            for (int z = z0 + LO; z <= z0 + HI; z++)
                for (int x = x0 + LO; x <= x0 + HI; x++) {
                    int b = level.getRawBrightness(new BlockPos(x, y, z), 0);
                    if (b != 15) out.add(List.of(x - x0, y, z - z0, b));
                }
        return out;
    }

    // The area's pending scheduled ticks: ["b" or "f", dx, dy, dz, type, delay, priority], sorted.
    static List<Object> pending(int x0, int z0) {
        List<String> rows = new ArrayList<>();
        List<Object> out = new ArrayList<>();
        long now = level.getGameTime();
        for (int cx = (x0 + LO) >> 4; cx <= (x0 + HI) >> 4; cx++)
            for (int cz = (z0 + LO) >> 4; cz <= (z0 + HI) >> 4; cz++) {
                LevelChunk chunk = level.getChunk(cx, cz);
                var packed = chunk.getTicksForSerialization(now);
                for (var t : packed.blocks()) {
                    if (!inWindow(t.pos(), x0, z0)) continue;
                    out.add(tickRow("b", x0, z0, t.pos(), BuiltInRegistries.BLOCK.getKey(t.type()).toString(), t.delay(), t.priority().getValue()));
                }
                for (var t : packed.fluids()) {
                    if (!inWindow(t.pos(), x0, z0)) continue;
                    out.add(tickRow("f", x0, z0, t.pos(), BuiltInRegistries.FLUID.getKey(t.type()).toString(), t.delay(), t.priority().getValue()));
                }
            }
        out.sort(Comparator.comparing(Object::toString));
        return out;
    }

    static List<Object> tickRow(String kind, int x0, int z0, BlockPos p, String type, int delay, int priority) {
        return List.of(kind, p.getX() - x0, p.getY(), p.getZ() - z0, type, delay, priority);
    }

    static boolean inWindow(BlockPos p, int x0, int z0) {
        return p.getX() >= x0 + LO && p.getX() <= x0 + HI && p.getZ() >= z0 + LO && p.getZ() <= z0 + HI;
    }

    static int pendingCount(int x0, int z0) {
        return pending(x0, z0).size();
    }

    // ---------------------------------------------------------------- server

    static void command(String cmd) {
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), cmd);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + System.getenv().getOrDefault("KILN_HARNESS_PORT", "25581"),
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:plains\"}",
                "spawn-protection=0",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "enable-rcon=false",
                "enable-query=false",
                "spawn-monsters=false",
                "generate-structures=false",
                "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    static MinecraftServer awaitServer() throws Exception {
        for (int i = 0; i < 600; i++) {
            for (Thread t : Thread.getAllStackTraces().keySet()) {
                if (!t.getName().equals("Server thread")) continue;
                Field holderField = Thread.class.getDeclaredField("holder");
                holderField.setAccessible(true);
                Object holder = holderField.get(t);
                Field taskField = holder.getClass().getDeclaredField("task");
                taskField.setAccessible(true);
                Object task = taskField.get(holder);
                for (Field f : task.getClass().getDeclaredFields()) {
                    f.setAccessible(true);
                    if (f.get(task) instanceof AtomicReference<?> ref && ref.get() instanceof MinecraftServer s) {
                        while (!s.isReady()) Thread.sleep(100);
                        return s;
                    }
                }
            }
            Thread.sleep(100);
        }
        throw new IllegalStateException("server did not start");
    }

    static String toJson(Object o) {
        if (o == null) return "null";
        if (o instanceof String s) return "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
        if (o instanceof Boolean || o instanceof Integer || o instanceof Long) return o.toString();
        if (o instanceof List<?> l) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < l.size(); i++) b.append(i > 0 ? "," : "").append(toJson(l.get(i)));
            return b.append("]").toString();
        }
        if (o instanceof Map<?, ?> m) {
            StringBuilder b = new StringBuilder("{");
            int i = 0;
            for (var e : m.entrySet()) b.append(i++ > 0 ? "," : "").append(toJson(e.getKey().toString())).append(':').append(toJson(e.getValue()));
            return b.append("}").toString();
        }
        return toJson(o.toString());
    }
}
