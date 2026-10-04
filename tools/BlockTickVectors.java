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
                case "seed" -> level.getRandom().setSeed((Long) op[1]);
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
