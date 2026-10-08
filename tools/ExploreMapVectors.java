// Differential test vectors for exploration maps (wp49): in a real vanilla 26.3 dedicated server with a generated
// world, `ServerLevel.findNearestMapStructure` finds the nearest structure of a kind around a point and the map
// for it is made as `ExplorationMapFunction` does (`MapItem.applyNewSavedData`, `renderBiomePreviewMap`,
// `addTargetDecoration`). One line per (structure tag, origin, zoom): where the structure is, the map's centre,
// colours and decorations, and the item's `minecraft:map_decorations` component.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/ExploreMapVectors.java <out.jsonl> [seed]

import io.netty.buffer.ByteBufUtil;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderSet;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.tags.TagKey;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.MapItem;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.saveddata.maps.MapDecorationTypes;
import net.minecraft.world.level.saveddata.maps.MapId;
import net.minecraft.world.level.saveddata.maps.MapItemSavedData;

public class ExploreMapVectors {
    static MinecraftServer server;

    public static void main(String[] args) throws Exception {
        Path out = Path.of(args[0]).toAbsolutePath();
        String seed = args.length > 1 ? args[1] : "12345";
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n", "server-port=25582", "online-mode=false", "level-name=world", "level-seed=" + seed,
                "level-type=minecraft\\:normal", "generate-structures=true", "max-tick-time=-1", "view-distance=3", "simulation-distance=3", "sync-chunk-writes=false",
                "enable-rcon=false", "enable-query=false", "spawn-monsters=false", "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "ExploreMapVectors main");
        main.start();
        server = awaitServer();
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            try {
                ServerLevel level = server.overworld();
                String[][] cases = {
                    {"minecraft:on_ocean_monument_maps", "monument", "2", "0", "0", "100", "true"},
                    {"minecraft:on_woodland_mansion_maps", "mansion", "2", "0", "0", "100", "true"},
                    {"minecraft:on_treasure_maps", "red_x", "1", "0", "0", "50", "false"},
                    {"minecraft:on_treasure_maps", "red_x", "1", "700", "-300", "50", "false"},
                    {"minecraft:on_buried_trial_chambers_maps", "trial_chambers", "2", "0", "0", "100", "true"},
                    {"minecraft:on_plains_village_maps", "village_plains", "2", "100", "100", "100", "true"},
                    {"minecraft:on_desert_village_maps", "village_desert", "2", "0", "0", "100", "true"},
                    {"minecraft:on_swamp_hut_maps", "swamp_hut", "2", "0", "0", "100", "true"},
                    {"minecraft:on_jungle_pyramid_maps", "jungle_temple", "2", "0", "0", "100", "true"},
                    {"minecraft:on_taiga_village_maps", "village_taiga", "2", "0", "0", "100", "true"},
                    {"minecraft:on_mineshaft_maps", "mineshaft", "2", "0", "0", "100", "true"},
                };
                for (String[] c : cases) {
                    TagKey<Structure> tag = TagKey.create(Registries.STRUCTURE, Identifier.parse(c[0]));
                    var set = level.registryAccess().lookupOrThrow(Registries.STRUCTURE).get(tag);
                    if (set.isEmpty()) {
                        lines.add("{\"tag\":\"" + c[0] + "\",\"error\":\"no such tag\"}");
                        continue;
                    }
                    BlockPos origin = new BlockPos(Integer.parseInt(c[3]), 64, Integer.parseInt(c[4]));
                    BlockPos found = level.findNearestMapStructure(set.get(), origin, Integer.parseInt(c[5]), Boolean.parseBoolean(c[6]));
                    if (found == null) {
                        lines.add("{\"tag\":\"" + c[0] + "\",\"origin\":[" + c[3] + "," + c[4] + "],\"found\":null}");
                        continue;
                    }
                    ItemStack stack = new ItemStack(Items.FILLED_MAP);
                    MapItem.applyNewSavedData(level, stack, found.getX(), found.getZ(), (byte) Integer.parseInt(c[2]), true, true);
                    MapItem.renderBiomePreviewMap(level, stack);
                    var deco = BuiltInRegistries.MAP_DECORATION_TYPE.get(Identifier.parse("minecraft:" + c[1])).orElseThrow();
                    MapItemSavedData.addTargetDecoration(stack, found, "+", deco);
                    MapId id = stack.get(DataComponents.MAP_ID);
                    MapItemSavedData data = level.getMapData(id);
                    var d = stack.get(DataComponents.MAP_DECORATIONS).decorations().get("+");
                    lines.add("{\"tag\":\"" + c[0] + "\",\"decoration\":\"" + c[1] + "\",\"zoom\":" + c[2] + ",\"radius\":" + c[5] + ",\"skip\":" + c[6] + ",\"origin\":[" + c[3] + "," + c[4] + "],\"found\":["
                            + found.getX() + "," + found.getY() + "," + found.getZ() + "],\"center\":[" + data.centerX + "," + data.centerZ + "],\"colors\":\""
                            + ByteBufUtil.hexDump(data.colors) + "\",\"component\":[" + BuiltInRegistries.MAP_DECORATION_TYPE.getId(d.type().value()) + "," + d.x() + ","
                            + d.z() + "," + d.rotation() + "]}");
                }
            } catch (Throwable t) {
                t.printStackTrace();
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("ExploreMapVectors: wrote " + lines.size() + " cases to " + out);
        server.halt(false);
        System.exit(0);
    }

    static MinecraftServer awaitServer() throws Exception {
        for (int i = 0; i < 6000; i++) {
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
}
