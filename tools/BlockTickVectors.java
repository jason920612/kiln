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
        // The clock starts before the setup: ticks it schedules are due from START_TIME on (set after,
        // they would wait out the previous scenario's length on top of their delay).
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
