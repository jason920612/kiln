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
//   bonemeal x y z   BoneMealItem.growCrop (a bone meal used on the block, with the level random)
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
    /// The height of the scenario being recorded (`Sc.height`) and the biome the window has now.
    static int curHeight = HEIGHT;
    static String curBiome = "minecraft:plains";

    // ---------------------------------------------------------------- scenarios

    static class Sc {
        final String name;
        long seed;
        int difficulty = 2;
        /// Height of the recorded window above the floor (trees need more than HEIGHT).
        int height = HEIGHT;
        /// The biome of the whole window (`fillbiome`); the replay reads it for bone meal on grass.
        String biome = "minecraft:plains";
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

        Sc tall() {
            height = 48;
            return this;
        }

        Sc biome(String b) {
            biome = b;
            return this;
        }

        /// `BoneMealItem.growCrop` on the block at area-relative x, y (absolute), z.
        Sc bonemeal(int x, int y, int z) {
            ops.add(new Object[] {"bonemeal", x, y, z});
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
        scenariosTrees(out);
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

    // ================================================================ family: trees
    // saplings and propagules growing into trees (random ticks, bone meal): the vanilla tree
    // features placed through TreeGrower, 2x2 groups, flowers nearby (bee nests), roofs, grounds;
    // bone meal on saplings, azaleas, huge mushrooms and grass (the biome's flowers).
    static int treeSeed = 4400;

    static final String[] SAPLINGS = {"oak", "birch", "spruce", "jungle", "acacia", "dark_oak", "cherry", "pale_oak", "poplar"};

    /// A sapling (or any block) at area-relative x, z on `ground` (the block below, y 99).
    static Sc plant(Sc sc, String block, int x, int z, String ground) {
        return sc.cmd("setblock ~" + x + " ~-1 ~" + z + " " + ground, "setblock ~" + x + " ~ ~" + z + " " + block);
    }

    static Sc newTree(String name) {
        return new Sc(name, treeSeed++).tall();
    }

    static void scenariosTrees(List<Sc> out) {
        // One sapling of every kind, many rounds: the stage first, then the tree.
        for (String t : SAPLINGS) {
            out.add(plant(newTree("trees_alone_" + t), "minecraft:" + t + "_sapling", 8, 8, "minecraft:dirt").rtTick(60, 1));
        }
        // 2x2 groups of every kind (mega trees for spruce, jungle, dark oak and pale oak), saplings
        // of different stages.
        for (String t : SAPLINGS) {
            Sc sc = newTree("trees_group_" + t);
            for (int dx = 0; dx < 2; dx++)
                for (int dz = 0; dz < 2; dz++) plant(sc, "minecraft:" + t + "_sapling[stage=" + ((dx + dz) % 2) + "]", 7 + dx, 7 + dz, "minecraft:dirt");
            out.add(sc.rtTick(50, 1));
        }
        // 3 x 3 saplings that crowd each other, and spaced ones.
        for (String t : new String[] {"oak", "spruce", "dark_oak"}) {
            Sc sc = newTree("trees_crowd_" + t);
            for (int dx = 0; dx < 3; dx++)
                for (int dz = 0; dz < 3; dz++) plant(sc, "minecraft:" + t + "_sapling", 6 + dx, 6 + dz, "minecraft:dirt");
            out.add(sc.rtTick(70, 1));
        }
        {
            Sc sc = newTree("trees_spaced_oak");
            for (int dx = 0; dx < 3; dx++)
                for (int dz = 0; dz < 3; dz++) plant(sc, "minecraft:oak_sapling", 3 + dx * 5, 3 + dz * 5, "minecraft:dirt");
            out.add(sc.rtTick(70, 1));
        }
        // A mixed 2x2 (another kind in it), and an L of three.
        {
            Sc sc = newTree("trees_mixed_group");
            plant(sc, "minecraft:spruce_sapling", 7, 7, "minecraft:dirt");
            plant(sc, "minecraft:spruce_sapling", 8, 7, "minecraft:dirt");
            plant(sc, "minecraft:spruce_sapling", 7, 8, "minecraft:dirt");
            plant(sc, "minecraft:oak_sapling", 8, 8, "minecraft:dirt");
            out.add(sc.rtTick(60, 1));
        }
        {
            Sc sc = newTree("trees_ell_dark_oak");
            plant(sc, "minecraft:dark_oak_sapling", 7, 7, "minecraft:dirt");
            plant(sc, "minecraft:dark_oak_sapling", 8, 7, "minecraft:dirt");
            plant(sc, "minecraft:dark_oak_sapling", 7, 8, "minecraft:dirt");
            out.add(sc.rtTick(40, 1));
        }
        // Flowers within the bees' range (x and z 2, y -1..+1 of the sapling): variants with bee nests.
        String[] flowers = {"minecraft:dandelion", "minecraft:poppy", "minecraft:cornflower"};
        int[][] near = {{2, 0, 0}, {-2, 0, 2}, {0, 1, -2}, {1, 1, 1}};
        for (String t : new String[] {"oak", "birch", "cherry", "spruce"}) {
            for (int i = 0; i < near.length; i++) {
                Sc sc = newTree("trees_flower_" + t + "_" + i);
                plant(sc, "minecraft:" + t + "_sapling", 8, 8, "minecraft:dirt");
                int[] o = near[i];
                if (o[1] == 1) sc.cmd("setblock ~" + (8 + o[0]) + " ~ ~" + (8 + o[2]) + " minecraft:dirt");
                sc.cmd("setblock ~" + (8 + o[0]) + " ~" + o[1] + " ~" + (8 + o[2]) + " " + flowers[i % flowers.length]);
                out.add(sc.rtTick(50, 1));
            }
        }
        // Flowers just out of range (3 away).
        for (int[] o : new int[][] {{3, 0, 0}, {0, 0, -3}}) {
            Sc sc = newTree("trees_noflower_" + o[0] + "_" + o[2]);
            plant(sc, "minecraft:oak_sapling", 8, 8, "minecraft:dirt");
            sc.cmd("setblock ~" + (8 + o[0]) + " ~ ~" + (8 + o[2]) + " minecraft:dandelion");
            out.add(sc.rtTick(50, 1));
        }
        // Roofs of glass (light passes) at heights where trees are clipped, fit or fail.
        for (String t : new String[] {"oak", "birch", "spruce", "acacia", "jungle", "cherry"}) {
            for (int k : new int[] {2, 4, 5, 6, 8}) {
                Sc sc = newTree("trees_roof_" + t + "_" + k);
                plant(sc, "minecraft:" + t + "_sapling", 8, 8, "minecraft:dirt");
                sc.cmd("fill ~5 ~" + k + " ~5 ~11 ~" + k + " ~11 minecraft:glass");
                out.add(sc.rtTick(60, 1));
            }
        }
        // A 2x2 under roofs, and a sapling shaded by leaves (too dark to grow).
        for (String t : new String[] {"dark_oak", "spruce", "jungle"}) {
            for (int k : new int[] {5, 9, 14}) {
                Sc sc = newTree("trees_roof_group_" + t + "_" + k);
                for (int dx = 0; dx < 2; dx++)
                    for (int dz = 0; dz < 2; dz++) plant(sc, "minecraft:" + t + "_sapling", 7 + dx, 7 + dz, "minecraft:dirt");
                sc.cmd("fill ~3 ~" + k + " ~3 ~12 ~" + k + " ~12 minecraft:glass");
                out.add(sc.rtTick(60, 1));
            }
        }
        {
            Sc sc = newTree("trees_shaded_oak");
            plant(sc, "minecraft:oak_sapling", 8, 8, "minecraft:dirt");
            sc.cmd("fill ~6 ~1 ~6 ~10 ~1 ~10 minecraft:oak_leaves[persistent=true]");
            out.add(sc.rtTick(40, 1));
        }
        // Grounds: podzol, coarse dirt, rooted dirt, mud, moss (grass blocks tick randomly and belong
        // to the spread family), and obstacles beside the trunk.
        for (String g : new String[] {"minecraft:podzol", "minecraft:coarse_dirt", "minecraft:rooted_dirt", "minecraft:mud", "minecraft:moss_block"}) {
            for (String t : new String[] {"oak", "spruce", "dark_oak"}) {
                Sc sc = newTree("trees_ground_" + g.substring(10) + "_" + t);
                int n = t.equals("dark_oak") ? 2 : 1;
                for (int dx = 0; dx < n; dx++)
                    for (int dz = 0; dz < n; dz++) plant(sc, "minecraft:" + t + "_sapling", 7 + dx, 7 + dz, g);
                out.add(sc.rtTick(50, 1));
            }
        }
        {
            Sc sc = newTree("trees_obstacles_oak");
            plant(sc, "minecraft:oak_sapling", 8, 8, "minecraft:dirt");
            sc.cmd("setblock ~9 ~2 ~8 minecraft:stone", "setblock ~8 ~1 ~10 minecraft:oak_leaves[persistent=true]", "setblock ~6 ~3 ~8 minecraft:cobblestone",
                    "setblock ~8 ~3 ~7 minecraft:vine[south=true]", "fill ~4 ~5 ~4 ~12 ~5 ~12 minecraft:oak_leaves[persistent=true]");
            out.add(sc.rtTick(60, 1));
        }
        {
            Sc sc = newTree("trees_water_oak");
            plant(sc, "minecraft:oak_sapling", 8, 8, "minecraft:dirt");
            sc.cmd("fill ~7 ~ ~7 ~9 ~2 ~9 minecraft:water", "setblock ~8 ~ ~8 minecraft:oak_sapling");
            out.add(sc.rtTick(30, 1));
        }
        // Mangrove propagules: planted, waterlogged, hanging, in a group.
        {
            Sc sc = newTree("trees_propagule_dry");
            plant(sc, "minecraft:mangrove_propagule[hanging=false,age=0,stage=0,waterlogged=false]", 8, 8, "minecraft:mud");
            out.add(sc.rtTick(60, 1));
        }
        {
            Sc sc = newTree("trees_propagule_water");
            sc.cmd("fill ~3 ~-1 ~3 ~13 ~-1 ~13 minecraft:mud", "fill ~3 ~ ~3 ~13 ~1 ~13 minecraft:water",
                    "setblock ~8 ~ ~8 minecraft:mangrove_propagule[hanging=false,age=0,stage=0,waterlogged=true]");
            out.add(sc.rtTick(60, 1));
        }
        {
            Sc sc = newTree("trees_propagule_hanging");
            sc.cmd("fill ~3 ~5 ~3 ~12 ~5 ~12 minecraft:mangrove_leaves[persistent=true]");
            for (int x = 4; x < 12; x += 2)
                for (int z = 4; z < 12; z += 2) sc.cmd("setblock ~" + x + " ~4 ~" + z + " minecraft:mangrove_propagule[hanging=true,age=" + ((x + z) % 4) + ",waterlogged=false]");
            out.add(sc.rtTick(30, 1));
        }
        {
            Sc sc = newTree("trees_propagule_group");
            for (int dx = 0; dx < 2; dx++)
                for (int dz = 0; dz < 2; dz++) plant(sc, "minecraft:mangrove_propagule[hanging=false,age=0,stage=0,waterlogged=false]", 7 + dx, 7 + dz, "minecraft:dirt");
            out.add(sc.rtTick(50, 1));
        }

        // ---------------------------------------------------------------- bone meal
        // Every sapling kind: many applications on each of two saplings and a 2x2 group.
        for (String t : SAPLINGS) {
            Sc sc = newTree("trees_bonemeal_" + t);
            plant(sc, "minecraft:" + t + "_sapling", 4, 4, "minecraft:dirt");
            plant(sc, "minecraft:" + t + "_sapling", 12, 4, "minecraft:dirt");
            for (int dx = 0; dx < 2; dx++)
                for (int dz = 0; dz < 2; dz++) plant(sc, "minecraft:" + t + "_sapling", 7 + dx, 11 + dz, "minecraft:dirt");
            for (int i = 0; i < 24; i++) {
                sc.bonemeal(4, 100, 4).bonemeal(12, 100, 4).bonemeal(7 + i % 2, 100, 11 + i / 2 % 2);
                if (i % 6 == 5) sc.tick(1);
            }
            out.add(sc);
        }
        {
            Sc sc = newTree("trees_bonemeal_mixed_rt");
            plant(sc, "minecraft:oak_sapling", 4, 4, "minecraft:dirt");
            plant(sc, "minecraft:birch_sapling", 10, 4, "minecraft:dirt");
            plant(sc, "minecraft:spruce_sapling", 4, 10, "minecraft:dirt");
            plant(sc, "minecraft:jungle_sapling", 10, 10, "minecraft:dirt");
            for (int i = 0; i < 20; i++) {
                sc.rt(1).bonemeal(4, 100, 4).bonemeal(10, 100, 4).bonemeal(4, 100, 10).bonemeal(10, 100, 10).tick(1);
            }
            out.add(sc);
        }
        {
            Sc sc = newTree("trees_bonemeal_propagule");
            plant(sc, "minecraft:mangrove_propagule[hanging=false,age=0,stage=0,waterlogged=false]", 5, 5, "minecraft:mud");
            sc.cmd("fill ~3 ~5 ~10 ~12 ~5 ~13 minecraft:mangrove_leaves[persistent=true]");
            for (int x = 4; x < 12; x += 3) sc.cmd("setblock ~" + x + " ~4 ~11 minecraft:mangrove_propagule[hanging=true,age=0,waterlogged=false]");
            for (int i = 0; i < 12; i++) {
                sc.bonemeal(5, 100, 5);
                for (int x = 4; x < 12; x += 3) sc.bonemeal(x, 104, 11);
            }
            out.add(sc);
        }
        for (String a : new String[] {"azalea", "flowering_azalea"}) {
            Sc sc = newTree("trees_bonemeal_" + a);
            plant(sc, "minecraft:" + a, 5, 5, "minecraft:dirt");
            plant(sc, "minecraft:" + a, 11, 5, "minecraft:moss_block");
            plant(sc, "minecraft:" + a, 8, 11, "minecraft:rooted_dirt");
            for (int i = 0; i < 20; i++) sc.bonemeal(5, 100, 5).bonemeal(11, 100, 5).bonemeal(8, 100, 11);
            out.add(sc);
        }
        // Huge mushrooms: brown and red on mycelium and podzol (the right ground), on dirt (fails)
        // and under a ceiling (fails).
        for (String m : new String[] {"brown_mushroom", "red_mushroom"}) {
            Sc sc = newTree("trees_bonemeal_" + m);
            plant(sc, "minecraft:" + m, 4, 4, "minecraft:mycelium");
            plant(sc, "minecraft:" + m, 12, 4, "minecraft:podzol");
            plant(sc, "minecraft:" + m, 4, 12, "minecraft:dirt");
            plant(sc, "minecraft:" + m, 12, 12, "minecraft:mycelium");
            sc.cmd("fill ~9 ~5 ~9 ~15 ~5 ~15 minecraft:glass");
            for (int i = 0; i < 16; i++) sc.bonemeal(4, 100, 4).bonemeal(12, 100, 4).bonemeal(4, 100, 12).bonemeal(12, 100, 12);
            out.add(sc);
        }
        // Grass: the biome's flowers (one walk in eight places one) and short grass, tall grass; in
        // biomes with and without flowers. Grass blocks do not random-tick here (bone meal only).
        for (String b : new String[] {"plains", "flower_forest", "meadow", "swamp", "cherry_grove", "pale_garden", "forest", "taiga", "sunflower_plains", "jungle"}) {
            Sc sc = newTree("trees_bonemeal_grass_" + b).biome("minecraft:" + b);
            sc.cmd("fill ~0 ~-1 ~0 ~15 ~-1 ~15 minecraft:grass_block", "setblock ~3 ~ ~3 minecraft:short_grass", "setblock ~9 ~ ~7 minecraft:short_grass",
                    "setblock ~9 ~ ~8 minecraft:fern", "setblock ~5 ~ ~10 minecraft:short_grass");
            for (int i = 0; i < 6; i++) sc.bonemeal(4 + i * 2, 99, 4 + i).bonemeal(8, 99, 8).bonemeal(3, 99, 12);
            out.add(sc);
        }
        {
            // Beside obstacles: walls, water, dirt strips (the walks stop where the ground is not grass).
            Sc sc = newTree("trees_bonemeal_grass_obstacles");
            sc.cmd("fill ~0 ~-1 ~0 ~15 ~-1 ~15 minecraft:grass_block", "fill ~6 ~-1 ~0 ~7 ~-1 ~15 minecraft:dirt", "fill ~10 ~ ~0 ~10 ~2 ~15 minecraft:stone",
                    "fill ~12 ~ ~4 ~13 ~ ~6 minecraft:water");
            for (int i = 0; i < 5; i++) sc.bonemeal(2, 99, 2 + i * 3).bonemeal(8 + i, 99, 8).bonemeal(14, 99, 14);
            out.add(sc);
        }
    }

    // ================================================================ family: misc
    // copper oxidation, turtle eggs, redstone ore, budding amethyst, dripstone, lightning rods
    static void scenariosMisc(List<Sc> out) {
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
        // Two fills (a fill is limited to 32768 blocks): the window up to the tallest scenario's top.
        command(String.format("fill %d %d %d %d %d %d minecraft:air", x0 + LO, FLOOR, z0 + LO, x0 + HI, Y0 + 23, z0 + HI));
        command(String.format("fill %d %d %d %d %d %d minecraft:air", x0 + LO, Y0 + 24, z0 + LO, x0 + HI, Y0 + 47, z0 + HI));
        // Ticks the previous scenario left (at times its clock reached, which this one starts below)
        // would otherwise run out much later than the setup does.
        var area = new net.minecraft.world.level.levelgen.structure.BoundingBox(x0 + LO, FLOOR, z0 + LO, x0 + HI, Y0 + 47, z0 + HI);
        level.getBlockTicks().clearArea(area);
        level.getFluidTicks().clearArea(area);
        curHeight = sc.height;
        if (!sc.biome.equals(curBiome)) {
            command(String.format("fillbiome %d %d %d %d %d %d %s", x0 + LO - 4, FLOOR - 3, z0 + LO - 4, x0 + HI + 4, FLOOR + 24, z0 + HI + 4, sc.biome));
            curBiome = sc.biome;
        }
        command(String.format("fill %d %d %d %d %d %d minecraft:stone", x0 + LO, FLOOR, z0 + LO, x0 + HI, FLOOR, z0 + HI));
        for (String c : sc.setup) command(String.format("execute positioned %d %d %d run %s", x0, Y0, z0, c));
        setGameTime(START_TIME);
        // Whatever the setup scheduled runs out first, so every scenario starts without pending ticks
        // (the replay places the starting blocks without updates).
        awaitLight();
        for (int i = 0; i < 400 && pendingCount(x0, z0) > 0; i++) {
            tickLevel();
        }
        if (pendingCount(x0, z0) > 0) {
            List<Object> pp = pending(x0, z0);
            List<?> row = (List<?>) pp.get(0);
            BlockPos q = new BlockPos(x0 + (Integer) row.get(1), (Integer) row.get(2), z0 + (Integer) row.get(3));
            throw new IllegalStateException("setup of " + sc.name + " never settles: " + pp.size() + " pending, first " + row + " block there " + level.getBlockState(q));
        }
        awaitLight();
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", sc.name);
        m.put("difficulty", sc.difficulty);
        m.put("seed", sc.seed);
        m.put("x0", x0);
        m.put("z0", z0);
        m.put("height", sc.height);
        m.put("biome", sc.biome);
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
                case "bonemeal" -> {
                    BlockPos p = new BlockPos(x0 + (Integer) op[1], (Integer) op[2], z0 + (Integer) op[3]);
                    net.minecraft.world.item.BoneMealItem.growCrop(new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.BONE_MEAL), level, p);
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
        for (int y = FLOOR; y < Y0 + curHeight; y++)
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
        for (int y = FLOOR; y < Y0 + curHeight; y++)
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
        for (int y = Y0; y < Y0 + curHeight; y++)
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
                "server-port=" + harnessPort(),
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

    /// $KILN_HARNESS_PORT, else the first free port of 25581-25583 (wp44's; waits while all are busy).
    static String harnessPort() {
        String env = System.getenv("KILN_HARNESS_PORT");
        if (env != null) return env;
        for (int i = 0; i < 900; i++) {
            for (int p = 25581; p <= 25583; p++) {
                try (var s = new java.net.ServerSocket(p)) {
                    return Integer.toString(p);
                } catch (java.io.IOException e) {
                    // busy
                }
            }
            try {
                Thread.sleep(2000);
            } catch (InterruptedException e) {
                throw new IllegalStateException(e);
            }
        }
        throw new IllegalStateException("no free harness port");
    }
}
