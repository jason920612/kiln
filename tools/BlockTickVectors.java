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
//   seed N   reseeds the level random (loot rolled by a drop is not replayed, so a scenario with a
//            drop reseeds after it)
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

        /** `EnderEyeItem.useOn` by a player (survival, standing above) clicking the top of the block at area-relative x, y (absolute), z. */
        Sc eye(int x, int y, int z) {
            ops.add(new Object[] {"eye", x, y, z});
            return this;
        }

        /** `Level.setBlock(pos, state, 3)` at area-relative x, y (absolute), z. */
        Sc set(int x, int y, int z, String state) {
            ops.add(new Object[] {"set", x, y, z, state});
            return this;
        }

        /** Reseeds the level random (after a block dropped: Kiln rolls drops with its own random). */
        Sc reseed(long s) {
            ops.add(new Object[] {"seed", s});
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
        scenariosEnd(out);
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
        farmWheat(out);
        farmCrops(out);
        farmStems(out);
        farmPlants(out);
        farmColumns(out);
    }

    // ---- helpers of the farming family (absolute y; x and z relative to the scenario's corner)

    interface Cell {
        boolean at(int x, int z);
    }

    static void put(Sc sc, int x, int y, int z, String block) {
        sc.cmd("setblock ~" + x + " ~" + (y - Y0) + " ~" + z + " " + block);
    }

    static void box(Sc sc, int x1, int y1, int z1, int x2, int y2, int z2, String block) {
        sc.cmd("fill ~" + x1 + " ~" + (y1 - Y0) + " ~" + z1 + " ~" + x2 + " ~" + (y2 - Y0) + " ~" + z2 + " " + block);
    }

    /** Farmland in the floor layer; moisture < 0 gives every cell its own (varying) value. */
    static void farm(Sc sc, int x1, int z1, int x2, int z2, int moisture) {
        for (int z = z1; z <= z2; z++)
            for (int x = x1; x <= x2; x++)
                put(sc, x, FLOOR, z, "minecraft:farmland[moisture=" + (moisture >= 0 ? moisture : (x * 3 + z) % 8) + "]");
    }

    /** Crops at y 100 with ages cycling over the cells (a `null` cell filter plants everywhere). */
    static void crops(Sc sc, int x1, int z1, int x2, int z2, String block, int maxAge, Cell cell) {
        for (int z = z1; z <= z2; z++)
            for (int x = x1; x <= x2; x++)
                if (cell == null || cell.at(x, z))
                    put(sc, x, Y0, z, "minecraft:" + block + "[age=" + ((x * 5 + z * 3) % (maxAge + 1)) + "]");
    }

    /** A cell at height y holding `content` inside four stone walls on a stone floor (water stays put). */
    static void well(Sc sc, int x, int y, int z, String content) {
        box(sc, x - 1, y - 1, z - 1, x + 1, y - 1, z + 1, "minecraft:stone");
        put(sc, x - 1, y, z, "minecraft:stone");
        put(sc, x + 1, y, z, "minecraft:stone");
        put(sc, x, y, z - 1, "minecraft:stone");
        put(sc, x, y, z + 1, "minecraft:stone");
        put(sc, x, y, z, content);
    }

    // ---- farmland and wheat
    static void farmWheat(List<Sc> out) {
        // Wheat on farmland with one water well: the plot's far side dries out and the farmland
        // holds because of the crop on it; ages from 0 to 7, growth speed by moisture.
        Sc s = new Sc("farm_wheat_water", 11);
        farm(s, 0, 0, 9, 9, -1);
        crops(s, 0, 0, 9, 9, "wheat", 7, null);
        well(s, 12, Y0, 2, "minecraft:water");
        well(s, -3, Y0, 7, "minecraft:water");
        out.add(s.rt(70));

        // Farmland with and without crops, no water: moisture runs down, bare farmland turns to dirt,
        // something put on top (stone) turns it to dirt after the scheduled tick; a well filled later.
        s = new Sc("farm_wheat_dry", 12);
        farm(s, 0, 0, 9, 9, -1);
        crops(s, 0, 0, 9, 9, "wheat", 7, (x, z) -> (x + z) % 2 == 0);
        well(s, 13, Y0, 4, "minecraft:air");
        s.rt(8).set(2, Y0, 2, "minecraft:stone").rtTick(6, 2).set(4, Y0, 4, "minecraft:air").rt(6)
                .set(3, Y0, 5, "minecraft:glass").rtTick(6, 1)
                .set(13, Y0, 4, "minecraft:water").rtTick(14, 1);
        out.add(s);

        // Crops in rows, in checkers and diagonals: the growth speed halves; farmland moisture on and off.
        s = new Sc("farm_wheat_rows", 13);
        farm(s, 0, 0, 11, 11, 7);
        crops(s, 0, 0, 11, 11, "wheat", 7, (x, z) -> x % 4 == 0 || (z % 5 == 2 && x % 2 == 1) || (x + z) % 7 == 0);
        well(s, -2, Y0, 2, "minecraft:water");
        well(s, -2, Y0, 9, "minecraft:water");
        well(s, 14, Y0, 2, "minecraft:water");
        out.add(s.rt(60));

        // Light: a roof, a light block of level 9 to 15 above every crop (survival needs 8, growth 9);
        // light blocks changed mid-way. Roof and light first: the crops never see the dark.
        s = new Sc("farm_wheat_light", 14);
        box(s, -1, 104, -1, 9, 104, 9, "minecraft:stone");
        for (int z = 0; z <= 8; z++)
            for (int x = 0; x <= 8; x++) put(s, x, 101, z, "minecraft:light[level=" + (9 + (x * 3 + z * 5) % 7) + "]");
        farm(s, 0, 0, 8, 8, 7);
        crops(s, 0, 0, 8, 8, "wheat", 7, null);
        well(s, 11, Y0, 4, "minecraft:water");
        s.rt(20).set(4, 101, 4, "minecraft:light[level=15]").set(0, 101, 0, "minecraft:light[level=9]").rtTick(10, 1)
                .set(2, 101, 2, "minecraft:light[level=10]").set(6, 101, 6, "minecraft:light[level=9]").rt(20)
                .set(3, 101, 3, "minecraft:light[level=2]").set(5, 100, 4, "minecraft:stone").tick(3).reseed(141);
        out.add(s);

        // Farmland and water distance: water exactly 4 and 5 blocks away, diagonally, one and two above.
        s = new Sc("farm_farmland_water_range", 15);
        for (int[] c : new int[][] {{2, 2}, {2, 8}, {10, 2}, {10, 8}, {2, 13}, {10, 13}}) put(s, c[0], FLOOR, c[1], "minecraft:farmland[moisture=3]");
        well(s, 6, Y0, 2, "minecraft:water");
        well(s, 7, Y0, 8, "minecraft:water");
        well(s, 14, Y0, 6, "minecraft:water");
        well(s, 15, Y0, 12, "minecraft:water");
        box(s, 3, 100, 12, 5, 100, 14, "minecraft:stone");
        well(s, 4, 101, 13, "minecraft:water");
        well(s, 12, Y0, 13, "minecraft:water");
        s.rt(10).rtTick(5, 3).set(4, 101, 13, "minecraft:air").set(7, Y0, 8, "minecraft:air").rt(12);
        out.add(s);
    }

    // ---- other crops
    static void farmCrops(List<Sc> out) {
        // Carrots and potatoes side by side in a checker (a different crop does not count as a row).
        Sc s = new Sc("farm_carrots_potatoes", 16);
        farm(s, 0, 0, 9, 9, 7);
        crops(s, 0, 0, 9, 9, "carrots", 7, (x, z) -> (x + z) % 2 == 0);
        crops(s, 0, 0, 9, 9, "potatoes", 7, (x, z) -> (x + z) % 2 == 1 && x < 8);
        well(s, 12, Y0, 4, "minecraft:water");
        well(s, -3, Y0, 4, "minecraft:water");
        out.add(s.rtTick(30, 1).rt(30));

        // Beetroots: max age 3, a third of the random rolls.
        s = new Sc("farm_beetroot", 17);
        farm(s, 0, 0, 9, 7, -1);
        crops(s, 0, 0, 9, 7, "beetroots", 3, (x, z) -> x % 3 != 1 || z % 2 == 0);
        well(s, 12, Y0, 3, "minecraft:water");
        out.add(s.rt(100));

        // Torchflower: ages 0 and 1, then the flower itself.
        s = new Sc("farm_torchflower", 18);
        farm(s, 0, 0, 7, 7, 7);
        crops(s, 0, 0, 7, 7, "torchflower_crop", 1, (x, z) -> (x + 2 * z) % 3 != 0);
        well(s, 10, Y0, 3, "minecraft:water");
        out.add(s.rt(90));

        // Light at the limits: a light block above every crop, levels 9 to 12 (brightness 8 and up: survival
        // needs 8, growth 9); at the end light taken away from one and a neighbour changed, so it pops.
        s = new Sc("farm_crops_light_limits", 19);
        box(s, -1, 104, -1, 6, 104, 8, "minecraft:stone");
        for (int z = 0; z <= 7; z++) for (int x = 0; x <= 5; x++) put(s, x, 101, z, "minecraft:light[level=" + (9 + (x + 2 * z) % 4) + "]");
        farm(s, 0, 0, 5, 7, 7);
        crops(s, 0, 0, 5, 7, "wheat", 7, (x, z) -> x % 2 == 0);
        crops(s, 0, 0, 5, 7, "carrots", 7, (x, z) -> x % 2 == 1 && z % 2 == 0);
        well(s, 8, Y0, 3, "minecraft:water");
        out.add(s.rt(30).set(2, 101, 2, "minecraft:light[level=15]").set(2, 100, 3, "minecraft:wheat[age=0]").rtTick(20, 2)
                .set(3, 101, 3, "minecraft:light[level=1]").set(4, 101, 3, "minecraft:light[level=1]").set(3, 101, 4, "minecraft:light[level=1]")
                .set(3, 101, 2, "minecraft:light[level=1]").set(3, 100, 3, "minecraft:carrots[age=1]").set(3, 100, 3, "minecraft:carrots[age=2]").tick(2).reseed(191));

        // Pitcher plants: one block until age 3, two blocks from age 3; stone above blocks the two-block growth.
        s = new Sc("farm_pitcher", 20);
        farm(s, 0, 0, 9, 9, 7);
        for (int z = 0; z <= 8; z += 2)
            for (int x = 0; x <= 8; x += 2) {
                int age = (x / 2 + z) % 5;
                put(s, x, Y0, z, "minecraft:pitcher_crop[age=" + age + ",half=lower]");
                if (age >= 3) put(s, x, 101, z, "minecraft:pitcher_crop[age=" + age + ",half=upper]");
                if (age == 2 && x >= 4) put(s, x, 101, z, "minecraft:stone");
            }
        well(s, 12, Y0, 4, "minecraft:water");
        out.add(s.rt(60).set(0, 101, 4, "minecraft:stone").reseed(201).rtTick(40, 1));

        // Pitcher plants in rows and a dark corner: growth speed and the light needed.
        s = new Sc("farm_pitcher_rows", 21);
        farm(s, 0, 0, 7, 3, 7);
        crops(s, 0, 0, 7, 3, "pitcher_crop", 2, null);
        for (int z = 0; z <= 3; z++) for (int x = 0; x <= 7; x++) put(s, x, Y0, z, "minecraft:pitcher_crop[age=" + ((x + z) % 3) + ",half=lower]");
        well(s, 10, Y0, 1, "minecraft:water");
        box(s, 4, 103, 0, 7, 103, 3, "minecraft:stone");
        out.add(s.rt(70));

        // Nether wart on soul sand: ages 0 to 3, a third of ... one in ten; the soul sand removed under a wart.
        s = new Sc("farm_nether_wart", 22);
        box(s, 0, FLOOR, 0, 8, FLOOR, 8, "minecraft:soul_sand");
        crops(s, 0, 0, 8, 8, "nether_wart", 3, null);
        out.add(s.rt(40).set(3, FLOOR, 3, "minecraft:stone").reseed(221).rt(10).set(3, Y0, 3, "minecraft:nether_wart[age=0]").set(4, Y0, 4, "minecraft:air").rtTick(40, 1));
    }

    // ---- stems
    static void farmStems(List<Sc> out) {
        for (String fruit : new String[] {"melon", "pumpkin"}) {
            boolean melon = fruit.equals("melon");
            Sc s = new Sc("farm_stem_" + fruit, melon ? 23 : 24);
            // Stems 4 apart on moist farmland; the ground around differs from stem to stem.
            String[] ground = {"minecraft:dirt", "minecraft:rooted_dirt", "minecraft:sandstone", "minecraft:stone", "minecraft:coarse_dirt", "minecraft:farmland[moisture=7]", "minecraft:podzol", "minecraft:mud"};
            for (int sz = 0; sz < 2; sz++)
                for (int sx = 0; sx < 4; sx++) {
                    int x = 2 + sx * 4, z = 2 + sz * 6;
                    String g = ground[sz * 4 + sx];
                    box(s, x - 1, FLOOR, z - 1, x + 1, FLOOR, z + 1, g);
                    put(s, x, FLOOR, z, "minecraft:farmland[moisture=7]");
                    put(s, x, Y0, z, "minecraft:" + fruit + "_stem[age=" + (sx * 2 + sz) % 8 + "]");
                }
            // a blocked side (a block where fruit would go) and mixed grounds around the last stem
            put(s, 3, Y0, 2, "minecraft:stone");
            put(s, 10, FLOOR, 7, "minecraft:stone");
            put(s, 11, FLOOR, 8, "minecraft:dirt");
            well(s, 5, Y0, 12, "minecraft:water");
            // an attached stem with its fruit, to be broken later
            box(s, 13, FLOOR, 11, 15, FLOOR, 13, "minecraft:dirt");
            put(s, 14, FLOOR, 12, "minecraft:farmland[moisture=7]");
            put(s, 14, Y0, 12, "minecraft:attached_" + fruit + "_stem[facing=east]");
            put(s, 15, Y0, 12, "minecraft:" + fruit);
            s.rt(40).rtTick(30, 1).set(15, Y0, 12, "minecraft:air").rtTick(30, 2);
            out.add(s);
        }
        // Stems of both kinds beside each other (rows count only the same stem), one in the dark.
        Sc s = new Sc("farm_stem_mixed", 25);
        farm(s, 0, 0, 9, 3, 7);
        box(s, 0, FLOOR, 4, 9, FLOOR, 7, "minecraft:rooted_dirt");
        for (int x = 0; x <= 8; x += 2) put(s, x, Y0, 1, "minecraft:" + (x % 4 == 0 ? "melon" : "pumpkin") + "_stem[age=" + (x % 7) + "]");
        for (int x = 1; x <= 9; x += 2) put(s, x, Y0, 2, "minecraft:" + (x % 3 == 0 ? "melon" : "pumpkin") + "_stem[age=" + (x % 8) + "]");
        well(s, 12, Y0, 2, "minecraft:water");
        box(s, 6, 104, 0, 9, 104, 3, "minecraft:stone");
        out.add(s.rt(80));
    }

    // ---- cocoa and sweet berries
    static void farmPlants(List<Sc> out) {
        // Cocoa on jungle logs (and on oak, which does not support it), all ages, a log removed later.
        Sc s = new Sc("farm_cocoa", 26);
        String[] dirs = {"north", "east", "south", "west"};
        int[][] off = {{0, -1}, {1, 0}, {0, 1}, {-1, 0}};
        for (int p = 0; p < 6; p++) {
            int x = 2 + p * 3, z = 4;
            String log = p == 4 ? "oak_log" : "jungle_log";
            box(s, x, 100, z, x, 103, z, "minecraft:" + log);
            for (int d = 0; d < 4; d++)
                for (int y = 100; y <= 102; y += 1 + (p + d) % 2) {
                    // facing points at the log: the cocoa sits on the opposite side
                    int fx = x - off[d][0], fz = z - off[d][1];
                    put(s, fx, y, fz, "minecraft:cocoa[age=" + ((p + d + y) % 3) + ",facing=" + dirs[d] + "]");
                }
        }
        out.add(s.rt(50).set(5, 101, 4, "minecraft:stone").reseed(261).rtTick(30, 1).set(11, 100, 4, "minecraft:air").reseed(262).rt(10));

        // Sweet berry bushes: growth needs light 9 above; bright, dim and dark patches.
        s = new Sc("farm_sweet_berry", 27);
        box(s, 0, FLOOR, 0, 9, FLOOR, 9, "minecraft:coarse_dirt");
        for (int z = 0; z <= 9; z += 2)
            for (int x = 0; x <= 9; x++)
                put(s, x, Y0, z, "minecraft:sweet_berry_bush[age=" + ((x + z) % 4) + "]");
        box(s, -1, 105, -1, 10, 105, 10, "minecraft:stone");
        for (int z = 1; z <= 9; z += 2)
            for (int x = 0; x <= 9; x += 2) put(s, x, 101, z, "minecraft:light[level=" + ((x + z * 2) % 16) + "]");
        out.add(s.rt(50).set(5, 101, 5, "minecraft:light[level=15]").set(3, 101, 3, "minecraft:air").rtTick(30, 1));

        // Sweet berries in plain daylight and on other soil.
        s = new Sc("farm_sweet_berry_day", 28);
        box(s, 0, FLOOR, 0, 9, FLOOR, 4, "minecraft:dirt");
        box(s, 0, FLOOR, 5, 9, FLOOR, 9, "minecraft:podzol");
        for (int z = 0; z <= 9; z += 3)
            for (int x = 0; x <= 9; x += 2) put(s, x, Y0, z, "minecraft:sweet_berry_bush[age=" + ((x / 2 + z) % 4) + "]");
        out.add(s.rt(80));
    }

    // ---- sugar cane, cactus, bamboo
    static void farmColumns(List<Sc> out) {
        // Sugar cane on sand at y 100 (water beside the ground block): columns of 1 to 3 grow to 3 and stop,
        // one grows from a ground without water (nothing happens until updated), water removed later breaks canes.
        Sc s = new Sc("farm_sugar_cane", 29);
        box(s, 0, Y0, 0, 9, Y0, 9, "minecraft:sand");
        int[][] wells = {{2, 2}, {7, 2}, {2, 7}, {7, 7}};
        for (int[] w : wells) put(s, w[0], Y0, w[1], "minecraft:water");
        for (int z = 0; z <= 9; z++)
            for (int x = 0; x <= 9; x++) {
                boolean well = false, beside = false;
                for (int[] w : wells) {
                    if (w[0] == x && w[1] == z) well = true;
                    if (Math.abs(w[0] - x) + Math.abs(w[1] - z) == 1) beside = true;
                }
                if (well || !(beside || (x * 3 + z) % 5 == 0)) continue;
                int h = 1 + (x + z) % 3;
                for (int k = 0; k < h; k++) put(s, x, 101 + k, z, "minecraft:sugar_cane[age=" + ((x * 7 + z * 5 + k * 3) % 16) + "]");
            }
        out.add(s.rt(60).set(2, Y0, 2, "minecraft:air").set(1, 103, 2, "minecraft:stone").set(2, 102, 3, "minecraft:stone").tick(6).reseed(291).rtTick(30, 1)
                .set(7, 102, 4, "minecraft:stone").set(8, 103, 1, "minecraft:stone").tick(6).reseed(292).rtTick(30, 2));

        // Sugar cane next to waterlogged blocks, on grass and dirt and on stone (which does not support it).
        s = new Sc("farm_sugar_cane_ground", 30);
        box(s, 0, Y0, 0, 9, Y0, 3, "minecraft:coarse_dirt");
        box(s, 0, Y0, 4, 9, Y0, 6, "minecraft:dirt");
        box(s, 0, Y0, 7, 9, Y0, 9, "minecraft:red_sand");
        put(s, 3, Y0, 1, "minecraft:oak_slab[type=top,waterlogged=true]");
        put(s, 6, Y0, 5, "minecraft:water");
        put(s, 3, Y0, 8, "minecraft:water");
        put(s, 5, Y0, 8, "minecraft:stone");
        for (int z = 0; z <= 9; z++)
            for (int x = 0; x <= 9; x++)
                if ((x * 2 + z) % 4 == 0 && !(x == 3 && z == 1) && !(x == 6 && z == 5) && !(x == 3 && z == 8))
                    put(s, x, 101, z, "minecraft:sugar_cane[age=" + ((x + z * 3) % 16) + "]");
        out.add(s.rt(60).set(5, 101, 8, "minecraft:sugar_cane[age=10]").set(4, 101, 8, "minecraft:stone").tick(6).reseed(301).rtTick(30, 1).set(6, Y0, 5, "minecraft:air").set(6, 101, 4, "minecraft:stone").tick(6).reseed(302).rtTick(30, 3));

        // Cactus on sand: heights 1 to 3, all ages, flowers at age 8, solid blocks put beside them, lava.
        s = new Sc("farm_cactus", 31);
        box(s, 0, Y0, 0, 12, Y0, 12, "minecraft:sand");
        for (int sz = 0; sz < 4; sz++)
            for (int sx = 0; sx < 4; sx++) {
                int x = 1 + sx * 3, z = 1 + sz * 3, h = 1 + (sx + sz) % 3;
                for (int k = 0; k < h; k++) put(s, x, 101 + k, z, "minecraft:cactus[age=" + ((sx * 5 + sz * 3 + k * 4 + 7) % 16) + "]");
            }
        put(s, 12, 101, 4, "minecraft:stone");
        put(s, 11, 101, 3, "minecraft:stone");
        put(s, 11, 101, 5, "minecraft:stone");
        out.add(s.rt(80).set(4, 101, 2, "minecraft:stone").tick(6).reseed(311).rtTick(10, 1).set(7, 102, 7, "minecraft:glass").tick(4).reseed(312).rt(10)
                .set(10, 101, 4, "minecraft:cactus[age=8]").rtTick(20, 1).set(11, 101, 4, "minecraft:lava").tick(40));

        // Cactus: columns from age 8 (to flower) and 15 (to grow), on sand and red sand, some on dirt (not supported).
        s = new Sc("farm_cactus_flowers", 32);
        box(s, 0, Y0, 0, 9, Y0, 9, "minecraft:red_sand");
        for (int sz = 0; sz < 3; sz++)
            for (int sx = 0; sx < 3; sx++) {
                int x = 1 + sx * 3, z = 1 + sz * 3, h = 1 + (sx + 2 * sz) % 3;
                for (int k = 0; k < h; k++) put(s, x, 101 + k, z, "minecraft:cactus[age=" + (k == h - 1 ? (sx + sz) % 2 == 0 ? 8 : 14 : (sx * 3 + k) % 16) + "]");
            }
        put(s, 1, Y0, 7, "minecraft:dirt");
        out.add(s.rt(90));

        // Bamboo: saplings and stalks of several heights; a roof stops some, leaves and stages as it grows.
        s = new Sc("farm_bamboo", 33);
        box(s, 0, FLOOR, 0, 11, FLOOR, 7, "minecraft:dirt");
        for (int x = 0; x <= 11; x += 2) put(s, x, Y0, 1, "minecraft:bamboo_sapling");
        for (int x = 0; x <= 11; x += 3) {
            int h = 1 + x % 4;
            for (int k = 0; k < h; k++)
                put(s, x, Y0 + k, 4, "minecraft:bamboo[age=" + (k > 1 ? 1 : 0) + ",leaves=" + (k == h - 1 ? "small" : k == h - 2 ? "large" : "none") + ",stage=0]");
        }
        for (int x = 1; x <= 11; x += 4) {
            put(s, x, Y0, 6, "minecraft:bamboo[age=0,leaves=none,stage=0]");
        }
        box(s, 7, 106, 3, 11, 106, 5, "minecraft:stone");
        out.add(s.rt(120));

        // Bamboo columns tall enough for the stage roll (from 11 high up, a quarter of the new segments get
        // stage 1, which stops the growth), in daylight.
        s = new Sc("farm_bamboo_tall", 34);
        box(s, 0, FLOOR, 0, 11, FLOOR, 2, "minecraft:podzol");
        for (int x = 0; x <= 11; x++) {
            int h = 10 + x % 2;
            for (int k = 0; k < h; k++)
                put(s, x, Y0 + k, 1, "minecraft:bamboo[age=" + (k > 1 ? 1 : 0) + ",leaves=" + (k == h - 1 ? "small" : k == h - 2 ? "large" : "none") + ",stage=0]");
        }
        out.add(s.rt(140));
        farmMore(out);
    }

    // ---- more seeds and variants of the above
    static void farmMore(List<Sc> out) {
        // Wheat and carrots in a big plot with rows, then rows broken up and swapped for another crop.
        Sc s = new Sc("farm_wheat_rows_big", 41);
        farm(s, 0, 0, 13, 13, 7);
        crops(s, 0, 0, 13, 13, "wheat", 7, (x, z) -> (x / 2 + z / 2) % 2 == 0 && !(x % 4 == 3));
        well(s, 4, Y0, 16, "minecraft:water");
        well(s, 10, Y0, -3, "minecraft:water");
        well(s, -3, Y0, 6, "minecraft:water");
        out.add(s.rt(30).set(4, Y0, 4, "minecraft:carrots[age=3]").set(5, Y0, 4, "minecraft:carrots[age=0]").set(5, Y0, 5, "minecraft:air").rt(30).set(7, Y0, 7, "minecraft:potatoes[age=2]").rt(30));

        // Potatoes in rows, dry farmland (moisture runs out: speed 1 instead of 3).
        s = new Sc("farm_potatoes_dry", 42);
        farm(s, 0, 0, 9, 5, -1);
        crops(s, 0, 0, 9, 5, "potatoes", 7, (x, z) -> z % 2 == 0 || x % 3 == 0);
        out.add(s.rt(80));

        // Torchflowers and beetroots in the same plot, with light blocks making every other crop dim (9 and 10).
        s = new Sc("farm_torchflower_beetroot_light", 43);
        box(s, -1, 104, -1, 8, 104, 5, "minecraft:stone");
        for (int z = 0; z <= 4; z++) for (int x = 0; x <= 7; x++) put(s, x, 101, z, "minecraft:light[level=" + (9 + (x + z) % 2) + "]");
        farm(s, 0, 0, 7, 4, 7);
        crops(s, 0, 0, 7, 4, "torchflower_crop", 1, (x, z) -> x % 2 == 0);
        crops(s, 0, 0, 7, 4, "beetroots", 3, (x, z) -> x % 2 == 1);
        well(s, 10, Y0, 2, "minecraft:water");
        out.add(s.rt(100));

        // Stems with and without light, fruit ground of every kind in one row; the fruit broken off one by one.
        s = new Sc("farm_stem_light", 44);
        box(s, -1, 104, -1, 8, 104, 12, "minecraft:stone");
        for (int x = 0; x <= 7; x += 2) put(s, x, 102, 1, "minecraft:light[level=" + (9 + x % 3) + "]");
        for (int x = 0; x <= 7; x += 2) {
            box(s, x - 1 < 0 ? 0 : x - 1, FLOOR, 0, x + 1, FLOOR, 2, x % 4 == 0 ? "minecraft:dirt" : "minecraft:coarse_dirt");
            put(s, x, FLOOR, 1, "minecraft:farmland[moisture=7]");
            put(s, x, Y0, 1, "minecraft:melon_stem[age=" + (3 + x % 5) + "]");
        }
        well(s, 10, Y0, 1, "minecraft:water");
        out.add(s.rt(60).rtTick(40, 1));

        // Sweet berries under light blocks of level 8, 9 and 10 right above them: the growth limit is 9.
        s = new Sc("farm_sweet_berry_limit", 45);
        box(s, 0, FLOOR, 0, 8, FLOOR, 5, "minecraft:podzol");
        box(s, -1, 104, -1, 9, 104, 6, "minecraft:stone");
        for (int z = 0; z <= 5; z += 1) for (int x = 0; x <= 8; x += 2) {
            put(s, x, Y0, z, "minecraft:sweet_berry_bush[age=" + ((x + z) % 3) + "]");
            put(s, x, 101, z, "minecraft:light[level=" + (8 + (x / 2 + z) % 3) + "]");
        }
        out.add(s.rt(120));

        // Cactus: planted in a ring, in a line (adjacent cacti are fine: only solid blocks and lava are not), at all ages.
        s = new Sc("farm_cactus_rows", 46);
        box(s, 0, Y0, 0, 11, Y0, 7, "minecraft:sand");
        for (int x = 0; x <= 10; x += 2) for (int k = 0; k < 1 + (x / 2) % 3; k++) put(s, x, 101 + k, 1, "minecraft:cactus[age=" + ((x * 3 + k * 5) % 16) + "]");
        for (int x = 0; x <= 10; x += 3) for (int k = 0; k < 2; k++) put(s, x, 101 + k, 5, "minecraft:cactus[age=" + (k == 0 ? 15 : 8) + "]");
        out.add(s.rt(120));

        // Cane at both sides of water and on a ring; flowers/age and top growth with air above, glass above one.
        s = new Sc("farm_sugar_cane_ring", 47);
        box(s, 0, Y0, 0, 8, Y0, 8, "minecraft:sand");
        box(s, 3, Y0, 3, 5, Y0, 5, "minecraft:water");
        put(s, 4, Y0, 4, "minecraft:sand");
        for (int i = 2; i <= 6; i++) { put(s, i, 101, 2, "minecraft:sugar_cane[age=" + (i * 3) + "]"); put(s, i, 101, 6, "minecraft:sugar_cane[age=" + (15 - i) + "]"); put(s, 2, 101, i, "minecraft:sugar_cane[age=" + (i * 2) + "]"); put(s, 6, 101, i, "minecraft:sugar_cane[age=15]"); }
        put(s, 4, 101, 4, "minecraft:sugar_cane[age=14]");
        put(s, 2, 102, 2, "minecraft:glass");
        out.add(s.rt(90));

        // Bamboo saplings and young bamboo under light blocks of level 8, 9 and 10 (growth needs 9 above).
        s = new Sc("farm_bamboo_light", 48);
        box(s, 0, FLOOR, 0, 9, FLOOR, 4, "minecraft:dirt");
        box(s, -1, 108, -1, 10, 108, 5, "minecraft:stone");
        for (int x = 0; x <= 9; x++) {
            put(s, x, Y0, 1, "minecraft:bamboo_sapling");
            put(s, x, 101, 1, "minecraft:light[level=" + (8 + x % 3) + "]");
            put(s, x, Y0, 3, "minecraft:bamboo[age=0,leaves=none,stage=0]");
            put(s, x, 101, 3, "minecraft:bamboo[age=0,leaves=small,stage=0]");
            put(s, x, 102, 3, "minecraft:light[level=" + (8 + (x + 1) % 3) + "]");
        }
        out.add(s.rt(100));

        // Pitcher plants at several light levels (survival needs 8; no light needed to grow), rows of them.
        s = new Sc("farm_pitcher_light", 49);
        box(s, -1, 104, -1, 8, 104, 4, "minecraft:stone");
        for (int x = 0; x <= 7; x++) for (int z = 0; z <= 3; z += 3) put(s, x, 102, z, "minecraft:light[level=" + (10 + x % 4) + "]");
        farm(s, 0, 0, 7, 3, 7);
        for (int x = 0; x <= 7; x++) for (int z = 0; z <= 3; z += 3) put(s, x, Y0, z, "minecraft:pitcher_crop[age=" + (x % 3) + ",half=lower]");
        well(s, 10, Y0, 1, "minecraft:water");
        out.add(s.rt(80));
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
        scenariosCuts(out);
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

    /// Plants cut in the middle or robbed of their support: what breaks drops (loot is rolled with the level
    /// random, which Kiln does not replay), so the scenarios reseed after the cut and keep growing.
    static void scenariosCuts(List<Sc> out) {
        Sc w = new Sc("growth_cut_weeping", 410).cmd("fill ~0 ~9 ~0 ~15 ~9 ~15 minecraft:stone");
        Sc t = new Sc("growth_cut_twisting", 411).cmd("fill ~0 ~10 ~0 ~15 ~10 ~15 minecraft:stone", "fill ~0 ~ ~0 ~15 ~ ~15 minecraft:stone");
        Sc c = new Sc("growth_cut_cave_vines", 412).cmd("fill ~0 ~9 ~0 ~15 ~9 ~15 minecraft:stone");
        Sc k = new Sc("growth_cut_kelp", 413);
        Sc ch = new Sc("growth_cut_chorus", 414).cmd("fill ~0 ~ ~0 ~15 ~ ~15 minecraft:end_stone");
        for (int i = 0; i < 6; i++) {
            int x = 2 + 2 * i, z = 3 + (i % 3) * 4;
            w.cmd(String.format("fill ~%d ~8 ~%d ~%d ~5 ~%d minecraft:weeping_vines_plant", x, z, x, z),
                    String.format("setblock ~%d ~4 ~%d minecraft:weeping_vines[age=%d]", x, z, i * 4));
            t.cmd(String.format("fill ~%d ~1 ~%d ~%d ~4 ~%d minecraft:twisting_vines_plant", x, z, x, z),
                    String.format("setblock ~%d ~5 ~%d minecraft:twisting_vines[age=%d]", x, z, i * 4));
            c.cmd(String.format("fill ~%d ~8 ~%d ~%d ~5 ~%d minecraft:cave_vines_plant[berries=%s]", x, z, x, z, i % 2 == 0 ? "true" : "false"),
                    String.format("setblock ~%d ~4 ~%d minecraft:cave_vines[age=%d,berries=%s]", x, z, i * 3, i % 3 == 0 ? "true" : "false"));
            ch.cmd(String.format("fill ~%d ~1 ~%d ~%d ~4 ~%d minecraft:chorus_plant", x, z, x, z),
                    String.format("setblock ~%d ~5 ~%d minecraft:chorus_flower[age=%d]", x, z, i % 3));
        }
        for (int i = 0; i < 4; i++) {
            int x = 1 + 4 * i, z = 1 + 7 * (i % 2);
            tank(k, x, z, 8, "minecraft:sand");
            k.cmd(String.format("fill ~%d ~1 ~%d ~%d ~4 ~%d minecraft:kelp_plant", x + 1, z + 1, x + 1, z + 1),
                    String.format("setblock ~%d ~5 ~%d minecraft:kelp[age=%d]", x + 1, z + 1, i * 6));
        }
        w.rtTick(30, 2);
        t.rtTick(30, 2);
        c.rtTick(30, 2);
        k.rtTick(30, 2);
        ch.rtTick(30, 2);
        for (int i = 0; i < 4; i++) {
            int x = 2 + 2 * i, z = 3 + (i % 3) * 4;
            w.set(x, 106, z, "minecraft:air");
            t.set(x, 103, z, "minecraft:air");
            c.set(x, 106, z, "minecraft:air");
            ch.set(x, 102, z, "minecraft:air");
            k.set(2 + 4 * i, 103, 2 + 7 * (i % 2), "minecraft:water");
        }
        out.add(w.tick(8).reseed(5001).rtTick(70, 2));
        out.add(t.tick(8).reseed(5002).rtTick(70, 2));
        out.add(c.tick(8).reseed(5003).rtTick(70, 2));
        out.add(k.tick(8).reseed(5004).rtTick(70, 2));
        out.add(ch.tick(8).reseed(5005).rtTick(70, 2));
        // vines over a wall lose pieces of it
        Sc v = new Sc("growth_cut_vines", 415).cmd(
                "fill ~0 ~ ~0 ~15 ~10 ~0 minecraft:stone", "fill ~0 ~10 ~0 ~15 ~10 ~8 minecraft:stone",
                "setblock ~4 ~5 ~1 minecraft:vine[north=true]", "setblock ~10 ~7 ~1 minecraft:vine[north=true]", "setblock ~13 ~3 ~1 minecraft:vine[north=true]",
                "setblock ~7 ~9 ~1 minecraft:vine[north=true,up=true]").rt(80);
        v.set(4, 105, 0, "minecraft:air").set(5, 105, 0, "minecraft:air").set(10, 107, 0, "minecraft:air").set(13, 103, 0, "minecraft:air")
                .set(7, 104, 0, "minecraft:air").set(8, 106, 0, "minecraft:air").set(9, 109, 0, "minecraft:air").tick(6).reseed(5006).rt(80)
                .set(2, 102, 0, "minecraft:air").set(11, 101, 0, "minecraft:air").set(12, 108, 0, "minecraft:air").set(6, 107, 0, "minecraft:air").tick(6).reseed(5007).rt(60);
        out.add(v);
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

    // ================================================================ family: end (wp44)
    // end portal frames: an eye of ender used on a frame (comparators, the ring that opens a portal)

    /** The twelve frames of a ring around the interior centred at area-relative (cx, cz), y 100, as setup commands; `eyes` says which have their eye, `wrong` faces the other way, `lowered` stands a block lower. */
    static List<String> ringFrames(int cx, int cz, boolean[] eyes, int wrong, int lowered) {
        String[] facing = {"south", "south", "south", "north", "north", "north", "east", "east", "east", "west", "west", "west"};
        java.util.Map<String, String> opposite = java.util.Map.of("south", "north", "north", "south", "east", "west", "west", "east");
        List<String> cmds = new ArrayList<>();
        for (int i = 0; i < 12; i++) {
            int[] at = frameAt(cx, cz, i);
            String f = i == wrong ? opposite.get(facing[i]) : facing[i];
            cmds.add(String.format("setblock ~%d ~%d ~%d minecraft:end_portal_frame[facing=%s,eye=%s]", at[0], i == lowered ? -1 : 0, at[1], f, eyes[i]));
        }
        return cmds;
    }

    static int[] frameAt(int cx, int cz, int i) {
        int g = i / 3, k = i % 3 - 1;
        return switch (g) {
            case 0 -> new int[] {cx + k, cz - 2};
            case 1 -> new int[] {cx + k, cz + 2};
            case 2 -> new int[] {cx - 2, cz + k};
            default -> new int[] {cx + 2, cz + k};
        };
    }

    static void scenariosEnd(List<Sc> out) {
        String[] interiors = {"air", "stone", "water", "short_grass", "torch", "lava", "oak_slab", "cobweb"};
        // Each of the twelve frames being the last to be filled, over different interiors.
        for (int last = 0; last < 12; last++) {
            boolean[] eyes = new boolean[12];
            java.util.Arrays.fill(eyes, true);
            eyes[last] = false;
            Sc sc = new Sc("end_ring/last" + last, 70 + last);
            for (String c : ringFrames(8, 8, eyes, -1, -1)) sc.cmd(c);
            String inside = interiors[last % interiors.length];
            sc.cmd("fill ~7 ~ ~7 ~9 ~ ~9 minecraft:" + inside);
            int[] f = frameAt(8, 8, last);
            sc.eye(f[0], 100, f[1]).tick(5);
            out.add(sc);
        }
        // Not a portal: a frame faces the wrong way / is a block lower / lacks its eye / eye already in / not a frame.
        for (int bad = 0; bad < 4; bad++) {
            for (int k = 0; k < 3; k++) {
                boolean[] eyes = new boolean[12];
                java.util.Arrays.fill(eyes, true);
                int last = 1 + k * 4;
                if (bad < 3) eyes[last] = false;
                int broken = (last + 5) % 12;
                Sc sc = new Sc("end_ring/broken" + bad + "_" + k, 90 + bad * 3 + k);
                List<String> frames = switch (bad) {
                    case 0 -> ringFrames(8, 8, eyes, broken, -1);
                    case 1 -> ringFrames(8, 8, eyes, -1, broken);
                    case 2 -> {
                        eyes[broken] = false;
                        yield ringFrames(8, 8, eyes, -1, -1);
                    }
                    default -> ringFrames(8, 8, eyes, -1, -1);
                };
                for (String c : frames) sc.cmd(c);
                int[] f = frameAt(8, 8, last);
                sc.eye(f[0], 100, f[1]);
                if (bad == 3) sc.eye(f[0], 100, f[1]);
                if (bad == 3) {
                    // On the filled frame again and on a block that is no frame.
                    sc.eye(8, 99, 8);
                }
                sc.tick(3);
                out.add(sc);
            }
        }
        // Comparators beside a frame read 15 once the eye is in, and 0 before.
        for (int k = 0; k < 4; k++) {
            Sc sc = new Sc("end_comparator/" + k, 120 + k);
            sc.cmd("setblock ~8 ~ ~8 minecraft:end_portal_frame[facing=south,eye=false]");
            String[] dirs = {"east", "west", "south", "north"};
            int[][] off = {{1, 0}, {-1, 0}, {0, 1}, {0, -1}};
            int[] o = off[k];
            sc.cmd(String.format("setblock ~%d ~ ~%d minecraft:comparator[facing=%s]", 8 + o[0], 8 + o[1], dirs[k]));
            sc.cmd(String.format("setblock ~%d ~ ~%d minecraft:redstone_wire", 8 + 2 * o[0], 8 + 2 * o[1]));
            if (k >= 2) {
                // Through a block (comparators read a container behind a conductor too).
                sc.cmd(String.format("setblock ~%d ~ ~%d minecraft:redstone_lamp", 8 + 3 * o[0], 8 + 3 * o[1]));
            }
            sc.tick(4).eye(8, 100, 8).tick(4);
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
        var area = new net.minecraft.world.level.levelgen.structure.BoundingBox(x0 + LO, FLOOR, z0 + LO, x0 + HI, Y0 + 47, z0 + HI);
        level.getBlockTicks().clearArea(area);
        level.getFluidTicks().clearArea(area);
        // Two fills (a fill is limited to 32768 blocks): the window up to the tallest scenario's top.
        command(String.format("fill %d %d %d %d %d %d minecraft:air", x0 + LO, FLOOR, z0 + LO, x0 + HI, Y0 + 23, z0 + HI));
        command(String.format("fill %d %d %d %d %d %d minecraft:air", x0 + LO, Y0 + 24, z0 + LO, x0 + HI, Y0 + 47, z0 + HI));
        curHeight = sc.height;
        if (!sc.biome.equals(curBiome)) {
            command(String.format("fillbiome %d %d %d %d %d %d %s", x0 + LO - 4, FLOOR - 7, z0 + LO - 4, x0 + HI + 4, FLOOR + 12, z0 + HI + 4, sc.biome));
            curBiome = sc.biome;
        }
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
                case "eye" -> useEye(new BlockPos(x0 + (Integer) op[1], (Integer) op[2], z0 + (Integer) op[3]));
                case "set" -> {
                    BlockPos p = new BlockPos(x0 + (Integer) op[1], (Integer) op[2], z0 + (Integer) op[3]);
                    var parsed = BlockStateParser.parseForBlock(level.registryAccess().lookupOrThrow(net.minecraft.core.registries.Registries.BLOCK), (String) op[4], false);
                    level.setBlock(p, parsed.blockState(), 3);
                }
                case "seed" -> level.getRandom().setSeed((Long) op[1]);
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

    static net.minecraft.server.level.ServerPlayer fakePlayer;

    /** `ItemStack.useOn` of an eye of ender clicked on the top of the block at `pos` by a survival player standing four blocks above it. */
    static void useEye(BlockPos pos) {
        if (fakePlayer == null) {
            var profile = new com.mojang.authlib.GameProfile(java.util.UUID.nameUUIDFromBytes(new byte[] {1}), "Kiln");
            fakePlayer = new net.minecraft.server.level.ServerPlayer(server, level, profile, net.minecraft.server.level.ClientInformation.createDefault());
        }
        fakePlayer.setPos(pos.getX() + 0.5, pos.getY() + 4, pos.getZ() + 0.5);
        var hand = net.minecraft.world.InteractionHand.MAIN_HAND;
        var stack = new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.ENDER_EYE);
        fakePlayer.setItemInHand(hand, stack);
        var hit = new net.minecraft.world.phys.BlockHitResult(net.minecraft.world.phys.Vec3.atCenterOf(pos).add(0, 0.5, 0), net.minecraft.core.Direction.UP, pos, false);
        stack.useOn(new net.minecraft.world.item.context.UseOnContext(fakePlayer, hand, hit));
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
        for (int y = FLOOR; y < Y0 + curHeight; y++)
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
        for (int y = Y0; y < Y0 + curHeight; y++)
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
        for (int i = 0; i < 3000; i++) {
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
