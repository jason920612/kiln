// Differential test vectors for entities that are data and little else (wp49): display entities, markers,
// interactions, mannequins. A real vanilla 26.3 dedicated server is started in-process; every case is an
// entity read from a compound the way `/summon` and the chunk loader read it (`EntityType.loadEntityRecursive`),
// then recorded as it saves (`Entity.save`) and as its viewers get it (the non-default synched data values, each
// as its network encoding).
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/EntityNbtVectors.java <out.jsonl> [name-filter]

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

public class EntityNbtVectors {
    record Case(String name, String type, String nbt) {}

    static List<Case> cases() {
        List<Case> c = new ArrayList<>();
        // ---- mannequins (wp50)
        c.add(new Case("mannequin_default", "mannequin", "{}"));
        c.add(new Case("mannequin_profile_name_string", "mannequin", "{profile:\"Notch\"}"));
        c.add(new Case("mannequin_profile_name", "mannequin", "{profile:{name:\"Steve\"}}"));
        c.add(new Case("mannequin_profile_id", "mannequin", "{profile:{id:[I;1,2,3,4]}}"));
        c.add(new Case("mannequin_profile_name_id", "mannequin", "{profile:{name:\"Alex\",id:[I;5,6,7,8]}}"));
        c.add(new Case("mannequin_profile_properties", "mannequin", "{profile:{name:\"Skin\",properties:[{name:\"textures\",value:\"abc\"}]}}"));
        c.add(new Case("mannequin_profile_properties_signed", "mannequin", "{profile:{name:\"Skin\",id:[I;1,1,1,1],properties:[{name:\"textures\",value:\"abc\",signature:\"sig\"},{name:\"x\",value:\"y\"}]}}"));
        c.add(new Case("mannequin_profile_properties_only", "mannequin", "{profile:{properties:[{name:\"textures\",value:\"abc\"}]}}"));
        c.add(new Case("mannequin_profile_texture", "mannequin", "{profile:{texture:\"minecraft:entity/player/wide/steve\"}}"));
        c.add(new Case("mannequin_profile_patch", "mannequin", "{profile:{name:\"P\",texture:\"minecraft:t\",cape:\"minecraft:c\",elytra:\"minecraft:e\",model:\"slim\"}}"));
        c.add(new Case("mannequin_profile_model_wide", "mannequin", "{profile:{model:\"wide\"}}"));
        c.add(new Case("mannequin_profile_empty", "mannequin", "{profile:{}}"));
        c.add(new Case("mannequin_profile_bad_name", "mannequin", "{profile:\"this name is far too long\"}"));
        c.add(new Case("mannequin_profile_bad_type", "mannequin", "{profile:5}"));
        c.add(new Case("mannequin_profile_bad_model", "mannequin", "{profile:{name:\"x\",model:\"huge\"}}"));
        c.add(new Case("mannequin_layers", "mannequin", "{hidden_layers:[\"cape\",\"hat\"]}"));
        c.add(new Case("mannequin_layers_all", "mannequin", "{hidden_layers:[\"cape\",\"jacket\",\"left_sleeve\",\"right_sleeve\",\"left_pants_leg\",\"right_pants_leg\",\"hat\"]}"));
        c.add(new Case("mannequin_layers_bad", "mannequin", "{hidden_layers:[\"cape\",\"nonsense\"]}"));
        c.add(new Case("mannequin_layers_dup", "mannequin", "{hidden_layers:[\"hat\",\"hat\"]}"));
        c.add(new Case("mannequin_main_hand_left", "mannequin", "{main_hand:\"left\"}"));
        c.add(new Case("mannequin_main_hand_bad", "mannequin", "{main_hand:\"middle\"}"));
        for (String pose : new String[] {"standing", "crouching", "swimming", "fall_flying", "sleeping", "dying", "spin_attack", "sitting", "long_jumping"}) {
            c.add(new Case("mannequin_pose_" + pose, "mannequin", "{pose:\"" + pose + "\"}"));
        }
        c.add(new Case("mannequin_immovable", "mannequin", "{immovable:1b}"));
        c.add(new Case("mannequin_description", "mannequin", "{description:'\"Guard\"'}"));
        c.add(new Case("mannequin_description_translatable", "mannequin", "{description:{translate:\"entity.minecraft.mannequin.label\"}}"));
        c.add(new Case("mannequin_description_hidden", "mannequin", "{hide_description:1b,description:'\"Guard\"'}"));
        c.add(new Case("mannequin_description_bad", "mannequin", "{description:5}"));
        c.add(new Case("mannequin_living", "mannequin", "{Health:7.5f,HurtTime:3s,DeathTime:0s,AbsorptionAmount:2.0f,Pos:[1.5d,64.0d,-3.5d],Rotation:[90.0f,10.0f],CustomName:'\"Bob\"',CustomNameVisible:1b,Tags:[\"npc\"],NoGravity:1b,Silent:1b,Invulnerable:1b,Glowing:1b}"));
        c.add(new Case("mannequin_equipment", "mannequin", "{equipment:{head:{id:\"minecraft:diamond_helmet\",count:1},mainhand:{id:\"minecraft:diamond_sword\",count:1},offhand:{id:\"minecraft:shield\",count:1},chest:{id:\"minecraft:leather_chestplate\",count:1,components:{\"minecraft:dyed_color\":255}}}}"));
        c.add(new Case("mannequin_mob_fields", "mannequin", "{PersistenceRequired:1b,CanPickUpLoot:1b,LeftHanded:1b,NoAI:1b,drop_chances:{head:1.0f},home_pos:[1,2,3],home_radius:4}"));
        c.add(new Case("mannequin_effects", "mannequin", "{active_effects:[{id:\"minecraft:speed\",amplifier:1b,duration:200}],attributes:[{id:\"minecraft:max_health\",base:40.0d}],Health:40.0f}"));
        c.add(new Case("mannequin_health_over", "mannequin", "{Health:99.0f}"));
        c.add(new Case("mannequin_health_zero", "mannequin", "{Health:0.0f}"));
        c.add(new Case("mannequin_air_fire", "mannequin", "{Air:100s,Fire:50s,FallDistance:3.5f,OnGround:1b,Motion:[0.1d,0.2d,0.3d]}"));
        c.add(new Case("mannequin_everything", "mannequin", "{profile:{name:\"Boss\"},hidden_layers:[\"cape\"],main_hand:\"left\",pose:\"crouching\",immovable:1b,description:'\"Hi\"',Health:15.0f}"));
        // ---- markers
        c.add(new Case("marker_empty", "marker", "{}"));
        c.add(new Case("marker_data", "marker", "{data:{a:1b,b:\"x\",list:[1,2,3],nested:{c:2.5d}}}"));
        c.add(new Case("marker_pos", "marker", "{Pos:[1.5d,64.0d,-3.5d],CustomName:'\"Marker\"',Tags:[\"t\"]}"));
        // ---- interactions
        c.add(new Case("interaction_default", "interaction", "{}"));
        c.add(new Case("interaction_size", "interaction", "{width:2.5f,height:3.0f,response:1b}"));
        c.add(new Case("interaction_attack", "interaction", "{attack:{player:[I;1,2,3,4],timestamp:100L},interaction:{player:[I;5,6,7,8],timestamp:200L}}"));
        c.add(new Case("interaction_bad", "interaction", "{width:\"wide\",response:7b}"));
        // ---- display entities: the shared part
        String[] types = {"block_display", "item_display", "text_display"};
        for (String t : types) {
            c.add(new Case(t + "_default", t, "{}"));
            c.add(new Case(t + "_transformation_full", t, "{transformation:{translation:[1.0f,2.0f,3.0f],left_rotation:[0.0f,0.0f,0.0f,1.0f],scale:[2.0f,2.0f,2.0f],right_rotation:[0.0f,0.7071068f,0.0f,0.7071068f]}}"));
            c.add(new Case(t + "_transformation_partial", t, "{transformation:{scale:[3.0f,1.0f,1.0f]}}"));
            c.add(new Case(t + "_transformation_matrix", t, "{transformation:[1.0f,0.0f,0.0f,0.5f,0.0f,2.0f,0.0f,1.0f,0.0f,0.0f,1.0f,0.0f,0.0f,0.0f,0.0f,1.0f]}"));
            c.add(new Case(t + "_transformation_axis_angle", t, "{transformation:{translation:[0.0f,0.0f,0.0f],left_rotation:{axis:[0.0f,1.0f,0.0f],angle:1.5707964f},scale:[1.0f,1.0f,1.0f],right_rotation:[0.0f,0.0f,0.0f,1.0f]}}"));
            c.add(new Case(t + "_transformation_bad", t, "{transformation:\"no\"}"));
            c.add(new Case(t + "_billboard_center", t, "{billboard:\"center\"}"));
            c.add(new Case(t + "_billboard_vertical", t, "{billboard:\"vertical\"}"));
            c.add(new Case(t + "_billboard_horizontal", t, "{billboard:\"horizontal\"}"));
            c.add(new Case(t + "_billboard_bad", t, "{billboard:\"sideways\"}"));
            c.add(new Case(t + "_brightness", t, "{brightness:{block:5,sky:12}}"));
            c.add(new Case(t + "_brightness_range", t, "{brightness:{block:20,sky:-1}}"));
            c.add(new Case(t + "_interpolation", t, "{interpolation_duration:40,start_interpolation:-5,teleport_duration:100}"));
            c.add(new Case(t + "_teleport_negative", t, "{teleport_duration:-7}"));
            c.add(new Case(t + "_render", t, "{view_range:2.5f,shadow_radius:1.5f,shadow_strength:0.25f,width:2.0f,height:4.0f,glow_color_override:16711680}"));
            c.add(new Case(t + "_misc", t, "{Pos:[0.5d,70.0d,0.5d],Rotation:[45.0f,10.0f],CustomName:'\"Hologram\"',Glowing:1b,Invulnerable:1b,Tags:[\"a\",\"b\"],NoGravity:1b}"));
        }
        // ---- block displays
        c.add(new Case("block_display_stone", "block_display", "{block_state:{id:\"minecraft:stone\"}}"));
        c.add(new Case("block_display_stairs", "block_display", "{block_state:{id:\"minecraft:oak_stairs\",properties:{facing:\"east\",half:\"top\",shape:\"straight\",waterlogged:\"false\"}}}"));
        c.add(new Case("block_display_partial_properties", "block_display", "{block_state:{id:\"minecraft:oak_stairs\",properties:{half:\"top\"}}}"));
        c.add(new Case("block_display_bad_property", "block_display", "{block_state:{id:\"minecraft:oak_stairs\",properties:{nope:\"x\"}}}"));
        c.add(new Case("block_display_unknown_block", "block_display", "{block_state:{id:\"minecraft:nonsense\"}}"));
        c.add(new Case("block_display_air", "block_display", "{block_state:{id:\"minecraft:air\"}}"));
        c.add(new Case("block_display_string_state", "block_display", "{block_state:\"minecraft:stone\"}"));
        c.add(new Case("block_display_state_stairs", "block_display", "{block_state:\"minecraft:oak_stairs[facing=east,half=top]\"}"));
        c.add(new Case("block_display_state_default_props", "block_display", "{block_state:\"minecraft:oak_stairs\"}"));
        c.add(new Case("block_display_state_bad_prop", "block_display", "{block_state:\"minecraft:stone[x=y]\"}"));
        c.add(new Case("block_display_state_bad_value", "block_display", "{block_state:\"minecraft:oak_stairs[facing=up]\"}"));
        c.add(new Case("block_display_state_unknown", "block_display", "{block_state:\"minecraft:nonsense\"}"));
        c.add(new Case("block_display_state_waterlogged", "block_display", "{block_state:\"minecraft:oak_slab[type=bottom,waterlogged=true]\"}"));
        c.add(new Case("block_display_state_tag", "block_display", "{block_state:\"#minecraft:logs\"}"));
        c.add(new Case("block_display_state_spaces", "block_display", "{block_state:\" minecraft:stone\"}"));
        c.add(new Case("block_display_state_no_namespace", "block_display", "{block_state:\"stone\"}"));
        c.add(new Case("block_display_cmp_bad_value", "block_display", "{block_state:{id:\"minecraft:oak_stairs\",properties:{facing:\"up\"}}}"));
        c.add(new Case("block_display_cmp_slab", "block_display", "{block_state:{id:\"minecraft:oak_slab\",properties:{type:\"bottom\",waterlogged:\"true\"}}}"));
        c.add(new Case("block_display_cmp_number_value", "block_display", "{block_state:{id:\"minecraft:oak_slab\",properties:{waterlogged:1b}}}"));
        c.add(new Case("block_display_cmp_name_key", "block_display", "{block_state:{Name:\"minecraft:stone\"}}"));
        c.add(new Case("block_display_cmp_default_props", "block_display", "{block_state:{id:\"minecraft:oak_stairs\",properties:{facing:\"north\",half:\"bottom\",shape:\"straight\",waterlogged:\"false\"}}}"));
        c.add(new Case("block_display_cmp_extra", "block_display", "{block_state:{id:\"minecraft:stone\",extra:1b}}"));
        c.add(new Case("block_display_cmp_redstone", "block_display", "{block_state:{id:\"minecraft:redstone_wire\",properties:{power:\"7\",east:\"side\"}}}"));
        c.add(new Case("block_display_cmp_water", "block_display", "{block_state:{id:\"minecraft:water\",properties:{level:\"3\"}}}"));
        c.add(new Case("block_display_state_chest","block_display", "{block_state:\"minecraft:chest[facing=west,type=left]\"}"));
        // ---- item displays
        c.add(new Case("item_display_diamond", "item_display", "{item:{id:\"minecraft:diamond\",count:1}}"));
        c.add(new Case("item_display_components", "item_display", "{item:{id:\"minecraft:diamond_sword\",count:1,components:{\"minecraft:custom_name\":'\"Excalibur\"',\"minecraft:damage\":5}},item_display:\"fixed\"}"));
        c.add(new Case("item_display_count", "item_display", "{item:{id:\"minecraft:stone\",count:64}}"));
        c.add(new Case("item_display_empty", "item_display", "{item:{id:\"minecraft:air\",count:1}}"));
        for (String ctx : new String[] {"none", "thirdperson_lefthand", "thirdperson_righthand", "firstperson_lefthand", "firstperson_righthand", "head", "gui", "ground", "fixed", "on_shelf", "nonsense"}) {
            c.add(new Case("item_display_context_" + ctx, "item_display", "{item:{id:\"minecraft:diamond\",count:1},item_display:\"" + ctx + "\"}"));
        }
        // ---- text displays
        c.add(new Case("text_display_plain", "text_display", "{text:'\"Hello\"'}"));
        c.add(new Case("text_display_object", "text_display", "{text:'{\"text\":\"Hi\",\"color\":\"red\",\"bold\":true}'}"));
        c.add(new Case("text_display_extra", "text_display", "{text:'[\"a\",{\"text\":\"b\",\"italic\":true},\"c\"]'}"));
        c.add(new Case("text_display_compound", "text_display", "{text:{text:\"Hi\",color:\"gold\"}}"));
        c.add(new Case("text_display_translate", "text_display", "{text:'{\"translate\":\"block.minecraft.stone\"}'}"));
        c.add(new Case("text_display_keybind", "text_display", "{text:'{\"keybind\":\"key.jump\"}'}"));
        c.add(new Case("text_display_selector", "text_display", "{text:'{\"selector\":\"@e[type=marker]\"}'}"));
        c.add(new Case("text_display_self", "text_display", "{text:'{\"selector\":\"@s\"}',CustomName:'\"Me\"'}"));
        c.add(new Case("text_display_bad", "text_display", "{text:'{\"text\":'}"));
        // Text components as NBT.
        c.add(new Case("text_display_nbt_extra", "text_display", "{text:{text:\"a\",extra:[\"b\",{text:\"c\",bold:1b}]}}"));
        c.add(new Case("text_display_nbt_list", "text_display", "{text:[\"a\",{text:\"b\",italic:1b},\"c\"]}"));
        c.add(new Case("text_display_nbt_style", "text_display", "{text:{text:\"s\",color:\"#ff8800\",bold:1b,italic:0b,underlined:1b,strikethrough:1b,obfuscated:1b,font:\"minecraft:uniform\",insertion:\"ins\",shadow_color:-1}}"));
        c.add(new Case("text_display_nbt_click", "text_display", "{text:{text:\"c\",click_event:{action:\"open_url\",url:\"https://example.com\"},hover_event:{action:\"show_text\",value:\"tip\"}}}"));
        c.add(new Case("text_display_nbt_translate", "text_display", "{text:{translate:\"block.minecraft.stone\",with:[\"x\",5]}}"));
        c.add(new Case("text_display_nbt_translate_fallback", "text_display", "{text:{translate:\"no.such.key\",fallback:\"Fallback %s\",with:[\"x\"]}}"));
        c.add(new Case("text_display_nbt_keybind", "text_display", "{text:{keybind:\"key.jump\"}}"));
        c.add(new Case("text_display_nbt_selector", "text_display", "{text:{selector:\"@e[type=marker]\"}}"));
        c.add(new Case("text_display_nbt_selector_self", "text_display", "{text:{selector:\"@s\"},CustomName:{text:\"Me\"}}"));
        c.add(new Case("text_display_nbt_selector_sep", "text_display", "{text:{selector:\"@e[type=text_display]\",separator:{text:\" | \"}},CustomName:{text:\"Me\"}}"));
        c.add(new Case("text_display_nbt_score", "text_display", "{text:{score:{name:\"@s\",objective:\"o\"}}}"));
        c.add(new Case("text_display_nbt_score_missing", "text_display", "{text:{score:{name:\"nobody\",objective:\"o\"}}}"));
        c.add(new Case("text_display_nbt_int", "text_display", "{text:5}"));
        c.add(new Case("text_display_nbt_bool", "text_display", "{text:1b}"));
        c.add(new Case("text_display_nbt_empty_list", "text_display", "{text:[]}"));
        c.add(new Case("text_display_nbt_string_plain", "text_display", "{text:\"plain\"}"));
        c.add(new Case("text_display_nbt_nested", "text_display", "{text:{text:\"a\",extra:[{text:\"b\",extra:[{text:\"c\",color:\"red\"}]}],color:\"blue\"}}"));
        c.add(new Case("text_display_options", "text_display", "{text:'\"x\"',line_width:50,background:-16777216,text_opacity:64b,shadow:1b,see_through:1b,default_background:1b}"));
        c.add(new Case("text_display_align_left", "text_display", "{text:'\"x\"',alignment:\"left\"}"));
        c.add(new Case("text_display_align_right", "text_display", "{text:'\"x\"',alignment:\"right\"}"));
        c.add(new Case("text_display_align_center", "text_display", "{text:'\"x\"',alignment:\"center\"}"));
        c.add(new Case("text_display_align_bad", "text_display", "{text:'\"x\"',alignment:\"justify\"}"));
        c.add(new Case("text_display_opacity_int", "text_display", "{text:'\"x\"',text_opacity:200}"));
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
        }, "EntityNbtVectors main");
        main.start();
        MinecraftServer server = awaitServer();
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
        System.out.println("EntityNbtVectors: wrote " + lines.size() + " cases to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static String run(ServerLevel level, Case c) throws Exception {
        net.minecraft.nbt.CompoundTag tag = net.minecraft.nbt.TagParser.parseCompoundFully(c.nbt);
        tag.putString("id", "minecraft:" + c.type);
        var in = net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess(), tag);
        Entity e = EntityType.loadEntityRecursive(in, level, EntitySpawnReason.COMMAND, x -> x);
        if (e == null) {
            return String.format(Locale.ROOT, "{\"name\":\"%s\",\"type\":\"minecraft:%s\",\"nbt\":%s,\"loaded\":false}", c.name, c.type, tagJson(tag));
        }
        var out = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess());
        e.save(out);
        net.minecraft.nbt.CompoundTag saved = out.buildResult();
        saved.remove("UUID");
        StringBuilder meta = new StringBuilder();
        var nonDefault = e.getEntityData().getNonDefaultValues();
        for (var v : nonDefault == null ? List.<net.minecraft.network.syncher.SynchedEntityData.DataValue<?>>of() : nonDefault) {
            RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), level.registryAccess());
            v.write(buf);
            if (meta.length() > 0) meta.append(',');
            meta.append('"').append(ByteBufUtil.hexDump(buf)).append('"');
        }
        // The box (the interaction's hit box, a marker's none).
        var bb = e.getBoundingBox();
        return String.format(Locale.ROOT, "{\"name\":\"%s\",\"type\":\"minecraft:%s\",\"nbt\":%s,\"loaded\":true,\"saved\":%s,\"meta\":[%s],\"box\":[%s,%s,%s,%s,%s,%s],\"pos\":[%s,%s,%s]}",
                c.name, c.type, tagJson(tag), tagJson(saved), meta, d(bb.minX), d(bb.minY), d(bb.minZ), d(bb.maxX), d(bb.maxY), d(bb.maxZ), d(e.getX()), d(e.getY()), d(e.getZ()));
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
