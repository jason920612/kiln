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
        // ---- kelp: 3x3 stone tanks (open on top) with a column of water of every depth
        int[] depths = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 3, 5, 7, 4, 6, 2};
        String[] bases = {"minecraft:sand", "minecraft:stone", "minecraft:gravel", "minecraft:dirt"};
        for (int sc = 0; sc < 3; sc++) {
            Sc s = new Sc("growth_kelp_" + sc, 300 + sc);
            for (int i = 0; i < 16; i++) {
                int x = 4 * (i % 4), z = 4 * (i / 4), depth = depths[(i + sc * 5) % 16];
                tank(s, x, z, depth, bases[(i + sc) % 4]);
                String head = "minecraft:kelp[age=" + new int[] {0, 5, 12, 20, 24, 25, 0, 3}[(i + 3 * sc) % 8] + "]";
                int body = (i + sc) % 3; // 0: a lone head at the bottom, 1 and 2: that many plants below the head
                if (body > 0 && depth > body) {
                    s.cmd(String.format("fill ~%d ~1 ~%d ~%d ~%d ~%d minecraft:kelp_plant", x + 1, z + 1, x + 1, body, z + 1));
                }
                s.cmd(String.format("setblock ~%d ~%d ~%d %s", x + 1, body + 1, z + 1, head));
            }
            out.add(s.rtTick(110, 2));
        }
        // kelp loses its ground or its water while it grows (the breaks come last: drops roll loot)
        out.add(new Sc("growth_kelp_changes", 310)
                .cmd("fill ~0 ~ ~0 ~15 ~ ~15 minecraft:sand")
                .cmd("fill ~0 ~1 ~0 ~15 ~8 ~15 minecraft:water")
                .cmd("fill ~1 ~1 ~1 ~14 ~1 ~14 minecraft:kelp_plant")
                .cmd("fill ~1 ~2 ~1 ~14 ~2 ~14 minecraft:kelp[age=0]")
                .rtTick(70, 3)
                .set(4, 100, 4, "minecraft:air").set(6, 103, 6, "minecraft:stone").set(8, 101, 8, "minecraft:water").tick(40));

        // ---- weeping, twisting and cave vines
        String[] ceilings = {"minecraft:stone", "minecraft:nether_wart_block", "minecraft:oak_leaves[persistent=true]", "minecraft:glass"};
        for (int sc = 0; sc < 3; sc++) {
            Sc s = new Sc("growth_weeping_" + sc, 320 + sc);
            s.cmd("fill ~0 ~9 ~0 ~15 ~9 ~15 " + ceilings[sc]);
            for (int i = 0; i < 16; i++) {
                int x = 1 + 4 * (i % 4) - (i % 2), z = 1 + 4 * (i / 4) - (i / 8);
                int floorY = new int[] {0, 6, 4, 2, 7, 0, 3, 5}[(i + sc) % 8]; // 0: open down to the floor
                if (floorY > 0) s.cmd(String.format("setblock ~%d ~%d ~%d minecraft:stone", x, floorY - 1, z));
                int ageIdx = (i + sc) % 6;
                int age = new int[] {0, 8, 20, 24, 25, 1}[ageIdx];
                int bodyLen = i % 3;
                if (bodyLen > 0) s.cmd(String.format("fill ~%d ~8 ~%d ~%d ~%d ~%d minecraft:weeping_vines_plant", x, z, x, 8 - bodyLen + 1, z));
                s.cmd(String.format("setblock ~%d ~%d ~%d minecraft:weeping_vines[age=%d]", x, 8 - bodyLen, z, age));
            }
            out.add(s.rtTick(130, 2));
        }
        for (int sc = 0; sc < 3; sc++) {
            Sc s = new Sc("growth_twisting_" + sc, 330 + sc);
            s.cmd("fill ~0 ~ ~0 ~15 ~ ~15 " + new String[] {"minecraft:stone", "minecraft:dirt", "minecraft:netherrack"}[sc]);
            for (int i = 0; i < 16; i++) {
                int x = 1 + 4 * (i % 4) - (i % 2), z = 1 + 4 * (i / 4) - (i / 8);
                int ceilY = new int[] {9, 3, 5, 7, 4, 10, 2, 6}[(i + sc) % 8];
                s.cmd(String.format("setblock ~%d ~%d ~%d minecraft:stone", x, ceilY, z));
                int age = new int[] {0, 8, 20, 24, 25, 1}[(i + 2 * sc) % 6];
                int bodyLen = i % 3;
                if (bodyLen > 0 && bodyLen < ceilY - 1) s.cmd(String.format("fill ~%d ~1 ~%d ~%d ~%d ~%d minecraft:twisting_vines_plant", x, z, x, bodyLen, z));
                s.cmd(String.format("setblock ~%d ~%d ~%d minecraft:twisting_vines[age=%d]", x, bodyLen < ceilY - 1 ? bodyLen + 1 : 1, z, age));
            }
            out.add(s.rtTick(130, 2));
        }
        for (int sc = 0; sc < 3; sc++) {
            Sc s = new Sc("growth_cave_vines_" + sc, 340 + sc);
            s.cmd("fill ~0 ~9 ~0 ~15 ~9 ~15 " + new String[] {"minecraft:stone", "minecraft:moss_block", "minecraft:dirt"}[sc]);
            for (int i = 0; i < 16; i++) {
                int x = 1 + 4 * (i % 4) - (i % 2), z = 1 + 4 * (i / 4) - (i / 8);
                int floorY = new int[] {0, 6, 4, 2, 7, 0, 3, 5}[(i + sc) % 8];
                if (floorY > 0) s.cmd(String.format("setblock ~%d ~%d ~%d minecraft:stone", x, floorY - 1, z));
                int age = new int[] {0, 8, 20, 24, 25, 1}[(i + sc) % 6];
                int bodyLen = i % 3;
                if (bodyLen > 0) s.cmd(String.format("fill ~%d ~8 ~%d ~%d ~%d ~%d minecraft:cave_vines_plant[berries=%s]", x, z, x, 8 - bodyLen + 1, z, i % 2 == 0 ? "true" : "false"));
                s.cmd(String.format("setblock ~%d ~%d ~%d minecraft:cave_vines[age=%d,berries=%s]", x, 8 - bodyLen, z, age, i % 4 == 1 ? "true" : "false"));
            }
            out.add(s.rtTick(130, 2));
        }
        // vine columns that are blocked at first and freed while they grow (the cuts come last: drops roll loot)
        out.add(new Sc("growth_vines_changes", 350).cmd(
                "fill ~0 ~9 ~0 ~15 ~9 ~15 minecraft:stone",
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:stone",
                "setblock ~3 ~8 ~3 minecraft:weeping_vines[age=0]", "setblock ~3 ~5 ~3 minecraft:stone",
                "setblock ~8 ~8 ~3 minecraft:cave_vines[age=0]", "setblock ~8 ~4 ~3 minecraft:glass",
                "setblock ~3 ~1 ~8 minecraft:twisting_vines[age=0]", "setblock ~3 ~3 ~8 minecraft:stone",
                "setblock ~12 ~1 ~12 minecraft:twisting_vines[age=3]", "setblock ~12 ~8 ~12 minecraft:weeping_vines[age=2]")
                .rtTick(50, 2).set(3, 105, 3, "minecraft:air").set(8, 104, 3, "minecraft:air").set(3, 103, 8, "minecraft:air")
                .rtTick(80, 2)
                .set(12, 106, 12, "minecraft:air").set(3, 102, 8, "minecraft:water").tick(60));

        // ---- vines spread over walls, ceilings and leaves
        out.add(new Sc("growth_vine_wall_0", 360).cmd(
                "fill ~0 ~ ~0 ~15 ~10 ~0 minecraft:stone",
                "setblock ~4 ~5 ~1 minecraft:vine[north=true]", "setblock ~10 ~7 ~1 minecraft:vine[north=true]",
                "setblock ~13 ~3 ~1 minecraft:vine[north=true]").rt(120));
        out.add(new Sc("growth_vine_wall_1", 361).cmd(
                "fill ~0 ~ ~0 ~15 ~10 ~0 minecraft:mossy_cobblestone",
                "fill ~0 ~ ~0 ~0 ~10 ~15 minecraft:stone",
                "fill ~0 ~10 ~0 ~15 ~10 ~15 minecraft:oak_planks",
                "setblock ~1 ~6 ~1 minecraft:vine[north=true,west=true]", "setblock ~7 ~9 ~1 minecraft:vine[north=true,up=true]",
                "setblock ~1 ~4 ~9 minecraft:vine[west=true]").rt(120));
        out.add(new Sc("growth_vine_wall_2", 362).cmd(
                "fill ~0 ~ ~0 ~15 ~10 ~0 minecraft:stone",
                "fill ~0 ~ ~15 ~15 ~10 ~15 minecraft:glass",
                "fill ~0 ~ ~0 ~0 ~10 ~15 minecraft:oak_leaves[persistent=true]",
                "fill ~15 ~ ~0 ~15 ~10 ~15 minecraft:stone_slab[type=double]",
                "setblock ~1 ~5 ~1 minecraft:vine[west=true]", "setblock ~14 ~5 ~1 minecraft:vine[east=true]",
                "setblock ~8 ~5 ~1 minecraft:vine[north=true]", "setblock ~8 ~5 ~14 minecraft:vine[south=true]").rt(120));
        // trees of leaves with vines on the sides and hanging down
        out.add(new Sc("growth_vine_leaves", 363).cmd(
                "fill ~2 ~6 ~2 ~13 ~9 ~13 minecraft:oak_leaves[persistent=true]",
                "fill ~4 ~ ~4 ~5 ~5 ~5 minecraft:oak_log",
                "setblock ~1 ~7 ~5 minecraft:vine[east=true]", "setblock ~14 ~8 ~9 minecraft:vine[west=true]",
                "setblock ~8 ~5 ~8 minecraft:vine[up=true]", "setblock ~6 ~6 ~1 minecraft:vine[south=true]").rt(120)
                .set(8, 105, 8, "minecraft:air").set(5, 107, 5, "minecraft:vine").rt(60));

        // ---- mushrooms in the dark
        String[] floors = {"minecraft:dirt", "minecraft:mycelium", "minecraft:podzol", "minecraft:stone", "minecraft:oak_planks", "minecraft:grass_block"};
        for (int sc = 0; sc < 5; sc++) {
            Sc s = new Sc("growth_mushroom_" + sc, 370 + sc);
            s.cmd("fill ~0 ~ ~0 ~15 ~ ~15 " + floors[sc % 3]);
            if (sc == 3) {
                s.cmd("fill ~0 ~ ~0 ~7 ~ ~15 minecraft:mycelium", "fill ~8 ~ ~0 ~15 ~ ~7 minecraft:podzol", "fill ~8 ~ ~8 ~15 ~ ~15 minecraft:stone");
            }
            if (sc != 2) s.cmd("fill ~0 ~3 ~0 ~15 ~3 ~15 minecraft:stone"); // a roof: dark
            else s.cmd("fill ~0 ~3 ~0 ~15 ~3 ~15 minecraft:glass", "fill ~4 ~3 ~4 ~11 ~3 ~11 minecraft:stone");
            int seeds = new int[] {3, 8, 5, 6, 7}[sc];
            for (int i = 0; i < seeds; i++) {
                int x = 2 + (i * 5) % 12, z = 2 + (i * 7) % 12;
                s.cmd(String.format("setblock ~%d ~1 ~%d minecraft:%s_mushroom", x, z, i % 3 == 2 ? "red" : "brown"));
            }
            if (sc == 1) for (int x = 6; x <= 9; x++) for (int z = 6; z <= 8; z++) s.cmd(String.format("setblock ~%d ~1 ~%d minecraft:brown_mushroom", x, z));
            out.add(s.rt(250));
        }

        // ---- nylium under covers
        for (int sc = 0; sc < 3; sc++) {
            Sc s = new Sc("growth_nylium_" + sc, 380 + sc);
            s.cmd("fill ~0 ~ ~0 ~15 ~ ~15 minecraft:netherrack");
            String[] nylium = {"minecraft:crimson_nylium", "minecraft:warped_nylium"};
            String[] ncovers = {"minecraft:stone", "minecraft:air", "minecraft:glass", "minecraft:oak_slab[type=bottom]", "minecraft:oak_slab[type=top]",
                    "minecraft:oak_leaves[persistent=true]", "minecraft:water", "minecraft:snow[layers=1]", "minecraft:snow[layers=3]", "minecraft:oak_stairs[half=bottom]",
                    "minecraft:oak_stairs[half=top]", "minecraft:crimson_roots", "minecraft:ice", "minecraft:oak_trapdoor[half=bottom,open=false]", "minecraft:tinted_glass",
                    "minecraft:cobweb", "minecraft:white_carpet", "minecraft:iron_bars", "minecraft:honey_block", "minecraft:chest", "minecraft:lantern", "minecraft:hopper"};
            for (int i = 0; i < 16; i++) {
                int x = 1 + 4 * (i % 4) - (i % 2), z = 1 + 4 * (i / 4) - (i / 8);
                s.cmd(String.format("setblock ~%d ~ ~%d %s", x, z, nylium[(i + sc) % 2]));
                s.cmd(String.format("setblock ~%d ~1 ~%d %s", x, z, ncovers[(i + 5 * sc) % ncovers.length]));
            }
            out.add(s.rt(30).set(2, 101, 2, "minecraft:stone").set(6, 101, 6, "minecraft:air").set(10, 101, 10, "minecraft:stone").rt(30));
        }

        // nylium that is covered one block at a time while it is ticked
        out.add(new Sc("growth_nylium_dyn", 385).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:netherrack", "fill ~0 ~ ~0 ~7 ~ ~15 minecraft:crimson_nylium", "fill ~8 ~ ~0 ~15 ~ ~15 minecraft:warped_nylium")
                .rt(5).set(2, 101, 2, "minecraft:stone").rt(5).set(10, 101, 3, "minecraft:glass").set(4, 101, 8, "minecraft:oak_slab[type=bottom]").rt(5)
                .set(12, 101, 12, "minecraft:snow[layers=2]").set(6, 101, 13, "minecraft:water").rt(5).set(2, 101, 2, "minecraft:air").set(1, 101, 14, "minecraft:dirt")
                .rt(5).set(9, 101, 9, "minecraft:ice").set(14, 101, 1, "minecraft:stone").rt(5).set(5, 101, 5, "minecraft:oak_stairs[half=top]").rt(15));

        // ---- chorus flowers on end stone
        for (int sc = 0; sc < 4; sc++) {
            Sc s = new Sc("growth_chorus_" + sc, 390 + sc);
            s.cmd("fill ~0 ~ ~0 ~15 ~ ~15 minecraft:end_stone");
            if (sc == 1) s.cmd("fill ~0 ~3 ~0 ~15 ~3 ~15 minecraft:stone_bricks"); // a low roof stops the trees
            if (sc == 2) {
                s.cmd("fill ~6 ~1 ~0 ~6 ~4 ~15 minecraft:purpur_block", "fill ~0 ~1 ~11 ~15 ~3 ~11 minecraft:purpur_block");
            }
            if (sc == 3) {
                s.cmd("setblock ~3 ~1 ~3 minecraft:chorus_plant", "setblock ~3 ~2 ~3 minecraft:chorus_plant", "setblock ~3 ~3 ~3 minecraft:chorus_flower[age=1]",
                        "setblock ~10 ~1 ~10 minecraft:chorus_plant", "setblock ~10 ~2 ~10 minecraft:chorus_flower[age=0]");
            }
            int n = new int[] {4, 6, 5, 3}[sc];
            for (int i = 0; i < n; i++) {
                int x = 1 + (i * 5) % 14, z = 1 + (i * 3 + sc) % 14;
                if (sc == 2 && (x == 6 || z == 11)) continue;
                s.cmd(String.format("setblock ~%d ~1 ~%d minecraft:chorus_flower[age=%d]", x, z, i % 3));
            }
            out.add(s.rtTick(130, 3));
        }
        // chorus trees that lose their ground (breaks last)
        out.add(new Sc("growth_chorus_changes", 394).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:end_stone",
                "setblock ~3 ~1 ~3 minecraft:chorus_flower[age=0]", "setblock ~10 ~1 ~5 minecraft:chorus_flower[age=1]", "setblock ~6 ~1 ~12 minecraft:chorus_flower[age=0]")
                .rtTick(40, 3).set(3, 99, 3, "minecraft:air").set(3, 100, 3, "minecraft:stone").set(10, 100, 5, "minecraft:air").rtTick(20, 3).tick(30));
        scenariosWet(out);
    }

    /// Corals that die out of water, scaffolding that settles or falls, sponges that soak up water.
    static void scenariosWet(List<Sc> out) {
        String[] kinds = {"tube", "brain", "bubble", "fire", "horn"};
        // ---- corals placed dry, beside water, and left dry
        for (int sc = 0; sc < 3; sc++) {
            Sc s = new Sc("wet_coral_" + sc, 400 + sc).cmd("fill ~0 ~ ~0 ~15 ~ ~15 minecraft:stone", "fill ~0 ~1 ~0 ~15 ~4 ~0 minecraft:stone");
            if (sc >= 1) s.cmd("fill ~8 ~ ~3 ~14 ~ ~12 minecraft:water");
            if (sc == 2) s.cmd("fill ~8 ~ ~1 ~14 ~ ~2 minecraft:stone", "fill ~8 ~ ~13 ~14 ~ ~14 minecraft:stone", "fill ~7 ~ ~3 ~7 ~ ~12 minecraft:stone");
            for (int i = 0; i < 5; i++) {
                String k = kinds[i];
                s.set(1 + 3 * i, 101, 3, "minecraft:" + k + "_coral");
                s.set(1 + 3 * i, 101, 6, "minecraft:" + k + "_coral[waterlogged=true]");
                s.set(1 + 3 * i, 101, 9, "minecraft:" + k + "_coral_fan");
                s.set(1 + 3 * i, 101, 1, "minecraft:" + k + "_coral_wall_fan[facing=south]");
                s.set(2 + 3 * i, 100, 12, "minecraft:" + k + "_coral_block");
            }
            s.tick(10).set(9, 100, 7, "minecraft:tube_coral_block").set(10, 100, 5, "minecraft:brain_coral[waterlogged=false]").set(12, 100, 9, "minecraft:fire_coral_fan");
            s.tick(130);
            if (sc >= 1) s.set(11, 100, 6, "minecraft:stone").set(9, 100, 8, "minecraft:air").set(13, 100, 4, "minecraft:stone").tick(130);
            s.set(2, 100, 3, "minecraft:air").set(2, 100, 2, "minecraft:stone");
            out.add(s.tick(20));
        }
        // ---- scaffolding: arms, towers, and what is left when a pillar goes
        out.add(new Sc("wet_scaffolding_0", 410).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:stone",
                "fill ~2 ~1 ~2 ~2 ~6 ~2 minecraft:scaffolding", "fill ~3 ~6 ~2 ~11 ~6 ~2 minecraft:scaffolding",
                "fill ~8 ~1 ~8 ~8 ~3 ~8 minecraft:scaffolding", "fill ~9 ~3 ~8 ~12 ~3 ~8 minecraft:scaffolding",
                "setblock ~5 ~2 ~12 minecraft:scaffolding", "setblock ~5 ~3 ~12 minecraft:scaffolding[waterlogged=true]")
                .tick(40).set(2, 100, 2, "minecraft:air").tick(40).set(2, 100, 2, "minecraft:stone").tick(40)
                .set(8, 102, 8, "minecraft:air").tick(40).tick(60));
        out.add(new Sc("wet_scaffolding_1", 411).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:grass_block",
                "fill ~1 ~1 ~1 ~14 ~1 ~1 minecraft:scaffolding", "fill ~1 ~2 ~1 ~1 ~8 ~1 minecraft:scaffolding", "fill ~14 ~2 ~1 ~14 ~5 ~1 minecraft:scaffolding",
                "fill ~5 ~1 ~5 ~10 ~1 ~10 minecraft:scaffolding", "fill ~5 ~2 ~5 ~10 ~2 ~5 minecraft:scaffolding")
                .tick(30).set(1, 100, 1, "minecraft:air").tick(60).set(14, 100, 1, "minecraft:water").tick(40)
                .set(7, 101, 7, "minecraft:air").set(8, 101, 8, "minecraft:air").tick(40));

        // ---- sponges
        out.add(new Sc("wet_sponge_0", 420).cmd(
                "fill ~-1 ~ ~-1 ~16 ~ ~16 minecraft:stone hollow", "fill ~0 ~ ~0 ~15 ~3 ~15 minecraft:water",
                "setblock ~3 ~1 ~3 minecraft:seagrass", "setblock ~5 ~1 ~5 minecraft:kelp[age=3]", "setblock ~6 ~1 ~5 minecraft:oak_slab[waterlogged=true,type=bottom]",
                "setblock ~7 ~1 ~7 minecraft:tall_seagrass[half=lower]", "setblock ~7 ~2 ~7 minecraft:tall_seagrass[half=upper]",
                "setblock ~9 ~1 ~9 minecraft:oak_stairs[waterlogged=true]", "setblock ~12 ~ ~12 minecraft:lava")
                .set(8, 101, 8, "minecraft:sponge").tick(20).set(2, 101, 12, "minecraft:sponge").tick(20).set(13, 101, 3, "minecraft:sponge").tick(40)
                .set(12, 101, 12, "minecraft:sponge").set(6, 103, 6, "minecraft:wet_sponge").tick(30));
        out.add(new Sc("wet_sponge_1", 421).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:stone", "fill ~2 ~1 ~2 ~13 ~1 ~13 minecraft:water", "fill ~5 ~1 ~5 ~10 ~4 ~10 minecraft:water",
                "setblock ~12 ~1 ~12 minecraft:sponge", "setblock ~2 ~1 ~13 minecraft:sponge")
                .tick(20).set(0, 101, 0, "minecraft:sponge").set(7, 101, 7, "minecraft:sponge").tick(30).set(3, 101, 3, "minecraft:water").tick(30)
                .set(12, 101, 12, "minecraft:air").set(12, 101, 12, "minecraft:sponge").tick(30));
    }

    /// A 3x3 stone tank, open on top, with a column of `depth` water blocks over `base` in its middle.
    static void tank(Sc sc, int x, int z, int depth, String base) {
        sc.cmd(String.format("fill ~%d ~ ~%d ~%d ~%d ~%d minecraft:stone hollow", x, z, x + 2, depth + 1, z + 2),
                String.format("setblock ~%d ~%d ~%d minecraft:air", x + 1, depth + 1, z + 1),
                String.format("fill ~%d ~1 ~%d ~%d ~%d ~%d minecraft:water", x + 1, z + 1, x + 1, depth, z + 1),
                String.format("setblock ~%d ~ ~%d %s", x + 1, z + 1, base));
    }

    // ================================================================ family: spread
    // grass, mycelium, nylium, saplings (trees), azalea, snow and ice melting
    static void scenariosSpread(List<Sc> out) {
        // ---- grass and mycelium over dirt
        for (int i = 0; i < 3; i++) {
            out.add(new Sc("spread_grass_flat_" + i, 100 + i).cmd(
                    "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt",
                    "setblock ~3 ~ ~3 minecraft:grass_block",
                    "setblock ~12 ~ ~11 minecraft:grass_block",
                    "setblock ~7 ~ ~13 minecraft:mycelium").rt(70));
        }
        for (int i = 0; i < 3; i++) {
            out.add(new Sc("spread_grass_terraces_" + i, 110 + i).cmd(
                    "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt",
                    "fill ~2 ~1 ~2 ~7 ~1 ~7 minecraft:dirt",
                    "fill ~4 ~2 ~4 ~6 ~2 ~6 minecraft:dirt",
                    "fill ~9 ~1 ~8 ~13 ~1 ~13 minecraft:dirt",
                    "fill ~10 ~2 ~9 ~12 ~3 ~11 minecraft:dirt",
                    "fill ~0 ~1 ~12 ~3 ~1 ~15 minecraft:coarse_dirt",
                    "fill ~8 ~1 ~0 ~11 ~1 ~3 minecraft:podzol",
                    "setblock ~1 ~ ~1 minecraft:grass_block",
                    "setblock ~5 ~2 ~5 minecraft:grass_block",
                    "setblock ~14 ~ ~14 minecraft:mycelium",
                    "setblock ~11 ~3 ~10 minecraft:mycelium").rt(80));
        }
        // under a roof the light fades with the distance from its edge; torches light patches again
        out.add(new Sc("spread_grass_roof_0", 120).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt",
                "fill ~0 ~4 ~0 ~15 ~4 ~15 minecraft:oak_planks",
                "setblock ~2 ~ ~2 minecraft:grass_block",
                "setblock ~13 ~ ~13 minecraft:mycelium",
                "setblock ~7 ~ ~7 minecraft:grass_block",
                "setblock ~8 ~1 ~8 minecraft:torch",
                "setblock ~3 ~1 ~12 minecraft:torch").rt(80));
        out.add(new Sc("spread_grass_roof_1", 121).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt",
                "fill ~0 ~2 ~0 ~15 ~2 ~15 minecraft:glass",
                "fill ~3 ~2 ~3 ~12 ~2 ~12 minecraft:stone",
                "setblock ~2 ~ ~2 minecraft:grass_block",
                "setblock ~8 ~ ~8 minecraft:grass_block",
                "setblock ~6 ~1 ~9 minecraft:soul_torch",
                "setblock ~9 ~1 ~4 minecraft:glowstone").rt(80));
        out.add(new Sc("spread_grass_roof_2", 122).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt",
                "fill ~1 ~5 ~1 ~14 ~5 ~14 minecraft:stone_slab[type=bottom]",
                "setblock ~3 ~ ~3 minecraft:mycelium",
                "setblock ~12 ~ ~12 minecraft:grass_block").rt(60)
                .set(7, 105, 7, "minecraft:air").rt(30).set(8, 105, 8, "minecraft:air").set(6, 105, 6, "minecraft:air").rt(30));
        // what covers grass: it dies to dirt unless the cover lets the light through
        String[] covers = {
                "minecraft:stone", "minecraft:oak_slab[type=bottom]", "minecraft:oak_slab[type=top]", "minecraft:oak_slab[type=double]",
                "minecraft:oak_stairs[half=bottom,facing=east]", "minecraft:oak_stairs[half=top,facing=east]", "minecraft:glass",
                "minecraft:tinted_glass", "minecraft:oak_leaves[persistent=true]", "minecraft:water", "minecraft:snow[layers=1]",
                "minecraft:snow[layers=2]", "minecraft:snow[layers=8]", "minecraft:oak_trapdoor[half=bottom,open=false]",
                "minecraft:oak_trapdoor[half=top,open=false]", "minecraft:oak_trapdoor[open=true,facing=north]", "minecraft:oak_fence",
                "minecraft:cobweb", "minecraft:ice", "minecraft:honey_block", "minecraft:slime_block", "minecraft:torch",
                "minecraft:white_carpet", "minecraft:chest", "minecraft:iron_bars",
                "minecraft:glowstone", "minecraft:hopper", "minecraft:composter", "minecraft:bell[attachment=floor]",
                "minecraft:dirt_path", "minecraft:oak_pressure_plate", "minecraft:short_grass",
                "minecraft:oak_sign", "minecraft:flower_pot", "minecraft:lantern", "minecraft:oak_button[face=floor]", "minecraft:cauldron",
                "minecraft:redstone_wire", "minecraft:white_stained_glass", "minecraft:glass_pane",
                "minecraft:sea_pickle[pickles=1]", "minecraft:chain[axis=y]",
                "minecraft:end_rod", "minecraft:iron_trapdoor[half=top]", "minecraft:powder_snow",
                "minecraft:amethyst_cluster", "minecraft:campfire", "minecraft:oak_leaves[persistent=true,waterlogged=true]",
                "minecraft:pale_moss_carpet", "minecraft:moss_carpet", "minecraft:sculk_vein[down=true]", "minecraft:big_dripleaf",
                "minecraft:oak_fence_gate", "minecraft:cobblestone_wall", "minecraft:scaffolding"};
        for (int part = 0; part < 3; part++) {
            for (int variant = 0; variant < 2; variant++) {
                Sc sc = new Sc("spread_grass_covers_" + (variant == 0 ? "grass_" : "dirt_") + part, 130 + part * 2 + variant).cmd(
                        "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt");
                for (int i = 0; i < 25; i++) {
                    int idx = part * 20 + i;
                    if (idx >= covers.length) break;
                    int x = 1 + 3 * (i % 5), z = 1 + 3 * (i / 5);
                    if (variant == 0) sc.cmd("setblock ~" + x + " ~ ~" + z + " minecraft:grass_block");
                    else sc.cmd("setblock ~" + (x + 1) + " ~ ~" + z + " minecraft:" + (i % 2 == 0 ? "grass_block" : "mycelium"));
                    sc.cmd("setblock ~" + x + " ~1 ~" + z + " " + covers[idx]);
                }
                out.add(sc.rt(60));
            }
        }
        // grass and mycelium compete for the same dirt, with snow on top coming and going
        for (int i = 0; i < 2; i++) {
            out.add(new Sc("spread_compete_" + i, 150 + i).cmd(
                    "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt",
                    "fill ~0 ~ ~0 ~1 ~ ~15 minecraft:grass_block",
                    "fill ~14 ~ ~0 ~15 ~ ~15 minecraft:mycelium",
                    "fill ~7 ~ ~6 ~8 ~ ~9 minecraft:podzol").rt(120));
        }
        out.add(new Sc("spread_snowy", 160).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt",
                "fill ~0 ~ ~0 ~3 ~ ~15 minecraft:grass_block",
                "setblock ~14 ~ ~14 minecraft:mycelium",
                "fill ~1 ~1 ~1 ~2 ~1 ~14 minecraft:snow[layers=1]",
                "fill ~3 ~1 ~5 ~3 ~1 ~9 minecraft:snow[layers=4]",
                "setblock ~3 ~ ~12 minecraft:podzol",
                "setblock ~3 ~1 ~12 minecraft:snow_block").rt(30)
                .set(6, 101, 6, "minecraft:snow_block").set(7, 101, 6, "minecraft:snow[layers=1]").set(6, 101, 7, "minecraft:powder_snow").rt(40)
                .set(2, 101, 3, "minecraft:air").set(1, 101, 3, "minecraft:stone").set(2, 101, 4, "minecraft:snow[layers=3]").rt(40));
        // blocks above and below the dirt change while it spreads
        out.add(new Sc("spread_changes", 170).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt",
                "setblock ~2 ~ ~2 minecraft:grass_block",
                "setblock ~12 ~ ~12 minecraft:mycelium").rt(20)
                .set(2, 101, 2, "minecraft:stone").rt(10).set(2, 101, 2, "minecraft:air").set(12, 101, 12, "minecraft:water").rt(30)
                .set(6, 100, 6, "minecraft:stone").set(6, 100, 7, "minecraft:coarse_dirt").set(5, 100, 5, "minecraft:dirt").rt(20));

        // ---- snow layers melt in block light above 11
        String[] lamps = {"minecraft:torch", "minecraft:glowstone", "minecraft:candle[lit=true,candles=4]", "minecraft:soul_torch",
                "minecraft:sea_lantern", "minecraft:lantern[hanging=false]", "minecraft:redstone_lamp[lit=true]", "minecraft:campfire[lit=true]"};
        for (int i = 0; i < 4; i++) {
            Sc sc = new Sc("snow_melt_" + i, 180 + i).cmd("fill ~0 ~ ~0 ~15 ~ ~15 minecraft:dirt");
            // rows of snow of every depth, a lamp at the start of each row
            for (int row = 0; row < 8; row++) {
                int z = 1 + 2 * row;
                sc.cmd("setblock ~0 ~1 ~" + z + " " + lamps[(row + i) % lamps.length]);
                for (int x = 1; x <= 8; x++) sc.cmd("setblock ~" + x + " ~1 ~" + z + " minecraft:snow[layers=" + (1 + (x + row + i) % 8) + "]");
            }
            sc.cmd("fill ~10 ~ ~0 ~15 ~ ~15 minecraft:grass_block", "fill ~10 ~1 ~0 ~15 ~1 ~15 minecraft:snow[layers=2]");
            out.add(sc.rt(40));
        }
        out.add(new Sc("snow_melt_tall", 190).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:grass_block",
                "fill ~0 ~1 ~0 ~15 ~1 ~15 minecraft:snow[layers=1]",
                "fill ~4 ~1 ~4 ~11 ~1 ~11 minecraft:snow[layers=8]",
                "setblock ~7 ~2 ~7 minecraft:glowstone").rt(10)
                .set(4, 101, 4, "minecraft:torch").set(11, 101, 11, "minecraft:lava").rt(10).set(2, 100, 2, "minecraft:air").set(3, 101, 3, "minecraft:glowstone").rt(30));

        // ---- ice melts in block light above 11 - 1; frosted ice fades with age
        for (int i = 0; i < 3; i++) {
            Sc sc = new Sc("ice_melt_" + i, 200 + i).cmd("fill ~0 ~ ~0 ~15 ~ ~15 minecraft:stone");
            for (int row = 0; row < 8; row++) {
                int z = 1 + 2 * row;
                sc.cmd("setblock ~0 ~1 ~" + z + " " + lamps[(row + i) % lamps.length]);
                for (int x = 1; x <= 8; x++) {
                    String b = (x + row) % 5 == 0 ? "minecraft:packed_ice" : (x + row) % 7 == 0 ? "minecraft:blue_ice" : "minecraft:ice";
                    sc.cmd("setblock ~" + x + " ~1 ~" + z + " " + b);
                }
            }
            sc.cmd("fill ~11 ~1 ~1 ~14 ~1 ~14 minecraft:ice", "fill ~12 ~2 ~2 ~13 ~2 ~13 minecraft:ice", "setblock ~12 ~3 ~7 minecraft:glowstone");
            out.add(sc.rtTick(40, 8));
        }
        out.add(new Sc("ice_melt_water", 210).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:stone",
                "fill ~2 ~1 ~2 ~13 ~1 ~13 minecraft:ice",
                "fill ~5 ~1 ~5 ~10 ~1 ~10 minecraft:water",
                "setblock ~7 ~1 ~7 minecraft:ice",
                "setblock ~1 ~1 ~1 minecraft:torch",
                "setblock ~14 ~1 ~14 minecraft:glowstone").rtTick(30, 6));
        out.add(new Sc("ice_melt_lava", 211).cmd(
                "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:stone",
                "fill ~3 ~1 ~3 ~12 ~1 ~12 minecraft:ice",
                "setblock ~7 ~1 ~7 minecraft:lava",
                "setblock ~1 ~1 ~1 minecraft:glowstone",
                "setblock ~14 ~1 ~14 minecraft:torch").rtTick(30, 6));
        for (int i = 0; i < 3; i++) {
            Sc sc = new Sc("ice_frosted_" + i, 220 + i).cmd(
                    "fill ~-1 ~ ~-1 ~12 ~ ~12 minecraft:stone hollow",
                    "fill ~0 ~ ~0 ~11 ~ ~11 minecraft:water");
            if (i == 1) sc.cmd("setblock ~1 ~1 ~1 minecraft:torch", "fill ~9 ~1 ~9 ~10 ~1 ~10 minecraft:glowstone");
            if (i == 2) sc.cmd("fill ~-1 ~1 ~-1 ~12 ~1 ~12 minecraft:stone hollow", "fill ~-1 ~2 ~-1 ~12 ~2 ~12 minecraft:stone");
            // a block of it, a line, a pair and a single, then more at different ages
            for (int x = 2; x <= 4; x++) for (int z = 2; z <= 4; z++) sc.set(x, 100, z, "minecraft:frosted_ice[age=" + ((x + z) % 4) + "]");
            for (int x = 6; x <= 10; x++) sc.set(x, 100, 7, "minecraft:frosted_ice[age=0]");
            sc.set(1, 100, 9, "minecraft:frosted_ice[age=1]").set(2, 100, 9, "minecraft:frosted_ice[age=2]").set(9, 100, 1, "minecraft:frosted_ice[age=3]");
            sc.rtTick(25, 15);
            sc.set(6, 100, 3, "minecraft:frosted_ice[age=0]").set(7, 100, 3, "minecraft:frosted_ice[age=0]").set(7, 100, 4, "minecraft:frosted_ice[age=1]");
            sc.rtTick(25, 15);
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
        // Ticks a scenario that ran long left pending would sit far in the future once the time is reset.
        setGameTime(START_TIME);
        var cleared = new net.minecraft.world.level.levelgen.structure.BoundingBox(x0 + LO, FLOOR, z0 + LO, x0 + HI, Y0 + HEIGHT - 1, z0 + HI);
        level.getBlockTicks().clearArea(cleared);
        level.getFluidTicks().clearArea(cleared);
        command("difficulty " + new String[] {"peaceful", "easy", "normal", "hard"}[sc.difficulty]);
        command(String.format("fill %d %d %d %d %d %d minecraft:air", x0 + LO, FLOOR, z0 + LO, x0 + HI, Y0 + HEIGHT - 1, z0 + HI));
        command(String.format("fill %d %d %d %d %d %d minecraft:stone", x0 + LO, FLOOR, z0 + LO, x0 + HI, FLOOR, z0 + HI));
        for (String c : sc.setup) command(String.format("execute positioned %d %d %d run %s", x0, Y0, z0, c));
        setGameTime(START_TIME);
        // Whatever the setup scheduled runs out first, so every scenario starts without pending ticks
        // (the replay places the starting blocks without updates).
        awaitLight();
        for (int i = 0; i < 400 && pendingCount(x0, z0) > 0; i++) {
            tickLevel();
        }
        if (pendingCount(x0, z0) > 0) throw new IllegalStateException("setup of " + sc.name + " never settles: " + pending(x0, z0).subList(0, Math.min(4, pendingCount(x0, z0))));
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

    /// Debugging aid (tools/block_seq_diff.py): with KILN_SEQ_TRACE=<file> every random tick of an `rt` op
    /// appends "SEQ x,y,z <level random state> <block>", which the Rust replay writes the same way, so
    /// the first block whose tick drew a different number of random numbers shows.
    static final String SEQ_TRACE = System.getenv("KILN_SEQ_TRACE");

    static void traceSeq(BlockPos p, BlockState s) {
        try {
            var rnd = level.getRandom();
            var f = rnd.getClass().getDeclaredField("seed");
            f.setAccessible(true);
            long seed = ((java.util.concurrent.atomic.AtomicLong) f.get(rnd)).get();
            Files.writeString(Path.of(SEQ_TRACE + "_vanilla.txt"), "SEQ " + p.getX() + "," + p.getY() + "," + p.getZ() + " " + seed + " "
                    + BuiltInRegistries.BLOCK.getKey(s.getBlock()) + "\n", java.nio.file.StandardOpenOption.CREATE, java.nio.file.StandardOpenOption.APPEND);
        } catch (Throwable e) {
            e.printStackTrace();
        }
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
            if (SEQ_TRACE != null) traceSeq(p, s);
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

    // [dx, dy, dz, brightness, block light] of every position above the floor whose raw brightness is
    // not 15 or whose block light is not 0 (snow, ice and frosted ice read the block light alone).
    static List<Object> light(int x0, int z0) {
        List<Object> out = new ArrayList<>();
        for (int y = Y0; y < Y0 + HEIGHT; y++)
            for (int z = z0 + LO; z <= z0 + HI; z++)
                for (int x = x0 + LO; x <= x0 + HI; x++) {
                    BlockPos p = new BlockPos(x, y, z);
                    int b = level.getRawBrightness(p, 0);
                    int bl = level.getBrightness(net.minecraft.world.level.LightLayer.BLOCK, p);
                    if (b != 15 || bl != 0) out.add(List.of(x - x0, y, z - z0, b, bl));
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
