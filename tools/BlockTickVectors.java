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
        miscSingles(out);
        miscDripstone(out);
        miscRedstone(out);
    }

    /** `fill` of the area-relative box x1..x2, y1..y2 (absolute), z1..z2. */
    static Sc miscFill(Sc sc, int x1, int y1, int z1, int x2, int y2, int z2, String block) {
        return sc.cmd(String.format("fill ~%d ~%d ~%d ~%d ~%d ~%d %s", x1, y1 - Y0, z1, x2, y2 - Y0, z2, block));
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

    // Turtle eggs, redstone ore, budding amethyst, dried ghasts.
    static void miscSingles(List<Sc> out) {
        // Turtle eggs at dawn (the hatch chance is 1 from tick 21062 of the day to 21905): every
        // random tick moves an egg a stage, then the third hatches it. Sand, red sand and stone below.
        {
            Sc sc = new Sc("misc_turtle_dawn", 200).cmd("time set 21500");
            for (int i = 0; i < 4; i++)
                for (int j = 0; j < 3; j++) {
                    miscPut(sc, 1 + 2 * i, 100, 1 + 2 * j, "sand");
                    miscPut(sc, 1 + 2 * i, 101, 1 + 2 * j, "turtle_egg[eggs=" + (i + 1) + ",hatch=" + j + "]");
                }
            miscPut(sc, 10, 100, 1, "red_sand");
            miscPut(sc, 10, 101, 1, "turtle_egg[eggs=4,hatch=2]");
            miscPut(sc, 12, 100, 3, "suspicious_sand");
            miscPut(sc, 12, 101, 3, "turtle_egg[eggs=2,hatch=1]");
            miscPut(sc, 14, 101, 1, "turtle_egg[eggs=3,hatch=0]");
            miscPut(sc, 14, 100, 5, "dirt");
            miscPut(sc, 14, 101, 5, "turtle_egg[eggs=1,hatch=2]");
            sc.rt(6).tick(1);
            out.add(sc);
        }
        // The same by day: 1 in 500 per random tick.
        {
            Sc sc = new Sc("misc_turtle_day", 201);
            for (int i = 0; i < 4; i++)
                for (int j = 0; j < 3; j++) {
                    miscPut(sc, 1 + 2 * i, 100, 1 + 2 * j, "sand");
                    miscPut(sc, 1 + 2 * i, 101, 1 + 2 * j, "turtle_egg[eggs=" + (i + 1) + ",hatch=" + (j % 3) + "]");
                }
            sc.rt(1500);
            out.add(sc);
        }
        // The edges of the dawn window.
        int[] edge = {21061, 21062, 21904, 21905, 0, 23999};
        for (int k = 0; k < edge.length; k++) {
            Sc sc = new Sc("misc_turtle_edge_" + edge[k], 210 + k).cmd("time set " + edge[k]);
            for (int i = 0; i < 8; i++) {
                miscPut(sc, 1 + 2 * i, 100, 1, "sand");
                miscPut(sc, 1 + 2 * i, 101, 1, "turtle_egg[eggs=" + (1 + i % 4) + ",hatch=" + (i % 3) + "]");
            }
            sc.rt(4);
            out.add(sc);
        }
        // Baby turtles hatch and the world goes on ticking.
        {
            Sc sc = new Sc("misc_turtle_hatch_tick", 220).cmd("time set 21500");
            for (int i = 0; i < 3; i++) {
                miscPut(sc, 2 + 3 * i, 100, 2, "sand");
                miscPut(sc, 2 + 3 * i, 101, 2, "turtle_egg[eggs=" + (i + 2) + ",hatch=2]");
            }
            sc.rt(1).tick(40).rt(1).tick(10);
            out.add(sc);
        }
        // Redstone ore: lit ore goes out on a random tick; unlit does not tick at all.
        {
            Sc sc = new Sc("misc_redstone_ore", 230);
            miscFill(sc, 1, 100, 1, 5, 100, 5, "redstone_ore[lit=true]");
            miscFill(sc, 1, 101, 1, 3, 101, 3, "deepslate_redstone_ore[lit=true]");
            miscFill(sc, 8, 100, 1, 11, 100, 4, "redstone_ore");
            miscPut(sc, 8, 101, 1, "redstone_ore[lit=true]");
            sc.rt(2).set(9, 100, 2, "minecraft:redstone_ore[lit=true]").set(10, 100, 3, "minecraft:deepslate_redstone_ore[lit=true]").rt(3).rt(10);
            out.add(sc);
        }
        // Budding amethyst in a water-filled stone box (all six sides grow in water) and in the air.
        {
            Sc sc = new Sc("misc_budding_water", 240);
            miscFill(sc, 1, 100, 1, 7, 106, 7, "stone");
            miscFill(sc, 2, 101, 2, 6, 105, 6, "water");
            miscPut(sc, 4, 103, 4, "budding_amethyst");
            miscPut(sc, 4, 104, 4, "small_amethyst_bud[facing=up,waterlogged=true]");
            miscPut(sc, 4, 102, 4, "medium_amethyst_bud[facing=down,waterlogged=true]");
            miscPut(sc, 5, 103, 4, "large_amethyst_bud[facing=east,waterlogged=true]");
            miscPut(sc, 3, 103, 4, "small_amethyst_bud[facing=east,waterlogged=true]");
            sc.rt(500);
            out.add(sc);
        }
        {
            Sc sc = new Sc("misc_budding_air", 241);
            miscFill(sc, 1, 100, 1, 9, 100, 9, "stone");
            miscPut(sc, 3, 101, 3, "budding_amethyst");
            miscPut(sc, 3, 102, 3, "small_amethyst_bud[facing=up]");
            miscPut(sc, 4, 101, 3, "medium_amethyst_bud[facing=east]");
            miscPut(sc, 3, 101, 4, "large_amethyst_bud[facing=south]");
            miscPut(sc, 2, 101, 3, "amethyst_cluster[facing=west]");
            miscPut(sc, 3, 101, 2, "small_amethyst_bud[facing=north]");
            miscPut(sc, 7, 101, 7, "budding_amethyst");
            miscPut(sc, 7, 102, 7, "budding_amethyst");
            miscPut(sc, 8, 102, 7, "medium_amethyst_bud[facing=south]");
            miscPut(sc, 7, 103, 7, "stone");
            sc.rt(500);
            out.add(sc);
        }
        {
            Sc sc = new Sc("misc_budding_cluster", 242);
            miscFill(sc, 2, 100, 2, 7, 102, 7, "budding_amethyst");
            miscFill(sc, 3, 101, 3, 6, 101, 6, "air");
            sc.rt(400);
            out.add(sc);
        }
        // Dried ghasts: waterlogged ones soak up (and the last hatches), dry ones dry out. The
        // block ticks 5000 ticks after a random tick found it wet.
        {
            Sc sc = new Sc("misc_dried_ghast", 250);
            for (int k = 0; k < 8; k++) {
                int x = 2 + 3 * (k % 4), z = 2 + 5 * (k / 4);
                miscFill(sc, x - 1, 100, z - 1, x + 1, 102, z + 1, "stone");
                boolean wet = k < 4;
                // waterlogged ones start at 0 or 1 and never reach the hatching (a baby ghast spawning
                // draws from the level random in vanilla; it is not a mob Kiln has)
                int hydration = wet ? k % 2 : 3 - k % 4;
                sc.cmd(String.format("setblock ~%d ~%d ~%d minecraft:dried_ghast[facing=%s,hydration=%d,waterlogged=%s]", x, 1, z, new String[] {"north", "east", "south", "west"}[k % 4], hydration, wet));
            }
            sc.rtTick(2, 5000);
            out.add(sc);
        }
        // A ghast that reaches full hydration hatches at the last tick of the run.
        {
            Sc sc = new Sc("misc_dried_ghast_hatch", 251);
            miscFill(sc, 1, 100, 1, 3, 102, 3, "stone");
            sc.cmd("setblock ~2 ~1 ~2 minecraft:dried_ghast[facing=west,hydration=3,waterlogged=true]");
            sc.rt(1).tick(5000);
            out.add(sc);
        }
    }

    // Pointed dripstone and sulfur spikes: growth, falling, dripping into cauldrons, mud to clay.
    static void miscDripstone(List<Sc> out) {
        // A dripstone ceiling under a pool: stalactites and stalagmites grow, merge.
        {
            Sc sc = new Sc("misc_dripstone_grow", 300);
            miscFill(sc, 1, 107, 1, 9, 107, 9, "dripstone_block");
            miscFill(sc, 0, 108, 0, 10, 108, 10, "stone");
            miscFill(sc, 1, 108, 1, 9, 108, 9, "water");
            for (int i = 0; i < 4; i++)
                for (int j = 0; j < 4; j++)
                    miscPut(sc, 2 + 2 * i, 106, 2 + 2 * j, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            miscPut(sc, 2, 100, 2, "pointed_dripstone[vertical_direction=up,thickness=tip]");
            miscPut(sc, 4, 100, 4, "pointed_dripstone[vertical_direction=up,thickness=tip]");
            miscPut(sc, 4, 101, 4, "pointed_dripstone[vertical_direction=up,thickness=frustum]");
            miscPut(sc, 6, 100, 8, "pointed_dripstone[vertical_direction=up,thickness=tip]");
            miscPut(sc, 8, 105, 8, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            miscPut(sc, 8, 106, 8, "pointed_dripstone[vertical_direction=down,thickness=frustum]");
            sc.rt(900);
            out.add(sc);
        }
        // Long columns: the growth length limit (7), merging into columns, waterlogged stalactites.
        {
            Sc sc = new Sc("misc_dripstone_columns", 301);
            miscFill(sc, 1, 107, 1, 8, 107, 8, "dripstone_block");
            miscFill(sc, 0, 108, 0, 9, 108, 9, "stone");
            miscFill(sc, 1, 108, 1, 8, 108, 8, "water");
            miscPut(sc, 2, 106, 2, "pointed_dripstone[vertical_direction=down,thickness=base]");
            miscPut(sc, 2, 105, 2, "pointed_dripstone[vertical_direction=down,thickness=middle]");
            miscPut(sc, 2, 104, 2, "pointed_dripstone[vertical_direction=down,thickness=frustum]");
            miscPut(sc, 2, 103, 2, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            miscPut(sc, 5, 106, 2, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            miscPut(sc, 5, 100, 2, "pointed_dripstone[vertical_direction=up,thickness=tip]");
            miscPut(sc, 7, 106, 5, "pointed_dripstone[vertical_direction=down,thickness=tip,waterlogged=true]");
            miscPut(sc, 2, 100, 6, "pointed_dripstone[vertical_direction=up,thickness=base]");
            miscPut(sc, 2, 101, 6, "pointed_dripstone[vertical_direction=up,thickness=middle]");
            miscPut(sc, 2, 102, 6, "pointed_dripstone[vertical_direction=up,thickness=tip]");
            miscPut(sc, 2, 106, 6, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            sc.rt(900);
            out.add(sc);
        }
        // Stalagmite search: water below stops it, blocks that cannot be dripped through stop it,
        // glass panes and slabs and carpets in the way.
        {
            Sc sc = new Sc("misc_dripstone_scan", 302);
            miscFill(sc, 1, 107, 1, 9, 107, 9, "dripstone_block");
            miscFill(sc, 0, 108, 0, 10, 108, 10, "stone");
            miscFill(sc, 1, 108, 1, 9, 108, 9, "water");
            for (int i = 0; i < 4; i++) miscPut(sc, 2 + 2 * i, 106, 2, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            for (int i = 0; i < 4; i++) miscPut(sc, 2 + 2 * i, 106, 6, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            miscPut(sc, 2, 103, 2, "oak_slab");
            miscPut(sc, 4, 100, 2, "stone");
            miscPut(sc, 4, 101, 2, "white_carpet");
            miscPut(sc, 6, 103, 2, "glass_pane");
            miscPut(sc, 8, 103, 2, "oak_fence");
            miscPut(sc, 2, 103, 6, "oak_trapdoor[open=true]");
            miscPut(sc, 4, 103, 6, "stone");
            // water in a closed stone cell (open water would spread over the floor)
            miscFill(sc, 5, 100, 5, 7, 102, 7, "stone");
            miscFill(sc, 6, 100, 6, 6, 101, 6, "water");
            miscPut(sc, 8, 103, 6, "cobweb");
            sc.rt(900);
            out.add(sc);
        }
        // Mud over the root turns to clay (water drips through it); a cauldron below a dripping
        // tip is filled 50 ticks plus the distance later.
        {
            Sc sc = new Sc("misc_dripstone_mud", 303);
            miscFill(sc, 1, 107, 1, 9, 107, 9, "dripstone_block");
            miscFill(sc, 1, 108, 1, 3, 108, 9, "mud");
            miscPut(sc, 2, 106, 2, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            miscPut(sc, 2, 106, 5, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            miscPut(sc, 7, 106, 2, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            sc.rt(30);
            out.add(sc);
        }
        {
            Sc sc = new Sc("misc_dripstone_cauldron", 304);
            // water pool over x 1..5, lava pool over x 8..12
            miscFill(sc, 1, 107, 1, 5, 107, 9, "dripstone_block");
            miscFill(sc, 0, 108, 0, 6, 108, 10, "stone");
            miscFill(sc, 1, 108, 1, 5, 108, 9, "water");
            miscFill(sc, 8, 107, 1, 12, 107, 9, "dripstone_block");
            miscFill(sc, 7, 108, 0, 13, 108, 10, "stone");
            miscFill(sc, 8, 108, 1, 12, 108, 9, "lava");
            for (int j = 0; j < 4; j++) {
                int z = 2 + 2 * j;
                miscPut(sc, 2, 106, z, "pointed_dripstone[vertical_direction=down,thickness=tip]");
                miscPut(sc, 4, 106, z, "pointed_dripstone[vertical_direction=down,thickness=tip]");
                miscPut(sc, 9, 106, z, "pointed_dripstone[vertical_direction=down,thickness=tip]");
                miscPut(sc, 11, 106, z, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            }
            miscPut(sc, 2, 100, 2, "cauldron");
            miscPut(sc, 2, 100, 4, "water_cauldron[level=1]");
            miscPut(sc, 2, 100, 6, "powder_snow_cauldron[level=1]");
            miscPut(sc, 2, 100, 8, "lava_cauldron");
            miscPut(sc, 4, 100, 2, "water_cauldron[level=3]");
            miscPut(sc, 4, 100, 4, "cauldron");
            miscPut(sc, 4, 101, 4, "white_carpet");
            miscPut(sc, 4, 100, 6, "cauldron");
            miscPut(sc, 4, 103, 6, "oak_trapdoor[open=true]");
            miscPut(sc, 9, 100, 2, "cauldron");
            miscPut(sc, 9, 100, 4, "water_cauldron[level=2]");
            miscPut(sc, 9, 100, 6, "lava_cauldron");
            miscPut(sc, 11, 100, 2, "cauldron");
            miscPut(sc, 11, 100, 4, "cauldron");
            sc.rtTick(60, 5);
            out.add(sc);
        }
        // Sulfur spikes grow on sulfur (no water needed) up to length 2.
        {
            Sc sc = new Sc("misc_sulfur_grow", 310);
            miscFill(sc, 1, 106, 1, 9, 106, 9, "sulfur");
            for (int i = 0; i < 4; i++)
                for (int j = 0; j < 3; j++)
                    miscPut(sc, 2 + 2 * i, 105, 2 + 3 * j, "sulfur_spike[vertical_direction=down,thickness=tip]");
            miscPut(sc, 2, 100, 2, "sulfur_spike[vertical_direction=up,thickness=tip]");
            miscPut(sc, 4, 100, 5, "sulfur_spike[vertical_direction=up,thickness=tip]");
            miscPut(sc, 6, 104, 8, "sulfur_spike[vertical_direction=down,thickness=frustum]");
            miscPut(sc, 6, 103, 8, "sulfur_spike[vertical_direction=down,thickness=tip]");
            sc.rt(900);
            out.add(sc);
        }
        // Falling: the ceiling goes and the stalactite column comes down with it (only a few ticks
        // pass: the falling entities are still on their way when the vector ends).
        {
            Sc sc = new Sc("misc_dripstone_fall", 320);
            miscPut(sc, 4, 105, 4, "dripstone_block");
            miscPut(sc, 4, 104, 4, "pointed_dripstone[vertical_direction=down,thickness=base]");
            miscPut(sc, 4, 103, 4, "pointed_dripstone[vertical_direction=down,thickness=middle]");
            miscPut(sc, 4, 102, 4, "pointed_dripstone[vertical_direction=down,thickness=tip]");
            miscPut(sc, 4, 100, 4, "pointed_dripstone[vertical_direction=up,thickness=tip]");
            miscPut(sc, 8, 106, 8, "dripstone_block");
            miscPut(sc, 8, 105, 8, "pointed_dripstone[vertical_direction=down,thickness=tip_merge]");
            miscPut(sc, 8, 104, 8, "pointed_dripstone[vertical_direction=up,thickness=tip_merge]");
            miscPut(sc, 8, 103, 8, "pointed_dripstone[vertical_direction=up,thickness=base]");
            sc.set(4, 105, 4, "minecraft:air").tick(3);
            out.add(sc);
        }
        {
            Sc sc = new Sc("misc_dripstone_fall_merged", 321);
            miscPut(sc, 8, 106, 8, "dripstone_block");
            miscPut(sc, 8, 105, 8, "pointed_dripstone[vertical_direction=down,thickness=frustum]");
            miscPut(sc, 8, 104, 8, "pointed_dripstone[vertical_direction=down,thickness=tip_merge]");
            miscPut(sc, 8, 103, 8, "pointed_dripstone[vertical_direction=up,thickness=tip_merge]");
            miscPut(sc, 8, 102, 8, "pointed_dripstone[vertical_direction=up,thickness=frustum]");
            miscPut(sc, 8, 101, 8, "pointed_dripstone[vertical_direction=up,thickness=base]");
            miscPut(sc, 8, 100, 8, "stone");
            sc.set(8, 106, 8, "minecraft:air").tick(3);
            out.add(sc);
        }
        {
            Sc sc = new Sc("misc_dripstone_stalagmite_break", 322);
            miscPut(sc, 4, 100, 4, "pointed_dripstone[vertical_direction=up,thickness=tip]");
            miscPut(sc, 4, 99, 4, "stone");
            miscPut(sc, 8, 100, 8, "pointed_dripstone[vertical_direction=up,thickness=base]");
            miscPut(sc, 8, 101, 8, "pointed_dripstone[vertical_direction=up,thickness=middle]");
            miscPut(sc, 8, 102, 8, "pointed_dripstone[vertical_direction=up,thickness=tip]");
            sc.set(4, 99, 4, "minecraft:air").tick(3).set(8, 99, 8, "minecraft:air").tick(3);
            out.add(sc);
        }
    }

    // Tripwires and hooks, target blocks, big dripleaves, copper bulbs.
    static void miscRedstone(List<Sc> out) {
        // A string between two hooks. A removed string counts as pressed (the hooks power for a
        // moment), a disarmed one detaches both hooks (the detach sound draws from the level random).
        {
            Sc sc = new Sc("misc_tripwire_line", 500);
            miscPut(sc, 1, 100, 5, "stone");
            miscPut(sc, 10, 100, 5, "stone");
            miscPut(sc, 1, 101, 5, "redstone_lamp");
            miscPut(sc, 10, 101, 5, "redstone_lamp");
            miscPut(sc, 2, 100, 5, "tripwire_hook[facing=east]");
            for (int x = 3; x <= 8; x++) miscPut(sc, x, 100, 5, "tripwire");
            miscPut(sc, 9, 100, 5, "tripwire_hook[facing=west]");
            sc.tick(12).set(5, 100, 5, "minecraft:air").tick(12).set(5, 100, 5, "minecraft:tripwire").tick(12)
                    .set(4, 100, 5, "minecraft:air").set(4, 100, 5, "minecraft:tripwire[powered=true]").tick(12)
                    .set(6, 100, 5, "minecraft:air").set(6, 100, 5, "minecraft:tripwire[disarmed=true]").tick(12)
                    .set(6, 100, 5, "minecraft:air").set(6, 100, 5, "minecraft:tripwire").tick(12)
                    .set(1, 100, 5, "minecraft:air").tick(12);
            out.add(sc);
        }
        // The same along z, with the far hook placed last.
        {
            Sc sc = new Sc("misc_tripwire_z", 501);
            miscPut(sc, 4, 100, 1, "stone");
            miscPut(sc, 4, 100, 11, "stone");
            miscPut(sc, 4, 100, 2, "tripwire_hook[facing=south]");
            for (int z = 3; z <= 9; z++) miscPut(sc, 4, 100, z, "tripwire");
            miscPut(sc, 4, 100, 10, "tripwire_hook[facing=north]");
            miscPut(sc, 5, 100, 1, "redstone_wire");
            sc.set(4, 100, 6, "minecraft:air").tick(12).set(4, 100, 6, "minecraft:tripwire").tick(12)
                    .set(4, 100, 10, "minecraft:air").tick(12).set(4, 100, 10, "minecraft:tripwire_hook[facing=north]").tick(12)
                    .set(4, 100, 11, "minecraft:air").tick(15);
            out.add(sc);
        }
        // Short strings: one wire between hooks, hooks side by side, a string with only one hook,
        // a hook with no wall.
        {
            Sc sc = new Sc("misc_tripwire_short", 502);
            miscPut(sc, 1, 100, 2, "stone");
            miscPut(sc, 5, 100, 2, "stone");
            miscPut(sc, 2, 100, 2, "tripwire_hook[facing=east]");
            miscPut(sc, 3, 100, 2, "tripwire");
            miscPut(sc, 4, 100, 2, "tripwire_hook[facing=west]");
            miscPut(sc, 1, 100, 6, "stone");
            miscPut(sc, 4, 100, 6, "stone");
            miscPut(sc, 2, 100, 6, "tripwire_hook[facing=east]");
            miscPut(sc, 3, 100, 6, "tripwire_hook[facing=west]");
            miscPut(sc, 1, 100, 9, "stone");
            miscPut(sc, 2, 100, 9, "tripwire_hook[facing=east]");
            for (int x = 3; x <= 6; x++) miscPut(sc, x, 100, 9, "tripwire");
            miscPut(sc, 12, 100, 4, "tripwire_hook[facing=east]");
            sc.set(3, 100, 2, "minecraft:air").tick(12).set(3, 100, 2, "minecraft:tripwire").tick(12)
                    .set(4, 100, 9, "minecraft:air").tick(12)
                    .set(7, 100, 9, "minecraft:tripwire_hook[facing=west]").tick(12)
                    .set(8, 100, 9, "minecraft:stone").set(7, 100, 9, "minecraft:air").tick(12)
                    .set(7, 100, 9, "minecraft:tripwire_hook[facing=west]").tick(12);
            out.add(sc);
        }
        // Two strings crossing in one block.
        {
            Sc sc = new Sc("misc_tripwire_cross", 503);
            miscPut(sc, 0, 100, 6, "stone");
            miscPut(sc, 12, 100, 6, "stone");
            miscPut(sc, 6, 100, 0, "stone");
            miscPut(sc, 6, 100, 12, "stone");
            miscPut(sc, 1, 100, 6, "tripwire_hook[facing=east]");
            miscPut(sc, 11, 100, 6, "tripwire_hook[facing=west]");
            miscPut(sc, 6, 100, 1, "tripwire_hook[facing=south]");
            miscPut(sc, 6, 100, 11, "tripwire_hook[facing=north]");
            for (int i = 2; i <= 10; i++) {
                miscPut(sc, i, 100, 6, "tripwire");
                if (i != 6) miscPut(sc, 6, 100, i, "tripwire");
            }
            sc.set(6, 100, 6, "minecraft:air").tick(12).set(6, 100, 6, "minecraft:tripwire").tick(12)
                    .set(3, 100, 6, "minecraft:air").tick(12).set(3, 100, 6, "minecraft:tripwire[disarmed=true]").tick(12)
                    .set(6, 100, 3, "minecraft:air").tick(12);
            out.add(sc);
        }
        // Target blocks: a new powered one resets at once, a replaced one keeps its power.
        {
            Sc sc = new Sc("misc_target", 510);
            miscPut(sc, 2, 100, 2, "target[power=7]");
            miscPut(sc, 4, 100, 2, "target");
            miscPut(sc, 3, 100, 2, "redstone_lamp");
            miscPut(sc, 5, 100, 2, "redstone_lamp");
            miscPut(sc, 2, 100, 4, "redstone_wire");
            miscPut(sc, 2, 100, 3, "redstone_lamp");
            sc.set(2, 100, 2, "minecraft:target[power=9]").tick(8).set(4, 100, 2, "minecraft:target[power=15]").tick(8)
                    .set(2, 100, 2, "minecraft:air").set(2, 100, 2, "minecraft:target[power=12]").tick(8)
                    .set(4, 100, 2, "minecraft:target[power=0]").tick(8);
            out.add(sc);
        }
        // Big dripleaf: a leaf on a stem and on dirt; a redstone signal levels a tilted leaf; a leaf
        // on top of a leaf makes it a stem; a leaf loses its support.
        {
            Sc sc = new Sc("misc_big_dripleaf", 520);
            miscPut(sc, 2, 100, 2, "dirt");
            miscPut(sc, 2, 101, 2, "big_dripleaf[facing=east,tilt=none]");
            miscPut(sc, 5, 100, 2, "clay");
            miscPut(sc, 5, 101, 2, "big_dripleaf_stem[facing=north]");
            miscPut(sc, 5, 102, 2, "big_dripleaf[facing=north,tilt=none]");
            miscPut(sc, 8, 100, 2, "dirt");
            miscPut(sc, 8, 101, 2, "big_dripleaf[facing=south,tilt=full]");
            miscPut(sc, 11, 100, 2, "dirt");
            miscPut(sc, 11, 101, 2, "big_dripleaf[facing=west,tilt=partial]");
            miscPut(sc, 2, 100, 6, "dirt");
            miscPut(sc, 2, 101, 6, "big_dripleaf[facing=east,tilt=unstable]");
            miscPut(sc, 5, 100, 6, "mud");
            miscPut(sc, 5, 101, 6, "big_dripleaf[facing=east]");
            sc.set(9, 101, 2, "minecraft:redstone_block").set(12, 101, 2, "minecraft:lever[face=floor,powered=true]")
                    .set(2, 101, 7, "minecraft:redstone_block").tick(5)
                    .set(2, 102, 2, "minecraft:big_dripleaf[facing=east]").set(5, 100, 2, "minecraft:air")
                    .set(5, 100, 6, "minecraft:air").set(2, 101, 7, "minecraft:air").tick(5);
            out.add(sc);
        }
        // Copper bulbs: a rising signal flips the light, a falling one only clears `powered`.
        {
            Sc sc = new Sc("misc_copper_bulb", 530);
            String[] bulbs = {"copper_bulb", "exposed_copper_bulb", "weathered_copper_bulb", "oxidized_copper_bulb", "waxed_copper_bulb", "waxed_oxidized_copper_bulb"};
            for (int i = 0; i < bulbs.length; i++) {
                miscPut(sc, 1 + 2 * i, 100, 2, bulbs[i]);
                miscPut(sc, 1 + 2 * i, 100, 3, i % 2 == 0 ? "air" : "lever[face=floor]");
            }
            miscPut(sc, 1, 100, 6, "copper_bulb[lit=true]");
            miscPut(sc, 3, 100, 6, "copper_bulb[lit=true,powered=true]");
            miscPut(sc, 5, 100, 6, "exposed_copper_bulb[powered=true]");
            for (int i = 0; i < bulbs.length; i++) sc.set(1 + 2 * i, 101, 2, "minecraft:redstone_block");
            sc.tick(2);
            for (int i = 0; i < bulbs.length; i++) sc.set(1 + 2 * i, 101, 2, "minecraft:air");
            sc.tick(2);
            for (int i = 0; i < bulbs.length; i++) sc.set(1 + 2 * i, 101, 2, "minecraft:redstone_block");
            sc.tick(2);
            sc.set(0, 100, 6, "minecraft:redstone_block").set(2, 100, 6, "minecraft:redstone_block").set(6, 100, 6, "minecraft:redstone_block").tick(2)
                    .set(0, 100, 6, "minecraft:air").set(2, 100, 6, "minecraft:air").set(6, 100, 6, "minecraft:air").tick(2);
            // bulbs placed new beside a signal flip when placed
            miscPut(sc, 9, 100, 9, "redstone_block");
            sc.set(10, 100, 9, "minecraft:copper_bulb").set(9, 101, 9, "minecraft:copper_bulb[lit=true]").tick(2);
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
        // What the previous scenario spawned (falling blocks, hatched turtles) does not carry over,
        // and every scenario starts at noon (a setup may `time set` something else).
        command("kill @e[type=!minecraft:player]");
        command("time set noon");
        // Scheduled ticks the previous scenario left in the area (due at times of its clock) go too.
        setGameTime(START_TIME);
        var area = new net.minecraft.world.level.levelgen.structure.BoundingBox(x0 + LO, FLOOR, z0 + LO, x0 + HI, Y0 + HEIGHT - 1, z0 + HI);
        level.getBlockTicks().clearArea(area);
        level.getFluidTicks().clearArea(area);
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
        if (pendingCount(x0, z0) > 0) throw new IllegalStateException("setup of " + sc.name + " never settles: " + pending(x0, z0));
        awaitLight();
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", sc.name);
        m.put("difficulty", sc.difficulty);
        m.put("seed", sc.seed);
        m.put("x0", x0);
        m.put("z0", z0);
        m.put("game_time", level.getGameTime());
        m.put("day_time", level.getOverworldClockTime());
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
