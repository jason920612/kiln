// wp75: what vanilla 26.3 makes of saved data with items it cannot load (unknown ids, counts out of range, slots outside the container, components it cannot read):
// block entities (BlockEntity.loadStatic, saveWithFullMetadata) and entities (EntityType.loadEntityRecursive, Entity.save), as the compound it saves. Built on EntityNbtVectors.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/LoadDropVectors.java <out.jsonl> [name-filter]

import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;

public class LoadDropVectors {
    record Case(String name, String kind, String type, String nbt) {}

    static String it(int slot, String id, int count) {
        return "{Slot:" + slot + "b,id:\"" + id + "\",count:" + count + "}";
    }

    static List<Case> cases() {
        List<Case> c = new ArrayList<>();
        String unknown = "minecraft:no_such_item";
        // ---- block entities: Items lists
        String[] boxes = {"chest", "barrel", "hopper", "dispenser", "dropper", "trapped_chest", "shulker_box", "furnace", "blast_furnace", "smoker", "brewing_stand", "crafter", "campfire", "chiseled_bookshelf"};
        for (String b : boxes) {
            c.add(new Case("be_" + b + "_unknown_item", "block", b, "{Items:[" + it(0, "minecraft:stone", 3) + "," + it(1, unknown, 1) + "," + it(2, "minecraft:dirt", 2) + "]}"));
            c.add(new Case("be_" + b + "_bad_counts", "block", b, "{Items:[" + it(0, "minecraft:stone", 0) + "," + it(1, "minecraft:stone", 65) + "," + it(2, "minecraft:iron_sword", 2) + "," + it(3, "minecraft:stone", -1) + "," + it(4, "minecraft:apple", 64) + "]}"));
            c.add(new Case("be_" + b + "_bad_slots", "block", b, "{Items:[" + it(0, "minecraft:stone", 1) + "," + it(100, "minecraft:dirt", 1) + ",{id:\"minecraft:apple\",count:2}," + it(-1, "minecraft:stick", 1) + ",{Slot:2b,count:1}," + it(2, "minecraft:dirt", 4) + "]}"));
            c.add(new Case("be_" + b + "_missing_count", "block", b, "{Items:[{Slot:0b,id:\"minecraft:stone\"},{Slot:1b,id:\"minecraft:dirt\",count:5}]}"));
            c.add(new Case("be_" + b + "_unknown_component", "block", b, "{Items:[{Slot:0b,id:\"minecraft:stone\",count:1,components:{\"minecraft:no_such_component\":1}},{Slot:1b,id:\"minecraft:dirt\",count:1,components:{\"minecraft:custom_name\":\"x\"}},{Slot:2b,id:\"minecraft:dirt\",count:1,components:{\"minecraft:damage\":\"x\"}}]}"));
            c.add(new Case("be_" + b + "_bundle", "block", b, "{Items:[{Slot:0b,id:\"minecraft:bundle\",count:1,components:{\"minecraft:bundle_contents\":[{id:\"" + unknown + "\",count:1},{id:\"minecraft:stone\",count:2}]}}," + it(1, "minecraft:apple", 1) + "]}"));
            c.add(new Case("be_" + b + "_shulker_inside", "block", b, "{Items:[{Slot:0b,id:\"minecraft:shulker_box\",count:1,components:{\"minecraft:container\":[{slot:0,item:{id:\"" + unknown + "\",count:1}},{slot:1,item:{id:\"minecraft:stone\",count:1}}]}}," + it(1, "minecraft:apple", 1) + "]}"));
        }
        c.add(new Case("be_chest_lootable", "block", "chest", "{LootTable:\"minecraft:chests/simple_dungeon\",LootTableSeed:5L,Items:[" + it(0, unknown, 1) + "]}"));
        // Single item block entities.
        c.add(new Case("be_decorated_pot_unknown", "block", "decorated_pot", "{item:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("be_decorated_pot_components", "block", "decorated_pot", "{components:{\"minecraft:pot_decorations\":{back:\"minecraft:brick\",left:{id:\"minecraft:arms_up_pottery_sherd\"},right:\"minecraft:brick\",front:\"minecraft:blade_pottery_sherd\"}}}"));
        c.add(new Case("be_decorated_pot_sherds", "block", "decorated_pot", "{sherds:{back:\"minecraft:brick\",left:{id:\"minecraft:arms_up_pottery_sherd\"},right:\"minecraft:brick\",front:\"minecraft:blade_pottery_sherd\"}}"));
        c.add(new Case("be_decorated_pot_old_list", "block", "decorated_pot", "{sherds:[\"minecraft:brick\",\"minecraft:brick\",\"minecraft:brick\",\"minecraft:brick\"]}"));
        c.add(new Case("be_decorated_pot_ok", "block", "decorated_pot", "{item:{id:\"minecraft:stone\",count:2}}"));
        c.add(new Case("be_decorated_pot_zero", "block", "decorated_pot", "{item:{id:\"minecraft:stone\",count:0}}"));
        c.add(new Case("be_jukebox_unknown", "block", "jukebox", "{RecordItem:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("be_jukebox_disc", "block", "jukebox", "{RecordItem:{id:\"minecraft:music_disc_13\",count:1}}"));
        c.add(new Case("be_lectern_unknown", "block", "lectern[facing=north,has_book=true]", "{Book:{id:\"" + unknown + "\",count:1},Page:2}"));
        c.add(new Case("be_lectern_not_a_book", "block", "lectern[facing=north,has_book=true]", "{Book:{id:\"minecraft:stone\",count:1},Page:2}"));
        c.add(new Case("be_lectern_book", "block", "lectern[facing=north,has_book=true]", "{Book:{id:\"minecraft:writable_book\",count:1},Page:0}"));
        // ---- entities
        c.add(new Case("e_item_unknown", "entity", "item", "{Item:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("e_item_ok", "entity", "item", "{Item:{id:\"minecraft:stone\",count:7},Age:5s}"));
        c.add(new Case("e_item_zero", "entity", "item", "{Item:{id:\"minecraft:stone\",count:0}}"));
        c.add(new Case("e_item_too_many", "entity", "item", "{Item:{id:\"minecraft:stone\",count:99}}"));
        c.add(new Case("e_item_missing", "entity", "item", "{Age:5s}"));
        c.add(new Case("e_item_bundle_unknown", "entity", "item", "{Item:{id:\"minecraft:bundle\",count:1,components:{\"minecraft:bundle_contents\":[{id:\"" + unknown + "\",count:1},{id:\"minecraft:stone\",count:2}]}}}"));
        c.add(new Case("e_item_frame_unknown", "entity", "item_frame", "{Item:{id:\"" + unknown + "\",count:1},ItemRotation:3b,Facing:1b,TileX:1,TileY:70,TileZ:1}"));
        c.add(new Case("e_item_frame_ok", "entity", "item_frame", "{Item:{id:\"minecraft:stone\",count:1},ItemRotation:3b,Facing:1b,TileX:1,TileY:70,TileZ:1}"));
        c.add(new Case("e_glow_item_frame_unknown", "entity", "glow_item_frame", "{Item:{id:\"" + unknown + "\",count:1},Facing:1b,TileX:1,TileY:70,TileZ:1}"));
        c.add(new Case("e_armor_stand_unknown_head", "entity", "armor_stand", "{equipment:{head:{id:\"" + unknown + "\",count:1},chest:{id:\"minecraft:iron_chestplate\",count:1}}}"));
        c.add(new Case("e_armor_stand_unknown_hand", "entity", "armor_stand", "{equipment:{mainhand:{id:\"" + unknown + "\",count:1}}}"));
        c.add(new Case("e_armor_stand_ok", "entity", "armor_stand", "{equipment:{head:{id:\"minecraft:iron_helmet\",count:1},mainhand:{id:\"minecraft:stick\",count:1}}}"));
        c.add(new Case("e_zombie_unknown_head", "entity", "zombie", "{equipment:{head:{id:\"" + unknown + "\",count:1},mainhand:{id:\"minecraft:iron_sword\",count:1}}}"));
        c.add(new Case("e_zombie_ok_equipment", "entity", "zombie", "{equipment:{head:{id:\"minecraft:iron_helmet\",count:1},mainhand:{id:\"minecraft:iron_sword\",count:1}}}"));
        c.add(new Case("e_zombie_drop_chances", "entity", "zombie", "{equipment:{head:{id:\"" + unknown + "\",count:1}},drop_chances:{head:1.0f}}"));
        c.add(new Case("e_skeleton_unknown_mainhand", "entity", "skeleton", "{equipment:{mainhand:{id:\"" + unknown + "\",count:1}}}"));
        c.add(new Case("e_horse_unknown_saddle", "entity", "horse", "{equipment:{saddle:{id:\"" + unknown + "\",count:1},body:{id:\"minecraft:iron_horse_armor\",count:1}}}"));
        c.add(new Case("e_horse_unknown_armor", "entity", "horse", "{equipment:{saddle:{id:\"minecraft:saddle\",count:1},body:{id:\"" + unknown + "\",count:1}}}"));
        c.add(new Case("e_donkey_chest_unknown", "entity", "donkey", "{ChestedHorse:1b,Items:[" + it(2, unknown, 1) + "," + it(3, "minecraft:stone", 4) + "]}"));
        c.add(new Case("e_llama_unknown", "entity", "llama", "{ChestedHorse:1b,Strength:3,Items:[" + it(2, unknown, 1) + "," + it(3, "minecraft:stone", 4) + "],equipment:{body:{id:\"" + unknown + "\",count:1}}}"));
        c.add(new Case("e_villager_inventory_unknown", "entity", "villager", "{Inventory:[{id:\"" + unknown + "\",count:2},{id:\"minecraft:bread\",count:3}]}"));
        c.add(new Case("e_villager_offer_unknown", "entity", "villager", "{Offers:{Recipes:[{buy:{id:\"" + unknown + "\",count:1},sell:{id:\"minecraft:emerald\",count:1},maxUses:4,uses:0},{buy:{id:\"minecraft:wheat\",count:20},sell:{id:\"minecraft:emerald\",count:1},maxUses:4,uses:0}]}}"));
        c.add(new Case("e_allay_inventory_unknown", "entity", "allay", "{Inventory:[{id:\"" + unknown + "\",count:1},{id:\"minecraft:stone\",count:1}]}"));
        c.add(new Case("e_chest_minecart_unknown", "entity", "chest_minecart", "{Items:[" + it(0, unknown, 1) + "," + it(1, "minecraft:stone", 2) + "]}"));
        c.add(new Case("e_hopper_minecart_unknown", "entity", "hopper_minecart", "{Items:[" + it(0, unknown, 1) + "," + it(1, "minecraft:stone", 2) + "]}"));
        c.add(new Case("e_oak_chest_boat_unknown", "entity", "oak_chest_boat", "{Items:[" + it(0, unknown, 1) + "," + it(1, "minecraft:stone", 2) + "]}"));
        c.add(new Case("e_fox_mouth_unknown", "entity", "fox", "{equipment:{mainhand:{id:\"" + unknown + "\",count:1}}}"));
        c.add(new Case("e_wolf_body_unknown", "entity", "wolf", "{equipment:{body:{id:\"" + unknown + "\",count:1}}}"));
        c.add(new Case("e_snowball_unknown", "entity", "snowball", "{Item:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("e_snowball_ok", "entity", "snowball", "{Item:{id:\"minecraft:snowball\",count:1}}"));
        c.add(new Case("e_splash_potion_unknown", "entity", "splash_potion", "{Item:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("e_arrow_unknown", "entity", "arrow", "{item:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("e_arrow_weapon_unknown", "entity", "arrow", "{weapon:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("e_trident_unknown", "entity", "trident", "{item:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("e_firework_unknown", "entity", "firework_rocket", "{FireworksItem:{id:\"" + unknown + "\",count:1}}"));
        c.add(new Case("e_eye_of_ender_unknown", "entity", "eye_of_ender", "{Item:{id:\"" + unknown + "\",count:1}}"));
        // ---- the block entity data of /setblock (wp75)
        c.add(new Case("setblock_pot_sherds", "setblock", "decorated_pot", "{sherds:{back:\"minecraft:brick\",left:{id:\"minecraft:arms_up_pottery_sherd\"},right:\"minecraft:brick\",front:\"minecraft:blade_pottery_sherd\"}}"));
        c.add(new Case("setblock_pot_components", "setblock", "decorated_pot", "{components:{\"minecraft:pot_decorations\":{back:\"minecraft:brick\",left:{id:\"minecraft:arms_up_pottery_sherd\"},right:\"minecraft:brick\",front:\"minecraft:blade_pottery_sherd\"}}}"));
        c.add(new Case("setblock_pot_old_list", "setblock", "decorated_pot", "{sherds:[\"minecraft:brick\",\"minecraft:brick\",\"minecraft:brick\",\"minecraft:brick\"]}"));
        c.add(new Case("setblock_chest_components", "setblock", "chest", "{components:{\"minecraft:custom_name\":\"x\"}}"));
        c.add(new Case("setblock_sign_components", "setblock", "oak_sign", "{components:{\"minecraft:custom_name\":\"x\"}}"));
        c.add(new Case("setblock_chest_unknown_item", "setblock", "chest", "{Items:[{Slot:0b,id:\"minecraft:no_such_item\",count:1},{Slot:1b,id:\"minecraft:stone\",count:2}]}"));
        // ---- neutral mobs: the anger they save (wp75)
        for (String t : new String[] {"wolf", "iron_golem", "polar_bear", "bee", "enderman", "zombified_piglin"}) {
            c.add(new Case("anger_" + t + "_both", "entity", t, "{anger_end_time:123456L,angry_at:[I;1,2,3,4]}"));
            c.add(new Case("anger_" + t + "_target_only", "entity", t, "{angry_at:[I;5,6,7,8]}"));
            c.add(new Case("anger_" + t + "_time_only", "entity", t, "{anger_end_time:99L}"));
            c.add(new Case("anger_" + t + "_none", "entity", t, "{}"));
            c.add(new Case("anger_" + t + "_bad_target", "entity", t, "{anger_end_time:5L,angry_at:[I;1,2]}"));
        }
        return c;
    }

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
        String filter = args.length > 1 ? args[1] : null;
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "LoadDropVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        // (The `setblock` cases need a loaded chunk, which needs the server thread free for a moment.)
        server.submit(() -> {
            ServerLevel l = server.overworld();
            l.setChunkForced(0, 0, true);
            l.getChunk(0, 0);
        }).get();
        Thread.sleep(3000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            ServerLevel level = server.overworld();
            level.tickRateManager().setFrozen(true);
            // A scoreboard for the text displays that read one.
            var src = server.createCommandSourceStack();
            server.getCommands().performPrefixedCommand(src, "scoreboard objectives add o dummy");
            for (Case c : cases()) {
                if (filter != null && !c.name.matches(filter) && !c.name.contains(filter)) continue;
                try {
                    lines.add(run(level, c));
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("{\"name\":\"" + c.name + "\",\"error\":\"" + t.toString().replace('"', '\'').replace('\\', '/') + "\"}");
                }
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("LoadDropVectors: wrote " + lines.size() + " cases to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static String run(ServerLevel level, Case c) throws Exception {
        net.minecraft.nbt.CompoundTag tag = net.minecraft.nbt.TagParser.parseCompoundFully(c.nbt);
        if (c.kind.equals("setblock")) {
            // The command, then what the block entity saves (the platform is air above y 64 at the chunk's middle).
            var pos = new net.minecraft.core.BlockPos(8, 64, 8);
            var server = level.getServer();
            level.setBlock(pos, net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(), 3);
            server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), "setblock 8 64 8 minecraft:" + c.type + c.nbt);
            var be = level.getBlockEntity(pos);
            String line;
            if (be == null) {
                line = String.format(Locale.ROOT, "{\"name\":\"%s\",\"kind\":\"setblock\",\"type\":\"%s\",\"loaded\":false}", c.name, c.type);
            } else {
                line = String.format(Locale.ROOT, "{\"name\":\"%s\",\"kind\":\"setblock\",\"type\":\"%s\",\"command\":\"%s\",\"loaded\":true,\"saved\":%s}", c.name, c.type, ("setblock 8 64 8 minecraft:" + c.type + c.nbt).replace("\\", "\\\\").replace("\"", "\\\""), tagJson(be.saveWithFullMetadata(level.registryAccess())));
            }
            level.setBlock(pos, net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(), 3);
            return line;
        }
        if (c.kind.equals("block")) {
            net.minecraft.world.level.block.state.BlockState state = net.minecraft.commands.arguments.blocks.BlockStateParser
                    .parseForBlock(net.minecraft.core.registries.BuiltInRegistries.BLOCK, "minecraft:" + c.type, false).blockState();
            var probe = ((net.minecraft.world.level.block.EntityBlock) state.getBlock()).newBlockEntity(net.minecraft.core.BlockPos.ZERO, state);
            tag.putString("id", net.minecraft.core.registries.BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(probe.getType()).toString());
            tag.putInt("x", 0);
            tag.putInt("y", 64);
            tag.putInt("z", 0);
            net.minecraft.world.level.block.entity.BlockEntity be = net.minecraft.world.level.block.entity.BlockEntity.loadStatic(new net.minecraft.core.BlockPos(0, 64, 0), state, tag, level.registryAccess());
            if (be == null) {
                return String.format(Locale.ROOT, "{\"name\":\"%s\",\"kind\":\"block\",\"type\":\"%s\",\"nbt\":%s,\"loaded\":false}", c.name, c.type, tagJson(tag));
            }
            net.minecraft.nbt.CompoundTag saved = be.saveWithFullMetadata(level.registryAccess());
            return String.format(Locale.ROOT, "{\"name\":\"%s\",\"kind\":\"block\",\"type\":\"%s\",\"nbt\":%s,\"loaded\":true,\"saved\":%s}", c.name, c.type, tagJson(tag), tagJson(saved));
        }
        tag.putString("id", "minecraft:" + c.type);
        var in = net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess(), tag);
        Entity e = EntityType.loadEntityRecursive(in, level, EntitySpawnReason.COMMAND, x -> x);
        if (e == null) {
            return String.format(Locale.ROOT, "{\"name\":\"%s\",\"kind\":\"entity\",\"type\":\"minecraft:%s\",\"nbt\":%s,\"loaded\":false}", c.name, c.type, tagJson(tag));
        }
        var out = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess());
        e.save(out);
        net.minecraft.nbt.CompoundTag saved = out.buildResult();
        saved.remove("UUID");
        return String.format(Locale.ROOT, "{\"name\":\"%s\",\"kind\":\"entity\",\"type\":\"minecraft:%s\",\"nbt\":%s,\"loaded\":true,\"saved\":%s,\"removed\":%b}", c.name, c.type, tagJson(tag), tagJson(saved), e.isRemoved());
    }

    static String d(double v) {
        return Double.toString(v);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + (System.getenv("KILN_HARNESS_PORT") != null ? System.getenv("KILN_HARNESS_PORT") : "25581"),
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:plains\"}",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "spawn-monsters=false",
                "spawn-animals=false",
                "generate-structures=false",
                "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
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

    static String tagJson(net.minecraft.nbt.Tag t) {
        if (t instanceof net.minecraft.nbt.CompoundTag c) {
            StringBuilder sb = new StringBuilder("{\"c\":{");
            boolean first = true;
            for (String k : c.keySet()) {
                if (!first) sb.append(',');
                first = false;
                sb.append('"').append(k).append("\":").append(tagJson(c.get(k)));
            }
            return sb.append("}}").toString();
        }
        if (t instanceof net.minecraft.nbt.ListTag l) {
            StringBuilder sb = new StringBuilder("{\"l\":[");
            for (int i = 0; i < l.size(); i++) {
                if (i > 0) sb.append(',');
                sb.append(tagJson(l.get(i)));
            }
            return sb.append("]}").toString();
        }
        if (t instanceof net.minecraft.nbt.ByteTag b) return "{\"b\":" + b.byteValue() + "}";
        if (t instanceof net.minecraft.nbt.ShortTag s) return "{\"s\":" + s.shortValue() + "}";
        if (t instanceof net.minecraft.nbt.IntTag i) return "{\"i\":" + i.intValue() + "}";
        if (t instanceof net.minecraft.nbt.LongTag g) return "{\"L\":\"" + g.longValue() + "\"}";
        if (t instanceof net.minecraft.nbt.FloatTag f) return "{\"f\":" + Float.toString(f.floatValue()) + "}";
        if (t instanceof net.minecraft.nbt.DoubleTag dd) return "{\"d\":" + Double.toString(dd.doubleValue()) + "}";
        if (t instanceof net.minecraft.nbt.StringTag st) return "{\"str\":\"" + st.value().replace("\\", "\\\\").replace("\"", "\\\"") + "\"}";
        if (t instanceof net.minecraft.nbt.IntArrayTag ia) return "{\"ia\":" + java.util.Arrays.toString(ia.getAsIntArray()) + "}";
        if (t instanceof net.minecraft.nbt.ByteArrayTag ba) return "{\"ba\":" + java.util.Arrays.toString(ba.getAsByteArray()) + "}";
        if (t instanceof net.minecraft.nbt.LongArrayTag la) {
            StringBuilder sb = new StringBuilder("{\"la\":[");
            long[] v = la.getAsLongArray();
            for (int i = 0; i < v.length; i++) sb.append(i > 0 ? "," : "").append('"').append(v[i]).append('"');
            return sb.append("]}").toString();
        }
        throw new IllegalArgumentException("tag " + t);
    }
}
