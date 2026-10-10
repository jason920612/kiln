// Differential test vectors for Kiln's player interactions with items and blocks (wp44-interact),
// recorded in a real vanilla 26.3 dedicated server started in-process: a mock player with a
// network connection is set up (game mode, place, inventory, blocks around it), then serverbound
// packets are run through ServerGamePacketListenerImpl one step at a time, and after every step
// the inventory, the clientbound packets of interest, the watched blocks (state and block entity
// data) and the item entities nearby are recorded.
//
// Scenario kinds (selected by name prefix): "equip" (Item.use of equippable items), "sign"
// (editing, dyes, glow ink, honeycomb, waxed signs, placing signs), "book" (ServerboundEditBook),
// "pick" (pick item from block/entity, bundle selection).
//
// usage (cwd = a scratch server directory, e.g. work/wp44/interact/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/InteractVectors.java <out.jsonl> [name-filter]
// (tools/interact_vectors.py sets this up)

import com.mojang.authlib.GameProfile;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.Connection;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.*;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

public class InteractVectors {
    static final int[] BASE = {0, 100, 0};
    static MinecraftServer server;
    static int players;

    // ---------------------------------------------------------------- scenario model

    static final class Case {
        final String name;
        String gameMode = "survival";
        double[] pos = {0.5, 100.0, 0.5};
        float yaw, pitch;
        boolean sneaking;
        int selected;
        // Setup commands run before the player is created (setblock, data merge, ...).
        List<String> commands = new ArrayList<>();
        // Slot key ("h0".."h8", "m9".."m35", "feet", "legs", "chest", "head", "offhand") -> stack.
        Map<String, ItemStack> slots = new LinkedHashMap<>();
        List<Map<String, Object>> steps = new ArrayList<>();
        // Blocks to watch (absolute coordinates).
        List<int[]> watch = new ArrayList<>();
        // Items whose use statistic is recorded.
        List<String> statItems = new ArrayList<>();
        // wp49: the food level the player starts with, and whether the food data is recorded.
        int food = 20;
        boolean watchFood;
        // wp49: the hanging entities (item frames, paintings) around are recorded after every step.
        boolean watchHanging;
        // wp49: the armor stands around (their saved data without the uuid) are recorded after every step.
        boolean watchStands;
        // wp49: the custom stats (by name) whose change is recorded.
        List<String> customStats = new ArrayList<>();
        // wp49: the bees that appear (where, and whether they target someone) are recorded after every step.
        boolean watchBees;
        // wp49: the menu packets are recorded.
        boolean watchMenus;
        // wp49: maps are watched.
        boolean watchMaps;
        // wp49: the watched block entities are ticked after every step.
        boolean tickLevel;
        // wp49: the living mobs around are recorded.
        boolean watchMobs;
        // wp53: the advancement criteria the player completes at each step are recorded.
        boolean watchAdv;
        // wp50: the cushions tick once after every step (the replay's level does), `tick_cushions` ticks them that many times.
        boolean tickCushions;
        // wp50: the player's position and the teleports sent are recorded after every step (and the connection's last good position is the start).
        boolean watchMove;
        // wp50: sound pitches are not recorded (an entity's voice pitch comes from its own random).
        boolean noPitch;
        // wp50: structure templates (by id) recorded after every step.
        List<String> templates = new ArrayList<>();
        // wp49: commands the replay runs together with the first step (after the level has settled), not before it
        // (a hive ages while the replay's level ticks; the recorded one stands still).
        List<String> late = new ArrayList<>();

        Case(String name) {
            this.name = name;
        }

        Case cmd(String c) {
            commands.add(c);
            return this;
        }

        Case slot(String key, ItemStack s) {
            slots.put(key, s);
            return this;
        }

        Case step(Map<String, Object> s) {
            steps.add(s);
            return this;
        }

        Case watch(int x, int y, int z) {
            watch.add(new int[] {x, y, z});
            return this;
        }

        Case watchBox(int x0, int y0, int z0, int x1, int y1, int z1) {
            for (int y = y0; y <= y1; y++) for (int z = z0; z <= z1; z++) for (int x = x0; x <= x1; x++) watch(x, y, z);
            return this;
        }

        Case template(String id) {
            templates.add(id);
            return this;
        }

        Case stat(String item) {
            statItems.add(item);
            return this;
        }

        Case food(int level) {
            food = level;
            watchFood = true;
            return this;
        }

        Case stands() {
            watchStands = true;
            return this;
        }

        Case bees() {
            watchBees = true;
            return this;
        }

        Case menus() {
            watchMenus = true;
            return this;
        }

        Case late(String c) {
            late.add(c);
            return this;
        }

        /** wp49: maps are recorded (their data and packets) and every step is followed by a tick of the player's maps. */
        Case maps() {
            watchMaps = true;
            return this;
        }

        Case moves() {
            watchMove = true;
            return this;
        }

        Case hanging() {
            watchHanging = true;
            return this;
        }

        /** wp53: the criteria of advancements completed at every step are recorded (`adv`). */
        Case advancements() {
            watchAdv = true;
            return this;
        }

        /** wp49: the level ticks the watched block entities after every step (and `wait` is that many ticks). */
        Case ticking() {
            tickLevel = true;
            return this;
        }

        /** wp49: the player is an operator (level 4). */
        boolean op;

        /** wp49: the level ticks also run the scheduled block ticks and the player's use of his item (a brush). */
        boolean fullTicks;

        Case fullTicking() {
            tickLevel = true;
            fullTicks = true;
            return this;
        }

        /** wp49: the living mobs around are recorded after every step. */
        Case mobs() {
            watchMobs = true;
            return this;
        }

        Case custom(String stat) {
            customStats.add(stat);
            return this;
        }
    }

    static Map<String, Object> op(Object... kv) {
        Map<String, Object> m = new LinkedHashMap<>();
        for (int i = 0; i < kv.length; i += 2) m.put((String) kv[i], kv[i + 1]);
        return m;
    }

    // ---------------------------------------------------------------- stacks

    static ItemStack stack(String item, int count) {
        ItemStack s = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(item)), count);
        return s;
    }

    static ItemStack stack(String item) {
        return stack(item, 1);
    }

    static ItemStack enchanted(String item, String ench, int level) {
        ItemStack s = stack(item);
        var holder = server.registryAccess().lookupOrThrow(Registries.ENCHANTMENT)
                .getOrThrow(ResourceKey.create(Registries.ENCHANTMENT, Identifier.parse(ench)));
        s.enchant(holder, level);
        return s;
    }

    static String hex(ItemStack s) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), server.registryAccess());
        ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, s);
        return ByteBufUtil.hexDump(buf);
    }

    static String nbtHex(net.minecraft.nbt.Tag tag) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        net.minecraft.nbt.NbtIo.writeAnyTag(tag, new DataOutputStream(bytes));
        return ByteBufUtil.hexDump(Unpooled.wrappedBuffer(bytes.toByteArray()));
    }

    // ---------------------------------------------------------------- equip scenarios

    static final String[] ARMOR_ITEMS = {
        "minecraft:diamond_helmet", "minecraft:iron_chestplate", "minecraft:leather_leggings", "minecraft:netherite_boots",
        "minecraft:golden_helmet", "minecraft:elytra", "minecraft:carved_pumpkin", "minecraft:player_head",
        "minecraft:zombie_head", "minecraft:creeper_head", "minecraft:turtle_helmet", "minecraft:chainmail_chestplate",
        "minecraft:shield", "minecraft:saddle", "minecraft:wolf_armor", "minecraft:leather_horse_armor", "minecraft:white_banner",
        "minecraft:copper_helmet", "minecraft:nautilus_shell", "minecraft:iron_nautilus_armor", "minecraft:totem_of_undying",
        "minecraft:oak_log", "minecraft:glass_pane", "minecraft:dragon_head", "minecraft:piglin_head",
    };

    static void equip(List<Case> out) {
        Case c;
        for (String item : ARMOR_ITEMS) {
            String n = item.substring(item.indexOf(':') + 1);
            // Into an empty slot, survival.
            c = new Case("equip_" + n + "_empty");
            c.slot("h0", stack(item)).step(op("op", "use", "hand", 0)).stat(item);
            out.add(c);
            // A stack of three.
            c = new Case("equip_" + n + "_stack");
            c.slot("h0", stack(item, 3)).step(op("op", "use", "hand", 0)).stat(item);
            out.add(c);
            // Creative.
            c = new Case("equip_" + n + "_creative");
            c.gameMode = "creative";
            c.slot("h0", stack(item, 2)).step(op("op", "use", "hand", 0)).stat(item);
            out.add(c);
            // Off hand.
            c = new Case("equip_" + n + "_offhand");
            c.slot("offhand", stack(item)).step(op("op", "use", "hand", 1)).stat(item);
            out.add(c);
        }
        // Swapping with something already worn.
        c = new Case("equip_swap_helmet");
        c.slot("h0", stack("minecraft:iron_helmet")).slot("head", stack("minecraft:diamond_helmet")).step(op("op", "use", "hand", 0));
        out.add(c);
        c = new Case("equip_swap_helmet_stack_inventory_full");
        c.slot("h0", stack("minecraft:iron_helmet", 2)).slot("head", stack("minecraft:diamond_helmet"));
        for (int i = 1; i < 9; i++) c.slot("h" + i, stack("minecraft:dirt", 64));
        for (int i = 9; i < 36; i++) c.slot("m" + i, stack("minecraft:stone", 64));
        c.step(op("op", "use", "hand", 0));
        out.add(c);
        c = new Case("equip_swap_helmet_stack_inventory_room");
        c.slot("h0", stack("minecraft:iron_helmet", 2)).slot("head", stack("minecraft:diamond_helmet")).step(op("op", "use", "hand", 0));
        out.add(c);
        c = new Case("equip_swap_creative");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:iron_helmet")).slot("head", stack("minecraft:diamond_helmet")).step(op("op", "use", "hand", 0));
        out.add(c);
        c = new Case("equip_swap_creative_stack");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:iron_helmet", 3)).slot("head", stack("minecraft:diamond_helmet")).step(op("op", "use", "hand", 0));
        out.add(c);
        // The same item (and components) already worn: nothing happens.
        c = new Case("equip_same_item");
        c.slot("h0", stack("minecraft:iron_helmet", 2)).slot("head", stack("minecraft:iron_helmet")).step(op("op", "use", "hand", 0));
        out.add(c);
        // Same item, different components.
        c = new Case("equip_same_item_enchanted");
        c.slot("h0", stack("minecraft:iron_helmet")).slot("head", enchanted("minecraft:iron_helmet", "minecraft:protection", 1))
                .step(op("op", "use", "hand", 0));
        out.add(c);
        // Curse of binding on the worn piece.
        c = new Case("equip_binding_curse");
        c.slot("h0", stack("minecraft:iron_helmet")).slot("head", enchanted("minecraft:diamond_helmet", "minecraft:binding_curse", 1))
                .step(op("op", "use", "hand", 0));
        out.add(c);
        c = new Case("equip_binding_curse_creative");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:iron_helmet")).slot("head", enchanted("minecraft:diamond_helmet", "minecraft:binding_curse", 1))
                .step(op("op", "use", "hand", 0));
        out.add(c);
        // Elytra over a chestplate swaps.
        c = new Case("equip_elytra_over_chestplate");
        c.slot("h0", stack("minecraft:elytra")).slot("chest", stack("minecraft:diamond_chestplate")).step(op("op", "use", "hand", 0));
        out.add(c);
        // Damaged stack and a named one keep their components.
        c = new Case("equip_damaged");
        ItemStack damaged = stack("minecraft:iron_boots");
        damaged.setDamageValue(77);
        c.slot("h0", damaged).step(op("op", "use", "hand", 0));
        out.add(c);
        // Spectator and adventure.
        c = new Case("equip_adventure");
        c.gameMode = "adventure";
        c.slot("h0", stack("minecraft:iron_helmet")).step(op("op", "use", "hand", 0));
        out.add(c);
        c = new Case("equip_spectator");
        c.gameMode = "spectator";
        c.slot("h0", stack("minecraft:iron_helmet")).step(op("op", "use", "hand", 0));
        out.add(c);
        // Two uses in a row (the second finds the same item worn).
        c = new Case("equip_twice");
        c.slot("h0", stack("minecraft:iron_helmet", 2)).step(op("op", "use", "hand", 0)).step(op("op", "use", "hand", 0));
        out.add(c);
        // Used while the cooldown of the item runs: nothing (cooldowns are server state).
        c = new Case("equip_on_cooldown");
        c.slot("h0", stack("minecraft:iron_helmet")).step(op("op", "cooldown", "item", "minecraft:iron_helmet", "ticks", 20))
                .step(op("op", "use", "hand", 0));
        out.add(c);
    }

    // ---------------------------------------------------------------- wp49: blocks eaten from and put things on

    /** A stone floor and the block at (2, 100, 0), a step from the player. */
    static Case blockCase(String name, String block) {
        Case c = new Case(name);
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 " + block).watch(2, 100, 0);
        return c;
    }

    static void cakes(List<Case> out) {
        Case c;
        for (int bites = 0; bites <= 6; bites++) {
            c = blockCase("cake_eat_" + bites, "minecraft:cake[bites=" + bites + "]").food(10).custom("minecraft:eat_cake_slice");
            c.step(useOn(2, 100, 0, 1, 0));
            c.step(useOn(2, 100, 0, 1, 0));
            out.add(c);
        }
        c = blockCase("cake_full_stomach", "minecraft:cake[bites=2]").food(20).custom("minecraft:eat_cake_slice");
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("cake_almost_full", "minecraft:cake[bites=2]").food(19).custom("minecraft:eat_cake_slice");
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("cake_creative_full", "minecraft:cake[bites=2]").food(20).custom("minecraft:eat_cake_slice");
        c.gameMode = "creative";
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("cake_adventure", "minecraft:cake[bites=2]").food(5).custom("minecraft:eat_cake_slice");
        c.gameMode = "adventure";
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("cake_with_apple_in_hand", "minecraft:cake[bites=1]").food(8).custom("minecraft:eat_cake_slice");
        c.slot("h0", stack("minecraft:apple", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("cake_sneaking_empty_hand", "minecraft:cake[bites=1]").food(8).custom("minecraft:eat_cake_slice");
        c.sneaking = true;
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("cake_sneaking_with_item", "minecraft:cake[bites=1]").food(8).custom("minecraft:eat_cake_slice");
        c.sneaking = true;
        c.slot("h0", stack("minecraft:apple", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // Candles on a whole cake only.
        for (String candle : new String[] {"candle", "red_candle", "white_candle"}) {
            for (int bites : new int[] {0, 1}) {
                c = blockCase("cake_" + candle + "_" + bites, "minecraft:cake[bites=" + bites + "]").food(10).stat("minecraft:" + candle);
                c.slot("h0", stack("minecraft:" + candle, 2)).step(useOn(2, 100, 0, 1, 0));
                out.add(c);
            }
        }
        c = blockCase("cake_candle_creative", "minecraft:cake[bites=0]").stat("minecraft:candle");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:candle", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // A cake with a candle is eaten like any (the candle drops, the cake is whole again to its first bite).
        for (String lit : new String[] {"false", "true"}) {
            c = blockCase("cake_candle_cake_eat_" + lit, "minecraft:blue_candle_cake[lit=" + lit + "]").food(10).custom("minecraft:eat_cake_slice");
            c.step(useOn(2, 100, 0, 1, 0));
            c.step(useOn(2, 100, 0, 1, 0));
            out.add(c);
        }
    }

    /** `use_on` with a chosen cursor (the hit position inside the block). */
    // ---------------------------------------------------------------- wp50: banners (placing, breaking) and the cauldron's washing

    static ItemStack parsed(String text) {
        try {
            var parser = new net.minecraft.commands.arguments.item.ItemParser(server.registryAccess());
            var input = parser.parse(new com.mojang.brigadier.StringReader(text));
            return new ItemStack(input.item(), 1, input.components());
        } catch (Exception e) {
            throw new RuntimeException(text + ": " + e.getMessage(), e);
        }
    }

    static final String LAYERS = "banner_patterns=[{pattern:\"minecraft:stripe_downright\",color:\"red\"},{pattern:\"minecraft:circle\",color:\"blue\"},{pattern:\"minecraft:border\",color:\"black\"}]";

    // ---------------------------------------------------------------- wp52: an item's components in the block entity it is placed as

    static void placeComponents52(List<Case> out) {
        Case c;
        String dia = "[{slot:0,item:{id:\"minecraft:diamond\",count:3}},{slot:4,item:{id:\"minecraft:stick\",count:20}}]";
        String[][] items = {
                {"head_profile", "minecraft:player_head[profile={name:\"Notch\"}]"},
                {"head_plain", "minecraft:player_head"},
                {"skull_named", "minecraft:skeleton_skull[custom_name='\"Bones\"']"},
                {"head_note", "minecraft:zombie_head[note_block_sound=\"minecraft:block.note_block.bell\"]"},
                {"chest_named", "minecraft:chest[custom_name='\"Box\"']"},
                {"chest_container", "minecraft:chest[container=" + dia + "]"},
                {"chest_loot", "minecraft:chest[container_loot={loot_table:\"minecraft:chests/simple_dungeon\",seed:5L}]"},
                {"chest_lock", "minecraft:chest[lock={components:{\"minecraft:custom_name\":'\"key\"'}}]"},
                {"barrel_container", "minecraft:barrel[container=" + dia + ",custom_name='\"B\"']"},
                {"shulker", "minecraft:shulker_box[container=" + dia + "]"},
                {"shulker_red_named", "minecraft:red_shulker_box[container=" + dia + ",custom_name='\"Red\"']"},
                {"dispenser", "minecraft:dispenser[container=" + dia + "]"},
                {"hopper_named", "minecraft:hopper[custom_name='\"H\"']"},
                {"furnace_named", "minecraft:furnace[custom_name='\"F\"']"},
                {"brewing_named", "minecraft:brewing_stand[custom_name='\"Brew\"']"},
                {"enchanting_named", "minecraft:enchanting_table[custom_name='\"Ench\"']"},
                {"beacon_named", "minecraft:beacon[custom_name='\"Light\"']"},
                {"decorated_pot", "minecraft:decorated_pot[pot_decorations={back:\"minecraft:brick\",left:\"minecraft:arms_up_pottery_sherd\",right:\"minecraft:brick\",front:\"minecraft:skull_pottery_sherd\"}]"},
                {"decorated_pot_container", "minecraft:decorated_pot[container=[{slot:0,item:{id:\"minecraft:diamond\",count:3}}]]"},
                {"beehive_bees", "minecraft:beehive[bees=[{entity_data:{id:\"minecraft:bee\"},min_ticks_in_hive:100,ticks_in_hive:5}]]"},
                {"bee_nest_bees", "minecraft:bee_nest[bees=[{entity_data:{id:\"minecraft:bee\"},min_ticks_in_hive:100,ticks_in_hive:5}],block_state={honey_level:\"3\"}]"},
                {"lectern", "minecraft:lectern"},
                {"crafter", "minecraft:crafter[custom_name='\"Craft\"']"},
                {"trapped_chest", "minecraft:trapped_chest[container=" + dia + "]"},
                {"ender_chest_named", "minecraft:ender_chest[custom_name='\"E\"']"},
                {"sign_named", "minecraft:oak_sign[custom_name='\"S\"']"},
                {"bed_color", "minecraft:red_bed"},
                {"chest_bed_data", "minecraft:chest[block_entity_data={id:\"minecraft:chest\",Items:[{Slot:1b,id:\"minecraft:stone\",count:2}]}]"},
                {"chest_bed_data_custom", "minecraft:chest[block_entity_data={id:\"minecraft:chest\",CustomName:'\"Data\"'}]"},
                {"spawner_data", "minecraft:spawner[block_entity_data={id:\"minecraft:mob_spawner\",SpawnData:{entity:{id:\"minecraft:zombie\"}},Delay:5s}]"},
        };
        for (String[] it : items) {
            for (String mode : new String[] {"survival", "creative"}) {
                c = blockCase("place52_" + it[0] + "_" + mode, "minecraft:air");
                c.gameMode = mode;
                c.slot("h0", parsed(it[1]));
                // (a hopper ticks: the level that ticks it makes it ready, as Kiln's replay does)
                if (it[0].equals("hopper_named")) c.ticking();
                c.step(useOn(2, 99, 0, 1, 0));
                out.add(c);
            }
        }
        // an operator in creative mode may place a block entity's data (survival and non-operators may not)
        for (String[] it : new String[][] {{"chest_bed_data", "minecraft:chest[block_entity_data={id:\"minecraft:chest\",Items:[{Slot:1b,id:\"minecraft:stone\",count:2}]}]"},
                {"spawner_data", "minecraft:spawner[block_entity_data={id:\"minecraft:mob_spawner\",SpawnData:{entity:{id:\"minecraft:zombie\"}},Delay:5s}]"}}) {
            c = blockCase("place52_op_" + it[0], "minecraft:air");
            c.gameMode = "creative";
            c.op = true;
            c.slot("h0", parsed(it[1]));
            c.step(useOn(2, 99, 0, 1, 0));
            out.add(c);
        }
    }

    // ---------------------------------------------------------------- wp53: advancement triggers

    static String nestBees(int n) {
        return "{bees:[" + "{entity_data:{id:\"minecraft:bee\"},min_ticks_in_hive:100,ticks_in_hive:5},".repeat(n).replaceAll(",$", "") + "]}";
    }

    static void advancements53(List<Case> out) {
        Case c;
        String silk = "minecraft:netherite_axe[enchantments={\"minecraft:silk_touch\":1,\"minecraft:efficiency\":5}]";
        String plain = "minecraft:netherite_axe[enchantments={\"minecraft:efficiency\":5}]";
        // `BeehiveBlock.playerDestroy` → `bee_nest_destroyed` (`husbandry/silk_touch_nest`: a silk touch tool, three bees inside)
        for (String block : new String[] {"bee_nest", "beehive"}) {
            for (int bees : new int[] {0, 2, 3}) {
                for (String tool : new String[] {"silk", "plain"}) {
                    String state = block.equals("beehive") ? "minecraft:beehive[facing=north,honey_level=0]" : "minecraft:bee_nest[facing=north,honey_level=0]";
                    c = new Case("adv53_" + block + "_" + bees + "_" + tool).advancements();
                    c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 " + state + nestBees(bees)).watch(2, 100, 0);
                    c.slot("h0", parsed(tool.equals("silk") ? silk : plain));
                    c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
                    out.add(c);
                }
            }
        }
        c = new Case("adv53_nest_creative").advancements();
        c.gameMode = "creative";
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 minecraft:bee_nest[facing=north,honey_level=0]" + nestBees(3)).watch(2, 100, 0);
        c.slot("h0", parsed(silk));
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
    }

    static void banners50(List<Case> out) {
        Case c;
        // ---- placing: a standing banner on the stone, a wall banner on its side; the block entity carries the layers, name and the rest.
        for (String[] b : new String[][] {
                {"plain", "minecraft:white_banner"},
                {"layers", "minecraft:red_banner[" + LAYERS + "]"},
                {"one_layer", "minecraft:blue_banner[banner_patterns=[{pattern:\"minecraft:skull\",color:\"white\"}]]"},
                {"named", "minecraft:green_banner[custom_name='\"Flag\"']"},
                {"lore", "minecraft:black_banner[lore=['\"x\"'],rarity=epic]"},
                {"everything", "minecraft:yellow_banner[" + LAYERS + ",custom_name='\"Pennant\"',lore=['\"y\"']]"},
                {"ominous", "minecraft:white_banner[banner_patterns=[{pattern:\"minecraft:rhombus\",color:\"cyan\"},{pattern:\"minecraft:stripe_bottom\",color:\"brown\"}],tooltip_display={hidden_components:[\"minecraft:banner_patterns\"]},item_name='{translate:\"block.minecraft.ominous_banner\"}']"}}) {
            c = blockCase("banner50_place_" + b[0], "minecraft:air");
            c.slot("h0", parsed(b[1])).stat(b[1].substring(0, b[1].indexOf('[') < 0 ? b[1].length() : b[1].indexOf('[')));
            c.step(useOn(2, 99, 0, 1, 0));
            out.add(c);
            c = blockCase("banner50_wall_" + b[0], "minecraft:stone").watch(1, 100, 0);
            c.slot("h0", parsed(b[1])).stat(b[1].substring(0, b[1].indexOf('[') < 0 ? b[1].length() : b[1].indexOf('[')));
            c.step(useOn(2, 100, 0, 4, 0));
            out.add(c);
        }
        c = blockCase("banner50_place_creative", "minecraft:air");
        c.gameMode = "creative";
        c.slot("h0", parsed("minecraft:red_banner[" + LAYERS + "]"));
        c.step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = blockCase("banner50_place_stack", "minecraft:air");
        ItemStack three = parsed("minecraft:red_banner[" + LAYERS + "]");
        three.setCount(3);
        c.slot("h0", three).stat("minecraft:red_banner");
        c.step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        // ---- breaking (`/setblock ... destroy`): the dropped banner keeps the layers and the name.
        String nbt = "{patterns:[{color:\"red\",pattern:\"minecraft:stripe_downright\"},{color:\"blue\",pattern:\"minecraft:circle\"}],CustomName:'\"Flag\"'}";
        for (String block : new String[] {"minecraft:white_banner[rotation=4]", "minecraft:orange_wall_banner[facing=east]"}) {
            c = blockCase("banner50_break_" + block.substring(10, block.indexOf('[')), block + nbt);
            c.step(op("op", "command", "command", "setblock 2 100 0 minecraft:air destroy"));
            out.add(c);
        }
        c = blockCase("banner50_break_plain", "minecraft:white_banner[rotation=4]");
        c.step(op("op", "command", "command", "setblock 2 100 0 minecraft:air destroy"));
        out.add(c);
    }

    /** wp50: a cushion case: the cushions around are recorded after every step. */
    static Case cushionCase(String name) {
        return new Case("cushion50_" + name).hanging();
    }

    static final String[] DYES = {"white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"};

    static void cushions50(List<Case> out) {
        Case c;
        // ---- putting one down on the top of a stone block (2, 99, 0): every color, a few clicks, the way the player faces
        for (String color : DYES) {
            c = cushionCase("place_" + color).stat("minecraft:" + color + "_cushion");
            c.cmd("setblock 2 99 0 minecraft:stone");
            c.slot("h0", stack("minecraft:" + color + "_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5));
            out.add(c);
        }
        float[] yaws = {0f, 45f, 90f, 135f, 180f, -135f, -90f, -45f, 22f, 359f, 720f};
        for (float yaw : yaws) {
            c = cushionCase("yaw_" + (int) yaw).stat("minecraft:red_cushion");
            c.yaw = yaw;
            c.cmd("setblock 2 99 0 minecraft:stone");
            c.slot("h0", stack("minecraft:red_cushion", 1)).step(useOnAt(2, 99, 0, 1, 0, 0.125, 1.0, 0.875));
            out.add(c);
        }
        c = cushionCase("place_twice").stat("minecraft:blue_cushion");
        c.cmd("setblock 2 99 0 minecraft:stone");
        c.slot("h0", stack("minecraft:blue_cushion", 5)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5)).step(useOnAt(2, 99, 0, 1, 0, 0.25, 1.0, 0.25));
        out.add(c);
        c = cushionCase("place_beside").stat("minecraft:blue_cushion");
        c.cmd("fill 2 99 0 3 99 1 minecraft:stone");
        c.slot("h0", stack("minecraft:blue_cushion", 5)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5)).step(useOnAt(3, 99, 0, 1, 0, 0.5, 1.0, 0.5)).step(useOnAt(2, 99, 1, 1, 0, 0.5, 1.0, 0.5));
        out.add(c);
        c = cushionCase("place_overlapping_heights").stat("minecraft:blue_cushion");
        c.cmd("setblock 2 99 0 minecraft:stone");
        // (The second click is lower in the same cell: the boxes overlap.)
        c.slot("h0", stack("minecraft:blue_cushion", 5)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 0.875, 0.5));
        out.add(c);
        for (String mode : new String[] {"creative", "adventure"}) {
            c = cushionCase("place_" + mode).stat("minecraft:green_cushion");
            c.gameMode = mode;
            c.cmd("setblock 2 99 0 minecraft:stone");
            c.slot("h0", stack("minecraft:green_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5));
            out.add(c);
        }
        c = cushionCase("place_offhand").stat("minecraft:green_cushion");
        c.cmd("setblock 2 99 0 minecraft:stone");
        c.slot("offhand", stack("minecraft:green_cushion", 2)).step(useOnAt(2, 99, 0, 1, 1, 0.5, 1.0, 0.5));
        out.add(c);
        c = cushionCase("place_named").stat("minecraft:green_cushion");
        c.cmd("setblock 2 99 0 minecraft:stone");
        c.slot("h0", parsed("minecraft:green_cushion[custom_name='\"Seat\"']")).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5))
                .step(attackEntity(2.5, 100.0, 0.5));
        out.add(c);
        // ---- the other faces, and a block over the place
        for (int face : new int[] {0, 2, 3, 4, 5}) {
            c = cushionCase("face_" + face).stat("minecraft:red_cushion");
            c.cmd("setblock 2 99 0 minecraft:stone");
            c.slot("h0", stack("minecraft:red_cushion", 2)).step(useOnAt(2, 99, 0, face, 0, 0.5, 0.5, 0.5));
            out.add(c);
        }
        c = cushionCase("blocked_above").stat("minecraft:red_cushion");
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 minecraft:stone");
        c.slot("h0", stack("minecraft:red_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5));
        out.add(c);
        c = cushionCase("blocked_above_glass").stat("minecraft:red_cushion");
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 minecraft:glass");
        c.slot("h0", stack("minecraft:red_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5));
        out.add(c);
        c = cushionCase("click_inside_stone").stat("minecraft:red_cushion");
        c.cmd("setblock 2 99 0 minecraft:stone");
        c.slot("h0", stack("minecraft:red_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 0.5, 0.5));
        out.add(c);
        c = cushionCase("click_in_the_air").stat("minecraft:red_cushion");
        c.cmd("setblock 2 99 0 minecraft:white_carpet");
        c.slot("h0", stack("minecraft:red_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5));
        out.add(c);
        // ---- on all sorts of blocks: [block, click height]
        String[][] grounds = {
                {"carpet", "minecraft:white_carpet", "0.0625"},
                {"slab_bottom", "minecraft:stone_slab[type=bottom]", "0.5"},
                {"slab_top", "minecraft:stone_slab[type=top]", "1.0"},
                {"slab_double", "minecraft:stone_slab[type=double]", "1.0"},
                {"stairs", "minecraft:oak_stairs[facing=east,half=bottom,shape=straight]", "1.0"},
                {"stairs_low", "minecraft:oak_stairs[facing=east,half=bottom,shape=straight]", "0.5"},
                {"stairs_top", "minecraft:oak_stairs[facing=east,half=top,shape=straight]", "1.0"},
                {"snow_1", "minecraft:snow[layers=1]", "0.125"},
                {"snow_2", "minecraft:snow[layers=2]", "0.25"},
                {"snow_8", "minecraft:snow[layers=8]", "1.0"},
                {"grass", "minecraft:short_grass", "0.5"},
                {"tall_grass", "minecraft:tall_grass[half=lower]", "0.875"},
                {"farmland", "minecraft:farmland[moisture=0]", "0.9375"},
                {"soul_sand", "minecraft:soul_sand", "0.875"},
                {"path", "minecraft:dirt_path", "0.9375"},
                {"honey", "minecraft:honey_block", "0.9375"},
                {"fence", "minecraft:oak_fence", "1.5"},
                {"wall", "minecraft:cobblestone_wall", "1.0"},
                {"glass", "minecraft:glass", "1.0"},
                {"leaves", "minecraft:oak_leaves", "1.0"},
                {"ice", "minecraft:ice", "1.0"},
                {"lily", "minecraft:lily_pad", "0.015625"},
                {"torch", "minecraft:torch", "0.625"},
                {"flower", "minecraft:poppy", "0.375"},
                {"lantern", "minecraft:lantern[hanging=false]", "0.5625"},
                {"cauldron_full_height", "minecraft:cauldron", "1.0"},
                {"water_cauldron", "minecraft:water_cauldron[level=3]", "1.0"},
                {"hopper", "minecraft:hopper[enabled=true,facing=down]", "1.0"},
                {"composter", "minecraft:composter[level=3]", "1.0"},
                {"composter_empty", "minecraft:composter[level=0]", "1.0"},
                {"campfire", "minecraft:campfire[lit=false]", "0.4375"},
                {"end_portal_frame", "minecraft:end_portal_frame[facing=north,eye=false]", "0.8125"},
                {"powder_snow", "minecraft:powder_snow", "1.0"},
                {"azalea", "minecraft:potted_poppy", "0.375"},
                {"slime", "minecraft:slime_block", "1.0"},
                {"cobweb", "minecraft:cobweb", "1.0"},
                {"vine", "minecraft:vine[north=true]", "1.0"},
                {"ladder", "minecraft:ladder[facing=north]", "1.0"},
                {"pointed_dripstone", "minecraft:pointed_dripstone[thickness=tip,vertical_direction=up]", "0.625"},
                {"bamboo", "minecraft:bamboo", "0.5"},
                {"sea_pickle", "minecraft:sea_pickle[pickles=1]", "0.375"},
                {"turtle_egg", "minecraft:turtle_egg", "0.4375"},
                {"conduit", "minecraft:conduit", "0.8125"},
                {"rail", "minecraft:rail", "0.125"},
                {"pressure_plate", "minecraft:stone_pressure_plate", "0.0625"},
                {"skull", "minecraft:skeleton_skull", "0.5"},
                {"banner", "minecraft:white_banner", "1.0"},
        };
        for (String[] g : grounds) {
            c = cushionCase("on_" + g[0]).stat("minecraft:orange_cushion");
            c.sneaking = true;
            c.cmd("setblock 2 98 0 " + (g[0].equals("lily") ? "minecraft:water" : g[0].equals("bamboo") ? "minecraft:dirt" : "minecraft:stone")).cmd("setblock 2 99 0 " + g[1]);
            c.slot("h0", stack("minecraft:orange_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, Double.parseDouble(g[2]), 0.5));
            out.add(c);
        }
        // ---- cauldrons, hoppers and composters are clicked on their collision shape: the corners and the middle
        for (String block : new String[] {"minecraft:cauldron", "minecraft:hopper[enabled=true,facing=down]", "minecraft:composter[level=0]"}) {
            String shortName = block.substring(10, block.contains("[") ? block.indexOf('[') : block.length());
            double[][] clicks = {{0.5, 1.0, 0.5}, {0.125, 1.0, 0.125}, {0.875, 1.0, 0.5}, {0.5, 0.5, 0.0625}, {0.5, 0.25, 0.5}, {0.0625, 0.875, 0.9375}};
            for (int i = 0; i < clicks.length; i++) {
                c = cushionCase("shape_" + shortName + "_" + i).stat("minecraft:orange_cushion");
                c.sneaking = shortName.equals("hopper");
                c.cmd("setblock 2 99 0 " + block);
                c.slot("h0", stack("minecraft:orange_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, clicks[i][0], clicks[i][1], clicks[i][2]));
                out.add(c);
            }
        }
        // the player looks down at the cauldron (the eyes at 101.62)
        c = cushionCase("shape_cauldron_pitch").stat("minecraft:orange_cushion");
        c.pitch = 60f;
        c.cmd("setblock 2 99 0 minecraft:cauldron");
        c.slot("h0", stack("minecraft:orange_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.25, 1.0, 0.375));
        out.add(c);
        // ---- places that are not air: water, lava, fire, a replaceable plant
        for (String above : new String[] {"minecraft:water", "minecraft:lava", "minecraft:fire", "minecraft:soul_fire", "minecraft:short_grass", "minecraft:snow[layers=1]", "minecraft:snow[layers=3]", "minecraft:vine[north=true]", "minecraft:oak_sign", "minecraft:cobweb", "minecraft:campfire[lit=true]", "minecraft:torch"}) {
            c = cushionCase("above_" + above.substring(10).replaceAll("[^a-z_]", "_")).stat("minecraft:orange_cushion");
            c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 " + above);
            c.slot("h0", stack("minecraft:orange_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5));
            out.add(c);
        }
        // the clicked block itself is the place (a plant): the cushion sits in it
        for (String plant : new String[] {"minecraft:short_grass", "minecraft:snow[layers=1]", "minecraft:snow[layers=2]", "minecraft:fire", "minecraft:water"}) {
            c = cushionCase("clicked_" + plant.substring(10).replaceAll("[^a-z_]", "_")).stat("minecraft:orange_cushion");
            c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 " + plant);
            c.slot("h0", stack("minecraft:orange_cushion", 2)).step(useOnAt(2, 100, 0, 1, 0, 0.5, 0.25, 0.5));
            out.add(c);
        }
        // ---- fire beside it: in the cell next to the cushion nothing happens; in its own cell it burns up
        c = cushionCase("fire_next_to").stat("minecraft:orange_cushion");
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 3 100 0 minecraft:fire");
        c.slot("h0", stack("minecraft:orange_cushion", 2)).step(useOnAt(2, 99, 0, 1, 0, 0.5, 1.0, 0.5));
        out.add(c);
        // ---- sitting
        for (String variant : new String[] {"plain", "sneaking", "item", "creative", "adventure", "named", "twice"}) {
            c = cushionCase("sit_" + variant);
            c.cmd("setblock 2 99 0 minecraft:stone").cmd("summon minecraft:cushion 2.5 100 0.5 {color:\"cyan\"" + (variant.equals("named") ? ",CustomName:'\"Sofa\"'" : "") + "}");
            if (variant.equals("sneaking")) c.sneaking = true;
            if (variant.equals("creative") || variant.equals("adventure")) c.gameMode = variant;
            if (variant.equals("item")) c.slot("h0", stack("minecraft:stick", 3));
            c.step(useEntity(2.5, 100.0, 0.5, 0, variant.equals("sneaking")));
            if (variant.equals("twice")) c.step(useEntity(2.5, 100.0, 0.5, 0, false));
            out.add(c);
        }
        c = cushionCase("sit_two_cushions");
        c.cmd("fill 2 99 0 3 99 0 minecraft:stone").cmd("summon minecraft:cushion 2.5 100 0.5 {color:\"cyan\"}").cmd("summon minecraft:cushion 3.5 100 0.5 {color:\"red\"}");
        c.step(useEntity(2.5, 100.0, 0.5, 0, false)).step(useEntity(3.5, 100.0, 0.5, 0, false));
        out.add(c);
        c = cushionCase("sit_low");
        c.cmd("setblock 2 99 0 minecraft:white_carpet").cmd("summon minecraft:cushion 2.5 99.0625 0.5 {color:\"gray\"}");
        c.step(useEntity(2.5, 99.0625, 0.5, 0, false));
        out.add(c);
        // ---- hitting
        for (String variant : new String[] {"survival", "creative", "adventure"}) {
            c = cushionCase("hit_" + variant);
            c.gameMode = variant;
            c.cmd("setblock 2 99 0 minecraft:stone").cmd("summon minecraft:cushion 2.5 100 0.5 {color:\"magenta\",CustomName:'\"Pouf\"'}");
            c.step(attackEntity(2.5, 100.0, 0.5));
            out.add(c);
        }
        c = cushionCase("hit_plain");
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("summon minecraft:cushion 2.5 100 0.5");
        c.step(attackEntity(2.5, 100.0, 0.5));
        out.add(c);
        c = cushionCase("hit_no_drops");
        c.cmd("gamerule entity_drops false").cmd("setblock 2 99 0 minecraft:stone").cmd("summon minecraft:cushion 2.5 100 0.5 {color:\"lime\"}");
        c.step(attackEntity(2.5, 100.0, 0.5)).step(op("op", "command", "command", "gamerule entity_drops true"));
        out.add(c);
        c = cushionCase("hit_sitting");
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("summon minecraft:cushion 2.5 100 0.5 {color:\"lime\"}");
        c.step(useEntity(2.5, 100.0, 0.5, 0, false)).step(attackEntity(2.5, 100.0, 0.5));
        out.add(c);
        // ---- the check every 100 ticks (the cushions are ticked by hand)
        c = cushionCase("tick_on_stone"); c.tickCushions = true;
        c.cmd("setblock 2 99 0 minecraft:stone").late("summon minecraft:cushion 2.5 100 0.5 {color:\"red\"}");
        c.step(op("op", "tick_cushions", "ticks", 100)).step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = cushionCase("tick_over_air"); c.tickCushions = true;
        c.late("summon minecraft:cushion 2.5 100 0.5 {color:\"red\",CustomName:'\"Floating\"'}");
        c.step(op("op", "tick_cushions", "ticks", 99)).step(op("op", "tick_cushions", "ticks", 1)).step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = cushionCase("tick_support_broken"); c.tickCushions = true;
        c.cmd("setblock 2 99 0 minecraft:stone").late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
        c.step(op("op", "tick_cushions", "ticks", 100)).step(op("op", "command", "command", "setblock 2 99 0 minecraft:air")).step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = cushionCase("tick_buried"); c.tickCushions = true;
        c.cmd("setblock 2 99 0 minecraft:stone").late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
        c.step(op("op", "command", "command", "setblock 2 100 0 minecraft:stone")).step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = cushionCase("tick_in_fire"); c.tickCushions = true;
        c.cmd("setblock 2 99 0 minecraft:stone").late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
        c.step(op("op", "command", "command", "setblock 2 100 0 minecraft:fire")).step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = cushionCase("tick_in_soul_fire"); c.tickCushions = true;
        c.cmd("setblock 2 99 0 minecraft:soul_sand").late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
        c.step(op("op", "command", "command", "setblock 2 100 0 minecraft:soul_fire")).step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = cushionCase("tick_carpet_support"); c.tickCushions = true;
        c.cmd("setblock 2 99 0 minecraft:white_carpet").late("summon minecraft:cushion 2.5 99.0625 0.5 {color:\"blue\"}");
        c.step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = cushionCase("tick_half_under"); c.tickCushions = true;
        c.cmd("setblock 2 99 0 minecraft:stone_slab[type=bottom]").late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
        c.step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = cushionCase("tick_saved_color"); c.tickCushions = true;
        c.cmd("setblock 2 99 0 minecraft:stone").late("summon minecraft:cushion 2.5 100 0.5 {color:\"nonsense\"}").late("summon minecraft:cushion 3.5 100 0.5 {color:\"purple\"}");
        c.step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        // wp52: fluids in its cell and under it (the water is placed, it does not flow in a level that does not tick).
        for (String fluid : new String[] {"minecraft:water", "minecraft:lava"}) {
            c = cushionCase("tick_in_" + fluid.replaceAll("[^a-z0-9]", "_")); c.tickCushions = true; c.noPitch = true;
            c.cmd("fill 1 99 -1 3 101 1 minecraft:stone hollow").late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
            c.step(op("op", "command", "command", "setblock 2 100 0 " + fluid)).step(op("op", "tick_cushions", "ticks", 100));
            out.add(c);
            c = cushionCase("tick_support_" + fluid.replaceAll("[^a-z0-9]", "_")); c.tickCushions = true; c.noPitch = true;
            c.cmd("fill 1 99 -1 3 101 1 minecraft:stone hollow").cmd("setblock 2 98 0 minecraft:stone").late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
            c.step(op("op", "command", "command", "setblock 2 99 0 " + fluid)).step(op("op", "tick_cushions", "ticks", 100));
            out.add(c);
        }
        // a piston head or a moving block in the cell
        for (String block : new String[] {"minecraft:piston_head[facing=up,type=normal]", "minecraft:moving_piston[facing=up,type=normal]", "minecraft:honey_block", "minecraft:slime_block", "minecraft:glass", "minecraft:oak_trapdoor[half=top,open=false]"}) {
            c = cushionCase("tick_with_" + block.replaceAll("[^a-z0-9]", "_")); c.tickCushions = true;
            c.cmd("setblock 2 99 0 minecraft:stone").late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
            c.step(op("op", "command", "command", "setblock 2 100 0 " + block)).step(op("op", "tick_cushions", "ticks", 100)).step(op("op", "tick_cushions", "ticks", 100));
            out.add(c);
        }
        c = cushionCase("tick_while_sitting"); c.tickCushions = true;
        c.late("summon minecraft:cushion 2.5 100 0.5 {color:\"blue\"}");
        c.step(useEntity(2.5, 100.0, 0.5, 0, false)).step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
    }

    /** A mannequin case: ticked after every step, the mannequins summoned at the start (age 0), no pitches. */
    static Case mannequinCase(String name, String summon) {
        Case c = new Case("mannequin50_" + name).hanging();
        c.tickCushions = true;
        c.noPitch = true;
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 3 99 0 minecraft:stone").late(summon);
        return c;
    }

    static void mannequins50(List<Case> out) {
        Case c;
        String at = "summon minecraft:mannequin 2.5 100 0.5";
        c = mannequinCase("idle", at);
        c.step(op("op", "tick_cushions", "ticks", 5)).step(op("op", "tick_cushions", "ticks", 40));
        out.add(c);
        // ---- hitting: the weapon, the mode, the armor
        String[][] weapons = {{"fist", ""}, {"stick", "minecraft:stick"}, {"sword", "minecraft:diamond_sword"}, {"axe", "minecraft:iron_axe"}, {"trident", "minecraft:trident"},
                {"sharp", "minecraft:diamond_sword[enchantments={\"minecraft:sharpness\":5}]"}, {"fire", "minecraft:wooden_sword[enchantments={\"minecraft:fire_aspect\":2}]"},
                {"knock", "minecraft:stick[enchantments={\"minecraft:knockback\":2}]"}, {"mace", "minecraft:mace"}};
        for (String[] w : weapons) {
            c = mannequinCase("hit_" + w[0], at);
            if (!w[1].isEmpty()) c.slot("h0", parsed(w[1]));
            c.step(attackEntity(2.5, 100.5, 0.5)).step(op("op", "tick_cushions", "ticks", 3));
            out.add(c);
        }
        for (String mode : new String[] {"creative", "adventure"}) {
            c = mannequinCase("hit_" + mode, at);
            c.gameMode = mode;
            c.slot("h0", parsed("minecraft:diamond_sword"));
            c.step(attackEntity(2.5, 100.5, 0.5)).step(op("op", "tick_cushions", "ticks", 3));
            out.add(c);
        }
        c = mannequinCase("hit_twice_at_once", at);
        c.slot("h0", parsed("minecraft:diamond_sword"));
        c.step(attackEntity(2.5, 100.5, 0.5)).step(attackEntity(2.5, 100.5, 0.5));
        out.add(c);
        c = mannequinCase("hit_again_later", at);
        c.slot("h0", parsed("minecraft:iron_sword"));
        c.step(attackEntity(2.5, 100.5, 0.5)).step(op("op", "tick_cushions", "ticks", 12)).step(attackEntity(2.5, 100.5, 0.5)).step(op("op", "tick_cushions", "ticks", 12)).step(attackEntity(2.5, 100.5, 0.5));
        out.add(c);
        c = mannequinCase("hit_armored", "summon minecraft:mannequin 2.5 100 0.5 {equipment:{chest:{id:\"minecraft:diamond_chestplate\",count:1},head:{id:\"minecraft:iron_helmet\",count:1}}}");
        c.slot("h0", parsed("minecraft:diamond_sword"));
        c.step(attackEntity(2.5, 100.5, 0.5));
        out.add(c);
        c = mannequinCase("hit_absorption", "summon minecraft:mannequin 2.5 100 0.5 {AbsorptionAmount:4.0f}");
        c.slot("h0", parsed("minecraft:diamond_sword"));
        c.step(attackEntity(2.5, 100.5, 0.5));
        out.add(c);
        c = mannequinCase("hit_resistance", "summon minecraft:mannequin 2.5 100 0.5 {active_effects:[{id:\"minecraft:resistance\",amplifier:1b,duration:200}]}");
        c.slot("h0", parsed("minecraft:diamond_sword"));
        c.step(attackEntity(2.5, 100.5, 0.5));
        out.add(c);
        // ---- dying
        c = mannequinCase("kill", "summon minecraft:mannequin 2.5 100 0.5 {Health:3.0f,equipment:{mainhand:{id:\"minecraft:diamond\",count:1},head:{id:\"minecraft:iron_helmet\",count:1}}}");
        c.slot("h0", parsed("minecraft:diamond_sword"));
        c.step(attackEntity(2.5, 100.5, 0.5)).step(op("op", "tick_cushions", "ticks", 10)).step(op("op", "tick_cushions", "ticks", 20));
        out.add(c);
        c = mannequinCase("kill_creative", "summon minecraft:mannequin 2.5 100 0.5 {Health:3.0f}");
        c.gameMode = "creative";
        c.slot("h0", parsed("minecraft:diamond_sword"));
        c.step(attackEntity(2.5, 100.5, 0.5)).step(op("op", "tick_cushions", "ticks", 30));
        out.add(c);
        c = mannequinCase("kill_named", "summon minecraft:mannequin 2.5 100 0.5 {Health:3.0f,CustomName:'\"Dummy\"'}");
        c.slot("h0", parsed("minecraft:diamond_sword"));
        c.step(attackEntity(2.5, 100.5, 0.5)).step(op("op", "tick_cushions", "ticks", 30));
        out.add(c);
        // ---- the body
        c = mannequinCase("fall", "summon minecraft:mannequin 2.5 112 0.5");
        c.step(op("op", "tick_cushions", "ticks", 5)).step(op("op", "tick_cushions", "ticks", 20)).step(op("op", "tick_cushions", "ticks", 20));
        out.add(c);
        c = mannequinCase("fall_far", "summon minecraft:mannequin 2.5 140 0.5");
        c.step(op("op", "tick_cushions", "ticks", 40)).step(op("op", "tick_cushions", "ticks", 40));
        out.add(c);
        c = mannequinCase("no_gravity", "summon minecraft:mannequin 2.5 105 0.5 {NoGravity:1b}");
        c.step(op("op", "tick_cushions", "ticks", 20));
        out.add(c);
        c = mannequinCase("motion", "summon minecraft:mannequin 2.5 100 0.5 {Motion:[0.3d,0.5d,0.1d]}");
        c.step(op("op", "tick_cushions", "ticks", 10)).step(op("op", "tick_cushions", "ticks", 30));
        out.add(c);
        c = mannequinCase("immovable", "summon minecraft:mannequin 2.5 105 0.5 {immovable:1b,Motion:[0.3d,0.0d,0.0d]}");
        c.step(op("op", "tick_cushions", "ticks", 10)).step(op("op", "tick_cushions", "ticks", 30));
        out.add(c);
        c = mannequinCase("burning", "summon minecraft:mannequin 2.5 100 0.5 {Fire:100s}");
        c.step(op("op", "tick_cushions", "ticks", 25)).step(op("op", "tick_cushions", "ticks", 25));
        out.add(c);
        c = mannequinCase("lava", at);
        c.cmd("setblock 2 99 0 minecraft:lava");
        c.step(op("op", "tick_cushions", "ticks", 15));
        out.add(c);
        c = mannequinCase("water", at);
        // (A pool closed all round: the replay's water would flow, the recorded level's does not.)
        c.cmd("fill 1 98 -1 3 102 1 minecraft:stone").cmd("fill 2 99 0 2 101 0 minecraft:water");
        c.step(op("op", "tick_cushions", "ticks", 30));
        out.add(c);
        c = mannequinCase("suffocate", "summon minecraft:mannequin 2.5 100 0.5");
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("setblock 2 101 0 minecraft:stone");
        c.step(op("op", "tick_cushions", "ticks", 12));
        out.add(c);
        for (String pose : new String[] {"crouching", "swimming", "fall_flying", "sleeping"}) {
            c = mannequinCase("pose_" + pose, "summon minecraft:mannequin 2.5 100 0.5 {pose:\"" + pose + "\"}");
            c.step(op("op", "tick_cushions", "ticks", 5));
            out.add(c);
        }
        c = mannequinCase("effects", "summon minecraft:mannequin 2.5 100 0.5 {Health:10.0f,active_effects:[{id:\"minecraft:poison\",amplifier:0b,duration:300},{id:\"minecraft:regeneration\",amplifier:1b,duration:100}]}");
        c.step(op("op", "tick_cushions", "ticks", 50)).step(op("op", "tick_cushions", "ticks", 50));
        out.add(c);
        c = mannequinCase("wither", "summon minecraft:mannequin 2.5 100 0.5 {active_effects:[{id:\"minecraft:wither\",amplifier:1b,duration:300}]}");
        c.step(op("op", "tick_cushions", "ticks", 100));
        out.add(c);
        c = mannequinCase("instant_damage", "summon minecraft:mannequin 2.5 100 0.5 {Health:15.0f}");
        c.cmd("effect give @e[type=minecraft:mannequin] minecraft:instant_damage 1 1");
        c.step(op("op", "tick_cushions", "ticks", 5));
        out.add(c);
        // ---- clicking: nothing reacts
        for (String item : new String[] {"minecraft:name_tag", "minecraft:lead", "minecraft:shears", "minecraft:stick", "minecraft:diamond_chestplate", "minecraft:saddle", "minecraft:bucket", "minecraft:apple"}) {
            c = mannequinCase("use_" + item.substring(10), at);
            c.slot("h0", item.equals("minecraft:name_tag") ? parsed("minecraft:name_tag[custom_name='\"Bob\"']") : stack(item));
            c.step(useEntity(2.5, 100.5, 0.5, 0, false)).step(useEntity(2.5, 100.5, 0.5, 0, true));
            out.add(c);
        }
        c = mannequinCase("use_empty", at);
        c.step(useEntity(2.5, 100.5, 0.5, 0, false));
        out.add(c);
        c = mannequinCase("commands", at);
        c.step(op("op", "command", "command", "damage @e[type=minecraft:mannequin] 5 minecraft:generic")).step(op("op", "command", "command", "kill @e[type=minecraft:mannequin]"));
        out.add(c);
    }

    // ---------------------------------------------------------------- wp50: structure blocks and jigsaw blocks

    static Map<String, Object> setStructure(String update, String mode, String name, int[] off, int[] size, String mirror, String rotation, String metadata,
            boolean ignoreEntities, boolean strict, boolean showAir, boolean showBox, double integrity, long seed) {
        return setStructureAt(new int[] {2, 100, 0}, update, mode, name, off, size, mirror, rotation, metadata, ignoreEntities, strict, showAir, showBox, integrity, seed);
    }

    static Map<String, Object> setStructureAt(int[] at, String update, String mode, String name, int[] off, int[] size, String mirror, String rotation, String metadata,
            boolean ignoreEntities, boolean strict, boolean showAir, boolean showBox, double integrity, long seed) {
        return op("op", "set_structure", "pos", List.of(at[0], at[1], at[2]), "update", update, "mode", mode, "name", name,
                "offset", List.of(off[0], off[1], off[2]), "size", List.of(size[0], size[1], size[2]), "mirror", mirror, "rotation", rotation,
                "metadata", metadata, "ignore_entities", ignoreEntities, "strict", strict, "show_air", showAir, "show_box", showBox,
                "integrity", integrity, "seed", seed);
    }

    /** A structure block screen's packet with the usual settings (offset (1, 0, 0), size 3x3x3). */
    static Map<String, Object> structPacket(String update, String mode, String name) {
        return setStructure(update, mode, name, new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L);
    }

    static Case structCase(String name, String id) {
        Case c = new Case("structure50_" + name).hanging();
        c.op = true;
        c.gameMode = "creative";
        c.cmd("fill -2 98 -6 12 98 8 minecraft:stone").cmd("fill -2 99 -6 12 106 8 minecraft:air")
                .cmd("setblock 2 100 0 minecraft:structure_block[mode=save]");
        c.watchBox(0, 100, -3, 7, 103, 5);
        c.watch(2, 100, 0);
        if (id != null) c.template(id);
        return c;
    }

    /** The cube x 3..5, y 100..102, z 0..2 filled with all sorts of blocks. */
    static void mixedContent(Case c) {
        c.cmd("setblock 3 100 0 minecraft:stone")
                .cmd("setblock 4 100 0 minecraft:oak_stairs[facing=east,half=bottom,shape=straight]")
                .cmd("setblock 5 100 0 minecraft:chest[facing=north]{Items:[{Slot:0b,id:\"minecraft:diamond\",count:3}]}")
                .cmd("setblock 3 100 1 minecraft:oak_sign[rotation=4]{front_text:{messages:['\"hello\"','\"\"','\"\"','\"\"']}}")
                .cmd("setblock 4 100 1 minecraft:glass")
                .cmd("setblock 5 100 1 minecraft:water")
                .cmd("setblock 3 101 0 minecraft:structure_void")
                .cmd("setblock 4 101 1 minecraft:lever[face=floor,facing=north,powered=true]")
                .cmd("setblock 5 100 2 minecraft:stone").cmd("setblock 5 101 2 minecraft:redstone_wire[power=7]")
                .cmd("setblock 3 102 2 minecraft:oak_fence").cmd("setblock 3 101 2 minecraft:oak_fence")
                .cmd("setblock 4 102 0 minecraft:jigsaw[orientation=east_up]{pool:\"minecraft:empty\",name:\"minecraft:a\",target:\"minecraft:b\",joint:\"aligned\",final_state:\"minecraft:stone\",placement_priority:3,selection_priority:2}")
                .cmd("setblock 5 102 1 minecraft:structure_block[mode=data]{metadata:\"chest\",name:\"minecraft:x\"}");
    }

    /** wp52: entities in the cube x 3..5, y 100..102, z 0..2 (and two just outside it). */
    static void entitiesContent(Case c, boolean rider) {
        // (Summoned in the order of their places: the simulation numbers the entities of one tick that way, vanilla by when they came.)
        c.cmd("setblock 3 100 0 minecraft:stone").cmd("setblock 5 101 2 minecraft:stone").cmd("setblock 5 102 1 minecraft:stone")
                .cmd("summon minecraft:marker 3.2 102.0 2.2 {UUID:[I;1,0,0,5],data:{a:1b}}")
                .cmd("summon minecraft:armor_stand 3.5 100.0 0.5 {UUID:[I;1,0,0,1],NoGravity:1b,ShowArms:1b}")
                .cmd(rider ? "summon minecraft:minecart 4.5 100.0 1.5 {UUID:[I;1,0,0,2],NoGravity:1b,Passengers:[{id:\"minecraft:armor_stand\",UUID:[I;1,0,0,3],NoGravity:1b}]}" : "summon minecraft:minecart 4.5 100.0 1.5 {UUID:[I;1,0,0,2],NoGravity:1b}")
                .cmd("summon minecraft:item 4.5 101.0 0.5 {UUID:[I;1,0,0,4],Item:{id:\"minecraft:stone\",count:3},Age:100s,PickupDelay:5s,NoGravity:1b}")
                .cmd("summon minecraft:item_frame 5 101 1 {UUID:[I;1,0,0,9],Facing:2b,Item:{id:\"minecraft:stick\",count:1}}")
                .cmd("summon minecraft:painting 5 102 0 {UUID:[I;1,0,0,10],facing:2b,variant:\"minecraft:kebab\"}")
                .cmd("summon minecraft:armor_stand 5.9 100.0 0.5 {UUID:[I;1,0,0,6],NoGravity:1b}")
                .cmd("summon minecraft:armor_stand 6.7 100.0 0.5 {UUID:[I;1,0,0,7],NoGravity:1b}")
                .cmd("summon minecraft:pig 8.5 100.0 1.5 {UUID:[I;1,0,0,8],NoAI:1b,Silent:1b}");
    }

    static void structures50(List<Case> out) {
        Case c;
        // ---- putting one down: a structure block remembers who placed it
        for (String block : new String[] {"minecraft:structure_block", "minecraft:jigsaw", "minecraft:command_block"}) {
            c = new Case("structure50_place_" + block.substring(10)).hanging();
            c.op = true;
            c.gameMode = "creative";
            c.cmd("setblock 2 99 0 minecraft:stone").watch(2, 100, 0);
            c.slot("h0", stack(block));
            c.step(useOn(2, 99, 0, 1, 0));
            out.add(c);
        }
        // ---- clicking: the structure block takes the click of an operator in creative mode (its screen opens), others put a block on it
        for (String who : new String[] {"op_creative", "op_survival", "creative", "survival"}) {
            c = new Case("structure50_click_" + who).hanging();
            c.op = who.startsWith("op");
            c.gameMode = who.endsWith("creative") ? "creative" : "survival";
            c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 minecraft:structure_block[mode=save]").watch(2, 100, 0).watch(2, 101, 0);
            c.slot("h0", stack("minecraft:cobblestone", 4)).stat("minecraft:cobblestone");
            c.step(useOn(2, 100, 0, 1, 0));
            out.add(c);
            c = new Case("structure50_click_jigsaw_" + who).hanging();
            c.op = who.startsWith("op");
            c.gameMode = who.endsWith("creative") ? "creative" : "survival";
            c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 minecraft:jigsaw[orientation=north_up]").watch(2, 100, 0).watch(2, 101, 0);
            c.slot("h0", stack("minecraft:cobblestone", 4)).stat("minecraft:cobblestone");
            c.step(useOn(2, 100, 0, 1, 0));
            out.add(c);
        }
        // ---- the screen's settings
        c = structCase("update_all", null);
        c.step(setStructure("UPDATE_DATA", "SAVE", "wp50:house", new int[] {-3, 2, 5}, new int[] {7, 8, 9}, "LEFT_RIGHT", "CLOCKWISE_90", "meta data", false, true, true, false, 0.25, 123456789012L));
        c.step(setStructure("UPDATE_DATA", "LOAD", "wp50:house", new int[] {1, 0, 0}, new int[] {1, 1, 1}, "FRONT_BACK", "COUNTERCLOCKWISE_90", "", true, false, false, true, 1.0, 0L));
        c.step(setStructure("UPDATE_DATA", "CORNER", "wp50:house", new int[] {1, 0, 0}, new int[] {1, 1, 1}, "NONE", "CLOCKWISE_180", "x", true, false, false, true, 0.5, -5L));
        c.step(setStructure("UPDATE_DATA", "DATA", "wp50:house", new int[] {0, 0, 0}, new int[] {0, 0, 0}, "NONE", "NONE", "player_spawn", true, false, false, true, 1.0, 0L));
        out.add(c);
        for (String name : new String[] {"", "house", "wp50:house", "Bad Name", "a:b:c", "wp50:dir/sub/house", "WP50:upper", "wp50:ok_name.1-2"}) {
            c = structCase("name_" + name.replaceAll("[^a-zA-Z0-9]", "_"), null);
            c.step(structPacket("UPDATE_DATA", "SAVE", name));
            out.add(c);
        }
        // a block entity that was not given its fields (made by /setblock): the defaults
        c = structCase("defaults", null);
        c.cmd("setblock 6 100 0 minecraft:structure_block").cmd("setblock 7 100 0 minecraft:structure_block[mode=corner]{name:\"wp50:q\",posX:100,posY:-100,sizeX:99,sizeZ:-3,rotation:\"BAD\",mode:\"CORNER\",integrity:7.5f}");
        c.watch(6, 100, 0).watch(7, 100, 0);
        c.step(op("op", "command", "command", "data merge block 2 100 0 {mode:\"LOAD\",integrity:0.5f,seed:5L}"));
        out.add(c);
        // not an operator (or not in creative): nothing happens
        for (String who : new String[] {"survival_op", "creative_not_op"}) {
            c = structCase("denied_" + who, "wp50:denied");
            c.op = who.equals("survival_op");
            c.gameMode = who.equals("survival_op") ? "survival" : "creative";
            mixedContent(c);
            c.step(structPacket("UPDATE_DATA", "SAVE", "wp50:denied")).step(structPacket("SAVE_AREA", "SAVE", "wp50:denied"));
            out.add(c);
        }
        // ---- saving an area
        c = structCase("save_mixed", "wp50:mixed");
        mixedContent(c);
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:mixed"));
        out.add(c);
        c = structCase("save_mixed_entities_flag", "wp50:mixed2");
        mixedContent(c);
        c.step(setStructure("SAVE_AREA", "SAVE", "wp50:mixed2", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", false, false, false, true, 1.0, 0L));
        out.add(c);
        // ---- wp52: the entities of the area (`fillEntityList`): not the players, a rider inside its vehicle, a painting by the block it hangs on
        for (String ignore : new String[] {"with", "ignored"}) {
            c = structCase("save_entities_" + ignore, "wp52:ents_" + ignore);
            entitiesContent(c, true);
            c.step(setStructure("SAVE_AREA", "SAVE", "wp52:ents_" + ignore, new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", ignore.equals("ignored"), false, false, true, 1.0, 0L));
            out.add(c);
        }
        // the area reaches over section borders (the order of the entities is the order of the sections)
        c = structCase("save_entities_sections", "wp52:sects");
        c.cmd("setblock -3 100 -2 minecraft:stone")
                .cmd("summon minecraft:armor_stand 3.5 100.0 -0.5 {UUID:[I;2,0,0,1],NoGravity:1b}")
                .cmd("summon minecraft:armor_stand -1.5 100.0 -0.5 {UUID:[I;2,0,0,2],NoGravity:1b}")
                .cmd("summon minecraft:armor_stand 3.5 100.0 0.5 {UUID:[I;2,0,0,3],NoGravity:1b}")
                .cmd("summon minecraft:armor_stand -1.5 100.0 0.5 {UUID:[I;2,0,0,4],NoGravity:1b}")
                .cmd("summon minecraft:armor_stand 3.9 100.0 0.5 {UUID:[I;2,0,0,5],NoGravity:1b}")
                .cmd("summon minecraft:armor_stand 1.5 120.0 0.5 {UUID:[I;2,0,0,6],NoGravity:1b}");
        c.step(setStructure("SAVE_AREA", "SAVE", "wp52:sects", new int[] {-5, 0, -2}, new int[] {9, 3, 4}, "NONE", "NONE", "", false, false, false, true, 1.0, 0L));
        out.add(c);
        // a template with entities comes back (the hanging ones, the stands and the items are what the vectors can see)
        c = structCase("load_entities", "wp52:back");
        entitiesContent(c, false);
        c.cmd("setblock 2 100 6 minecraft:structure_block[mode=load]").watch(2, 100, 6);
        c.step(setStructure("SAVE_AREA", "SAVE", "wp52:back", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", false, false, false, true, 1.0, 0L));
        c.step(setStructureAt(new int[] {2, 100, 6}, "UPDATE_DATA", "LOAD", "wp52:back", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", false, false, false, true, 1.0, 0L));
        c.step(setStructureAt(new int[] {2, 100, 6}, "LOAD_AREA", "LOAD", "wp52:back", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", false, false, false, true, 1.0, 0L));
        // turned and mirrored, the entities go with the blocks
        c.step(setStructureAt(new int[] {2, 100, 6}, "LOAD_AREA", "LOAD", "wp52:back", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "LEFT_RIGHT", "CLOCKWISE_90", "", false, false, false, true, 1.0, 0L));
        c.stands();
        out.add(c);
        c = structCase("save_air", "wp50:air");
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:air"));
        out.add(c);
        c = structCase("save_one_block", "wp50:one");
        c.cmd("setblock 3 100 0 minecraft:diamond_block");
        c.step(setStructure("SAVE_AREA", "SAVE", "wp50:one", new int[] {1, 0, 0}, new int[] {1, 1, 1}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("save_size_zero", "wp50:zero");
        c.cmd("setblock 3 100 0 minecraft:diamond_block");
        c.step(setStructure("SAVE_AREA", "SAVE", "wp50:zero", new int[] {1, 0, 0}, new int[] {0, 3, 3}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("save_negative_offset", "wp50:neg");
        c.cmd("fill -1 100 -2 1 101 0 minecraft:cobblestone");
        c.step(setStructure("SAVE_AREA", "SAVE", "wp50:neg", new int[] {-3, 0, -2}, new int[] {3, 2, 3}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("save_twice", "wp50:twice");
        c.cmd("setblock 3 100 0 minecraft:stone");
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:twice")).step(op("op", "command", "command", "setblock 4 100 0 minecraft:gold_block")).step(structPacket("SAVE_AREA", "SAVE", "wp50:twice"));
        out.add(c);
        c = structCase("save_no_name", null);
        mixedContent(c);
        c.step(structPacket("SAVE_AREA", "SAVE", ""));
        out.add(c);
        c = structCase("save_in_load_mode", "wp50:wrongmode");
        mixedContent(c);
        c.step(structPacket("SAVE_AREA", "LOAD", "wp50:wrongmode"));
        out.add(c);
        c = structCase("save_big", "wp50:big");
        c.cmd("fill 3 100 0 20 120 20 minecraft:copper_block hollow");
        c.step(setStructure("SAVE_AREA", "SAVE", "wp50:big", new int[] {1, 0, 0}, new int[] {20, 20, 20}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("save_blocks_with_entities", "wp50:bes");
        c.cmd("setblock 3 100 0 minecraft:barrel[facing=up]{Items:[{Slot:3b,id:\"minecraft:apple\",count:5},{Slot:4b,id:\"minecraft:stick\",count:1}],CustomName:'\"Box\"'}")
                .cmd("setblock 4 100 0 minecraft:furnace[facing=south,lit=false]{BurnTime:10s}")
                .cmd("setblock 5 100 0 minecraft:spawner{SpawnData:{entity:{id:\"minecraft:pig\"}}}")
                .cmd("setblock 3 100 1 minecraft:white_banner[rotation=3]{patterns:[{color:\"red\",pattern:\"minecraft:stripe_top\"}]}")
                .cmd("setblock 4 100 1 minecraft:player_head[rotation=2]")
                .cmd("setblock 5 100 1 minecraft:command_block[facing=north]{Command:\"say hi\"}")
                .cmd("setblock 3 100 2 minecraft:lectern[facing=west]")
                .cmd("setblock 4 100 2 minecraft:bell[attachment=floor,facing=north]")
                .cmd("setblock 5 100 2 minecraft:decorated_pot");
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:bes"));
        out.add(c);
        // ---- loading an area it saved (cleared in between)
        String[][] places = {
                {"same", "NONE", "NONE"}, {"rot90", "NONE", "CLOCKWISE_90"}, {"rot180", "NONE", "CLOCKWISE_180"}, {"rot270", "NONE", "COUNTERCLOCKWISE_90"},
                {"mirror_lr", "LEFT_RIGHT", "NONE"}, {"mirror_fb", "FRONT_BACK", "NONE"}, {"mirror_lr_rot90", "LEFT_RIGHT", "CLOCKWISE_90"}, {"mirror_fb_rot270", "FRONT_BACK", "COUNTERCLOCKWISE_90"}};
        for (String[] pl : places) {
            c = structCase("load_" + pl[0], "wp50:l_" + pl[0]);
            mixedContent(c);
            c.step(structPacket("SAVE_AREA", "SAVE", "wp50:l_" + pl[0]));
            c.step(op("op", "command", "command", "fill 3 100 0 5 102 2 minecraft:air"));
            c.step(setStructure("LOAD_AREA", "LOAD", "wp50:l_" + pl[0], new int[] {1, 0, 0}, new int[] {3, 3, 3}, pl[1], pl[2], "", true, false, false, true, 1.0, 0L));
            out.add(c);
        }
        c = structCase("load_other_offset", "wp50:off");
        mixedContent(c);
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:off"));
        c.step(op("op", "command", "command", "fill 3 100 0 5 102 2 minecraft:air"));
        c.step(setStructure("LOAD_AREA", "LOAD", "wp50:off", new int[] {2, 1, 2}, new int[] {3, 3, 3}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("load_over_blocks", "wp50:over");
        c.cmd("setblock 3 100 0 minecraft:stone").cmd("setblock 4 101 1 minecraft:oak_planks");
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:over"));
        c.step(op("op", "command", "command", "fill 3 100 0 5 102 2 minecraft:obsidian"));
        c.step(setStructure("LOAD_AREA", "LOAD", "wp50:over", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        for (double integrity : new double[] {0.0, 0.3, 0.5, 0.9}) {
            for (long seed : new long[] {42L, -7L, 9999999999L}) {
                c = structCase("load_integrity_" + (int) (integrity * 10) + "_" + seed, "wp50:i" + (int) (integrity * 10) + "s" + Math.abs(seed));
                c.cmd("fill 3 100 0 5 102 2 minecraft:bricks");
                c.step(structPacket("SAVE_AREA", "SAVE", "wp50:i" + (int) (integrity * 10) + "s" + Math.abs(seed)));
                c.step(op("op", "command", "command", "fill 3 100 0 5 102 2 minecraft:air"));
                c.step(setStructure("LOAD_AREA", "LOAD", "wp50:i" + (int) (integrity * 10) + "s" + Math.abs(seed), new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", true, false, false, true, integrity, seed));
                out.add(c);
            }
        }
        c = structCase("load_strict", "wp50:strict");
        mixedContent(c);
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:strict"));
        c.step(op("op", "command", "command", "fill 3 100 0 5 102 2 minecraft:air"));
        c.step(setStructure("LOAD_AREA", "LOAD", "wp50:strict", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", true, true, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("load_not_found", null);
        c.step(setStructure("LOAD_AREA", "LOAD", "wp50:nowhere", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("load_wrong_mode", "wp50:lm");
        mixedContent(c);
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:lm"));
        c.step(setStructure("LOAD_AREA", "DATA", "wp50:lm", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("load_other_size", "wp50:size");
        mixedContent(c);
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:size"));
        c.step(op("op", "command", "command", "fill 3 100 0 5 102 2 minecraft:air"));
        c.step(setStructure("LOAD_AREA", "LOAD", "wp50:size", new int[] {1, 0, 0}, new int[] {5, 1, 2}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        c.step(setStructure("LOAD_AREA", "LOAD", "wp50:size", new int[] {1, 0, 0}, new int[] {3, 3, 3}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("load_vanilla_template", "minecraft:igloo/top");
        c.step(setStructure("LOAD_AREA", "LOAD", "minecraft:igloo/top", new int[] {1, 0, 0}, new int[] {7, 5, 8}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        c.step(setStructure("LOAD_AREA", "LOAD", "minecraft:igloo/top", new int[] {1, 0, 0}, new int[] {7, 5, 8}, "NONE", "CLOCKWISE_90", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        c = structCase("load_updates_data", "minecraft:igloo/top");
        c.step(op("op", "command", "command", "data merge block 2 100 0 {mode:\"LOAD\",name:\"minecraft:igloo/top\"}"));
        c.step(setStructure("LOAD_AREA", "LOAD", "minecraft:igloo/top", new int[] {1, 0, 0}, new int[] {1, 1, 1}, "NONE", "NONE", "", true, false, false, true, 1.0, 0L));
        out.add(c);
        // ---- scanning for corners
        for (String variant : new String[] {"two", "one", "three", "none", "other_name", "far", "flat", "mode_load", "inverted"}) {
            c = structCase("scan_" + variant, null);
            c.cmd("setblock 2 100 0 minecraft:structure_block[mode=save]{name:\"wp50:scan\"}");
            switch (variant) {
                case "two" -> c.cmd("setblock 3 100 0 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}").cmd("setblock 7 104 5 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}");
                case "one" -> c.cmd("setblock 6 103 4 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}");
                case "three" -> c.cmd("setblock 3 100 0 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}").cmd("setblock 7 104 5 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}")
                        .cmd("setblock 9 101 -3 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}");
                case "other_name" -> c.cmd("setblock 3 100 0 minecraft:structure_block[mode=corner]{name:\"wp50:other\"}").cmd("setblock 7 104 5 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}");
                case "far" -> c.cmd("setblock 3 100 0 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}").cmd("setblock 90 104 5 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}");
                case "flat" -> c.cmd("setblock 3 100 0 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}").cmd("setblock 7 100 5 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}");
                case "mode_load" -> c.cmd("setblock 3 100 0 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}").cmd("setblock 7 104 5 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}");
                case "inverted" -> c.cmd("setblock -1 104 -3 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}").cmd("setblock 7 100 5 minecraft:structure_block[mode=corner]{name:\"wp50:scan\"}");
                default -> { }
            }
            c.step(structPacket("SCAN_AREA", variant.equals("mode_load") ? "LOAD" : "SAVE", "wp50:scan"));
            out.add(c);
        }
        // ---- redstone: save when powered, load when powered, unload a corner
        for (String mode : new String[] {"SAVE", "LOAD", "CORNER", "DATA"}) {
            c = structCase("power_" + mode.toLowerCase(), "wp50:p_" + mode.toLowerCase());
            mixedContent(c);
            c.step(structPacket("UPDATE_DATA", "SAVE", "wp50:p_" + mode.toLowerCase()));
            c.step(structPacket("SAVE_AREA", "SAVE", "wp50:p_" + mode.toLowerCase()));
            c.step(op("op", "command", "command", "fill 3 100 0 5 102 2 minecraft:air"));
            c.step(structPacket("UPDATE_DATA", mode, "wp50:p_" + mode.toLowerCase()));
            c.step(op("op", "command", "command", "setblock 2 101 0 minecraft:redstone_block"));
            c.step(op("op", "command", "command", "fill 3 100 0 5 102 2 minecraft:air"));
            c.step(op("op", "command", "command", "setblock 2 101 0 minecraft:air"));
            c.step(op("op", "command", "command", "setblock 2 101 0 minecraft:redstone_block"));
            out.add(c);
        }
        c = structCase("power_save_new_content", "wp50:p_new");
        c.cmd("setblock 3 100 0 minecraft:stone");
        c.step(structPacket("UPDATE_DATA", "SAVE", "wp50:p_new"));
        c.step(op("op", "command", "command", "setblock 2 101 0 minecraft:redstone_block")).step(op("op", "command", "command", "setblock 3 100 0 minecraft:gold_block"))
                .step(op("op", "command", "command", "setblock 2 101 0 minecraft:air")).step(op("op", "command", "command", "setblock 2 101 0 minecraft:redstone_block"));
        out.add(c);
        c = structCase("power_load_missing", null);
        c.step(structPacket("UPDATE_DATA", "LOAD", "wp50:missing")).step(op("op", "command", "command", "setblock 2 101 0 minecraft:redstone_block"));
        out.add(c);
        c = structCase("power_by_item", "wp50:p_item");
        c.cmd("setblock 3 100 0 minecraft:stone");
        c.slot("h0", stack("minecraft:redstone_block"));
        c.step(structPacket("UPDATE_DATA", "SAVE", "wp50:p_item")).step(useOn(2, 100, 0, 5, 0));
        out.add(c);
        c = structCase("power_corner_unload", "wp50:p_corner");
        c.cmd("setblock 3 100 0 minecraft:stone");
        c.step(structPacket("SAVE_AREA", "SAVE", "wp50:p_corner"))
                .step(structPacket("UPDATE_DATA", "CORNER", "wp50:p_corner")).step(op("op", "command", "command", "setblock 2 101 0 minecraft:redstone_block"))
                .step(structPacket("UPDATE_DATA", "LOAD", "wp50:p_corner")).step(op("op", "command", "command", "setblock 2 101 0 minecraft:air")).step(op("op", "command", "command", "setblock 2 101 0 minecraft:redstone_block"));
        out.add(c);
        // ---- jigsaw blocks
        String[][] jigsaws = {
                {"all", "minecraft:a", "minecraft:b", "minecraft:village/plains/houses", "minecraft:stone", "ALIGNED", "5", "-3"},
                {"rollable", "minecraft:door", "minecraft:door", "minecraft:empty", "minecraft:air", "ROLLABLE", "0", "0"},
                {"namespaced", "wp50:x/y", "wp50:z", "wp50:pool/sub", "minecraft:oak_stairs[facing=east]", "ALIGNED", "100", "100"},
                {"bad_state", "minecraft:a", "minecraft:b", "minecraft:empty", "not a block state", "ROLLABLE", "1", "1"},
                {"long_state", "minecraft:a", "minecraft:b", "minecraft:empty", "x".repeat(300), "ROLLABLE", "1", "1"}};
        for (String[] j : jigsaws) {
            c = new Case("structure50_jigsaw_" + j[0]).hanging();
            c.op = true;
            c.gameMode = "creative";
            c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 minecraft:jigsaw[orientation=up_north]").watch(2, 100, 0);
            c.step(op("op", "set_jigsaw", "pos", List.of(2, 100, 0), "name", j[1], "target", j[2], "pool", j[3], "final_state", j[4], "joint", j[5], "selection", Integer.parseInt(j[6]), "placement", Integer.parseInt(j[7])));
            out.add(c);
        }
        for (String who : new String[] {"survival", "creative_not_op"}) {
            c = new Case("structure50_jigsaw_denied_" + who).hanging();
            c.op = who.equals("survival");
            c.gameMode = who.equals("survival") ? "survival" : "creative";
            c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 0 minecraft:jigsaw[orientation=up_north]").watch(2, 100, 0);
            c.step(op("op", "set_jigsaw", "pos", List.of(2, 100, 0), "name", "minecraft:a", "target", "minecraft:b", "pool", "minecraft:empty", "final_state", "minecraft:air", "joint", "ROLLABLE", "selection", 0, "placement", 0));
            out.add(c);
        }
        c = new Case("structure50_jigsaw_wrong_block").hanging();
        c.op = true;
        c.gameMode = "creative";
        c.cmd("setblock 2 100 0 minecraft:stone").watch(2, 100, 0);
        c.step(op("op", "set_jigsaw", "pos", List.of(2, 100, 0), "name", "minecraft:a", "target", "minecraft:b", "pool", "minecraft:empty", "final_state", "minecraft:air", "joint", "ROLLABLE", "selection", 0, "placement", 0));
        out.add(c);
        c = new Case("structure50_jigsaw_defaults").hanging();
        c.op = true;
        c.gameMode = "creative";
        c.cmd("setblock 2 100 0 minecraft:jigsaw[orientation=north_up]").cmd("setblock 3 100 0 minecraft:jigsaw[orientation=up_east]").cmd("setblock 4 100 0 minecraft:jigsaw[orientation=east_up]{joint:\"bad\",pool:\"bad pool\",placement_priority:\"x\"}")
                .watch(2, 100, 0).watch(3, 100, 0).watch(4, 100, 0);
        c.step(op("op", "command", "command", "data merge block 2 100 0 {name:\"minecraft:set\"}"));
        out.add(c);
    }

    // ---------------------------------------------------------------- wp50: "moved wrongly"

    static Map<String, Object> move(double x, double y, double z, boolean onGround) {
        return op("op", "move", "to", List.of(x, y, z), "on_ground", onGround, "hcol", false);
    }

    /** A flat floor of stone at y = 98 over x = -4..12 (z = -6..8) with air above, the player standing at (3.5, 99, 0.5). */
    static Case moveCase(String name, String mode, boolean sneaking) {
        Case c = new Case("moves50_" + name).moves();
        c.gameMode = mode;
        c.sneaking = sneaking;
        c.pos = new double[] {3.5, 99.0, 0.5};
        c.cmd("fill -4 98 -6 12 98 8 minecraft:stone").cmd("fill -4 99 -6 12 106 8 minecraft:air");
        return c;
    }

    static void moves50(List<Case> out) {
        Case c;
        // ---- plain moves are taken as they come
        for (double d : new double[] {0.0, 0.1, 0.25, 0.9}) {
            c = moveCase("walk_" + (int) (d * 100), "survival", false);
            c.step(move(3.5 + d, 99.0, 0.5, true));
            out.add(c);
        }
        c = moveCase("jump", "survival", false);
        c.step(move(3.5, 99.4, 0.5, false)).step(op("op", "accept_teleport"));
        out.add(c);
        c = moveCase("fall_through_floor", "survival", false);
        c.step(move(3.5, 97.6, 0.5, false));
        out.add(c);
        c = moveCase("into_wall", "survival", false);
        c.cmd("fill 4 99 -2 4 101 2 minecraft:stone");
        c.step(move(4.0, 99.0, 0.5, true));
        out.add(c);
        c = moveCase("up_a_step", "survival", false);
        c.cmd("setblock 4 99 0 minecraft:stone");
        c.step(move(4.2, 100.0, 0.5, true));
        out.add(c);
        c = moveCase("up_a_step_too_high", "survival", false);
        c.cmd("fill 4 99 0 4 100 0 minecraft:stone");
        c.step(move(4.2, 101.0, 0.5, true));
        out.add(c);
        // ---- a sneaking player does not walk off an edge: the server's body stays and the client's claim is wrong beyond a quarter block
        for (String mode : new String[] {"survival", "creative", "spectator", "adventure"}) {
            for (double d : new double[] {0.5, 0.9, 1.05, 1.4}) {
                c = moveCase("edge_" + mode + "_" + (int) (d * 100), mode, true);
                c.cmd("fill 4 98 -6 12 98 8 minecraft:air");
                c.step(move(3.5 + d, 99.0, 0.5, true));
                out.add(c);
            }
        }
        c = moveCase("edge_not_sneaking", "survival", false);
        c.cmd("fill 4 98 -6 12 98 8 minecraft:air");
        c.step(move(4.9, 99.0, 0.5, false));
        out.add(c);
        // ---- a teleport waits for its answer: moves before it are not heard
        c = moveCase("edge_then_walk", "survival", true);
        c.cmd("fill 4 98 -6 12 98 8 minecraft:air");
        c.step(move(4.9, 99.0, 0.5, true)).step(move(3.6, 99.0, 0.5, true)).step(op("op", "accept_teleport")).step(move(3.6, 99.0, 0.5, true));
        out.add(c);
    }

    // ---------------------------------------------------------------- wp52: a move that is put back still checks the fall; the grace time after an impulse

    static void moves52(List<Case> out) {
        Case c;
        for (double fall : new double[] {2.0, 8.0, 18.0}) {
            // taken: the fall counts when the client says it landed
            c = moveCase("fall_taken_" + (int) fall, "survival", false);
            c.step(op("op", "set_fall", "distance", fall)).step(move(3.7, 99.0, 0.5, true));
            out.add(c);
            // put back (into a wall): the fall is checked where the player is
            c = moveCase("fall_rejected_" + (int) fall, "survival", false);
            c.cmd("fill 4 99 -2 4 101 2 minecraft:stone");
            c.step(op("op", "set_fall", "distance", fall)).step(move(4.0, 99.0, 0.5, true));
            out.add(c);
            c = moveCase("fall_rejected_air_" + (int) fall, "survival", false);
            c.cmd("fill 4 99 -2 4 101 2 minecraft:stone");
            c.step(op("op", "set_fall", "distance", fall)).step(move(4.0, 99.0, 0.5, false));
            out.add(c);
            // the edge of a sneaking player: the server's body stays, the claim is wrong
            c = moveCase("fall_edge_" + (int) fall, "survival", true);
            c.cmd("fill 4 98 -6 12 98 8 minecraft:air");
            c.step(op("op", "set_fall", "distance", fall)).step(move(4.9, 99.0, 0.5, true));
            out.add(c);
        }
        // creative players take no fall damage, put back or not
        c = moveCase("fall_rejected_creative", "creative", false);
        c.cmd("fill 4 99 -2 4 101 2 minecraft:stone");
        c.step(op("op", "set_fall", "distance", 20.0)).step(move(4.0, 99.0, 0.5, true));
        out.add(c);
        // the grace time after an impulse: a claim far from the body is taken
        for (int grace : new int[] {0, 40}) {
            c = moveCase("grace_" + grace, "survival", true);
            c.cmd("fill 4 98 -6 12 98 8 minecraft:air");
            c.step(op("op", "set_grace", "ticks", grace)).step(move(4.9, 99.0, 0.5, true));
            out.add(c);
        }
        c = moveCase("grace_into_wall", "survival", false);
        c.cmd("fill 4 99 -2 4 101 2 minecraft:stone");
        c.step(op("op", "set_grace", "ticks", 40)).step(move(4.0, 99.0, 0.5, true));
        out.add(c);
    }

    static void cauldrons50(List<Case> out) {
        Case c;
        String[] customs = {"minecraft:fill_cauldron", "minecraft:use_cauldron", "minecraft:clean_armor", "minecraft:clean_banner", "minecraft:clean_shulker_box"};
        List<String[]> held = new ArrayList<>(List.of(
                new String[] {"empty_hand", ""},
                new String[] {"bucket", "minecraft:bucket"},
                new String[] {"water_bucket", "minecraft:water_bucket"},
                new String[] {"lava_bucket", "minecraft:lava_bucket"},
                new String[] {"powder_snow_bucket", "minecraft:powder_snow_bucket"},
                new String[] {"glass_bottle", "minecraft:glass_bottle"},
                new String[] {"water_bottle", "minecraft:potion[potion_contents={potion:\"minecraft:water\"}]"},
                new String[] {"healing_potion", "minecraft:potion[potion_contents={potion:\"minecraft:healing\"}]"},
                new String[] {"leather_dyed", "minecraft:leather_chestplate[dyed_color=16711680]"},
                new String[] {"leather_plain", "minecraft:leather_chestplate"},
                new String[] {"leather_horse_armor_dyed", "minecraft:leather_horse_armor[dyed_color=255]"},
                new String[] {"red_shulker", "minecraft:red_shulker_box"},
                new String[] {"plain_shulker", "minecraft:shulker_box"},
                new String[] {"banner_layers", "minecraft:red_banner[" + LAYERS + "]"},
                new String[] {"banner_one", "minecraft:blue_banner[banner_patterns=[{pattern:\"minecraft:skull\",color:\"white\"}]]"},
                new String[] {"banner_plain", "minecraft:white_banner"},
                new String[] {"banner_stack", "minecraft:red_banner[" + LAYERS + "]"},
                new String[] {"tipped_arrow", "minecraft:tipped_arrow[potion_contents={potion:\"minecraft:healing\"}]"},
                new String[] {"stone", "minecraft:stone"}));
        for (String[] cauldron : new String[][] {
                {"empty", "minecraft:cauldron"}, {"water1", "minecraft:water_cauldron[level=1]"}, {"water2", "minecraft:water_cauldron[level=2]"},
                {"water3", "minecraft:water_cauldron[level=3]"}, {"lava", "minecraft:lava_cauldron"}, {"snow1", "minecraft:powder_snow_cauldron[level=1]"},
                {"snow3", "minecraft:powder_snow_cauldron[level=3]"}}) {
            for (String[] h : held) {
                for (boolean creative : new boolean[] {false, true}) {
                    if (creative && !(h[0].equals("bucket") || h[0].equals("water_bucket") || h[0].equals("glass_bottle") || h[0].equals("banner_layers") || h[0].equals("water_bottle"))) continue;
                    c = blockCase("cauldron50_" + cauldron[0] + "_" + h[0] + (creative ? "_creative" : ""), cauldron[1]);
                    if (creative) c.gameMode = "creative";
                    if (!h[1].isEmpty()) {
                        ItemStack s = parsed(h[1]);
                        if (h[0].equals("banner_stack")) s.setCount(3);
                        c.slot("h0", s);
                        c.stat(h[1].contains("[") ? h[1].substring(0, h[1].indexOf('[')) : h[1]);
                    }
                    for (String custom : customs) c.custom(custom);
                    c.step(useOn(2, 100, 0, 1, 0));
                    out.add(c);
                }
            }
        }
        // Rain and snow filling are server ticks (verified elsewhere); a sneaking player with an item uses the item, not the cauldron.
        c = blockCase("cauldron50_sneaking_bucket", "minecraft:water_cauldron[level=3]");
        c.sneaking = true;
        c.slot("h0", stack("minecraft:bucket")).stat("minecraft:bucket");
        for (String custom : customs) c.custom(custom);
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("cauldron50_adventure_bucket", "minecraft:water_cauldron[level=3]");
        c.gameMode = "adventure";
        c.slot("h0", stack("minecraft:bucket")).stat("minecraft:bucket");
        for (String custom : customs) c.custom(custom);
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
    }

    static Map<String, Object> useOnAt(int x, int y, int z, int face, int hand, double cx, double cy, double cz) {
        return op("op", "use_on", "hand", hand, "pos", List.of(x, y, z), "face", face, "cursor", List.of(cx, cy, cz));
    }

    /** wp49: campfires (food on the fire), flower pots, chiseled bookshelves. */
    static void blocks49(List<Case> out) {
        Case c;
        // ---- campfires
        // (The fire is out where the block entity is compared: Kiln's level ticks between the steps, vanilla's is frozen.)
        String lit = "minecraft:campfire[facing=north,lit=false,waterlogged=false,signal_fire=false]";
        c = blockCase("campfire_five_foods", lit).custom("minecraft:interact_with_campfire");
        c.slot("h0", stack("minecraft:beef", 6));
        for (int i = 0; i < 6; i++) c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // A lit fire: the food goes on all the same (the block entity is not compared: it cooks in Kiln's ticks).
        c = blockCase("campfire_lit_food", "minecraft:campfire[facing=north,lit=true,waterlogged=false,signal_fire=false]").custom("minecraft:interact_with_campfire");
        c.watch.clear();
        c.slot("h0", stack("minecraft:potato", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("campfire_soul", "minecraft:soul_campfire[facing=east,lit=false,waterlogged=false,signal_fire=false]").custom("minecraft:interact_with_campfire");
        c.slot("h0", stack("minecraft:cod", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("campfire_creative", lit).custom("minecraft:interact_with_campfire");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:chicken", 2)).step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("campfire_offhand", lit).custom("minecraft:interact_with_campfire");
        c.slot("offhand", stack("minecraft:kelp", 2)).step(useOn(2, 100, 0, 1, 1));
        out.add(c);
        c = blockCase("campfire_not_food", lit).custom("minecraft:interact_with_campfire");
        c.slot("h0", stack("minecraft:apple", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("campfire_sneaking_food", lit).custom("minecraft:interact_with_campfire");
        c.sneaking = true;
        c.slot("h0", stack("minecraft:beef", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("campfire_adventure", lit).custom("minecraft:interact_with_campfire");
        c.gameMode = "adventure";
        c.slot("h0", stack("minecraft:beef", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // A block against the campfire goes up next to it (the fire does not take the click).
        c = blockCase("campfire_place_block_against", lit);
        c.slot("h0", stack("minecraft:stone", 2)).step(useOn(2, 100, 0, 4, 0));
        out.add(c);

        // ---- flower pots
        String[] plants = {"poppy", "dandelion", "oak_sapling", "cactus", "azalea", "flowering_azalea", "bamboo", "red_mushroom", "fern", "dead_bush",
                "crimson_fungus", "warped_roots", "torchflower", "wither_rose", "closed_eyeblossom", "cherry_sapling", "mangrove_propagule", "lily_of_the_valley"};
        for (String plant : plants) {
            c = blockCase("pot_plant_" + plant, "minecraft:flower_pot").custom("minecraft:pot_flower").stat("minecraft:" + plant);
            c.slot("h0", stack("minecraft:" + plant, 2)).step(useOn(2, 100, 0, 1, 0));
            out.add(c);
        }
        c = blockCase("pot_not_a_plant", "minecraft:flower_pot").custom("minecraft:pot_flower");
        c.slot("h0", stack("minecraft:stone", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_empty_hand_empty_pot", "minecraft:flower_pot").custom("minecraft:pot_flower");
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_take_plant_empty_hand", "minecraft:potted_poppy").custom("minecraft:pot_flower");
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_take_plant_stone_in_hand", "minecraft:potted_oak_sapling").custom("minecraft:pot_flower");
        c.slot("h0", stack("minecraft:stone", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_second_plant_refused", "minecraft:potted_poppy").custom("minecraft:pot_flower");
        c.slot("h0", stack("minecraft:dandelion", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_take_azalea", "minecraft:potted_azalea_bush");
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_take_inventory_full", "minecraft:potted_cactus");
        for (int i = 0; i < 9; i++) c.slot("h" + i, stack("minecraft:dirt", 64));
        for (int i = 9; i < 36; i++) c.slot("m" + i, stack("minecraft:stone", 64));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_creative_plant", "minecraft:flower_pot").custom("minecraft:pot_flower");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:poppy", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_sneaking_plant", "minecraft:flower_pot").custom("minecraft:pot_flower");
        c.sneaking = true;
        c.slot("h0", stack("minecraft:poppy", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_adventure_plant", "minecraft:flower_pot").custom("minecraft:pot_flower");
        c.gameMode = "adventure";
        c.slot("h0", stack("minecraft:poppy", 2)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);

        // ---- chiseled bookshelves (facing west: toward the player; the front's z runs across)
        double[][] spots = {{0.17, 0.75}, {0.5, 0.75}, {0.83, 0.75}, {0.17, 0.25}, {0.5, 0.25}, {0.83, 0.25}};
        String shelf = "minecraft:chiseled_bookshelf[facing=west]";
        for (int slot = 0; slot < 6; slot++) {
            c = blockCase("shelf_insert_" + slot, shelf).stat("minecraft:book");
            c.slot("h0", stack("minecraft:book", 3)).step(useOnAt(2, 100, 0, 4, 0, 0.0, spots[slot][1], spots[slot][0]));
            out.add(c);
        }
        for (String book : new String[] {"writable_book", "written_book", "enchanted_book", "knowledge_book", "stone", "paper"}) {
            c = blockCase("shelf_insert_" + book, shelf).stat("minecraft:" + book);
            c.slot("h0", stack("minecraft:" + book, 2)).step(useOnAt(2, 100, 0, 4, 0, 0.0, 0.75, 0.5));
            out.add(c);
        }
        // Boundaries of the thirds and halves.
        double[][] edges = {{0.33, 0.51}, {0.34, 0.49}, {0.66, 0.99}, {0.67, 0.0}, {0.0, 0.5}, {1.0, 0.5}};
        for (int k = 0; k < edges.length; k++) {
            c = blockCase("shelf_insert_edge_" + k, shelf);
            c.slot("h0", stack("minecraft:book", 6));
            c.step(useOnAt(2, 100, 0, 4, 0, 0.0, edges[k][1], edges[k][0]));
            out.add(c);
        }
        // Other faces of the shelf, and from other sides.
        c = blockCase("shelf_wrong_face_top", shelf);
        c.slot("h0", stack("minecraft:book", 3)).step(useOnAt(2, 100, 0, 1, 0, 0.5, 1.0, 0.5));
        out.add(c);
        c = blockCase("shelf_wrong_face_back", shelf);
        c.slot("h0", stack("minecraft:book", 3)).step(useOnAt(2, 100, 0, 5, 0, 1.0, 0.5, 0.5));
        out.add(c);
        for (String face : new String[] {"north", "south", "east"}) {
            int f = switch (face) { case "north" -> 2; case "south" -> 3; default -> 5; };
            c = blockCase("shelf_facing_" + face, "minecraft:chiseled_bookshelf[facing=" + face + "]");
            c.slot("h0", stack("minecraft:book", 3));
            c.step(useOnAt(2, 100, 0, f, 0, f == 5 ? 1.0 : 0.3, 0.8, f == 2 ? 0.0 : (f == 3 ? 1.0 : 0.3)));
            out.add(c);
        }
        // Taking books: all six in place, one click on each (the first slot's comparator-visible state remembered).
        String full = shelf.replace("]", ",slot_0_occupied=true,slot_1_occupied=true,slot_2_occupied=true,slot_3_occupied=true,slot_4_occupied=true,slot_5_occupied=true]")
                + "{Items:[{Slot:0b,id:\"minecraft:book\",count:1},{Slot:1b,id:\"minecraft:written_book\",count:1,components:{\"minecraft:written_book_content\":{title:\"A\",author:\"B\",pages:[]}}},"
                + "{Slot:2b,id:\"minecraft:enchanted_book\",count:1},{Slot:3b,id:\"minecraft:writable_book\",count:1},{Slot:4b,id:\"minecraft:knowledge_book\",count:1},{Slot:5b,id:\"minecraft:book\",count:1}],last_interacted_slot:2}";
        c = blockCase("shelf_take_all", full);
        for (int slot = 0; slot < 6; slot++) c.step(useOnAt(2, 100, 0, 4, 0, 0.0, spots[slot][1], spots[slot][0]));
        out.add(c);
        c = blockCase("shelf_take_empty_slot", shelf);
        c.step(useOnAt(2, 100, 0, 4, 0, 0.0, 0.75, 0.5));
        out.add(c);
        c = blockCase("shelf_take_inventory_full", full);
        for (int i = 0; i < 9; i++) c.slot("h" + i, stack("minecraft:dirt", 64));
        for (int i = 9; i < 36; i++) c.slot("m" + i, stack("minecraft:stone", 64));
        c.step(useOnAt(2, 100, 0, 4, 0, 0.0, 0.25, 0.83));
        out.add(c);
        c = blockCase("shelf_insert_occupied_slot", full);
        c.slot("h0", stack("minecraft:book", 3)).step(useOnAt(2, 100, 0, 4, 0, 0.0, 0.75, 0.5));
        out.add(c);
        c = blockCase("shelf_creative_insert", shelf);
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:book", 3)).step(useOnAt(2, 100, 0, 4, 0, 0.0, 0.75, 0.5));
        out.add(c);
        c = blockCase("shelf_sneaking_insert", shelf);
        c.sneaking = true;
        c.slot("h0", stack("minecraft:book", 3)).step(useOnAt(2, 100, 0, 4, 0, 0.0, 0.75, 0.5));
        out.add(c);
        c = blockCase("shelf_take_with_item_in_hand", full);
        c.slot("h0", stack("minecraft:apple", 3)).step(useOnAt(2, 100, 0, 4, 0, 0.0, 0.75, 0.17));
        out.add(c);
    }

    static Map<String, Object> useEntity(double x, double y, double z, int hand, boolean sneak) {
        return op("op", "use_entity", "pos", List.of(x, y, z), "hand", hand, "sneak", sneak);
    }

    static Map<String, Object> useStand(double x, double y, double z, int hand, boolean sneak, double hitY) {
        return op("op", "use_entity", "pos", List.of(x, y, z), "hand", hand, "sneak", sneak, "hit", List.of(0.0, hitY, 0.0));
    }

    static Map<String, Object> attackEntity(double x, double y, double z) {
        return op("op", "attack_entity", "pos", List.of(x, y, z));
    }

    /** wp49: beehives and bee nests: a bottle or shears on a full hive, its bees coming out (angry, or calmed by smoke). */
    static ItemStack parseStack(String snbt) {
        try {
            var tag = net.minecraft.nbt.TagParser.parseCompoundFully(snbt);
            return ItemStack.CODEC.parse(server.registryAccess().createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE), tag).getOrThrow();
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    /** wp49: decorated pots: items in, the head shake, breaking (cracked, sherds), placing with sherds. */
    static void pots49(List<Case> out) {
        Case c;
        String pot = "minecraft:decorated_pot[facing=north,cracked=false,waterlogged=false]";
        String sherds = "sherds:{back:{id:\"minecraft:archer_pottery_sherd\"},left:{id:\"minecraft:brick\"},right:{id:\"minecraft:blade_pottery_sherd\"},front:{id:\"minecraft:angler_pottery_sherd\"}}";
        c = blockCase("pot_insert_into_empty", pot).stat("minecraft:stone");
        c.slot("h0", stack("minecraft:stone", 3));
        c.step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_insert_grows", pot + "{item:{id:\"minecraft:stone\",count:60}," + sherds + "}").stat("minecraft:stone");
        c.slot("h0", stack("minecraft:stone", 10));
        for (int i = 0; i < 6; i++) c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_insert_other_item", pot + "{item:{id:\"minecraft:stone\",count:5}}").stat("minecraft:dirt");
        c.slot("h0", stack("minecraft:dirt", 4));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_empty_hand", pot + "{item:{id:\"minecraft:stone\",count:5}}");
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_creative_insert", pot).stat("minecraft:stone");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:stone", 2));
        c.step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_adventure_insert", pot).stat("minecraft:stone");
        c.gameMode = "adventure";
        c.slot("h0", stack("minecraft:stone", 2));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_unstackable_item", pot).stat("minecraft:iron_sword");
        c.slot("h0", stack("minecraft:iron_sword", 2));
        c.step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("pot_sneaking_insert", pot).stat("minecraft:stone");
        c.sneaking = true;
        c.slot("h0", stack("minecraft:stone", 2));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // Breaking: by hand a pot drops itself (with its sherds and the item inside it), cracked by a pickaxe it drops
        // the sherds, with silk touch itself again.
        String full = pot + "{item:{id:\"minecraft:stone\",count:5}," + sherds + "}";
        c = blockCase("pot_break_by_hand", full);
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
        c = blockCase("pot_break_creative", full);
        c.gameMode = "creative";
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
        c = blockCase("pot_break_pickaxe", full);
        c.slot("h0", stack("minecraft:iron_pickaxe"));
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
        c = blockCase("pot_break_axe", full);
        c.slot("h0", stack("minecraft:iron_axe"));
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
        c = blockCase("pot_break_silk_pickaxe", full);
        c.slot("h0", enchanted("minecraft:iron_pickaxe", "minecraft:silk_touch", 1));
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
        c = blockCase("pot_break_empty_pickaxe", pot);
        c.slot("h0", stack("minecraft:diamond_pickaxe"));
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
        // Placing a decorated pot item with sherds (and something inside it).
        ItemStack placed = parseStack("{id:'minecraft:decorated_pot',count:2,components:{'minecraft:pot_decorations':{back:{id:'minecraft:archer_pottery_sherd'},front:{id:'minecraft:brick'}},"
                + "'minecraft:container':[{slot:0,item:{id:'minecraft:stone',count:3}}]}}");
        c = new Case("pot_place_with_sherds").cmd("setblock 2 99 0 minecraft:stone").watch(2, 100, 0);
        c.slot("h0", placed);
        c.step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("pot_place_plain").cmd("setblock 2 99 0 minecraft:stone").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:decorated_pot", 2));
        c.step(useOn(2, 99, 0, 1, 0)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
    }

    /** wp49: copper golem statues: the pose turning, the axe bringing the golem back, honeycomb and the axe on the others. */
    static void statues49(List<Case> out) {
        Case c;
        String statue = "minecraft:copper_golem_statue[facing=north,copper_golem_pose=standing,waterlogged=false]";
        c = blockCase("statue_pose_cycle", statue);
        for (int i = 0; i < 5; i++) c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_exposed_cycle", "minecraft:exposed_copper_golem_statue[facing=east,copper_golem_pose=sitting,waterlogged=false]");
        for (int i = 0; i < 3; i++) c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_waxed_cycle", "minecraft:waxed_copper_golem_statue[facing=south,copper_golem_pose=star,waterlogged=false]");
        for (int i = 0; i < 3; i++) c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_item_click", statue);
        c.slot("h0", stack("minecraft:stone", 3));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_axe_golem_back", statue).mobs().stat("minecraft:iron_axe");
        c.slot("h0", stack("minecraft:iron_axe"));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_axe_golem_back_named", "minecraft:copper_golem_statue[facing=west,copper_golem_pose=running,waterlogged=false]{components:{\"minecraft:custom_name\":\"Rusty\"}}").mobs().stat("minecraft:iron_axe");
        c.slot("h0", stack("minecraft:iron_axe"));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_axe_exposed", "minecraft:exposed_copper_golem_statue[facing=north,copper_golem_pose=standing,waterlogged=false]").mobs().stat("minecraft:iron_axe");
        c.slot("h0", stack("minecraft:iron_axe"));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_axe_waxed", "minecraft:waxed_exposed_copper_golem_statue[facing=north,copper_golem_pose=standing,waterlogged=false]").mobs().stat("minecraft:iron_axe");
        c.slot("h0", stack("minecraft:iron_axe"));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_honeycomb", statue).stat("minecraft:honeycomb");
        c.slot("h0", stack("minecraft:honeycomb", 2));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_honeycomb_waxed", "minecraft:waxed_copper_golem_statue[facing=north,copper_golem_pose=standing,waterlogged=false]").stat("minecraft:honeycomb");
        c.slot("h0", stack("minecraft:honeycomb", 2));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_adventure_click", statue);
        c.gameMode = "adventure";
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_sneaking_click", statue);
        c.sneaking = true;
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("statue_break_by_hand", statue);
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
        c = blockCase("statue_break_pickaxe", statue);
        c.slot("h0", stack("minecraft:iron_pickaxe"));
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
    }

    /** wp49: lecterns: putting a book on, the menu (pages, jump, take), redstone pulse, breaking. */
    /** wp49: a landscape across the map around the origin, built with commands (the same ones build it in the replay). */
    static Case mapCase(String name) {
        Case c = new Case(name).maps();
        for (int y = 62; y <= 70; y += 2) c.cmd("fill -64 " + y + " -64 63 " + Math.min(y + 1, 70) + " 63 minecraft:air");
        // The ground: stone up to y=61 (lakes and pools are flush with it, so nothing flows).
        for (int y = 56; y <= 60; y += 2) c.cmd("fill -64 " + y + " -64 63 " + (y + 1) + " 63 minecraft:stone");
        c.cmd("fill -64 61 -64 63 61 63 minecraft:stone");
        c.cmd("fill -60 62 -60 -20 63 -20 minecraft:slime_block");
        c.cmd("fill 0 62 -60 40 62 -20 minecraft:sand");
        c.cmd("fill 10 63 -50 30 64 -30 minecraft:sand");
        c.cmd("fill -60 60 20 -20 61 60 minecraft:water");
        c.cmd("fill -50 58 30 -30 59 50 minecraft:water");
        c.cmd("fill 30 62 30 33 69 33 minecraft:stone");
        c.cmd("fill 40 62 40 50 62 50 minecraft:snow_block");
        c.cmd("fill 40 61 52 50 61 62 minecraft:ice");
        c.cmd("fill 50 60 -10 60 61 0 minecraft:lava");
        c.cmd("fill -10 62 -10 0 62 0 minecraft:glass");
        c.cmd("fill 10 62 10 20 62 20 minecraft:oak_planks");
        c.cmd("fill 20 62 10 22 62 12 minecraft:red_wool");
        c.cmd("fill -30 62 -10 -25 66 -5 minecraft:oak_leaves[persistent=true]");
        c.cmd("fill -5 62 40 -5 69 40 minecraft:gold_block");
        return c;
    }

    static void maps49(List<Case> out) {
        Case c;
        c = mapCase("map_empty_use_hand").stat("minecraft:map");
        c.slot("h0", stack("minecraft:map", 1));
        c.step(op("op", "use", "hand", 0)).step(op("op", "map_wait", "ticks", 20)).step(op("op", "command", "command", "setblock 5 62 5 minecraft:gold_block"))
                .step(op("op", "map_wait", "ticks", 20));
        out.add(c);
        c = mapCase("map_empty_use_stack").stat("minecraft:map");
        c.slot("h0", stack("minecraft:map", 2));
        c.step(op("op", "use", "hand", 0)).step(op("op", "select", "slot", 1)).step(op("op", "map_wait", "ticks", 20));
        out.add(c);
        c = mapCase("map_empty_use_creative").stat("minecraft:map");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:map", 2));
        c.step(op("op", "use", "hand", 0)).step(op("op", "select", "slot", 1)).step(op("op", "map_wait", "ticks", 20));
        out.add(c);
        c = mapCase("map_empty_use_offhand").stat("minecraft:map");
        c.slot("offhand", stack("minecraft:map", 1));
        c.step(op("op", "use", "hand", 1)).step(op("op", "map_wait", "ticks", 20));
        out.add(c);
        c = mapCase("map_empty_use_full_inventory").stat("minecraft:map");
        c.slot("h0", stack("minecraft:map", 2));
        for (int i = 1; i < 9; i++) c.slot("h" + i, stack("minecraft:dirt", 64));
        for (int i = 9; i < 36; i++) c.slot("m" + i, stack("minecraft:cobblestone", 64));
        c.step(op("op", "use", "hand", 0));
        out.add(c);
        // The map follows the player, who leaves it and comes back.
        c = mapCase("map_player_marker");
        c.slot("h0", stack("minecraft:map", 1));
        c.step(op("op", "use", "hand", 0)).step(op("op", "map_wait", "ticks", 20)).step(op("op", "command", "command", "tp Interact 40.5 100 -30.5"))
                .step(op("op", "map_wait", "ticks", 20)).step(op("op", "command", "command", "tp Interact 90.5 100 90.5")).step(op("op", "map_wait", "ticks", 12))
                .step(op("op", "command", "command", "tp Interact -20.5 100 30.5")).step(op("op", "map_wait", "ticks", 20));
        out.add(c);
        // Banners are put on the map and taken off it, and go when the banner does.
        c = mapCase("map_banner").watch(10, 62, 10).cmd("setblock 10 62 10 minecraft:red_banner[rotation=4]");
        c.slot("h0", stack("minecraft:map", 1));
        c.step(op("op", "use", "hand", 0)).step(op("op", "map_wait", "ticks", 20)).step(useOn(10, 62, 10, 1, 0)).step(op("op", "map_wait", "ticks", 6))
                .step(useOn(10, 62, 10, 1, 0)).step(op("op", "map_wait", "ticks", 6)).step(useOn(10, 62, 10, 1, 0))
                .step(op("op", "command", "command", "setblock 10 62 10 minecraft:air")).step(op("op", "map_wait", "ticks", 20));
        out.add(c);
        c = mapCase("map_banner_named").watch(10, 62, 10).cmd("setblock 10 62 10 minecraft:blue_banner[rotation=8]{CustomName:'\"Home\"'}");
        c.slot("h0", stack("minecraft:map", 1));
        c.step(op("op", "use", "hand", 0)).step(op("op", "map_wait", "ticks", 20)).step(useOn(10, 62, 10, 1, 0)).step(op("op", "map_wait", "ticks", 6));
        out.add(c);
        // The cartography table: paper zooms a map out, a glass pane locks it, an empty map copies it.
        String table = "setblock 2 99 0 minecraft:stone";
        for (String extra : new String[] {"paper", "glass_pane", "map"}) {
            c = mapCase("map_cartography_" + extra).menus().cmd(table).cmd("setblock 2 100 0 minecraft:cartography_table");
            c.slot("h0", stack("minecraft:map", 1)).slot("h1", stack("minecraft:" + extra, 2));
            c.step(op("op", "use", "hand", 0)).step(op("op", "map_wait", "ticks", 20)).step(useOn(2, 100, 0, 1, 0))
                    .step(op("op", "menu_click", "slot", 30, "button", 0, "input", 1)).step(op("op", "menu_click", "slot", 31, "button", 0, "input", 1))
                    .step(op("op", "menu_click", "slot", 2, "button", 0, "input", 1));
            if (extra.equals("paper")) {
                // The zoomed map goes in again for a second zoom.
                c.step(op("op", "menu_click", "slot", 38, "button", 0, "input", 1)).step(op("op", "menu_click", "slot", 31, "button", 0, "input", 1))
                        .step(op("op", "wait", "ticks", 1)).step(op("op", "menu_click", "slot", 2, "button", 0, "input", 1));
            }
            c.step(op("op", "menu_close"));
            out.add(c);
        }
        // Taking the result by hand, a glass pane on an already locked map, other things in the slots.
        c = mapCase("map_cartography_pickup").menus().cmd(table).cmd("setblock 2 100 0 minecraft:cartography_table");
        c.slot("h0", stack("minecraft:map", 1)).slot("h1", stack("minecraft:glass_pane", 3));
        c.step(op("op", "use", "hand", 0)).step(op("op", "map_wait", "ticks", 20)).step(useOn(2, 100, 0, 1, 0))
                .step(op("op", "menu_click", "slot", 30, "button", 0, "input", 1)).step(op("op", "menu_click", "slot", 31, "button", 0, "input", 1))
                .step(op("op", "menu_click", "slot", 2, "button", 0, "input", 0)).step(op("op", "menu_click", "slot", 0, "button", 0, "input", 0))
                .step(op("op", "menu_close"));
        out.add(c);
        c = mapCase("map_cartography_not_a_map").menus().cmd(table).cmd("setblock 2 100 0 minecraft:cartography_table");
        c.slot("h0", stack("minecraft:map", 1)).slot("h1", stack("minecraft:stick", 2)).slot("h2", stack("minecraft:paper", 2));
        c.step(op("op", "use", "hand", 0)).step(useOn(2, 100, 0, 1, 0))
                .step(op("op", "menu_click", "slot", 30, "button", 0, "input", 1)).step(op("op", "menu_click", "slot", 31, "button", 0, "input", 1))
                .step(op("op", "menu_click", "slot", 32, "button", 0, "input", 1)).step(op("op", "menu_close"));
        out.add(c);
        // A map in an item frame: the frame is marked on the map, every player is sent the map, and the marker goes with the map.
        c = mapCase("map_frame").hanging().cmd("setblock 2 100 0 minecraft:stone");
        c.slot("h0", stack("minecraft:map", 1)).slot("h1", stack("minecraft:item_frame", 1));
        c.step(op("op", "use", "hand", 0)).step(op("op", "map_wait", "ticks", 20))
                .step(op("op", "select", "slot", 1)).step(useOn(2, 100, 0, 5, 0)).step(op("op", "select", "slot", 0))
                .step(useEntity(3.0, 100.5, 0.5, 0, false)).step(op("op", "map_wait", "ticks", 70))
                .step(op("op", "attack_entity", "pos", List.of(3.0, 100.5, 0.5))).step(op("op", "map_wait", "ticks", 12));
        out.add(c);
        c = mapCase("map_frame_glow_south").hanging().cmd("setblock 2 100 0 minecraft:stone").cmd("setblock 3 100 -1 minecraft:stone");
        c.slot("h0", stack("minecraft:map", 1)).slot("h1", stack("minecraft:glow_item_frame", 1));
        c.step(op("op", "use", "hand", 0)).step(op("op", "map_wait", "ticks", 20))
                .step(op("op", "select", "slot", 1)).step(useOn(3, 100, -1, 3, 0)).step(op("op", "select", "slot", 0))
                .step(useEntity(3.0, 100.5, 0.0, 0, false)).step(op("op", "map_wait", "ticks", 70))
                .step(useEntity(3.0, 100.5, 0.0, 0, false)).step(op("op", "map_wait", "ticks", 12));
        out.add(c);
        // A banner outside the map's area is not taken.
        c = mapCase("map_banner_outside").cmd("setblock 70 62 70 minecraft:red_banner[rotation=4]");
        c.slot("h0", stack("minecraft:map", 1));
        c.step(op("op", "use", "hand", 0)).step(useOn(70, 62, 70, 1, 0)).step(op("op", "map_wait", "ticks", 6));
        out.add(c);
    }

    /** wp49: a vault at (3, 100, 0) put there when the level has settled; the level ticks it after every step. */
    static Case vaultCase(String name, String props, String nbt) {
        Case c = new Case(name).ticking();
        c.cmd("setblock 3 99 0 minecraft:stone").late("setblock 3 100 0 minecraft:vault[" + props + "]" + nbt).watch(3, 100, 0);
        return c;
    }

    static Map<String, Object> waitTicks(int n) {
        return op("op", "wait", "ticks", n);
    }

    static String playerUuidTag() {
        UUID u = UUID.nameUUIDFromBytes("Interact".getBytes());
        long hi = u.getMostSignificantBits(), lo = u.getLeastSignificantBits();
        return "[I;" + (int) (hi >> 32) + "," + (int) hi + "," + (int) (lo >> 32) + "," + (int) lo + "]";
    }

    static void vaults49(List<Case> out) {
        Case c;
        String stone = "config:{loot_table:\"minecraft:blocks/stone\",key_item:{id:\"minecraft:trial_key\",count:1}}";
        // Waking up and going back to sleep with the players near.
        c = vaultCase("vault_wake", "vault_state=inactive", "{" + stone + "}");
        c.step(waitTicks(1)).step(waitTicks(19)).step(waitTicks(1)).step(waitTicks(40));
        out.add(c);
        c = vaultCase("vault_far_player", "vault_state=inactive", "{" + stone + "}");
        c.pos = new double[] {0.5, 100.0, 12.5};
        c.step(waitTicks(21)).step(waitTicks(20));
        out.add(c);
        c = vaultCase("vault_creative_player", "vault_state=inactive", "{" + stone + "}");
        c.gameMode = "creative";
        c.step(waitTicks(2)).step(waitTicks(20));
        out.add(c);
        c = vaultCase("vault_spectator_player", "vault_state=inactive", "{" + stone + "}");
        c.gameMode = "spectator";
        c.step(waitTicks(2)).step(waitTicks(20));
        out.add(c);
        c = vaultCase("vault_edge_of_range", "vault_state=inactive", "{" + stone + "}");
        c.pos = new double[] {-0.5, 100.0, 0.5};
        c.step(waitTicks(3)).step(waitTicks(20));
        out.add(c);
        // A key opens it: unlocking, then the reward comes out, then it closes (the player has been rewarded).
        c = vaultCase("vault_key", "vault_state=active", "{" + stone + "}").stat("minecraft:trial_key");
        c.slot("h0", stack("minecraft:trial_key", 2));
        c.step(useOn(3, 100, 0, 1, 0)).step(waitTicks(13)).step(waitTicks(1)).step(waitTicks(1)).step(waitTicks(20)).step(waitTicks(1)).step(waitTicks(20)).step(waitTicks(25));
        out.add(c);
        c = vaultCase("vault_key_creative", "vault_state=active", "{" + stone + "}").stat("minecraft:trial_key");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:trial_key", 1));
        c.step(useOn(3, 100, 0, 1, 0)).step(waitTicks(14)).step(waitTicks(21)).step(waitTicks(20));
        out.add(c);
        c = vaultCase("vault_key_offhand", "vault_state=active", "{" + stone + "}").stat("minecraft:trial_key");
        c.slot("offhand", stack("minecraft:trial_key", 1));
        c.step(useOn(3, 100, 0, 1, 1)).step(waitTicks(14)).step(waitTicks(21));
        out.add(c);
        // A key of the wrong kind, too few of them, something else, nothing: only the failing sound (once every 15 ticks).
        c = vaultCase("vault_wrong_key", "vault_state=active", "{" + stone + "}").stat("minecraft:ominous_trial_key");
        c.slot("h0", stack("minecraft:ominous_trial_key", 1));
        c.step(useOn(3, 100, 0, 1, 0)).step(useOn(3, 100, 0, 1, 0)).step(waitTicks(14)).step(useOn(3, 100, 0, 1, 0)).step(waitTicks(15)).step(useOn(3, 100, 0, 1, 0));
        out.add(c);
        c = vaultCase("vault_two_keys_needed", "vault_state=active", "{config:{loot_table:\"minecraft:blocks/stone\",key_item:{id:\"minecraft:trial_key\",count:2}}}").stat("minecraft:trial_key");
        c.slot("h0", stack("minecraft:trial_key", 1));
        c.step(useOn(3, 100, 0, 1, 0)).step(waitTicks(1));
        out.add(c);
        c = vaultCase("vault_two_keys_given", "vault_state=active", "{config:{loot_table:\"minecraft:blocks/stone\",key_item:{id:\"minecraft:trial_key\",count:2}}}").stat("minecraft:trial_key");
        c.slot("h0", stack("minecraft:trial_key", 3));
        c.step(useOn(3, 100, 0, 1, 0)).step(waitTicks(14)).step(waitTicks(21));
        out.add(c);
        c = vaultCase("vault_stick_and_empty_hand", "vault_state=active", "{" + stone + "}");
        c.slot("h0", stack("minecraft:stick", 1));
        c.step(useOn(3, 100, 0, 1, 0)).step(op("op", "select", "slot", 1)).step(useOn(3, 100, 0, 1, 0));
        out.add(c);
        // The player has been rewarded before: the key is refused with its own sound.
        c = vaultCase("vault_rewarded_player", "vault_state=active",
                "{" + stone + ",server_data:{rewarded_players:[" + playerUuidTag() + "],state_updating_resumes_at:100000L}}").stat("minecraft:trial_key");
        c.slot("h0", stack("minecraft:trial_key", 2));
        c.step(useOn(3, 100, 0, 1, 0)).step(useOn(3, 100, 0, 1, 0)).step(waitTicks(14)).step(useOn(3, 100, 0, 1, 0));
        out.add(c);
        // Items waiting to come out, one every second; the pitch rises with the progress.
        c = vaultCase("vault_eject_three", "vault_state=ejecting",
                "{" + stone + ",server_data:{items_to_eject:[{id:\"minecraft:stone\",count:1},{id:\"minecraft:dirt\",count:2},{id:\"minecraft:sand\",count:3}],total_ejections_needed:3,state_updating_resumes_at:101L}}");
        c.step(waitTicks(1)).step(waitTicks(20)).step(waitTicks(20)).step(waitTicks(20)).step(waitTicks(20));
        out.add(c);
        c = vaultCase("vault_eject_one", "vault_state=ejecting",
                "{" + stone + ",server_data:{items_to_eject:[{id:\"minecraft:diamond\",count:1}],total_ejections_needed:1,state_updating_resumes_at:101L}}");
        c.step(waitTicks(1)).step(waitTicks(20)).step(waitTicks(20));
        out.add(c);
        // Ominous vaults want ominous keys.
        c = vaultCase("vault_ominous_key", "vault_state=inactive,ominous=true",
                "{config:{loot_table:\"minecraft:blocks/stone\",key_item:{id:\"minecraft:ominous_trial_key\",count:1}}}").stat("minecraft:ominous_trial_key");
        c.slot("h0", stack("minecraft:ominous_trial_key", 1));
        c.step(waitTicks(1)).step(useOn(3, 100, 0, 1, 0)).step(waitTicks(14)).step(waitTicks(21)).step(waitTicks(20));
        out.add(c);
        // What the vault shows while waiting comes from another table when it is told to.
        c = vaultCase("vault_display_override", "vault_state=inactive",
                "{config:{loot_table:\"minecraft:blocks/stone\",override_loot_table_to_display:\"minecraft:blocks/composter\",key_item:{id:\"minecraft:trial_key\",count:1}}}");
        c.step(waitTicks(1)).step(waitTicks(20));
        out.add(c);
        // A vault with no key does not wake up its display.
        c = vaultCase("vault_no_key_item", "vault_state=inactive", "{config:{loot_table:\"minecraft:blocks/stone\",key_item:{id:\"minecraft:air\",count:1}}}");
        c.step(waitTicks(1)).step(waitTicks(20));
        out.add(c);
        // A smaller range.
        c = vaultCase("vault_small_range", "vault_state=inactive", "{config:{loot_table:\"minecraft:blocks/stone\",activation_range:2.0d,deactivation_range:2.5d,key_item:{id:\"minecraft:trial_key\",count:1}}}");
        c.step(waitTicks(1)).step(waitTicks(20));
        out.add(c);
    }

    /** wp49: a trial spawner at (3, 100, 0) (floor under it and under the place its mobs appear) put there when the level has settled. */
    static Case trialCase(String name, String props, String nbt) {
        Case c = new Case(name).ticking().mobs();
        c.cmd("setblock 3 99 0 minecraft:stone").cmd("setblock 5 99 2 minecraft:stone").late("setblock 3 100 0 minecraft:trial_spawner[" + props + "]" + nbt).watch(3, 100, 0);
        return c;
    }

    static void trials49(List<Case> out) {
        Case c;
        // Zombies that stand still where they appear (the place is given, so nothing is left to the level's random).
        String zombie = "{data:{entity:{id:\"minecraft:zombie\",NoAI:1b,Invulnerable:1b,Pos:[5.5d,100.0d,2.5d]}},weight:1}";
        String key = "loot_tables_to_eject:[{data:\"minecraft:spawners/trial_chamber/key\",weight:1}]";
        String normal = "normal_config:{total_mobs:4.0f,simultaneous_mobs:2.0f,ticks_between_spawn:40,spawn_potentials:[" + zombie + "]," + key + "},target_cooldown_length:200";
        String kill = "kill @e[type=minecraft:zombie]";
        Map<String, Object> killMobs = op("op", "kill_mobs");
        c = trialCase("trial_wave", "trial_spawner_state=inactive", "{" + normal + "}");
        c.step(waitTicks(10)).step(waitTicks(30)).step(waitTicks(30)).step(waitTicks(30)).step(waitTicks(30)).step(killMobs);
        for (int i = 0; i < 3; i++) {
            c.step(waitTicks(25)).step(waitTicks(25)).step(waitTicks(30)).step(killMobs);
        }
        for (int i = 0; i < 14; i++) c.step(waitTicks(20));
        out.add(c);
        // Left alone the zombies stay; the spawner keeps to its limit of two at a time.
        c = trialCase("trial_limit", "trial_spawner_state=inactive", "{" + normal + "}");
        for (int i = 0; i < 12; i++) c.step(waitTicks(20));
        out.add(c);
        c = trialCase("trial_default_config", "trial_spawner_state=inactive", "");
        c.step(waitTicks(5)).step(waitTicks(40)).step(waitTicks(40));
        out.add(c);
        c = trialCase("trial_far_player", "trial_spawner_state=inactive", "{" + normal + "}");
        c.pos = new double[] {0.5, 100.0, 30.5};
        c.step(waitTicks(30)).step(waitTicks(30)).step(waitTicks(30));
        out.add(c);
        c = trialCase("trial_creative_player", "trial_spawner_state=inactive", "{" + normal + "}");
        c.gameMode = "creative";
        c.step(waitTicks(30)).step(waitTicks(30)).step(waitTicks(30));
        out.add(c);
        c = trialCase("trial_spectator_player", "trial_spawner_state=inactive", "{" + normal + "}");
        c.gameMode = "spectator";
        c.step(waitTicks(30)).step(waitTicks(30)).step(waitTicks(30));
        out.add(c);
        c = trialCase("trial_peaceful", "trial_spawner_state=inactive", "{" + normal + "}");
        c.step(waitTicks(30)).step(waitTicks(30)).step(waitTicks(30)).step(killMobs).step(cmdStep("difficulty peaceful")).step(waitTicks(30)).step(waitTicks(30)).step(cmdStep("difficulty easy"))
                .step(waitTicks(30)).step(waitTicks(30)).step(waitTicks(30));
        out.add(c);
        c = trialCase("trial_rule_off", "trial_spawner_state=inactive", "{" + normal + "}");
        c.step(waitTicks(30)).step(waitTicks(30)).step(cmdStep("gamerule spawner_blocks_work false")).step(waitTicks(30)).step(waitTicks(30)).step(cmdStep("gamerule spawner_blocks_work true"))
                .step(waitTicks(30)).step(waitTicks(30));
        out.add(c);
        // Saved states: the spawner wakes up in the middle of a trial.
        c = trialCase("trial_resume_cooldown", "trial_spawner_state=cooldown", "{" + normal + ",cooldown_ends_at:230L}");
        for (int i = 0; i < 6; i++) c.step(waitTicks(20));
        out.add(c);
        c = trialCase("trial_resume_ejecting", "trial_spawner_state=waiting_for_reward_ejection",
                "{" + normal + ",cooldown_ends_at:300L,registered_players:[" + playerUuidTag() + "]}");
        for (int i = 0; i < 12; i++) c.step(waitTicks(20));
        out.add(c);
        c = trialCase("trial_resume_active", "trial_spawner_state=active", "{" + normal + ",total_mobs_spawned:3,registered_players:[" + playerUuidTag() + "]}");
        for (int i = 0; i < 8; i++) c.step(waitTicks(20));
        out.add(c);
        // The config the datapack holds under a key (the trial chambers' own); no mob is watched, they appear around the spawner.
        c = new Case("trial_key_config").ticking();
        c.cmd("setblock 3 99 0 minecraft:stone").late("setblock 3 100 0 minecraft:trial_spawner[trial_spawner_state=inactive]{normal_config:\"minecraft:trial_chamber/melee/zombie/normal\",ominous_config:\"minecraft:trial_chamber/melee/zombie/ominous\"}").watch(3, 100, 0);
        c.step(waitTicks(5)).step(waitTicks(20)).step(waitTicks(20)).step(waitTicks(20));
        out.add(c);
        // A spawn egg changes what it spawns and starts it over.
        c = trialCase("trial_egg", "trial_spawner_state=active", "{" + normal + ",total_mobs_spawned:1,registered_players:[" + playerUuidTag() + "]}");
        c.slot("h0", stack("minecraft:skeleton_spawn_egg", 2));
        c.step(waitTicks(3)).step(useOn(3, 100, 0, 1, 0)).step(waitTicks(1)).step(waitTicks(1));
        out.add(c);
        // Bad Omen turns it ominous.
        String ominous = "ominous_config:{total_mobs:6.0f,simultaneous_mobs:3.0f,ticks_between_spawn:40,spawn_potentials:[" + zombie + "],"
                + "items_to_drop_when_ominous:\"minecraft:empty\"," + key + "}";
        c = trialCase("trial_ominous", "trial_spawner_state=waiting_for_players", "{" + normal + "," + ominous + "}");
        c.step(waitTicks(25)).step(cmdStep("effect give @a minecraft:bad_omen 600 0")).step(waitTicks(20)).step(waitTicks(20)).step(waitTicks(20)).step(waitTicks(20)).step(waitTicks(20));
        for (int i = 0; i < 3; i++) c.step(killMobs).step(waitTicks(30)).step(waitTicks(30));
        out.add(c);
    }

    static void lecterns49(List<Case> out) {
        Case c;
        String stand = "minecraft:lectern[facing=north,has_book=false,powered=false]";
        String written = "{id:'minecraft:written_book',count:1,components:{'minecraft:written_book_content':{title:'Tome',author:'Me',pages:['One','Two','Three','Four','Five']}}}";
        String writable = "{id:'minecraft:writable_book',count:3,components:{'minecraft:writable_book_content':{pages:['a','b']}}}";
        String with = "minecraft:lectern[facing=east,has_book=true,powered=false]{Book:" + written + ",Page:1}";
        c = blockCase("lectern_place_written_book", stand).stat("minecraft:written_book");
        c.slot("h0", parseStack(written));
        c.step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("lectern_place_writable_book", stand).stat("minecraft:writable_book");
        c.slot("h0", parseStack(writable));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("lectern_place_book_creative", stand);
        c.gameMode = "creative";
        c.slot("h0", parseStack(writable));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("lectern_place_plain_book", stand);
        c.slot("h0", stack("minecraft:book", 2));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("lectern_empty_hand_no_book", stand);
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("lectern_sneaking_with_book", stand);
        c.sneaking = true;
        c.slot("h0", parseStack(written));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // The menu: opened, pages turned, jumped to, the book taken.
        // (Kiln's level ticks between the steps and the pulse ends two ticks after the turn; the recorded level stands
        // still: a turn is the last step.)
        c = blockCase("lectern_open_and_next", with).menus().custom("minecraft:interact_with_lectern");
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", 2));
        out.add(c);
        c = blockCase("lectern_open_and_previous", with).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", 1));
        out.add(c);
        for (int jump : new int[] {100, 103, 104, 150}) {
            c = blockCase("lectern_jump_" + jump, with).menus();
            c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", jump));
            out.add(c);
        }
        c = blockCase("lectern_next_past_the_end", with.replace("Page:1", "Page:4")).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", 2));
        out.add(c);
        c = blockCase("lectern_first_page_back", with.replace("Page:1", "Page:0")).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", 1));
        out.add(c);
        c = blockCase("lectern_take_book", with).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", 3));
        out.add(c);
        c = blockCase("lectern_take_book_full_inventory", with).menus();
        for (int i = 0; i < 9; i++) c.slot("h" + i, stack("minecraft:dirt", 64));
        for (int i = 9; i < 36; i++) c.slot("m" + i, stack("minecraft:cobblestone", 64));
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", 3));
        out.add(c);
        c = blockCase("lectern_take_book_adventure", with).menus();
        c.gameMode = "adventure";
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", 3));
        out.add(c);
        c = blockCase("lectern_unknown_button", with).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_button", "button", 50));
        out.add(c);
        c = blockCase("lectern_spectator_open", with).menus();
        c.gameMode = "spectator";
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("lectern_open_holding_book", with).menus();
        c.slot("h0", parseStack(writable));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // Broken: the book pops out (and the lectern gives up its power).
        c = blockCase("lectern_break_drops_book", with);
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
        c = blockCase("lectern_break_empty", stand);
        c.step(op("op", "dig", "pos", List.of(2, 100, 0)));
        out.add(c);
    }

    /** wp49: a brushing case: suspicious `block` at (2, 99, 0), the player (at the origin) looking down at its top, the level ticking whole. */
    static Case brushCase(String name, String block) {
        Case c = new Case(name).fullTicking();
        c.yaw = -90f;
        c.pitch = 39f;
        c.cmd("setblock 2 98 0 minecraft:stone").cmd("setblock 2 99 0 " + block);
        c.watch(2, 99, 0);
        return c;
    }

    static final String SAND = "minecraft:suspicious_sand[dusted=0]";

    /** wp49: archaeology (brushing suspicious sand and gravel). */
    static void brushes49(List<Case> out) {
        Case c;
        String emerald = "{item:{id:\"minecraft:emerald\",count:3}}";
        // The first brush at the fifth tick of use, then every tenth; the tenth breaks the block.
        c = brushCase("brush_all_the_way", SAND + emerald);
        c.slot("h0", stack("minecraft:brush", 1)).stat("minecraft:brush").step(useOn(2, 99, 0, 1, 0)).step(wait(4));
        for (int i = 0; i < 10; i++) c.step(wait(10));
        out.add(c);
        c = brushCase("brush_gravel", "minecraft:suspicious_gravel[dusted=0]{item:{id:\"minecraft:clay_ball\",count:1}}");
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(4));
        for (int i = 0; i < 10; i++) c.step(wait(10));
        out.add(c);
        c = brushCase("brush_nothing_inside", SAND);
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(4));
        for (int i = 0; i < 10; i++) c.step(wait(10));
        out.add(c);
        c = brushCase("brush_loot_table", SAND + "{LootTable:\"minecraft:archaeology/desert_pyramid\",LootTableSeed:12345L}");
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(4));
        for (int i = 0; i < 10; i++) c.step(wait(10));
        out.add(c);
        c = brushCase("brush_loot_table_cold_ruin", "minecraft:suspicious_gravel[dusted=0]{LootTable:\"minecraft:archaeology/ocean_ruin_cold\",LootTableSeed:777L}");
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(4));
        for (int i = 0; i < 10; i++) c.step(wait(10));
        out.add(c);
        // Used up halfway: the brushing fades by itself, two at a time.
        c = brushCase("brush_fades", SAND + emerald);
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(4)).step(wait(10)).step(wait(10)).step(wait(10)).step(wait(10));
        c.step(op("op", "release_use")).step(wait(30)).step(wait(30)).step(wait(30));
        out.add(c);
        c = brushCase("brush_released_early", SAND + emerald);
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(4)).step(wait(3)).step(op("op", "release_use")).step(wait(20));
        out.add(c);
        c = brushCase("brush_off_hand", SAND + emerald);
        c.slot("offhand", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 1)).step(wait(4));
        for (int i = 0; i < 10; i++) c.step(wait(10));
        out.add(c);
        c = brushCase("brush_creative", SAND + emerald);
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(4));
        for (int i = 0; i < 10; i++) c.step(wait(10));
        out.add(c);
        c = brushCase("brush_worn", SAND + emerald);
        ItemStack worn = stack("minecraft:brush", 1);
        worn.setDamageValue(worn.getMaxDamage() - 1);
        c.slot("h0", worn).step(useOn(2, 99, 0, 1, 0)).step(wait(4));
        for (int i = 0; i < 10; i++) c.step(wait(10));
        out.add(c);
        c = brushCase("brush_adventure", SAND + emerald);
        c.gameMode = "adventure";
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(10));
        out.add(c);
        // On other blocks the brush only brushes.
        c = brushCase("brush_on_stone", "minecraft:stone");
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(4)).step(wait(10));
        out.add(c);
        c = brushCase("brush_on_air", "minecraft:air");
        c.slot("h0", stack("minecraft:brush", 1)).step(useOn(2, 99, 0, 1, 0)).step(wait(10));
        out.add(c);
        // Nothing to hold on to: the block falls (and the brush with nothing under it).
        c = brushCase("brush_block_falls", "minecraft:air");
        c.late("setblock 2 99 0 " + SAND + emerald).late("setblock 2 98 0 minecraft:air");
        c.slot("h0", stack("minecraft:brush", 1)).step(wait(5));
        out.add(c);
        c = brushCase("brush_place_block_item", "minecraft:air");
        c.slot("h0", stack("minecraft:suspicious_sand", 2)).step(useOn(2, 98, 0, 1, 0)).step(wait(3));
        out.add(c);
        c = brushCase("brush_put_down_stone", "minecraft:air");
        c.slot("h0", stack("minecraft:stone", 2)).step(useOn(2, 98, 0, 1, 0)).step(wait(3));
        out.add(c);
        c = brushCase("brush_put_down_sand", "minecraft:air");
        c.slot("h0", stack("minecraft:suspicious_gravel", 2)).step(useOn(2, 98, 0, 1, 0)).step(wait(3));
        out.add(c);
    }

    static Map<String, Object> setCommandBlock(int y, String command, String mode, boolean track, boolean conditional, boolean auto) {
        return op("op", "set_command_block", "pos", List.of(2, y, 0), "command", command, "mode", mode, "track", track, "conditional", conditional, "auto", auto);
    }

    /** wp49: command blocks: who can put one up and open it, and what the screen's settings do. */
    static void commandBlocks49(List<Case> out) {
        Case c;
        for (String kind : new String[] {"command_block", "chain_command_block", "repeating_command_block"}) {
            String[][] who = {{"survival", "op"}, {"creative", "op"}, {"creative", "player"}, {"adventure", "op"}};
            for (String[] w : who) {
                c = new Case("cmd_place_" + kind + "_" + w[0] + "_" + w[1]).watch(2, 100, 0).fullTicking();
                c.gameMode = w[0];
                c.op = w[1].equals("op");
                c.cmd("setblock 2 99 0 minecraft:stone");
                c.slot("h0", stack("minecraft:" + kind, 2)).step(useOn(2, 99, 0, 1, 0)).step(wait(3));
                out.add(c);
            }
        }
        // The screen.
        String cb = "minecraft:command_block[conditional=false,facing=up]{Command:\"say hi\",TrackOutput:1b}";
        String[][] who = {{"creative", "op"}, {"survival", "op"}, {"creative", "player"}, {"spectator", "op"}};
        for (String[] w : who) {
            c = blockCase("cmd_open_" + w[0] + "_" + w[1], cb).menus().fullTicking();
            c.gameMode = w[0];
            c.op = w[1].equals("op");
            c.step(useOn(2, 100, 0, 1, 0)).step(wait(2));
            out.add(c);
        }
        // The settings.
        for (String mode : new String[] {"sequence", "auto", "redstone"}) {
            for (boolean conditional : new boolean[] {false, true}) {
                for (boolean auto : new boolean[] {false, true}) {
                    String name = "cmd_set_" + mode + (conditional ? "_cond" : "") + (auto ? "_auto" : "");
                    c = blockCase(name, cb).fullTicking();
                    c.gameMode = "creative";
                    c.op = true;
                    c.step(setCommandBlock(100, "say set", mode, true, conditional, auto)).step(wait(4)).step(wait(30));
                    out.add(c);
                }
            }
        }
        c = blockCase("cmd_set_no_track", cb).fullTicking();
        c.gameMode = "creative";
        c.op = true;
        c.step(setCommandBlock(100, "say set", "redstone", false, false, true)).step(wait(4));
        out.add(c);
        c = blockCase("cmd_set_empty", cb).fullTicking();
        c.gameMode = "creative";
        c.op = true;
        c.step(setCommandBlock(100, "", "redstone", true, false, true)).step(wait(4));
        out.add(c);
        c = blockCase("cmd_set_slash", cb).fullTicking();
        c.gameMode = "creative";
        c.op = true;
        c.step(setCommandBlock(100, "/say slashed", "redstone", true, false, true)).step(wait(4));
        out.add(c);
        for (String[] w : new String[][] {{"survival", "op"}, {"creative", "player"}, {"adventure", "op"}, {"spectator", "op"}}) {
            c = blockCase("cmd_set_refused_" + w[0] + "_" + w[1], cb).fullTicking();
            c.gameMode = w[0];
            c.op = w[1].equals("op");
            c.step(setCommandBlock(100, "say set", "auto", true, true, true)).step(wait(4));
            out.add(c);
        }
        c = blockCase("cmd_set_on_stone", "minecraft:stone").fullTicking();
        c.gameMode = "creative";
        c.op = true;
        c.step(setCommandBlock(100, "say set", "auto", true, true, true)).step(wait(4));
        out.add(c);
        // Mode change turns the block into the other kind (and keeps its facing).
        for (String from : new String[] {"command_block", "chain_command_block", "repeating_command_block"}) {
            for (String mode : new String[] {"sequence", "auto", "redstone"}) {
                c = blockCase("cmd_mode_" + from + "_" + mode, "minecraft:" + from + "[conditional=false,facing=east]{Command:\"say hi\"}").fullTicking();
                c.gameMode = "creative";
                c.op = true;
                c.step(setCommandBlock(100, "say hi", mode, true, true, false)).step(wait(4));
                out.add(c);
            }
        }
        // A chain behind a block that is set to run.
        c = blockCase("cmd_set_runs_chain", "minecraft:command_block[conditional=false,facing=east]{Command:\"say first\"}").fullTicking();
        c.cmd("setblock 3 100 0 minecraft:chain_command_block[conditional=false,facing=east]{Command:\"say second\",auto:1b}");
        c.gameMode = "creative";
        c.op = true;
        c.step(setCommandBlock(100, "say first", "auto", true, false, true)).step(wait(4)).step(wait(10));
        out.add(c);
    }

    static Map<String, Object> wait(int ticks) {
        return op("op", "wait", "ticks", ticks);
    }

    /** wp49: crafters: put up facing the way the player looks, and their screen (slots that switch off, the result that only shows). */
    static void crafters49(List<Case> out) {
        Case c;
        // ---- put up: the front is where the player's back is, the top follows the way he faces
        for (float pitch : new float[] {0f, 60f, -60f, 89f, -89f}) {
            for (float yaw : new float[] {0f, 90f, 180f, -90f, 45f, 140f}) {
                c = new Case("crafter_place_" + (int) pitch + "_" + (int) yaw).watch(2, 100, 0);
                c.yaw = yaw;
                c.pitch = pitch;
                c.cmd("setblock 2 99 0 minecraft:stone");
                c.slot("h0", stack("minecraft:crafter", 2)).step(useOn(2, 99, 0, 1, 0));
                out.add(c);
            }
        }
        // Other blocks that face the way the player looks, not the way the face he clicked would have them.
        for (String item : new String[] {"dispenser", "dropper", "observer", "piston", "sticky_piston", "barrel"}) {
            for (float[] look : new float[][] {{0f, 0f}, {90f, 0f}, {135f, -50f}, {200f, 30f}, {0f, 80f}, {45f, -80f}}) {
                c = new Case("crafter_look_" + item + "_" + (int) look[0] + "_" + (int) look[1]).watch(2, 100, 0);
                c.yaw = look[0];
                c.pitch = look[1];
                c.cmd("setblock 2 99 0 minecraft:stone");
                c.slot("h0", stack("minecraft:" + item, 2)).step(useOn(2, 99, 0, 1, 0));
                out.add(c);
            }
        }
        // Put onto power it is triggered at once.
        c = new Case("crafter_place_powered").watch(2, 100, 0);
        c.cmd("setblock 2 99 0 minecraft:stone").cmd("setblock 2 100 1 minecraft:redstone_block");
        c.slot("h0", stack("minecraft:crafter", 2)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        // ---- the screen
        String planks = "{Items:[{Slot:0b,id:\"minecraft:oak_planks\",count:1},{Slot:1b,id:\"minecraft:oak_planks\",count:1},{Slot:3b,id:\"minecraft:oak_planks\",count:2},{Slot:4b,id:\"minecraft:oak_planks\",count:1}]}";
        String at = "minecraft:crafter[orientation=north_up,crafting=false,triggered=false]";
        c = blockCase("crafter_open_empty", at).menus();
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("crafter_open_recipe", at + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("crafter_open_disabled", at + "{Items:[{Slot:1b,id:\"minecraft:dirt\",count:3}],disabled_slots:[I;2,5,8]}").menus();
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("crafter_open_powered", "minecraft:crafter[orientation=north_up,crafting=false,triggered=true]" + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("crafter_open_sneaking_with_item", at);
        c.sneaking = true;
        c.slot("h0", stack("minecraft:stone", 3));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = blockCase("crafter_open_spectator", at + planks).menus();
        c.gameMode = "spectator";
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // Switching slots off and on.
        c = blockCase("crafter_slot_off", at).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_slot_state", "slot", 2, "enabled", false));
        out.add(c);
        c = blockCase("crafter_slot_off_then_on", at).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_slot_state", "slot", 2, "enabled", false)).step(op("op", "menu_slot_state", "slot", 2, "enabled", true));
        out.add(c);
        c = blockCase("crafter_slot_off_full_slot", at + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_slot_state", "slot", 3, "enabled", false));
        out.add(c);
        c = blockCase("crafter_slot_off_outside", at).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_slot_state", "slot", 12, "enabled", false)).step(op("op", "menu_slot_state", "slot", 45, "enabled", false));
        out.add(c);
        c = blockCase("crafter_slot_off_already_off", at + "{disabled_slots:[I;4]}").menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "menu_slot_state", "slot", 4, "enabled", false));
        out.add(c);
        // Items in and out. (Menu slots: the crafter's 0-8, the player's main 9-35, the hotbar 36-44, the result 45.)
        c = blockCase("crafter_put_item_in", at).menus();
        c.slot("h0", stack("minecraft:oak_planks", 8));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(4, 0, 0)).step(click(4, 1, 0));
        out.add(c);
        c = blockCase("crafter_put_item_in_right_click", at).menus();
        c.slot("h0", stack("minecraft:oak_planks", 8));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(4, 1, 0)).step(click(4, 1, 0)).step(click(5, 1, 0));
        out.add(c);
        c = blockCase("crafter_put_item_in_disabled_slot", at + "{disabled_slots:[I;4]}").menus();
        c.slot("h0", stack("minecraft:oak_planks", 8));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(4, 0, 0));
        out.add(c);
        c = blockCase("crafter_recipe_by_hand", at).menus();
        c.slot("h0", stack("minecraft:oak_planks", 8));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(0, 1, 0)).step(click(1, 1, 0)).step(click(3, 1, 0)).step(click(4, 1, 0));
        out.add(c);
        c = blockCase("crafter_shift_click_in", at).menus();
        c.slot("h0", stack("minecraft:oak_planks", 70)).slot("m10", stack("minecraft:dirt", 20));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 1)).step(click(10, 0, 1));
        out.add(c);
        c = blockCase("crafter_shift_click_in_skips_disabled", at + "{disabled_slots:[I;0,1,2]}").menus();
        c.slot("h0", stack("minecraft:oak_planks", 4));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 1));
        out.add(c);
        c = blockCase("crafter_shift_click_out", at + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(click(3, 0, 1)).step(click(0, 0, 1));
        out.add(c);
        c = blockCase("crafter_take_item", at + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(click(3, 0, 0)).step(click(40, 0, 0));
        out.add(c);
        c = blockCase("crafter_hotbar_swap", at + planks).menus();
        c.slot("h2", stack("minecraft:stone", 5));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(3, 2, 2)).step(click(8, 2, 2));
        out.add(c);
        c = blockCase("crafter_swap_into_disabled", at + "{disabled_slots:[I;8]}").menus();
        c.slot("h2", stack("minecraft:stone", 5));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(8, 2, 2));
        out.add(c);
        c = blockCase("crafter_double_click", at + planks).menus();
        c.slot("h0", stack("minecraft:oak_planks", 10));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(36, 0, 6));
        out.add(c);
        c = blockCase("crafter_drag", at).menus();
        c.slot("h0", stack("minecraft:oak_planks", 9));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(-999, 0, 5)).step(click(0, 1, 5)).step(click(1, 1, 5)).step(click(2, 1, 5)).step(click(-999, 2, 5));
        out.add(c);
        c = blockCase("crafter_drag_over_disabled", at + "{disabled_slots:[I;1]}").menus();
        c.slot("h0", stack("minecraft:oak_planks", 9));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(-999, 0, 5)).step(click(0, 1, 5)).step(click(1, 1, 5)).step(click(2, 1, 5)).step(click(-999, 2, 5));
        out.add(c);
        // The result only shows.
        c = blockCase("crafter_result_click", at + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(click(45, 0, 0)).step(click(45, 1, 0));
        out.add(c);
        c = blockCase("crafter_result_shift_click", at + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(click(45, 0, 1));
        out.add(c);
        c = blockCase("crafter_result_carried_on_it", at + planks).menus();
        c.slot("h0", stack("minecraft:crafting_table", 2));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(45, 0, 0));
        out.add(c);
        c = blockCase("crafter_result_hotbar_swap", at + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(click(45, 0, 2));
        out.add(c);
        c = blockCase("crafter_result_throw", at + planks).menus();
        c.step(useOn(2, 100, 0, 1, 0)).step(click(45, 0, 4)).step(click(45, 1, 4));
        out.add(c);
        c = blockCase("crafter_result_double_click", at + planks).menus();
        c.slot("h0", stack("minecraft:crafting_table", 2));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(click(36, 0, 6));
        out.add(c);
        c = blockCase("crafter_result_clone_creative", at + planks).menus();
        c.gameMode = "creative";
        c.step(useOn(2, 100, 0, 1, 0)).step(click(45, 2, 3));
        out.add(c);
        c = blockCase("crafter_close_returns_cursor", at).menus();
        c.slot("h0", stack("minecraft:oak_planks", 8));
        c.step(useOn(2, 100, 0, 1, 0)).step(click(36, 0, 0)).step(op("op", "menu_close_tick"));
        out.add(c);
    }

    /** A click on the open menu (`ContainerInput` ordinal: 0 pickup, 1 quick move, 2 swap, 3 clone, 4 throw, 5 quick craft, 6 pickup all). */
    static Map<String, Object> click(int slot, int button, int input) {
        return op("op", "menu_click", "slot", slot, "button", button, "input", input);
    }

    static Case hiveCase(String name, String block) {
        Case c = new Case(name);
        c.cmd("setblock 2 99 0 minecraft:stone").late("setblock 2 100 0 " + block).watch(2, 100, 0);
        return c;
    }

    static void hives49(List<Case> out) {
        Case c;
        String bees2 = "{bees:[{entity_data:{id:\"minecraft:bee\",HasNectar:1b},min_ticks_in_hive:100000,ticks_in_hive:7},{entity_data:{id:\"minecraft:bee\"},min_ticks_in_hive:100000,ticks_in_hive:0}]}";
        for (String kind : new String[] {"beehive", "bee_nest"}) {
            for (String facing : new String[] {"west", "north"}) {
                c = hiveCase("hive_bottle_" + kind + "_" + facing, "minecraft:" + kind + "[facing=" + facing + ",honey_level=5]" + bees2).bees().stat("minecraft:glass_bottle");
                c.slot("h0", stack("minecraft:glass_bottle", 2));
                c.step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0));
                out.add(c);
            }
        }
        c = hiveCase("hive_bottle_no_bees", "minecraft:beehive[facing=west,honey_level=5]").bees().stat("minecraft:glass_bottle");
        c.slot("h0", stack("minecraft:glass_bottle", 1));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = hiveCase("hive_bottle_level_4", "minecraft:beehive[facing=west,honey_level=4]" + bees2).bees().stat("minecraft:glass_bottle");
        c.slot("h0", stack("minecraft:glass_bottle", 1));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = hiveCase("hive_bottle_creative", "minecraft:beehive[facing=west,honey_level=5]" + "{bees:[{entity_data:{id:\"minecraft:bee\"},min_ticks_in_hive:100000,ticks_in_hive:0},{entity_data:{id:\"minecraft:bee\"},min_ticks_in_hive:100000,ticks_in_hive:0}]}").bees().stat("minecraft:glass_bottle");
        c.gameMode = "creative";
        c.slot("h0", stack("minecraft:glass_bottle", 3));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = hiveCase("hive_bottle_full_inventory", "minecraft:beehive[facing=west,honey_level=5]" + bees2).bees().stat("minecraft:glass_bottle");
        c.slot("h0", stack("minecraft:glass_bottle", 2));
        for (int i = 1; i < 9; i++) c.slot("h" + i, stack("minecraft:dirt", 64));
        for (int i = 9; i < 36; i++) c.slot("m" + i, stack("minecraft:cobblestone", 64));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        for (String kind : new String[] {"beehive", "bee_nest"}) {
            c = hiveCase("hive_shears_" + kind, "minecraft:" + kind + "[facing=west,honey_level=5]" + bees2).bees().stat("minecraft:shears");
            c.slot("h0", stack("minecraft:shears"));
            c.step(useOn(2, 100, 0, 1, 0)).step(useOn(2, 100, 0, 1, 0));
            out.add(c);
        }
        c = hiveCase("hive_shears_level_3", "minecraft:beehive[facing=west,honey_level=3]" + bees2).bees().stat("minecraft:shears");
        c.slot("h0", stack("minecraft:shears"));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // Smoke from a lit campfire below calms them: they stay in the hive's front without a target.
        c = hiveCase("hive_bottle_smoked", "minecraft:beehive[facing=west,honey_level=5]" + bees2).bees().stat("minecraft:glass_bottle");
        c.cmd("setblock 2 99 0 minecraft:campfire[lit=true]");
        c.slot("h0", stack("minecraft:glass_bottle", 1));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = hiveCase("hive_bottle_smoked_hay", "minecraft:beehive[facing=west,honey_level=5]" + bees2).bees().stat("minecraft:glass_bottle");
        c.cmd("setblock 2 99 0 minecraft:hay_block").cmd("setblock 2 98 0 minecraft:campfire[lit=true]");
        c.slot("h0", stack("minecraft:glass_bottle", 1));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = hiveCase("hive_bottle_unlit_campfire", "minecraft:beehive[facing=west,honey_level=5]" + bees2).bees().stat("minecraft:glass_bottle");
        c.cmd("setblock 2 99 0 minecraft:campfire[lit=false]");
        c.slot("h0", stack("minecraft:glass_bottle", 1));
        c.step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = hiveCase("hive_empty_hand_and_stick", "minecraft:beehive[facing=west,honey_level=5]" + bees2).bees();
        c.slot("h1", stack("minecraft:stick"));
        c.step(useOn(2, 100, 0, 1, 0)).step(op("op", "select", "slot", 1)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        // Breaking a hive: creative keeps the bees in the item it drops; survival lets them out (angry) and the drop is
        // the plain item, or with silk touch the hive with its bees; honey is kept in the item (a state).
        String dig = "dig";
        c = hiveCase("hive_break_creative_with_bees", "minecraft:bee_nest[facing=west,honey_level=0]" + bees2).bees();
        c.gameMode = "creative";
        c.step(op("op", dig, "pos", List.of(2, 100, 0)));
        out.add(c);
        c = hiveCase("hive_break_creative_honey_only", "minecraft:beehive[facing=west,honey_level=3]").bees();
        c.gameMode = "creative";
        c.step(op("op", dig, "pos", List.of(2, 100, 0)));
        out.add(c);
        c = hiveCase("hive_break_creative_empty", "minecraft:beehive[facing=west,honey_level=0]").bees();
        c.gameMode = "creative";
        c.step(op("op", dig, "pos", List.of(2, 100, 0)));
        out.add(c);
        c = hiveCase("hive_break_survival_axe", "minecraft:bee_nest[facing=west,honey_level=2]" + bees2).bees();
        c.slot("h0", enchanted("minecraft:netherite_axe", "minecraft:efficiency", 5));
        c.step(op("op", dig, "pos", List.of(2, 100, 0)));
        out.add(c);
        c = hiveCase("hive_break_survival_silk_touch", "minecraft:bee_nest[facing=west,honey_level=4]" + bees2).bees();
        ItemStack silkAxe = enchanted("minecraft:netherite_axe", "minecraft:efficiency", 5);
        silkAxe.enchant(server.registryAccess().lookupOrThrow(Registries.ENCHANTMENT).getOrThrow(ResourceKey.create(Registries.ENCHANTMENT, Identifier.parse("minecraft:silk_touch"))), 1);
        c.slot("h0", silkAxe);
        c.step(op("op", dig, "pos", List.of(2, 100, 0)));
        out.add(c);
        c = hiveCase("hive_break_survival_smoked", "minecraft:beehive[facing=west,honey_level=0]" + bees2).bees();
        c.cmd("setblock 2 99 0 minecraft:campfire[lit=true]");
        c.slot("h0", enchanted("minecraft:netherite_axe", "minecraft:efficiency", 5));
        c.step(op("op", dig, "pos", List.of(2, 100, 0)));
        out.add(c);
        // A player far from the hive: the bees come out but do not turn on them.
        c = hiveCase("hive_bottle_player_far", "minecraft:beehive[facing=west,honey_level=5]" + bees2).bees().stat("minecraft:glass_bottle");
        c.slot("h0", stack("minecraft:glass_bottle", 1));
        c.pos = new double[] {-1.5, 100.0, 0.5};
        c.step(useOnAt(2, 100, 0, 1, 0, 0.5, 1.0, 0.5));
        out.add(c);
    }

    /** wp49: item frames, glow item frames and paintings. */
    /** wp49: bells (a click on the body or the beam, from every side), bells put up against every kind of support. */
    static void bells49(List<Case> out) {
        Case c;
        String[] attach = {"floor", "ceiling", "single_wall", "double_wall"};
        String[] facings = {"north", "east"};
        // faces: down 0, up 1, north 2, south 3, west 4, east 5
        for (String a : attach) {
            for (String f : facings) {
                for (int face = 0; face < 6; face++) {
                    c = blockCase("bell_" + a + "_" + f + "_face" + face, "minecraft:bell[attachment=" + a + ",facing=" + f + ",powered=false]").custom("minecraft:bell_ring");
                    c.step(useOnAt(2, 100, 0, face, 0, 0.5, 0.5, 0.5));
                    out.add(c);
                }
            }
            // The top beam (above 0.8124 of the block) does not ring it.
            c = blockCase("bell_" + a + "_beam", "minecraft:bell[attachment=" + a + ",facing=north,powered=false]").custom("minecraft:bell_ring");
            c.step(useOnAt(2, 100, 0, 2, 0, 0.5, 0.9, 0.5)).step(useOnAt(2, 100, 0, 5, 0, 0.5, 0.9, 0.5)).step(useOnAt(2, 100, 0, 2, 0, 0.5, 0.8, 0.5));
            out.add(c);
        }
        String floor = "minecraft:bell[attachment=floor,facing=north,powered=false]";
        c = blockCase("bell_creative", floor).custom("minecraft:bell_ring");
        c.gameMode = "creative";
        c.step(useOnAt(2, 100, 0, 2, 0, 0.5, 0.5, 0.5));
        out.add(c);
        c = blockCase("bell_adventure", floor).custom("minecraft:bell_ring");
        c.gameMode = "adventure";
        c.step(useOnAt(2, 100, 0, 2, 0, 0.5, 0.5, 0.5));
        out.add(c);
        c = blockCase("bell_sneaking", floor).custom("minecraft:bell_ring");
        c.sneaking = true;
        c.step(useOnAt(2, 100, 0, 2, 0, 0.5, 0.5, 0.5));
        out.add(c);
        c = blockCase("bell_sneaking_with_block", floor).custom("minecraft:bell_ring");
        c.sneaking = true;
        c.slot("h0", stack("minecraft:stone", 3)).step(useOnAt(2, 100, 0, 2, 0, 0.5, 0.5, 0.5));
        out.add(c);
        c = blockCase("bell_with_block_in_hand", floor).custom("minecraft:bell_ring");
        c.slot("h0", stack("minecraft:stone", 3)).step(useOnAt(2, 100, 0, 2, 0, 0.5, 0.5, 0.5));
        out.add(c);
        c = blockCase("bell_ring_twice", "minecraft:bell[attachment=ceiling,facing=east,powered=false]").custom("minecraft:bell_ring");
        c.step(useOnAt(2, 100, 0, 2, 0, 0.5, 0.5, 0.5)).step(useOnAt(2, 100, 0, 4, 0, 0.5, 0.5, 0.5));
        out.add(c);
        // Redstone: a bell rings when power reaches it, once.
        c = blockCase("bell_powered", floor);
        c.step(cmdStep("setblock 3 100 0 minecraft:redstone_block")).step(cmdStep("setblock 3 100 0 minecraft:air"))
                .step(cmdStep("setblock 2 101 0 minecraft:redstone_block"));
        out.add(c);

        // ---- put up against a support
        Object[][] places = {
            {"floor", 2, 99, 0, 1},       // on a stone
            {"ceiling", 2, 101, 0, 0},    // under a stone
            {"wall_east", 1, 100, 0, 5},  // against a stone to the west, on its east face
            {"wall_west", 3, 100, 0, 4},
            {"wall_north", 2, 100, 1, 2},
            {"wall_south", 2, 100, -1, 3},
        };
        for (Object[] pl : places) {
            c = new Case("bell_place_" + pl[0]);
            c.cmd("setblock " + pl[1] + " " + pl[2] + " " + pl[3] + " minecraft:stone").watch(2, 100, 0);
            c.slot("h0", stack("minecraft:bell", 3)).step(useOn((int) pl[1], (int) pl[2], (int) pl[3], (int) pl[4], 0));
            out.add(c);
        }
        // Between two stones: held by both walls.
        c = new Case("bell_place_double_wall_x").cmd("setblock 1 100 0 minecraft:stone").cmd("setblock 3 100 0 minecraft:stone").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:bell", 3)).step(useOn(1, 100, 0, 5, 0));
        out.add(c);
        c = new Case("bell_place_double_wall_z").cmd("setblock 2 100 1 minecraft:stone").cmd("setblock 2 100 -1 minecraft:stone").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:bell", 3)).step(useOn(2, 100, 1, 2, 0));
        out.add(c);
        // One wall, a floor below: the wall holds it first.
        c = new Case("bell_place_wall_with_floor").cmd("setblock 1 100 0 minecraft:stone").cmd("setblock 2 99 0 minecraft:stone").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:bell", 3)).step(useOn(1, 100, 0, 5, 0));
        out.add(c);
        // A wall that does not hold it (a bottom slab's side has no full face), with and without a floor.
        c = new Case("bell_place_on_slab_side_floor").cmd("setblock 1 100 0 minecraft:oak_slab[type=bottom]").cmd("setblock 2 99 0 minecraft:stone").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:bell", 3)).step(useOn(1, 100, 0, 5, 0));
        out.add(c);
        c = new Case("bell_place_on_slab_side_nothing").cmd("setblock 1 100 0 minecraft:oak_slab[type=bottom]").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:bell", 3)).step(useOn(1, 100, 0, 5, 0));
        out.add(c);
        // On the top of a fence (no full face), and under slabs.
        c = new Case("bell_place_on_fence").cmd("setblock 2 99 0 minecraft:oak_fence").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:bell", 3)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("bell_place_under_bottom_slab").cmd("setblock 2 101 0 minecraft:oak_slab[type=bottom]").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:bell", 3)).step(useOn(2, 101, 0, 0, 0));
        out.add(c);
        c = new Case("bell_place_under_top_slab").cmd("setblock 2 101 0 minecraft:oak_slab[type=top]").watch(2, 100, 0);
        c.slot("h0", stack("minecraft:bell", 3)).step(useOn(2, 101, 0, 0, 0));
        out.add(c);
        // Losing the support: the stone under a floor bell goes; a double wall bell loses one wall, then the other.
        c = blockCase("bell_floor_loses_floor", floor);
        c.step(cmdStep("setblock 2 99 0 minecraft:air"));
        out.add(c);
        c = new Case("bell_double_loses_one").cmd("setblock 1 100 0 minecraft:stone").cmd("setblock 3 100 0 minecraft:stone")
                .cmd("setblock 2 100 0 minecraft:bell[attachment=double_wall,facing=east,powered=false]").watch(2, 100, 0);
        c.step(cmdStep("setblock 1 100 0 minecraft:air")).step(cmdStep("setblock 3 100 0 minecraft:air"));
        out.add(c);
        c = new Case("bell_single_gains_second").cmd("setblock 1 100 0 minecraft:stone")
                .cmd("setblock 2 100 0 minecraft:bell[attachment=single_wall,facing=east,powered=false]").watch(2, 100, 0);
        c.step(cmdStep("setblock 3 100 0 minecraft:stone")).step(cmdStep("setblock 3 100 0 minecraft:air"));
        out.add(c);
    }

    static Map<String, Object> cmdStep(String cmd) {
        return op("op", "command", "command", cmd);
    }

    /** wp49: armor stands (placing one, dressing one, taking its things, hitting it). */
    static void stands49(List<Case> out) {
        Case c;
        String floor = "setblock 2 99 0 minecraft:stone";
        // ---- put up
        float[] yaws = {0f, 22f, 23f, 45f, 67.4f, 90f, 135f, 179f, -45f, -100f};
        for (float yaw : yaws) {
            c = new Case("stand_place_yaw_" + (int) (yaw * 10)).stands().stat("minecraft:armor_stand");
            c.yaw = yaw;
            c.cmd(floor);
            c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 99, 0, 1, 0));
            out.add(c);
        }
        for (int face = 0; face < 6; face++) {
            c = new Case("stand_place_face_" + face).stands().stat("minecraft:armor_stand");
            c.cmd("setblock 2 100 0 minecraft:stone");
            c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 100, 0, face, 0));
            out.add(c);
        }
        c = new Case("stand_place_creative").stands().stat("minecraft:armor_stand");
        c.gameMode = "creative";
        c.cmd(floor);
        c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("stand_place_adventure").stands().stat("minecraft:armor_stand");
        c.gameMode = "adventure";
        c.cmd(floor);
        c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("stand_place_offhand").stands().stat("minecraft:armor_stand");
        c.cmd(floor);
        c.slot("offhand", stack("minecraft:armor_stand", 3)).step(useOn(2, 99, 0, 1, 1));
        out.add(c);
        c = new Case("stand_place_twice").stands().stat("minecraft:armor_stand");
        c.cmd(floor);
        c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 99, 0, 1, 0)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("stand_place_in_grass").stands().stat("minecraft:armor_stand");
        c.cmd(floor).cmd("setblock 2 100 0 minecraft:short_grass");
        c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 100, 0, 1, 0));
        out.add(c);
        c = new Case("stand_place_under_roof").stands().stat("minecraft:armor_stand");
        c.cmd(floor).cmd("setblock 2 101 0 minecraft:stone");
        c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("stand_place_in_slab").stands().stat("minecraft:armor_stand");
        c.cmd(floor).cmd("setblock 2 102 0 minecraft:oak_slab[type=bottom]");
        c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("stand_place_on_slab_top").stands().stat("minecraft:armor_stand");
        c.cmd("setblock 2 99 0 minecraft:oak_slab[type=top]");
        c.slot("h0", stack("minecraft:armor_stand", 3)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("stand_place_named").stands().stat("minecraft:armor_stand");
        c.cmd(floor);
        c.slot("h0", named("minecraft:armor_stand", 2, "Bob")).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("stand_place_with_data").stands().stat("minecraft:armor_stand");
        c.cmd(floor);
        c.slot("h0", standWithData()).step(useOn(2, 99, 0, 1, 0));
        out.add(c);

        // ---- dress one: the item goes where it belongs (or to the hand), whatever the height clicked
        String summon = "summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,ShowArms:1b}";
        String[] items = {"minecraft:diamond_helmet", "minecraft:iron_chestplate", "minecraft:leather_leggings", "minecraft:netherite_boots", "minecraft:carved_pumpkin",
                "minecraft:player_head", "minecraft:elytra", "minecraft:iron_sword", "minecraft:shield", "minecraft:stone", "minecraft:white_banner", "minecraft:turtle_helmet",
                "minecraft:saddle", "minecraft:wolf_armor"};
        for (String item : items) {
            c = new Case("stand_dress_" + item.substring(10)).stands();
            c.cmd(summon);
            c.slot("h0", stack(item, 3)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
            out.add(c);
        }
        c = new Case("stand_dress_offhand_item").stands();
        c.cmd(summon);
        c.slot("offhand", stack("minecraft:diamond_chestplate", 1)).step(useStand(2.5, 100, 0.5, 1, false, 0.5));
        out.add(c);
        c = new Case("stand_dress_no_arms").stands();
        c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b}");
        c.slot("h0", stack("minecraft:iron_sword", 1)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        c = new Case("stand_dress_creative").stands();
        c.gameMode = "creative";
        c.cmd(summon);
        c.slot("h0", stack("minecraft:iron_sword", 2)).step(useStand(2.5, 100, 0.5, 0, false, 0.5)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        c = new Case("stand_dress_spectator").stands();
        c.gameMode = "spectator";
        c.cmd(summon);
        c.slot("h0", stack("minecraft:iron_sword", 2)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        c = new Case("stand_dress_marker").stands();
        c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,ShowArms:1b,Marker:1b}");
        c.slot("h0", stack("minecraft:iron_helmet", 2)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        c = new Case("stand_dress_name_tag").stands();
        c.cmd(summon);
        c.slot("h0", named("minecraft:name_tag", 1, "Zed")).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        // Swapping: what is worn comes into the hand; a stack of two puts one on (only on an empty slot).
        c = new Case("stand_swap_helmet").stands();
        c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,ShowArms:1b,equipment:{head:{id:\"minecraft:iron_helmet\",count:1}}}");
        c.slot("h0", stack("minecraft:diamond_helmet", 1)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        c = new Case("stand_swap_helmet_stack").stands();
        c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,ShowArms:1b,equipment:{head:{id:\"minecraft:iron_helmet\",count:1}}}");
        c.slot("h0", stack("minecraft:diamond_helmet", 2)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        c = new Case("stand_swap_helmet_creative").stands();
        c.gameMode = "creative";
        c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,ShowArms:1b,equipment:{head:{id:\"minecraft:iron_helmet\",count:1}}}");
        c.slot("h0", stack("minecraft:diamond_helmet", 1)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        c = new Case("stand_stack_one").stands();
        c.cmd(summon);
        c.slot("h0", stack("minecraft:iron_helmet", 3)).step(useStand(2.5, 100, 0.5, 0, false, 0.5)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);

        // ---- take things off, by the height clicked (a full stand, empty hand and a stone in the hand)
        String full = "summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,ShowArms:1b,equipment:{mainhand:{id:\"minecraft:iron_sword\",count:1},offhand:{id:\"minecraft:shield\",count:1},"
                + "feet:{id:\"minecraft:iron_boots\",count:1},legs:{id:\"minecraft:iron_leggings\",count:1},chest:{id:\"minecraft:iron_chestplate\",count:1},head:{id:\"minecraft:iron_helmet\",count:1}}}";
        double[] ys = {0.0, 0.05, 0.1, 0.3, 0.5, 0.55, 0.85, 0.9, 1.0, 1.2, 1.55, 1.6, 1.9};
        for (double y : ys) {
            c = new Case("stand_take_y" + (int) Math.round(y * 100)).stands();
            c.cmd(full);
            c.step(useStand(2.5, 100, 0.5, 0, false, y)).step(useStand(2.5, 100, 0.5, 0, false, y));
            out.add(c);
        }
        for (double y : new double[] {0.05, 0.5, 1.0, 1.7}) {
            c = new Case("stand_take_small_y" + (int) Math.round(y * 100)).stands();
            c.cmd(full.replace("{NoGravity:1b,ShowArms:1b,", "{NoGravity:1b,ShowArms:1b,Small:1b,"));
            c.step(useStand(2.5, 100, 0.5, 0, false, y)).step(useStand(2.5, 100, 0.5, 0, false, y)).step(useStand(2.5, 100, 0.5, 0, false, y));
            out.add(c);
        }
        c = new Case("stand_take_hands").stands();
        c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,ShowArms:1b,equipment:{offhand:{id:\"minecraft:shield\",count:1}}}");
        c.step(useStand(2.5, 100, 0.5, 0, false, 0.5)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
        out.add(c);
        c = new Case("stand_take_with_stone").stands();
        c.cmd(full);
        c.slot("h0", stack("minecraft:stone", 3)).step(useStand(2.5, 100, 0.5, 0, false, 1.9));
        out.add(c);
        c = new Case("stand_take_nothing").stands();
        c.cmd(summon);
        c.step(useStand(2.5, 100, 0.5, 0, false, 1.9));
        out.add(c);
        // Disabled slots: nothing may be taken (8), nothing put on (16), the slot is gone (1 << filter bit).
        int[] disabled = {1 << 1, 1 << 2, 1 << 3, 1 << 4, 1 << 0, 1 << 5, 1 << 9, 1 << 10, 1 << 11, 1 << 12, 1 << 13, 1 << 16, 1 << 17, 1 << 18, 1 << 19, 1 << 20, 1 << 21, 0xffffff};
        for (int d : disabled) {
            c = new Case("stand_disabled_" + d + "_take").stands();
            c.cmd(full.replace("{NoGravity:1b,ShowArms:1b,", "{NoGravity:1b,ShowArms:1b,DisabledSlots:" + d + ","));
            c.step(useStand(2.5, 100, 0.5, 0, false, 1.9)).step(useStand(2.5, 100, 0.5, 0, false, 1.0)).step(useStand(2.5, 100, 0.5, 0, false, 0.05)).step(useStand(2.5, 100, 0.5, 0, false, 0.5));
            out.add(c);
            c = new Case("stand_disabled_" + d + "_put").stands();
            c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,ShowArms:1b,DisabledSlots:" + d + "}");
            c.slot("h0", stack("minecraft:iron_helmet", 3)).slot("h1", stack("minecraft:iron_chestplate", 3)).slot("h2", stack("minecraft:iron_leggings", 3)).slot("h3", stack("minecraft:iron_boots", 3))
                    .slot("h4", stack("minecraft:iron_sword", 3));
            c.step(useStand(2.5, 100, 0.5, 0, false, 0.5)).step(op("op", "select", "slot", 1)).step(useStand(2.5, 100, 0.5, 0, false, 0.5)).step(op("op", "select", "slot", 2))
                    .step(useStand(2.5, 100, 0.5, 0, false, 0.5)).step(op("op", "select", "slot", 3)).step(useStand(2.5, 100, 0.5, 0, false, 0.5)).step(op("op", "select", "slot", 4))
                    .step(useStand(2.5, 100, 0.5, 0, false, 0.5));
            out.add(c);
        }

        // ---- the saved data of a stand made with all sorts of data
        c = new Case("stand_nbt_pose").stands();
        c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,Pose:{Head:[10f,20f,30f],Body:[1f,2f,3f],LeftArm:[-10f,0f,-10f],RightLeg:[5f,5f,5f]},Small:1b,ShowArms:1b,NoBasePlate:1b,Invisible:1b,"
                + "DisabledSlots:4144,CustomName:\"Zed\",CustomNameVisible:1b,Tags:[\"a\",\"b\"],Rotation:[45f,10f],Health:7f,Silent:1b,Glowing:1b,Air:200s,Invulnerable:1b}");
        c.step(op("op", "select", "slot", 0));
        out.add(c);
        c = new Case("stand_nbt_marker").stands();
        c.cmd("summon minecraft:armor_stand 2.5 100 0.5 {NoGravity:1b,Marker:1b,Pose:{LeftArm:[1f,1f,1f]},equipment:{head:{id:\"minecraft:player_head\",count:1}}}");
        c.step(op("op", "select", "slot", 0));
        out.add(c);

        // ---- hit it
        c = new Case("stand_hit_twice").stands();
        c.cmd(full);
        c.step(attackEntity(2.5, 100, 0.5)).step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_waiting").stands();
        c.cmd(full);
        c.step(attackEntity(2.5, 100, 0.5)).step(op("op", "wait", "ticks", 3)).step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_waiting_long").stands();
        c.cmd(full);
        c.step(attackEntity(2.5, 100, 0.5)).step(op("op", "wait", "ticks", 6)).step(attackEntity(2.5, 100, 0.5)).step(op("op", "wait", "ticks", 2)).step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_once").stands();
        c.cmd(full);
        c.step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_creative").stands();
        c.gameMode = "creative";
        c.cmd(full);
        c.step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_adventure").stands();
        c.gameMode = "adventure";
        c.cmd(full);
        c.step(attackEntity(2.5, 100, 0.5)).step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_named").stands();
        c.cmd(full.replace("{NoGravity:1b,ShowArms:1b,", "{NoGravity:1b,ShowArms:1b,CustomName:\"Bob\","));
        c.step(attackEntity(2.5, 100, 0.5)).step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_invisible").stands();
        c.cmd(full.replace("{NoGravity:1b,ShowArms:1b,", "{NoGravity:1b,ShowArms:1b,Invisible:1b,"));
        c.step(attackEntity(2.5, 100, 0.5)).step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_marker").stands();
        c.cmd(full.replace("{NoGravity:1b,ShowArms:1b,", "{NoGravity:1b,ShowArms:1b,Marker:1b,"));
        c.step(attackEntity(2.5, 100, 0.5)).step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        c = new Case("stand_hit_invulnerable").stands();
        c.cmd(full.replace("{NoGravity:1b,ShowArms:1b,", "{NoGravity:1b,ShowArms:1b,Invulnerable:1b,"));
        c.step(attackEntity(2.5, 100, 0.5)).step(attackEntity(2.5, 100, 0.5));
        out.add(c);
        // Damage of other kinds (the /damage command).
        String[] kinds = {"minecraft:explosion", "minecraft:player_explosion", "minecraft:in_fire", "minecraft:on_fire", "minecraft:lava", "minecraft:hot_floor", "minecraft:generic",
                "minecraft:fall", "minecraft:drown", "minecraft:arrow", "minecraft:out_of_world", "minecraft:generic_kill", "minecraft:magic", "minecraft:lightning_bolt", "minecraft:wither",
                "minecraft:cactus", "minecraft:fireball", "minecraft:player_attack", "minecraft:mob_attack", "minecraft:thrown", "minecraft:fireworks", "minecraft:campfire", "minecraft:freeze"};
        for (String kind : kinds) {
            c = new Case("stand_damage_" + kind.substring(10)).stands();
            c.cmd(full);
            c.step(cmdStep("damage @e[type=minecraft:armor_stand,limit=1] 5 " + kind)).step(cmdStep("damage @e[type=minecraft:armor_stand,limit=1] 5 " + kind))
                    .step(cmdStep("damage @e[type=minecraft:armor_stand,limit=1] 5 " + kind)).step(cmdStep("damage @e[type=minecraft:armor_stand,limit=1] 20 " + kind));
            out.add(c);
        }
        c = new Case("stand_kill_command").stands();
        c.cmd(full);
        c.step(cmdStep("kill @e[type=minecraft:armor_stand]"));
        out.add(c);
    }

    static ItemStack named(String item, int count, String name) {
        ItemStack s = stack(item, count);
        s.set(DataComponents.CUSTOM_NAME, net.minecraft.network.chat.Component.literal(name));
        return s;
    }

    /** An armor stand item with `entity_data` (arms, small, no base plate). */
    static ItemStack standWithData() {
        ItemStack s = stack("minecraft:armor_stand", 2);
        net.minecraft.nbt.CompoundTag tag = new net.minecraft.nbt.CompoundTag();
        tag.putBoolean("ShowArms", true);
        tag.putBoolean("Small", true);
        tag.putBoolean("NoBasePlate", true);
        s.set(DataComponents.ENTITY_DATA, net.minecraft.world.item.component.TypedEntityData.of(net.minecraft.world.entity.EntityTypes.ARMOR_STAND, tag));
        return s;
    }

    static void frames49(List<Case> out) {
        Case c;
        // ---- a stone block at (2, 100, 0), a frame on each of its faces (the player at 0.5, 100, 0.5)
        String[] names = {"down", "up", "north", "south", "west", "east"};
        for (int face = 0; face < 6; face++) {
            for (String kind : new String[] {"item_frame", "glow_item_frame"}) {
                c = new Case("frame_place_" + kind + "_" + names[face]).hanging().stat("minecraft:" + kind);
                c.cmd("setblock 2 100 0 minecraft:stone");
                c.slot("h0", stack("minecraft:" + kind, 3)).step(useOn(2, 100, 0, face, 0));
                out.add(c);
            }
        }
        c = new Case("frame_place_creative").hanging().stat("minecraft:item_frame");
        c.gameMode = "creative";
        c.cmd("setblock 2 100 0 minecraft:stone");
        c.slot("h0", stack("minecraft:item_frame", 3)).step(useOn(2, 100, 0, 4, 0));
        out.add(c);
        c = new Case("frame_place_adventure").hanging().stat("minecraft:item_frame");
        c.gameMode = "adventure";
        c.cmd("setblock 2 100 0 minecraft:stone");
        c.slot("h0", stack("minecraft:item_frame", 3)).step(useOn(2, 100, 0, 4, 0));
        out.add(c);
        c = new Case("frame_place_offhand").hanging().stat("minecraft:item_frame");
        c.cmd("setblock 2 100 0 minecraft:stone");
        c.slot("offhand", stack("minecraft:item_frame", 3)).step(useOn(2, 100, 0, 4, 1));
        out.add(c);
        // Two on the same face: the second finds the first in the way; one on another face of the block is fine.
        c = new Case("frame_place_twice").hanging().stat("minecraft:item_frame");
        c.cmd("setblock 2 100 0 minecraft:stone");
        c.slot("h0", stack("minecraft:item_frame", 4)).step(useOn(2, 100, 0, 4, 0)).step(useOn(2, 100, 0, 4, 0)).step(useOn(2, 100, 0, 2, 0));
        out.add(c);
        // A frame beside a frame: the neighbour's wall is a block that has its own frame in front.
        c = new Case("frame_place_row").hanging().stat("minecraft:item_frame");
        c.cmd("fill 2 99 -1 2 101 1 minecraft:stone");
        c.slot("h0", stack("minecraft:item_frame", 6));
        for (int z = -1; z <= 1; z++) for (int y = 99; y <= 101; y += 1) if (y != 100 || z != 0) c.step(useOn(2, y, z, 4, 0));
        out.add(c);
        // On things that are not a wall: glass, a slab, a fence, a repeater's side, a stair.
        for (String block : new String[] {"minecraft:glass", "minecraft:oak_slab[type=bottom]", "minecraft:oak_fence", "minecraft:repeater[facing=east]", "minecraft:oak_stairs[facing=east]",
                "minecraft:oak_leaves", "minecraft:iron_bars", "minecraft:ice", "minecraft:barrier", "minecraft:oak_trapdoor[half=top,open=false]"}) {
            c = new Case("frame_place_on_" + block.replaceAll("[^a-z_]", "_")).hanging();
            c.cmd("setblock 2 100 0 " + block).watch(2, 100, 0);
            c.slot("h0", stack("minecraft:item_frame", 2)).step(useOn(2, 100, 0, 4, 0));
            out.add(c);
        }
        // ---- using a frame: item in, rotations, item out, frame out
        c = new Case("frame_use").hanging();
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:item_frame 1 100 0 {Facing:4b}");
        c.slot("h0", stack("minecraft:apple", 2));
        c.step(useEntity(1.96875, 100.5, 0.5, 0, false));
        for (int i = 0; i < 9; i++) c.step(useEntity(1.96875, 100.5, 0.5, 0, false));
        out.add(c);
        c = new Case("frame_use_empty_hand").hanging();
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:item_frame 1 100 0 {Facing:4b}");
        c.step(useEntity(1.96875, 100.5, 0.5, 0, false));
        out.add(c);
        c = new Case("frame_use_creative").hanging();
        c.gameMode = "creative";
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:item_frame 1 100 0 {Facing:4b}");
        c.slot("h0", stack("minecraft:diamond_sword", 1));
        c.step(useEntity(1.96875, 100.5, 0.5, 0, false)).step(useEntity(1.96875, 100.5, 0.5, 0, false));
        out.add(c);
        c = new Case("frame_use_sneaking").hanging();
        c.sneaking = true;
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:item_frame 1 100 0 {Facing:4b}");
        c.slot("h0", stack("minecraft:apple", 2));
        c.step(useEntity(1.96875, 100.5, 0.5, 0, true)).step(useEntity(1.96875, 100.5, 0.5, 0, true));
        out.add(c);
        c = new Case("frame_use_offhand").hanging();
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:glow_item_frame 1 100 0 {Facing:4b}");
        c.slot("offhand", stack("minecraft:stick", 2));
        c.step(useEntity(1.96875, 100.5, 0.5, 1, false)).step(useEntity(1.96875, 100.5, 0.5, 1, false));
        out.add(c);
        c = new Case("frame_use_fixed").hanging();
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:item_frame 1 100 0 {Facing:4b,Fixed:1b,Item:{id:\"minecraft:stick\",count:1}}");
        c.slot("h0", stack("minecraft:apple", 2));
        c.step(useEntity(1.96875, 100.5, 0.5, 0, false)).step(attackEntity(1.96875, 100.5, 0.5));
        out.add(c);
        c = new Case("frame_use_adventure").hanging();
        c.gameMode = "adventure";
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:item_frame 1 100 0 {Facing:4b}");
        c.slot("h0", stack("minecraft:apple", 2));
        c.step(useEntity(1.96875, 100.5, 0.5, 0, false));
        out.add(c);
        // ---- hitting a frame: the item first, then the frame
        for (String mode : new String[] {"survival", "creative", "adventure"}) {
            c = new Case("frame_hit_" + mode).hanging();
            c.gameMode = mode;
            c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:item_frame 1 100 0 {Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}");
            c.step(attackEntity(1.96875, 100.5, 0.5)).step(attackEntity(1.96875, 100.5, 0.5));
            out.add(c);
        }
        c = new Case("frame_hit_drop_chance").hanging();
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:glow_item_frame 1 100 0 {Facing:4b,ItemDropChance:0f,Item:{id:\"minecraft:diamond\",count:1}}");
        c.step(attackEntity(1.96875, 100.5, 0.5)).step(attackEntity(1.96875, 100.5, 0.5));
        out.add(c);
        c = new Case("frame_hit_named").hanging();
        c.cmd("setblock 2 100 0 minecraft:stone").cmd("summon minecraft:item_frame 1 100 0 {Facing:4b,CustomName:'\"Gallery\"'}");
        c.step(attackEntity(1.96875, 100.5, 0.5));
        out.add(c);
        // ---- paintings on walls of several sizes (the variant among the ones of one area is the entity's random: only the area is compared)
        int[][] walls = {{1, 1}, {2, 1}, {1, 2}, {2, 2}, {3, 3}, {4, 2}, {4, 3}, {4, 4}, {3, 4}, {5, 5}};
        for (int[] w : walls) {
            c = new Case("painting_wall_" + w[0] + "x" + w[1]).hanging().stat("minecraft:painting");
            // (A wall w wide from z -1 and h high from y 100, the click on (3, 100, 0).)
            c.cmd("fill 3 100 -1 3 " + (100 + w[1] - 1) + " " + (w[0] - 2) + " minecraft:stone").cmd("setblock 3 100 0 minecraft:stone");
            c.slot("h0", stack("minecraft:painting", 2)).step(useOn(3, 100, 0, 4, 0));
            out.add(c);
        }
        c = new Case("painting_on_floor").hanging().stat("minecraft:painting");
        c.cmd("fill 1 99 -2 4 99 3 minecraft:stone");
        c.slot("h0", stack("minecraft:painting", 2)).step(useOn(2, 99, 0, 1, 0));
        out.add(c);
        c = new Case("painting_no_wall").hanging().stat("minecraft:painting");
        c.cmd("setblock 3 100 0 minecraft:stone");
        c.slot("h0", stack("minecraft:painting", 2)).step(useOn(3, 100, 0, 4, 0));
        out.add(c);
        c = new Case("painting_creative").hanging().stat("minecraft:painting");
        c.gameMode = "creative";
        c.cmd("fill 3 100 -1 3 101 0 minecraft:stone");
        c.slot("h0", stack("minecraft:painting", 2)).step(useOn(3, 100, 0, 4, 0));
        out.add(c);
        c = new Case("painting_two_on_a_wall").hanging().stat("minecraft:painting");
        c.cmd("fill 3 100 -3 3 103 3 minecraft:stone");
        c.slot("h0", stack("minecraft:painting", 4)).step(useOn(3, 100, 0, 4, 0)).step(useOn(3, 100, 0, 4, 0)).step(useOn(3, 102, 2, 4, 0));
        out.add(c);
        c = new Case("painting_hit").hanging();
        c.cmd("fill 3 99 -2 3 103 3 minecraft:stone").cmd("summon minecraft:painting 2 100 0 {facing:1b,variant:\"minecraft:courbet\"}");
        c.step(attackEntity(2.96875, 100.5, 1.0));
        out.add(c);
        c = new Case("painting_hit_creative").hanging();
        c.gameMode = "creative";
        c.cmd("fill 3 99 -2 3 103 3 minecraft:stone").cmd("summon minecraft:painting 2 100 0 {facing:1b,variant:\"minecraft:courbet\"}");
        c.step(attackEntity(2.96875, 100.5, 1.0));
        out.add(c);
    }

    // ---------------------------------------------------------------- sign scenarios

    static final int SX = 4, SY = 100, SZ = 4;
    static final double[][] AROUND = {{0, 2}, {0, -2}, {2, 0}, {-2, 0}, {1.5, 1.5}, {-1.5, -1.5}, {2, -1}, {-1, 2}};

    /** wp53: `advancement/criterion` for every criterion the player has completed. */
    static java.util.TreeSet<String> doneCriteria(ServerPlayer p) {
        var done = new java.util.TreeSet<String>();
        for (var holder : server.getAdvancements().getAllAdvancements()) {
            for (String crit : p.getAdvancements().getOrStartProgress(holder).getCompletedCriteria()) done.add(holder.id() + "/" + crit);
        }
        return done;
    }

    /** wp53: the player's advancement progress is cleared (the profile's advancements outlive a case). */
    static void resetAdvancements(ServerPlayer p) {
        for (var holder : server.getAdvancements().getAllAdvancements()) {
            for (String crit : new ArrayList<String>(java.util.stream.StreamSupport.stream(p.getAdvancements().getOrStartProgress(holder).getCompletedCriteria().spliterator(), false).toList())) {
                p.getAdvancements().revoke(holder, crit);
            }
        }
    }

    static Map<String, Object> useOn(int x, int y, int z, int face, int hand) {
        return op("op", "use_on", "hand", hand, "pos", List.of(x, y, z), "face", face, "cursor", List.of(0.5, 0.5, 0.5));
    }

    static Map<String, Object> signUpdate(boolean front, String... lines) {
        return op("op", "sign_update", "pos", List.of(SX, SY, SZ), "front", front, "lines", List.of(lines));
    }

    static String messages(String... lines) {
        StringBuilder b = new StringBuilder("[");
        for (int i = 0; i < 4; i++) b.append(i > 0 ? "," : "").append("\"").append(i < lines.length ? lines[i] : "").append("\"");
        return b.append("]").toString();
    }

    /** A sign with text on both sides (front lines "Front", back lines "Back"). */
    static String signState(String block, String props, String front, String back, boolean waxed) {
        return String.format("setblock %d %d %d minecraft:%s[%s]{front_text:{messages:%s},back_text:{messages:%s},is_waxed:%db}",
                SX, SY, SZ, block, props, front, back, waxed ? 1 : 0);
    }

    static Case signCase(String name, String block, String props, String support) {
        Case c = new Case(name);
        c.cmd("setblock " + support);
        c.cmd(String.format("setblock %d %d %d minecraft:%s[%s]", SX, SY, SZ, block, props));
        c.watch(SX, SY, SZ);
        return c;
    }

    static void signs(List<Case> out) {
        // The side a player faces: standing, wall, hanging and wall hanging signs, from around.
        String[][] kinds = {
            {"standing", "oak_sign", "rotation=%d", "4 99 4 minecraft:stone"},
            {"wall", "oak_wall_sign", "facing=%s", "4 100 5 minecraft:stone"},
            {"hanging", "oak_hanging_sign", "rotation=%d,attached=false", "4 101 4 minecraft:stone"},
            {"wallhanging", "oak_wall_hanging_sign", "facing=%s", "4 101 4 minecraft:stone"},
        };
        int[] rotations = {0, 3, 4, 8, 12, 13};
        String[] facings = {"north", "south", "east", "west"};
        for (String[] k : kinds) {
            boolean rotating = k[2].contains("%d");
            for (int r = 0; r < (rotating ? rotations.length : facings.length); r++) {
                String props = rotating ? String.format(k[2], rotations[r]) : String.format(k[2], facings[r]);
                String tag = rotating ? "r" + rotations[r] : facings[r];
                for (int a = 0; a < AROUND.length; a++) {
                    Case c = signCase("sign_side_" + k[0] + "_" + tag + "_p" + a, k[1], props, k[3]);
                    c.pos = new double[] {SX + 0.5 + AROUND[a][0], 100.0, SZ + 0.5 + AROUND[a][1]};
                    c.step(useOn(SX, SY, SZ, 1, 0));
                    out.add(c);
                }
            }
        }
        // The editing cycle on a standing sign: open, edit, open again, edit the other side.
        Case c = signCase("sign_edit_cycle", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.step(useOn(SX, SY, SZ, 1, 0))
                .step(signUpdate(true, "Hello", "", "World", "last"))
                .step(useOn(SX, SY, SZ, 1, 0))
                .step(signUpdate(true, "Again", "b", "c", "d"))
                .step(signUpdate(true, "No lock", "", "", ""));
        out.add(c);
        c = signCase("sign_edit_back", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ - 1.5};
        c.step(useOn(SX, SY, SZ, 1, 0)).step(signUpdate(false, "Back", "side", "", "x")).step(signUpdate(true, "Front", "", "", ""));
        out.add(c);
        // Updates without having opened the editor are refused.
        c = signCase("sign_update_unlocked", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.step(signUpdate(true, "sneaky", "", "", ""));
        out.add(c);
        // Formatting codes are stripped; long lines and unicode survive.
        c = signCase("sign_update_formatting", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.step(useOn(SX, SY, SZ, 1, 0)).step(signUpdate(true, "§cRed§r text", "plain §", "café 中文", "x".repeat(90)));
        out.add(c);
        // A line's style (color) stays when its text changes.
        c = signCase("sign_update_keeps_style", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(String.format("data merge block %d %d %d {front_text:{messages:[{text:\"a\",color:\"red\",bold:1b},\"b\",{text:\"\",italic:1b},\"\"]}}", SX, SY, SZ));
        c.step(useOn(SX, SY, SZ, 1, 0)).step(signUpdate(true, "new", "text", "here", "now"));
        out.add(c);
        // Dyes, glow ink, ink sac and honeycomb on text.
        String[] dyes = {"red_dye", "blue_dye", "black_dye", "white_dye", "lime_dye"};
        for (String dye : dyes) {
            c = signCase("sign_dye_" + dye, "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
            c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
            c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
            c.slot("h0", stack("minecraft:" + dye, 3)).stat("minecraft:" + dye);
            c.step(useOn(SX, SY, SZ, 1, 0)).step(useOn(SX, SY, SZ, 1, 0));
            out.add(c);
        }
        c = signCase("sign_dye_back_side", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ - 1.5};
        c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
        c.slot("h0", stack("minecraft:green_dye", 2)).stat("minecraft:green_dye");
        c.step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        c = signCase("sign_dye_creative", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.gameMode = "creative";
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
        c.slot("h0", stack("minecraft:red_dye", 2)).stat("minecraft:red_dye");
        c.step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        c = signCase("sign_dye_adventure", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.gameMode = "adventure";
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
        c.slot("h0", stack("minecraft:red_dye", 2)).stat("minecraft:red_dye");
        c.step(useOn(SX, SY, SZ, 1, 0)).step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        c = signCase("sign_dye_empty_text", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.slot("h0", stack("minecraft:red_dye", 2)).stat("minecraft:red_dye");
        c.step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        c = signCase("sign_dye_offhand", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
        c.slot("offhand", stack("minecraft:red_dye", 2)).stat("minecraft:red_dye");
        c.step(useOn(SX, SY, SZ, 1, 1));
        out.add(c);
        c = signCase("sign_dye_sneaking", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.sneaking = true;
        c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
        c.slot("h0", stack("minecraft:red_dye", 2)).stat("minecraft:red_dye");
        c.step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        for (String item : new String[] {"glow_ink_sac", "ink_sac", "honeycomb"}) {
            c = signCase("sign_item_" + item, "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
            c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
            c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
            c.slot("h0", stack("minecraft:" + item, 3)).stat("minecraft:" + item);
            c.step(useOn(SX, SY, SZ, 1, 0)).step(useOn(SX, SY, SZ, 1, 0));
            out.add(c);
        }
        // Honeycomb works on an empty sign too (its `canApplyToSign` is always true).
        c = signCase("sign_honeycomb_empty", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.slot("h0", stack("minecraft:honeycomb", 3)).stat("minecraft:honeycomb");
        c.step(useOn(SX, SY, SZ, 1, 0)).step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        // Glow ink then ink sac then glow ink on the same side (hotbar slots 0, 1, 2).
        c = signCase("sign_glow_cycle", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
        c.slot("h0", stack("minecraft:glow_ink_sac", 2)).slot("h1", stack("minecraft:ink_sac", 2)).slot("h2", stack("minecraft:honeycomb"));
        c.step(useOn(SX, SY, SZ, 1, 0)).step(op("op", "select", "slot", 1)).step(useOn(SX, SY, SZ, 1, 0))
                .step(op("op", "select", "slot", 2)).step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        // Waxed signs: sound on interaction, no dyes, no editing.
        for (String[] k : new String[][] {{"standing", "oak_sign", "rotation=0", "4 99 4 minecraft:stone"},
                {"hanging", "oak_hanging_sign", "rotation=0,attached=false", "4 101 4 minecraft:stone"}}) {
            c = signCase("sign_waxed_" + k[0], k[1], k[2], k[3]);
            c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
            c.cmd(signState(k[1], k[2], messages("Front"), messages("Back"), true));
            c.slot("h0", stack("minecraft:red_dye", 2)).slot("h1", stack("minecraft:honeycomb", 2));
            c.step(useOn(SX, SY, SZ, 1, 0)).step(op("op", "select", "slot", 1)).step(useOn(SX, SY, SZ, 1, 0))
                    .step(op("op", "select", "slot", 2)).step(useOn(SX, SY, SZ, 1, 0)).step(signUpdate(true, "no", "", "", ""));
            out.add(c);
        }
        // Another player holds the editing lock.
        c = signCase("sign_other_editor", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(signState("oak_sign", "rotation=0", messages("Front"), messages("Back"), false));
        c.slot("h0", stack("minecraft:red_dye", 2)).slot("h1", stack("minecraft:stone", 4));
        c.step(op("op", "lock_sign", "pos", List.of(SX, SY, SZ))).step(useOn(SX, SY, SZ, 1, 0))
                .step(op("op", "select", "slot", 1)).step(useOn(SX, SY, SZ, 1, 0)).step(signUpdate(true, "x", "", "", ""));
        out.add(c);
        // Text that is not plain cannot be edited; click events on a waxed sign.
        c = signCase("sign_not_editable", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(String.format("data merge block %d %d %d {front_text:{messages:[{translate:\"block.minecraft.stone\"},\"\",\"\",\"\"]}}", SX, SY, SZ));
        c.step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        c = signCase("sign_click_event_waxed", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(String.format("data merge block %d %d %d {is_waxed:1b,front_text:{messages:[{text:\"run\",click_event:{action:\"run_command\",command:\"say hi\"}},\"\",\"\",\"\"]}}", SX, SY, SZ));
        c.step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        c = signCase("sign_click_event_open", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.cmd(String.format("data merge block %d %d %d {front_text:{messages:[{text:\"run\",click_event:{action:\"run_command\",command:\"say hi\"}},\"\",\"\",\"\"]}}", SX, SY, SZ));
        c.step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        // Adventure players cannot open the editor; spectators neither.
        c = signCase("sign_adventure_open", "oak_sign", "rotation=0", "4 99 4 minecraft:stone");
        c.gameMode = "adventure";
        c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
        c.step(useOn(SX, SY, SZ, 1, 0));
        out.add(c);
        // Placing sign items: the editor opens for the placer.
        String[][] placing = {
            {"sign_place_standing", "minecraft:oak_sign", "4 99 4", "1"},
            {"sign_place_wall", "minecraft:oak_sign", "4 100 5", "2"},
            {"sign_place_hanging", "minecraft:oak_hanging_sign", "4 101 4", "0"},
            {"sign_place_hanging_side", "minecraft:oak_hanging_sign", "4 100 5", "2"},
            {"sign_place_bamboo", "minecraft:bamboo_sign", "4 99 4", "1"},
            {"sign_place_cherry_hanging", "minecraft:cherry_hanging_sign", "4 101 4", "0"},
        };
        for (String[] p : placing) {
            c = new Case(p[0]);
            String[] xyz = p[2].split(" ");
            c.cmd("setblock " + p[2] + " minecraft:stone");
            c.pos = new double[] {SX + 0.5, 100.0, SZ + 2.5};
            c.slot("h0", stack(p[1], 2)).stat(p[1]);
            c.watch(SX, SY, SZ).watch(Integer.parseInt(xyz[0]), Integer.parseInt(xyz[1]) + (p[3].equals("1") ? 1 : 0), Integer.parseInt(xyz[2]));
            c.step(useOn(Integer.parseInt(xyz[0]), Integer.parseInt(xyz[1]), Integer.parseInt(xyz[2]), Integer.parseInt(p[3]), 0));
            c.step(signUpdate(true, "placed", "", "", ""));
            out.add(c);
        }
    }

    // ---------------------------------------------------------------- book scenarios

    static ItemStack writable(String... pages) {
        ItemStack s = stack("minecraft:writable_book");
        List<net.minecraft.server.network.Filterable<String>> list = new ArrayList<>();
        for (String p : pages) list.add(net.minecraft.server.network.Filterable.passThrough(p));
        s.set(DataComponents.WRITABLE_BOOK_CONTENT, new net.minecraft.world.item.component.WritableBookContent(list));
        return s;
    }

    static ItemStack written(String title, String author, int generation, String... pages) {
        ItemStack s = stack("minecraft:written_book");
        List<net.minecraft.server.network.Filterable<net.minecraft.network.chat.Component>> list = new ArrayList<>();
        for (String p : pages) list.add(net.minecraft.server.network.Filterable.passThrough(net.minecraft.network.chat.Component.literal(p)));
        s.set(DataComponents.WRITTEN_BOOK_CONTENT, new net.minecraft.world.item.component.WrittenBookContent(
                net.minecraft.server.network.Filterable.passThrough(title), author, generation, list, true));
        return s;
    }

    static Map<String, Object> editBook(int slot, String title, String... pages) {
        return op("op", "edit_book", "slot", slot, "pages", List.of(pages), "title", title);
    }

    static void books(List<Case> out) {
        Case c;
        c = new Case("book_edit_pages");
        c.slot("h0", stack("minecraft:writable_book")).step(editBook(0, null, "Page 1", "Page 2"));
        out.add(c);
        c = new Case("book_edit_replace");
        c.slot("h0", writable("old", "older")).step(editBook(0, null, "new1", "new2", "new3"));
        out.add(c);
        c = new Case("book_edit_empty");
        c.slot("h0", writable("old")).step(editBook(0, null));
        out.add(c);
        c = new Case("book_sign");
        c.slot("h0", writable("a", "b")).step(editBook(0, "My Book", "a", "b"));
        out.add(c);
        c = new Case("book_sign_formatting");
        c.slot("h0", writable("x")).step(editBook(0, "T§1", "§cred", "x\ny", "café 中"));
        out.add(c);
        c = new Case("book_sign_empty_title");
        c.slot("h0", writable("x")).step(editBook(0, "", "only page"));
        out.add(c);
        c = new Case("book_sign_no_pages");
        c.slot("h0", writable("x")).step(editBook(0, "Blank"));
        out.add(c);
        c = new Case("book_sign_creative");
        c.gameMode = "creative";
        c.slot("h0", writable("x")).step(editBook(0, "Creative", "p"));
        out.add(c);
        c = new Case("book_edit_offhand");
        c.slot("offhand", writable("x")).step(editBook(40, null, "off"));
        c.step(editBook(40, "Signed", "off", "hand"));
        out.add(c);
        c = new Case("book_sign_offhand");
        c.slot("offhand", writable("x")).step(editBook(40, "Signed", "off", "hand"));
        out.add(c);
        c = new Case("book_edit_other_hotbar_slot");
        c.slot("h5", writable("x")).step(editBook(5, null, "slot five"));
        out.add(c);
        c = new Case("book_edit_not_hotbar");
        c.slot("m9", writable("x")).step(editBook(9, null, "no"));
        out.add(c);
        c = new Case("book_edit_wrong_item");
        c.slot("h0", stack("minecraft:stone")).step(editBook(0, null, "no")).step(editBook(0, "T", "no"));
        out.add(c);
        c = new Case("book_edit_already_signed");
        c.slot("h0", written("T", "A", 0, "p")).step(editBook(0, null, "no")).step(editBook(0, "T2", "no"));
        out.add(c);
        c = new Case("book_edit_empty_slot");
        c.step(editBook(0, "T", "no"));
        out.add(c);
        c = new Case("book_edit_bad_slot");
        c.slot("h0", writable("x")).step(editBook(12, "T", "no")).step(editBook(-1, "T", "no")).step(editBook(41, "T", "no"));
        out.add(c);
        c = new Case("book_edit_damaged_components");
        ItemStack named = writable("x");
        named.set(DataComponents.CUSTOM_NAME, net.minecraft.network.chat.Component.literal("My Notes"));
        c.slot("h0", named).step(editBook(0, "Signed", "page"));
        out.add(c);
        c = new Case("book_use_writable");
        c.slot("h0", writable("x")).stat("minecraft:writable_book").step(op("op", "use", "hand", 0));
        out.add(c);
        c = new Case("book_use_writable_offhand");
        c.slot("offhand", writable("x")).stat("minecraft:writable_book").step(op("op", "use", "hand", 1));
        out.add(c);
        c = new Case("book_use_written");
        c.slot("h0", written("T", "A", 0, "p")).stat("minecraft:written_book").step(op("op", "use", "hand", 0));
        out.add(c);
        c = new Case("book_use_written_spectator");
        c.gameMode = "spectator";
        c.slot("h0", written("T", "A", 0, "p")).stat("minecraft:written_book").step(op("op", "use", "hand", 0));
        out.add(c);
    }

    // ---------------------------------------------------------------- pick scenarios

    static Map<String, Object> pickBlock(int x, int y, int z, boolean include) {
        return op("op", "pick_block", "pos", List.of(x, y, z), "include", include);
    }

    // ---------------------------------------------------------------- wp52: middle click on an entity (`handlePickItemFromEntity`)

    static void pickEntities52(List<Case> out) {
        Case c;
        String[][] things = {
                {"armor_stand", "summon minecraft:armor_stand 4 100 1 {NoGravity:1b}"},
                {"item_frame_empty", "setblock 5 100 1 minecraft:stone|summon minecraft:item_frame 4 100 1 {Facing:5b}"},
                {"item_frame_item", "setblock 5 100 1 minecraft:stone|summon minecraft:item_frame 4 100 1 {Facing:5b,Item:{id:\"minecraft:diamond\",count:3}}"},
                {"glow_frame", "setblock 5 100 1 minecraft:stone|summon minecraft:glow_item_frame 4 100 1 {Facing:5b}"},
                {"named_frame", "setblock 5 100 1 minecraft:stone|summon minecraft:item_frame 4 100 1 {Facing:5b,CustomName:'\"Fr\"'}"},
                {"painting", "setblock 5 100 1 minecraft:stone|summon minecraft:painting 4 100 1 {facing:1b,variant:\"minecraft:kebab\"}"},
                {"cushion_red", "setblock 4 99 1 minecraft:stone|summon minecraft:cushion 4.5 100 1.5 {Color:\"red\"}"},
                {"cushion_white", "setblock 4 99 1 minecraft:stone|summon minecraft:cushion 4.5 100 1.5 {}"},
                {"mannequin", "summon minecraft:mannequin 4.5 100 1.5 {}"},
                {"end_crystal", "summon minecraft:end_crystal 4.5 100 1.5 {}"},
                {"minecart", "summon minecraft:minecart 4.5 100 1.5 {NoGravity:1b}"},
                {"chest_minecart", "summon minecraft:chest_minecart 4.5 100 1.5 {NoGravity:1b}"},
                {"hopper_minecart", "summon minecraft:hopper_minecart 4.5 100 1.5 {NoGravity:1b}"},
                {"furnace_minecart", "summon minecraft:furnace_minecart 4.5 100 1.5 {NoGravity:1b}"},
                {"tnt_minecart", "summon minecraft:tnt_minecart 4.5 100 1.5 {NoGravity:1b}"},
                {"command_minecart", "summon minecraft:command_block_minecart 4.5 100 1.5 {NoGravity:1b}"},
                {"spawner_minecart", "summon minecraft:spawner_minecart 4.5 100 1.5 {NoGravity:1b}"},
                {"oak_boat", "summon minecraft:oak_boat 4.5 100 1.5 {NoGravity:1b}"},
                {"cherry_chest_boat", "summon minecraft:cherry_chest_boat 4.5 100 1.5 {NoGravity:1b}"},
                {"bamboo_raft", "summon minecraft:bamboo_raft 4.5 100 1.5 {NoGravity:1b}"},
                {"item", "summon minecraft:item 4.5 100 1.5 {Item:{id:\"minecraft:stone\",count:1},NoGravity:1b}"},
                {"marker", "summon minecraft:marker 4.5 100 1.5 {}"},
                {"snowball", "summon minecraft:snowball 4.5 100 1.5 {NoGravity:1b}"},
                {"tnt", "summon minecraft:tnt 4.5 100 1.5 {NoGravity:1b,fuse:80}"},
                {"pig", "summon minecraft:pig 4.5 100 1.5 {NoAI:1b}"},
                {"cow", "summon minecraft:cow 4.5 100 1.5 {NoAI:1b}"},
                {"wolf", "summon minecraft:wolf 4.5 100 1.5 {NoAI:1b}"},
                {"zombie", "summon minecraft:zombie 4.5 100 1.5 {NoAI:1b}"},
                {"villager", "summon minecraft:villager 4.5 100 1.5 {NoAI:1b}"},
                {"iron_golem", "summon minecraft:iron_golem 4.5 100 1.5 {NoAI:1b}"},
                {"creeper", "summon minecraft:creeper 4.5 100 1.5 {NoAI:1b}"},
                {"slime", "summon minecraft:slime 4.5 100 1.5 {NoAI:1b,Size:1}"},
                {"bat", "summon minecraft:bat 4.5 100 1.5 {NoAI:1b}"},
                {"giant", "summon minecraft:giant 4.5 100 1.5 {NoAI:1b}"},
                {"ender_dragon", "summon minecraft:ender_dragon 4.5 101 1.5 {NoAI:1b}"},
                {"wither", "summon minecraft:wither 4.5 101 1.5 {NoAI:1b}"},
                {"zombie_horse", "summon minecraft:zombie_horse 4.5 100 1.5 {NoAI:1b}"},
                {"copper_golem", "summon minecraft:copper_golem 4.5 100 1.5 {NoAI:1b}"},
        };
        for (String[] t : things) {
            c = new Case("pickent52_" + t[0]);
            c.gameMode = "creative";
            for (String cmd : t[1].split("\\|")) c.cmd(cmd);
            c.step(op("op", "pick_entity", "pos", List.of(4.5, 100.0, 1.5), "include", false));
            out.add(c);
        }
        // A survival player gets what he has: from the main inventory into the hotbar, or nothing.
        for (String[] t : new String[][] {{"pig", "minecraft:pig_spawn_egg", "summon minecraft:pig 4.5 100 1.5 {NoAI:1b}"}, {"minecart", "minecraft:minecart", "summon minecraft:minecart 4.5 100 1.5 {NoGravity:1b}"}}) {
            c = new Case("pickent52_survival_has_" + t[0]);
            c.gameMode = "survival";
            c.cmd(t[2]).slot("m20", stack(t[1], 4)).slot("h0", stack("minecraft:dirt"));
            c.step(op("op", "pick_entity", "pos", List.of(4.5, 100.0, 1.5), "include", false));
            out.add(c);
            c = new Case("pickent52_survival_hasnt_" + t[0]);
            c.gameMode = "survival";
            c.cmd(t[2]).slot("h0", stack("minecraft:dirt"));
            c.step(op("op", "pick_entity", "pos", List.of(4.5, 100.0, 1.5), "include", false));
            out.add(c);
        }
        // Out of reach: nothing.
        c = new Case("pickent52_far");
        c.gameMode = "creative";
        c.cmd("summon minecraft:pig 40.5 100 1.5 {NoAI:1b}");
        c.step(op("op", "pick_entity", "pos", List.of(40.5, 100.0, 1.5), "include", false));
        out.add(c);
        // An item frame holding an item: the item (a second pick finds it in the hotbar).
        c = new Case("pickent52_frame_twice");
        c.gameMode = "creative";
        c.cmd("setblock 5 100 1 minecraft:stone").cmd("summon minecraft:item_frame 4 100 1 {Facing:5b,Item:{id:\"minecraft:stick\",count:1}}");
        c.step(op("op", "pick_entity", "pos", List.of(4.5, 100.0, 1.5), "include", false)).step(op("op", "pick_entity", "pos", List.of(4.5, 100.0, 1.5), "include", true));
        out.add(c);
    }

    static void picks(List<Case> out) {
        Case c;
        String[] modes = {"survival", "creative"};
        for (String gm : modes) {
            // Nothing like it in the inventory.
            c = new Case("pick_" + gm + "_none");
            c.gameMode = gm;
            c.cmd("setblock 4 100 4 minecraft:stone").step(pickBlock(4, 100, 4, false));
            out.add(c);
            // In the hotbar: selected.
            c = new Case("pick_" + gm + "_hotbar");
            c.gameMode = gm;
            c.cmd("setblock 4 100 4 minecraft:stone").slot("h3", stack("minecraft:stone", 5)).slot("h0", stack("minecraft:dirt"))
                    .step(pickBlock(4, 100, 4, false));
            out.add(c);
            // In the main inventory: swapped into a free hotbar slot.
            c = new Case("pick_" + gm + "_main");
            c.gameMode = gm;
            c.cmd("setblock 4 100 4 minecraft:stone").slot("m20", stack("minecraft:stone", 7)).slot("h0", stack("minecraft:dirt"))
                    .step(pickBlock(4, 100, 4, false));
            out.add(c);
            c = new Case("pick_" + gm + "_main_hotbar_full");
            c.gameMode = gm;
            c.cmd("setblock 4 100 4 minecraft:stone").slot("m20", stack("minecraft:stone", 7));
            for (int i = 0; i < 9; i++) c.slot("h" + i, i % 2 == 0 ? stack("minecraft:dirt", 3) : enchanted("minecraft:diamond_sword", "minecraft:sharpness", 1));
            c.selected = 2;
            c.step(pickBlock(4, 100, 4, false));
            out.add(c);
            c = new Case("pick_" + gm + "_main_hotbar_all_enchanted");
            c.gameMode = gm;
            c.cmd("setblock 4 100 4 minecraft:stone").slot("m20", stack("minecraft:stone", 7));
            for (int i = 0; i < 9; i++) c.slot("h" + i, enchanted("minecraft:diamond_sword", "minecraft:sharpness", 1 + i));
            c.selected = 4;
            c.step(pickBlock(4, 100, 4, false));
            out.add(c);
            // Different components do not match.
            c = new Case("pick_" + gm + "_named_not_matching");
            c.gameMode = gm;
            ItemStack named = stack("minecraft:stone");
            named.set(DataComponents.CUSTOM_NAME, net.minecraft.network.chat.Component.literal("Special"));
            c.cmd("setblock 4 100 4 minecraft:stone").slot("h1", named);
            c.step(pickBlock(4, 100, 4, false));
            out.add(c);
            // Out of reach, air, unloaded.
            c = new Case("pick_" + gm + "_far");
            c.gameMode = gm;
            c.cmd("setblock 4 100 4 minecraft:stone").slot("h1", stack("minecraft:stone"));
            c.pos = new double[] {4.5, 100.0, 12.5};
            c.step(pickBlock(4, 100, 4, false));
            out.add(c);
            c = new Case("pick_" + gm + "_air");
            c.gameMode = gm;
            c.slot("h1", stack("minecraft:stone"));
            c.step(pickBlock(4, 100, 4, false));
            out.add(c);
        }
        // Blocks and what they give.
        String[] blocks = {
            "minecraft:oak_sign[rotation=3]", "minecraft:oak_wall_sign[facing=north]", "minecraft:oak_hanging_sign[rotation=3,attached=false]",
            "minecraft:oak_door[half=upper]", "minecraft:red_bed[part=head]", "minecraft:farmland", "minecraft:grass_block",
            "minecraft:wheat[age=7]", "minecraft:potatoes[age=7]", "minecraft:carrots[age=2]", "minecraft:beetroots[age=3]",
            "minecraft:melon_stem[age=7]", "minecraft:pumpkin_stem[age=3]", "minecraft:attached_melon_stem[facing=north]", 
            "minecraft:wall_torch[facing=east]", "minecraft:redstone_wall_torch[facing=east]", "minecraft:water[level=0]", "minecraft:lava[level=0]",
            "minecraft:fire", "minecraft:nether_portal[axis=x]", "minecraft:tall_grass[half=lower]", "minecraft:cake[bites=2]", "minecraft:candle_cake",
            "minecraft:potted_poppy", "minecraft:sweet_berry_bush[age=3]",
            "minecraft:kelp_plant", "minecraft:bamboo_sapling", "minecraft:tripwire", "minecraft:frogspawn", "minecraft:redstone_ore[lit=true]",
            "minecraft:piston_head[facing=up,type=normal]", "minecraft:moving_piston[facing=up,type=normal]", "minecraft:oak_wall_hanging_sign[facing=east]",
            "minecraft:white_wall_banner[facing=north]", "minecraft:player_wall_head[facing=north]", "minecraft:torchflower_crop[age=1]",
            "minecraft:pitcher_crop[age=2,half=lower]", "minecraft:bubble_column[drag=true]", "minecraft:infested_stone", "minecraft:spawner",
            "minecraft:chest[facing=north,type=single]", "minecraft:lit_furnace", "minecraft:furnace[lit=true,facing=north]",
            "minecraft:oak_leaves", "minecraft:snow[layers=3]", "minecraft:cocoa[age=2,facing=north]", "minecraft:nether_wart[age=3]", "minecraft:end_portal",
            "minecraft:vault", "minecraft:trial_spawner", "minecraft:copper_golem_statue[facing=north,copper_golem_pose=standing]",
        };
        for (int i = 0; i < blocks.length; i++) {
            c = new Case("pick_block_" + i);
            c.gameMode = "creative";
            c.cmd("setblock 4 100 4 " + blocks[i]);
            c.step(pickBlock(4, 100, 4, false));
            out.add(c);
        }
        // Blocks whose block entity shapes the item even without data (kiln leaves the banners and decorated pots, whose picked
        // item carries the block entity's patterns and sherds, to the table of items: pick_table.jsonl).
        String[][] shaped = {
            {"beehive", "minecraft:beehive[facing=north]{bees:[{entity_data:{id:\"minecraft:bee\"},min_ticks_in_hive:100,ticks_in_hive:5}]}"},
            {"skull", "minecraft:player_head[rotation=3]{profile:{name:\"Notch\"}}"},
        };
        for (String[] d : shaped) {
            c = new Case("pick_shaped_" + d[0]);
            c.gameMode = "creative";
            c.cmd("setblock 4 100 4 " + d[1]);
            c.watch(4, 100, 4);
            c.step(pickBlock(4, 100, 4, false));
            out.add(c);
        }
        // With block data: only creative players get it, and kiln does not carry a block entity's data into
        // the picked item (block_entity_data and the block entity's components), so the blocks that have one are
        // recorded for survival players (who get none) only.
        String[][] data = {
            {"minecraft:chest[facing=north]{Items:[{Slot:0b,id:\"minecraft:diamond\",count:3}]}", "chest"},
            {"minecraft:oak_sign[rotation=0]{front_text:{messages:[\"hi\",\"\",\"\",\"\"]}}", "sign"},
            {"minecraft:furnace[facing=north]{CustomName:\"Hot\"}", "furnace"},
            {"minecraft:player_head[rotation=0]", "head"},
            {"minecraft:white_banner[rotation=0]{patterns:[{pattern:\"minecraft:border\",color:\"red\"}]}", "banner"},
            {"minecraft:jukebox[has_record=false]", "jukebox"},
            {"minecraft:repeater[delay=3,facing=north]", "repeater"},
            {"minecraft:oak_stairs[facing=east,half=top]", "stairs"},
        };
        for (String[] d : data) {
            boolean hasBlockEntity = !d[1].equals("repeater") && !d[1].equals("stairs");
            for (String gm : modes) {
                if (hasBlockEntity && gm.equals("creative")) continue;
                c = new Case("pick_data_" + d[1] + "_" + gm);
                c.gameMode = gm;
                c.cmd("setblock 4 100 4 " + d[0]);
                c.watch(4, 100, 4);
                c.step(pickBlock(4, 100, 4, true));
                c.step(pickBlock(4, 100, 4, false));
                out.add(c);
            }
        }
    }

    // ---------------------------------------------------------------- driving

    static void command(String cmd) {
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), cmd);
    }

    static ServerPlayer mockPlayer(String name) {
        CommonListenerCookie cookie = CommonListenerCookie.createInitial(
                new GameProfile(UUID.nameUUIDFromBytes(name.getBytes()), name), false);
        ServerPlayer p = new ServerPlayer(server, server.overworld(), cookie.gameProfile(), cookie.clientInformation());
        Connection connection = new Connection(PacketFlow.SERVERBOUND);
        new EmbeddedChannel(connection);
        server.getPlayerList().placeNewPlayer(connection, p, cookie);
        return p;
    }

    static List<Object> drain(ServerPlayer p) throws Exception {
        List<Object> out = new ArrayList<>();
        Object connection = get(p.connection, "connection");
        EmbeddedChannel ch = (EmbeddedChannel) get(connection, "channel");
        Object o;
        while ((o = ch.readOutbound()) != null) {
            if (o instanceof ClientboundBundlePacket b) {
                for (Packet<?> q : b.subPackets()) out.add(q);
            } else {
                out.add(o);
            }
        }
        return out;
    }

    /** The game time a case begins at (`lastHit` of an armor stand is compared with it). */
    static final long START_TIME = 100;

    static void setup(ServerPlayer p, Case c) throws Exception {
        ((net.minecraft.world.level.storage.ServerLevelData) server.overworld().getLevelData()).setGameTime(START_TIME);
        p.setGameMode(GameType.byName(c.gameMode));
        call(p.connection, "markClientLoaded");
        if (c.op) command("op " + p.getGameProfile().name());
        p.snapTo(c.pos[0], c.pos[1], c.pos[2], c.yaw, c.pitch);
        if (c.fullTicks) settleRotation(p);
        p.setDeltaMovement(Vec3.ZERO);
        p.setOnGround(true);
        p.fallDistance = 0.0;
        if (c.watchMove) p.connection.resetPosition();
        if (c.sneaking) {
            p.setShiftKeyDown(true);
            p.setPose(Pose.CROUCHING);
        }
        p.getInventory().clearContent();
        p.getInventory().setSelectedSlot(c.selected);
        for (var e : c.slots.entrySet()) {
            String k = e.getKey();
            ItemStack s = e.getValue().copy();
            switch (k) {
                case "feet" -> p.setItemSlot(EquipmentSlot.FEET, s);
                case "legs" -> p.setItemSlot(EquipmentSlot.LEGS, s);
                case "chest" -> p.setItemSlot(EquipmentSlot.CHEST, s);
                case "head" -> p.setItemSlot(EquipmentSlot.HEAD, s);
                case "offhand" -> p.setItemSlot(EquipmentSlot.OFFHAND, s);
                default -> p.getInventory().setItem(Integer.parseInt(k.substring(1)), s);
            }
        }
        call(p, "detectEquipmentUpdates");
        p.getFoodData().setFoodLevel(c.food);
        p.tickCount = 0;
        // One tick, like a connected player's first (it clears `firstTick`).
        p.commonTick();
        p.tick();
        call(p.connection, "tickPlayer");
        // The chunks around the player count as sent, so changed blocks are broadcast to it.
        server.overworld().getChunkSource().chunkMap.move(p);
        ((it.unimi.dsi.fastutil.longs.LongSet) get(p.connection.chunkSender, "pendingChunks")).clear();
        // The mock client never confirms the teleport the join sent: accept it.
        field(p.connection.getClass(), "awaitingPositionFromClient").set(p.connection, null);
    }

    static final String[] SLOT_KEYS = {"feet", "legs", "chest", "head", "offhand"};
    static final EquipmentSlot[] SLOTS = {EquipmentSlot.FEET, EquipmentSlot.LEGS, EquipmentSlot.CHEST, EquipmentSlot.HEAD, EquipmentSlot.OFFHAND};

    static Map<String, Object> inventory(ServerPlayer p) {
        Map<String, Object> m = new LinkedHashMap<>();
        List<String> items = new ArrayList<>();
        for (int i = 0; i < 36; i++) items.add(hex(p.getInventory().getItem(i)));
        m.put("items", items);
        for (int i = 0; i < SLOTS.length; i++) m.put(SLOT_KEYS[i], hex(p.getItemBySlot(SLOTS[i])));
        m.put("selected", p.getInventory().getSelectedSlot());
        return m;
    }

    static String soundName(ClientboundSoundPacket s) {
        return s.getSound().unwrapKey().map(k -> k.identifier().toString()).orElse("?");
    }

    /// wp49: the menu packets (open, contents, slots, data, close) are recorded for cases that watch menus.
    static boolean recordMenus;
    /// wp49: map packets (and the entity sounds around them) are recorded for cases that watch maps.
    static boolean recordMaps;

    static int teleports;

    static List<Object> packets(ServerPlayer p) throws Exception {
        List<Object> out = new ArrayList<>();
        for (Object o : drain(p)) {
            if (o instanceof net.minecraft.network.protocol.game.ClientboundPlayerPositionPacket) teleports++;
            if (o instanceof ClientboundSoundPacket s) {
                if (recordNoPitch && soundName(s).startsWith("minecraft:block.")) continue;
                out.add(op("t", "sound", "name", soundName(s), "source", s.getSource().getName(),
                        "pos", new double[] {s.getX(), s.getY(), s.getZ()}, "volume", s.getVolume(), "pitch", recordNoPitch ? 0.0f : s.getPitch()));
            } else if (o instanceof ClientboundOpenSignEditorPacket e) {
                out.add(op("t", "open_sign_editor", "pos", List.of(e.pos().getX(), e.pos().getY(), e.pos().getZ()),
                        "front", e.slot() == net.minecraft.world.level.block.entity.SignTextSlot.FRONT));
            } else if (o instanceof ClientboundBlockUpdatePacket u) {
                out.add(op("t", "block_update", "pos", List.of(u.getPos().getX(), u.getPos().getY(), u.getPos().getZ()),
                        "state", net.minecraft.world.level.block.Block.getId(u.getBlockState())));
            } else if (o instanceof ClientboundBlockEntityDataPacket d) {
                out.add(op("t", "block_entity_data", "pos", List.of(d.getPos().getX(), d.getPos().getY(), d.getPos().getZ()),
                        "type", BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(d.getType()).toString(), "tag", nbtHex(d.getTag())));
            } else if (o instanceof ClientboundOpenBookPacket b) {
                out.add(op("t", "open_book", "hand", b.hand() == InteractionHand.MAIN_HAND ? 0 : 1));
            } else if (o instanceof ClientboundSetHeldSlotPacket h) {
                out.add(op("t", "set_held_slot", "slot", h.slot()));
            } else if (o instanceof ClientboundSystemChatPacket a) {
                out.add(op("t", "system_chat", "overlay", a.overlay(), "text", nbtHex(net.minecraft.network.chat.ComponentSerialization.CODEC
                        .encodeStart(server.registryAccess().createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE), a.content()).getOrThrow())));
            } else if (o instanceof ClientboundLevelEventPacket l) {
                out.add(op("t", "level_event", "event", l.getType(), "pos", List.of(l.getPos().getX(), l.getPos().getY(), l.getPos().getZ()),
                        "data", l.getData(), "global", l.isGlobalEvent()));
            } else if (recordMaps && o instanceof ClientboundSoundEntityPacket s) {
                out.add(op("t", "sound_entity", "name", s.getSound().unwrapKey().map(k -> k.identifier().toString()).orElse("?"),
                        "source", s.getSource().getName(), "volume", s.getVolume(), "pitch", s.getPitch()));
            } else if (recordMaps && o instanceof ClientboundMapItemDataPacket m) {
                out.add(op("t", "map", "id", m.mapId().id(), "scale", (int) m.scale(), "locked", m.locked(),
                        "decos", m.decorations().map(l -> (Object) decoRows(l)).orElse(null),
                        "patch", m.colorPatch().map(pp -> (Object) List.of(pp.startX(), pp.startY(), pp.width(), pp.height(), ByteBufUtil.hexDump(pp.mapColors()))).orElse(null)));
            } else if (recordMenus && o instanceof ClientboundOpenScreenPacket m) {
                out.add(op("t", "open_screen", "id", m.getContainerId(), "type", BuiltInRegistries.MENU.getId(m.getType()), "title", nbtHex(net.minecraft.network.chat.ComponentSerialization.CODEC
                        .encodeStart(server.registryAccess().createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE), m.getTitle()).getOrThrow())));
            } else if (recordMenus && o instanceof ClientboundContainerSetContentPacket m) {
                List<Object> items = new ArrayList<>();
                for (ItemStack st : m.items()) items.add(hex(st));
                out.add(op("t", "set_content", "id", m.containerId(), "items", items, "carried", hex(m.carriedItem())));
            } else if (recordMenus && o instanceof ClientboundContainerSetSlotPacket m) {
                out.add(op("t", "set_slot", "id", m.getContainerId(), "slot", m.getSlot(), "item", hex(m.getItem())));
            } else if (recordMenus && o instanceof ClientboundContainerSetDataPacket m) {
                out.add(op("t", "set_data", "id", m.getContainerId(), "index", m.getId(), "value", m.getValue()));
            } else if (recordMenus && o instanceof ClientboundContainerClosePacket m) {
                out.add(op("t", "container_close", "id", m.getContainerId()));
            }
        }
        return out;
    }

    /** wp50: the structure templates in the manager (the ones asked for, without making any): id to the saved NBT as hex. */
    static Map<String, Object> templatesOf(Case c) throws Exception {
        Map<String, Object> out = new LinkedHashMap<>();
        Object repo = get(server.overworld().getStructureTemplateManager(), "structureRepository");
        for (String id : c.templates) {
            Object entry = ((Map<?, ?>) repo).get(Identifier.parse(id));
            if (entry instanceof Optional<?> o && o.isPresent()) {
                var t = (net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate) o.get();
                out.put(id, nbtHex(t.save(new net.minecraft.nbt.CompoundTag())));
            } else {
                out.put(id, null);
            }
        }
        return out;
    }

    static List<Object> blocks(Case c) throws Exception {
        ServerLevel level = server.overworld();
        List<Object> out = new ArrayList<>();
        for (int[] w : c.watch) {
            BlockPos pos = new BlockPos(w[0], w[1], w[2]);
            BlockState st = level.getBlockState(pos);
            BlockEntity be = level.getBlockEntity(pos);
            out.add(op("pos", List.of(w[0], w[1], w[2]), "state", net.minecraft.world.level.block.Block.getId(st),
                    "be", be == null ? null : nbtHex(be.saveWithFullMetadata(server.registryAccess()))));
        }
        return out;
    }

    static net.minecraft.world.entity.Entity nearestHanging(double x, double y, double z) {
        ServerLevel level = server.overworld();
        net.minecraft.world.entity.Entity best = null;
        double bd = 1e18;
        for (var e : level.getEntities((net.minecraft.world.entity.Entity) null, new AABB(x - 8, y - 8, z - 8, x + 8, y + 8, z + 8),
                en -> en instanceof net.minecraft.world.entity.decoration.HangingEntity || en instanceof net.minecraft.world.entity.decoration.ArmorStand || en instanceof net.minecraft.world.entity.decoration.Cushion || en instanceof net.minecraft.world.entity.decoration.Mannequin)) {
            double d = e.position().distanceToSqr(x, y, z);
            if (d < bd) { bd = d; best = e; }
        }
        return best;
    }

    /** Player.attackStrengthTicker moves on (the mock player does not tick). */
    static void addAttackTicks(ServerPlayer p, int n) {
        try {
            java.lang.reflect.Field f = net.minecraft.world.entity.LivingEntity.class.getDeclaredField("attackStrengthTicker");
            f.setAccessible(true);
            f.setInt(p, f.getInt(p) + n);
        } catch (ReflectiveOperationException x) {
            throw new IllegalStateException(x);
        }
    }

    static int invulnerableTime(net.minecraft.world.entity.Entity e) {
        try {
            java.lang.reflect.Field f = net.minecraft.world.entity.Entity.class.getDeclaredField("invulnerableTime");
            f.setAccessible(true);
            return f.getInt(e);
        } catch (ReflectiveOperationException x) {
            throw new IllegalStateException(x);
        }
    }

    /** The hanging entities in the scenario's area, sorted: [type, x, y, z, facing, item, rotation, painting area]. */
    static List<Object> hangings() {
        ServerLevel level = server.overworld();
        List<Object[]> rows = new ArrayList<>();
        for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.decoration.HangingEntity.class, new AABB(-16, 90, -16, 32, 120, 32))) {
            String type = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString();
            String item = null;
            int rot = 0, area = 0;
            if (e instanceof net.minecraft.world.entity.decoration.ItemFrame f) {
                item = f.getItem().isEmpty() ? null : hex(f.getItem());
                rot = f.getRotation();
            }
            if (e instanceof net.minecraft.world.entity.decoration.painting.Painting pt) {
                area = pt.getVariant().value().area();
                if (System.getenv("INTERACT_DEBUG") != null) System.out.println("DEBUG painting " + pt.getVariant().getRegisteredName() + " " + pt.getVariant().value().width() + "x" + pt.getVariant().value().height() + " bb " + pt.getBoundingBox());
            }
            rows.add(new Object[] {type, e.getX(), e.getY(), e.getZ(), e.getDirection().get3DDataValue(), item, rot, area});
        }
        // wp50: mannequins: [type, x, y, z, 0, "health,pose,hurtTime,deathTime,height,invulnerableTime", 0, 0].
        for (var mq : level.getEntitiesOfClass(net.minecraft.world.entity.decoration.Mannequin.class, new AABB(-16, 60, -16, 32, 330, 32))) {
            String state = String.format(java.util.Locale.ROOT, "%.4f,%s,%d,%d,%.4f,%d", mq.getHealth(), mq.getPose().getSerializedName(), mq.hurtTime, mq.deathTime, mq.getBbHeight(), invulnerableTime(mq));
            rows.add(new Object[] {BuiltInRegistries.ENTITY_TYPE.getKey(mq.getType()).toString(), mq.getX(), mq.getY(), mq.getZ(), 0, state, 0, 0});
        }
        // wp50: cushions: [type, x, y, z, 0, color, riders, the riders' seat height in ten thousandths].
        for (var cu : level.getEntitiesOfClass(net.minecraft.world.entity.decoration.Cushion.class, new AABB(-16, 60, -16, 32, 330, 32))) {
            long seat = 0;
            if (!cu.getPassengers().isEmpty()) {
                var rider = cu.getPassengers().get(0);
                seat = (long) Math.floor(cu.getPassengerRidingPosition(rider).subtract(rider.getVehicleAttachmentPoint(cu)).y * 1.0e4);
            }
            rows.add(new Object[] {BuiltInRegistries.ENTITY_TYPE.getKey(cu.getType()).toString(), cu.getX(), cu.getY(), cu.getZ(), 0, cu.getColor().getName(), cu.getPassengers().size(), seat});
        }
        rows.sort(Comparator.comparing((Object[] r) -> (String) r[0]).thenComparingDouble(r -> (Double) r[1]).thenComparingDouble(r -> (Double) r[2]).thenComparingDouble(r -> (Double) r[3]));
        List<Object> out = new ArrayList<>();
        for (Object[] r : rows) out.add(java.util.Arrays.asList(r));
        return out;
    }

    /** The armor stands in the scenario's area, sorted by position: [x, y, z, saved data (hex)]. */
    static List<Object> stands() throws Exception {
        ServerLevel level = server.overworld();
        List<Object[]> rows = new ArrayList<>();
        for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.decoration.ArmorStand.class, new AABB(-16, 90, -16, 32, 120, 32))) {
            var out = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess());
            e.saveWithoutId(out);
            net.minecraft.nbt.CompoundTag tag = out.buildResult();
            tag.remove("UUID");
            rows.add(new Object[] {e.getX(), e.getY(), e.getZ(), nbtHex(tag)});
        }
        rows.sort(Comparator.comparingDouble((Object[] r) -> (Double) r[0]).thenComparingDouble(r -> (Double) r[1]).thenComparingDouble(r -> (Double) r[2]));
        List<Object> out = new ArrayList<>();
        for (Object[] r : rows) out.add(java.util.Arrays.asList(r));
        return out;
    }

    /** wp49: the living mobs around, sorted: [type, x, y, z]. */
    static List<Object> mobRows() {
        List<Object[]> rows = new ArrayList<>();
        for (var e : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.Mob.class, new AABB(-32, 60, -32, 64, 140, 64))) {
            if (!e.isAlive()) continue;
            rows.add(new Object[] {BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString(), e.getX(), e.getY(), e.getZ()});
        }
        rows.sort(Comparator.comparing((Object[] r) -> (String) r[0]).thenComparingDouble(r -> (Double) r[1]).thenComparingDouble(r -> (Double) r[2]).thenComparingDouble(r -> (Double) r[3]));
        List<Object> out = new ArrayList<>();
        for (Object[] r : rows) out.add(java.util.Arrays.asList(r));
        return out;
    }

    static boolean mobCase;
    static boolean recordNoPitch;

    static List<Object> itemEntities() {
        ServerLevel level = server.overworld();
        List<Object> out = new ArrayList<>();
        List<ItemEntity> items = new ArrayList<>(level.getEntitiesOfClass(ItemEntity.class, new AABB(-16, 90, -16, 32, 120, 32)));
        items.sort(Comparator.comparing((ItemEntity e) -> hex(e.getItem())));
        for (ItemEntity e : items) {
            if (mobCase && e.getItem().is(net.minecraft.world.item.Items.ROTTEN_FLESH)) continue;
            out.add(op("item", hex(e.getItem()), "pos", new double[] {e.getX(), e.getY(), e.getZ()}));
        }
        return out;
    }

    static void step(ServerPlayer p, Case c, Map<String, Object> s) throws Exception {
        ServerLevel level = server.overworld();
        switch ((String) s.get("op")) {
            case "use" -> {
                InteractionHand hand = (int) s.get("hand") == 0 ? InteractionHand.MAIN_HAND : InteractionHand.OFF_HAND;
                p.connection.handleUseItem(new ServerboundUseItemPacket(hand, 1, p.getYRot(), p.getXRot()));
            }
            case "use_on" -> {
                InteractionHand hand = (int) s.get("hand") == 0 ? InteractionHand.MAIN_HAND : InteractionHand.OFF_HAND;
                @SuppressWarnings("unchecked")
                List<Integer> at = (List<Integer>) s.get("pos");
                @SuppressWarnings("unchecked")
                List<Double> cursor = (List<Double>) s.get("cursor");
                BlockPos pos = new BlockPos(at.get(0), at.get(1), at.get(2));
                Direction face = Direction.from3DDataValue((int) s.get("face"));
                Vec3 loc = new Vec3(pos.getX() + cursor.get(0), pos.getY() + cursor.get(1), pos.getZ() + cursor.get(2));
                p.connection.handleUseItemOn(new ServerboundUseItemOnPacket(hand, new BlockHitResult(loc, face, pos, false), 1));
            }
            case "sign_update" -> {
                @SuppressWarnings("unchecked")
                List<Integer> at = (List<Integer>) s.get("pos");
                @SuppressWarnings("unchecked")
                List<String> lines = (List<String>) s.get("lines");
                var slot = (boolean) s.get("front") ? net.minecraft.world.level.block.entity.SignTextSlot.FRONT
                        : net.minecraft.world.level.block.entity.SignTextSlot.BACK;
                p.connection.handleSignUpdate(new ServerboundSignUpdatePacket(new BlockPos(at.get(0), at.get(1), at.get(2)), lines, slot));
            }
            case "edit_book" -> {
                @SuppressWarnings("unchecked")
                List<String> pages = (List<String>) s.get("pages");
                String title = (String) s.get("title");
                p.connection.handleEditBook(new ServerboundEditBookPacket((int) s.get("slot"), pages, Optional.ofNullable(title)));
            }
            // wp52: middle click on the entity nearest to `pos`.
            case "pick_entity" -> {
                @SuppressWarnings("unchecked")
                List<Double> at = (List<Double>) s.get("pos");
                net.minecraft.world.entity.Entity target = null;
                double bd = 1e18;
                for (var e : server.overworld().getEntities((net.minecraft.world.entity.Entity) null, new AABB(at.get(0) - 8, at.get(1) - 8, at.get(2) - 8, at.get(0) + 8, at.get(1) + 8, at.get(2) + 8),
                        en -> !(en instanceof net.minecraft.world.entity.player.Player))) {
                    double d = e.position().distanceToSqr(at.get(0), at.get(1), at.get(2));
                    if (d < bd) { bd = d; target = e; }
                }
                if (target == null) throw new IllegalStateException("no entity near " + at);
                p.connection.handlePickItemFromEntity(new net.minecraft.network.protocol.game.ServerboundPickItemFromEntityPacket(target.getId(), (boolean) s.get("include")));
            }
            case "pick_block" -> {
                @SuppressWarnings("unchecked")
                List<Integer> at = (List<Integer>) s.get("pos");
                p.connection.handlePickItemFromBlock(new ServerboundPickItemFromBlockPacket(new BlockPos(at.get(0), at.get(1), at.get(2)), (boolean) s.get("include")));
            }
            // wp49: right click or attack on the hanging entity nearest to `pos`.
            case "use_entity", "attack_entity" -> {
                @SuppressWarnings("unchecked")
                List<Double> at = (List<Double>) s.get("pos");
                var target = nearestHanging(at.get(0), at.get(1), at.get(2));
                if (target == null) throw new IllegalStateException("no hanging entity near " + at);
                if (s.get("op").equals("attack_entity")) {
                    p.connection.handleAttack(new ServerboundAttackPacket(target.getId()));
                } else {
                    InteractionHand hand = (int) s.get("hand") == 0 ? InteractionHand.MAIN_HAND : InteractionHand.OFF_HAND;
                    Vec3 where = target.position();
                    if (s.containsKey("hit")) {
                        @SuppressWarnings("unchecked")
                        List<Double> hit = (List<Double>) s.get("hit");
                        where = new Vec3(hit.get(0), hit.get(1), hit.get(2));
                    }
                    p.connection.handleInteract(new ServerboundInteractPacket(target.getId(), hand, where, (boolean) s.get("sneak")));
                }
            }
            case "command" -> command((String) s.get("command"));
            // wp49: the mobs are killed, and their corpses (which a level that does not tick would keep) go.
            case "kill_mobs" -> {
                command("kill @e[type=minecraft:zombie]");
                for (var m : level.getEntitiesOfClass(net.minecraft.world.entity.Mob.class, new AABB(-64, -64, -64, 64, 320, 64))) if (!m.isAlive()) m.discard();
            }
            // wp49: the game time moves on (the level itself does not tick here).
            case "wait" -> {
                if (c.tickLevel) {
                    for (int i = 0; i < (int) s.get("ticks"); i++) {
                        levelTick(c);
                        broadcastChanges();
                    }
                } else {
                    var data = (net.minecraft.world.level.storage.ServerLevelData) level.getLevelData();
                    data.setGameTime(data.getGameTime() + (int) s.get("ticks"));
                }
            }
            // wp50: the cushions in the area tick `ticks` times (the level itself does not tick here).
            case "tick_cushions" -> {
                if (c.noPitch) addAttackTicks(p, (int) s.get("ticks"));
                for (int i = 0; i < (int) s.get("ticks"); i++) {
                    for (var cu : level.getEntitiesOfClass(net.minecraft.world.entity.decoration.Cushion.class, new AABB(-16, 60, -16, 32, 330, 32))) cu.tick();
                    for (var mq : level.getEntitiesOfClass(net.minecraft.world.entity.decoration.Mannequin.class, new AABB(-16, 60, -16, 32, 330, 32))) mq.tick();
                }
            }
            // wp50: the client says it moved to a position (`ServerboundMovePlayerPacket.Pos`); the server may put it back.
            case "move" -> {
                @SuppressWarnings("unchecked") List<Number> to = (List<Number>) s.get("to");
                // (The player's own tick sets the key from its input; the scenario keeps it held.)
                if (c.sneaking) p.setShiftKeyDown(true);
                p.connection.handleMovePlayer(new net.minecraft.network.protocol.game.ServerboundMovePlayerPacket.Pos(to.get(0).doubleValue(), to.get(1).doubleValue(), to.get(2).doubleValue(),
                        (boolean) s.get("on_ground"), (boolean) s.get("hcol")));
                // (Each step is a tick of its own: the connection takes one position per tick.)
                field(p.connection.getClass(), "receivedPositionThisTick").set(p.connection, false);
                field(p.connection.getClass(), "knownMovePacketCount").set(p.connection, get(p.connection, "receivedMovePacketCount"));
                field(p.connection.getClass(), "firstGoodX").set(p.connection, p.getX());
                field(p.connection.getClass(), "firstGoodY").set(p.connection, p.getY());
                field(p.connection.getClass(), "firstGoodZ").set(p.connection, p.getZ());
            }
            // wp52: what the player has fallen so far, and the grace time after an impulse.
            case "set_fall" -> p.fallDistance = ((Number) s.get("distance")).doubleValue();
            case "set_grace" -> p.applyPostImpulseGraceTime(((Number) s.get("ticks")).intValue());
            case "accept_teleport" -> {
                var at = (Vec3) get(p.connection, "awaitingPositionFromClient");
                if (at != null) p.connection.handleAcceptTeleportPacket(new net.minecraft.network.protocol.game.ServerboundAcceptTeleportationPacket((int) get(p.connection, "awaitingTeleport"), at.x, at.y, at.z, p.getYRot(), p.getXRot()));
            }
            // wp50: the structure block screen's packet.
            case "set_structure" -> {
                @SuppressWarnings("unchecked") List<Integer> at = (List<Integer>) s.get("pos");
                @SuppressWarnings("unchecked") List<Integer> off = (List<Integer>) s.get("offset");
                @SuppressWarnings("unchecked") List<Integer> size = (List<Integer>) s.get("size");
                p.connection.handleSetStructureBlock(new net.minecraft.network.protocol.game.ServerboundSetStructureBlockPacket(new BlockPos(at.get(0), at.get(1), at.get(2)),
                        net.minecraft.world.level.block.entity.StructureBlockEntity.UpdateType.valueOf((String) s.get("update")),
                        net.minecraft.world.level.block.state.properties.StructureMode.valueOf((String) s.get("mode")), (String) s.get("name"),
                        new BlockPos(off.get(0), off.get(1), off.get(2)), new net.minecraft.core.Vec3i(size.get(0), size.get(1), size.get(2)),
                        net.minecraft.world.level.block.Mirror.valueOf((String) s.get("mirror")), net.minecraft.world.level.block.Rotation.valueOf((String) s.get("rotation")),
                        (String) s.get("metadata"), (boolean) s.get("ignore_entities"), (boolean) s.get("strict"), (boolean) s.get("show_air"), (boolean) s.get("show_box"),
                        ((Number) s.get("integrity")).floatValue(), ((Number) s.get("seed")).longValue()));
            }
            // wp50: the jigsaw block screen's packets.
            case "set_jigsaw" -> {
                @SuppressWarnings("unchecked") List<Integer> at = (List<Integer>) s.get("pos");
                p.connection.handleSetJigsawBlock(new net.minecraft.network.protocol.game.ServerboundSetJigsawBlockPacket(new BlockPos(at.get(0), at.get(1), at.get(2)),
                        Identifier.parse((String) s.get("name")), Identifier.parse((String) s.get("target")), Identifier.parse((String) s.get("pool")), (String) s.get("final_state"),
                        net.minecraft.world.level.block.entity.JigsawBlockEntity.JointType.valueOf((String) s.get("joint")), (int) s.get("selection"), (int) s.get("placement")));
            }
            case "jigsaw_generate" -> {
                @SuppressWarnings("unchecked") List<Integer> at = (List<Integer>) s.get("pos");
                p.connection.handleJigsawGenerate(new net.minecraft.network.protocol.game.ServerboundJigsawGeneratePacket(new BlockPos(at.get(0), at.get(1), at.get(2)), (int) s.get("levels"), (boolean) s.get("keep")));
            }
            case "select" -> p.connection.handleSetCarriedItem(new ServerboundSetCarriedItemPacket((int) s.get("slot")));
            // wp49: a click on a menu button (`ServerboundContainerButtonClickPacket`) of the player's open menu.
            case "menu_button" -> p.connection.handleContainerButtonClick(new ServerboundContainerButtonClickPacket(p.containerMenu.containerId, (int) s.get("button")));
            // wp49: a crafter's slot switched off or on from its screen.
            case "menu_slot_state" -> {
                p.connection.handleContainerSlotStateChanged(new net.minecraft.network.protocol.game.ServerboundContainerSlotStateChangedPacket((int) s.get("slot"), p.containerMenu.containerId, (boolean) s.get("enabled")));
                // (The player's tick sends what changed.)
                p.containerMenu.broadcastChanges();
            }
            // wp49: the menu is closed and the player's tick that follows sends what changed in his inventory.
            case "menu_close_tick" -> {
                p.connection.handleContainerClose(new ServerboundContainerClosePacket(p.containerMenu.containerId));
                p.containerMenu.broadcastChanges();
            }
            // wp49: the command block screen's settings.
            case "set_command_block" -> {
                @SuppressWarnings("unchecked")
                List<Integer> at = (List<Integer>) s.get("pos");
                var mode = switch ((String) s.get("mode")) {
                    case "sequence" -> net.minecraft.world.level.block.entity.CommandBlockEntity.Mode.SEQUENCE;
                    case "auto" -> net.minecraft.world.level.block.entity.CommandBlockEntity.Mode.AUTO;
                    default -> net.minecraft.world.level.block.entity.CommandBlockEntity.Mode.REDSTONE;
                };
                p.connection.handleSetCommandBlock(new net.minecraft.network.protocol.game.ServerboundSetCommandBlockPacket(new BlockPos(at.get(0), at.get(1), at.get(2)), (String) s.get("command"), mode,
                        (boolean) s.get("track"), (boolean) s.get("conditional"), (boolean) s.get("auto")));
            }
            // wp49: the use key is let go (`ServerboundPlayerActionPacket.Action.RELEASE_USE_ITEM`).
            case "release_use" -> p.connection.handlePlayerAction(new ServerboundPlayerActionPacket(ServerboundPlayerActionPacket.Action.RELEASE_USE_ITEM, BlockPos.ZERO, Direction.DOWN, 1));
            // wp49: the player closes the menu.
            case "menu_close" -> p.connection.handleContainerClose(new ServerboundContainerClosePacket(p.containerMenu.containerId));
            // wp49: the player starts breaking the block (it goes at once in creative or with a tool that breaks it in a tick).
            case "dig" -> {
                @SuppressWarnings("unchecked")
                List<Integer> at = (List<Integer>) s.get("pos");
                p.connection.handlePlayerAction(new ServerboundPlayerActionPacket(ServerboundPlayerActionPacket.Action.START_DESTROY_BLOCK,
                        new BlockPos(at.get(0), at.get(1), at.get(2)), Direction.UP, 1));
            }
            case "cooldown" -> p.getCooldowns().addCooldown(stack((String) s.get("item")), (int) s.get("ticks"));
            // wp49: a click on the open menu's slot (`input` is the `ContainerInput` ordinal), predicting nothing.
            case "menu_click" -> p.connection.handleContainerClick(new ServerboundContainerClickPacket(p.containerMenu.containerId, p.containerMenu.getStateId(),
                    (short) (int) s.get("slot"), (byte) (int) s.get("button"), net.minecraft.world.inventory.ContainerInput.values()[(int) s.get("input")],
                    it.unimi.dsi.fastutil.ints.Int2ObjectMaps.emptyMap(), net.minecraft.network.HashedStack.EMPTY));
            // wp49: `ticks` server ticks pass for the player's maps (the step's own tick is one of them).
            case "map_wait" -> {
                for (int i = 0; i < (int) s.get("ticks") - 1; i++) mapTick(p);
            }
            case "lock_sign" -> {
                @SuppressWarnings("unchecked")
                List<Integer> at = (List<Integer>) s.get("pos");
                var be = (net.minecraft.world.level.block.entity.SignBlockEntity) level.getBlockEntity(new BlockPos(at.get(0), at.get(1), at.get(2)));
                be.setAllowedPlayerEditor(new UUID(7, 7));
            }
            default -> throw new IllegalArgumentException("unknown op " + s.get("op"));
        }
        if (System.getenv("BRUSH_DEBUG") != null) {
            System.out.println("BDEBUG " + c.name + " op " + s.get("op") + " time " + level.getGameTime() + " using " + p.isUsingItem() + " remaining " + p.getUseItemRemainingTicks() + " pos " + p.position()
                    + " rot " + p.getYRot() + "/" + p.getXRot() + " eye " + p.getEyePosition() + " held " + p.getMainHandItem()
                    + " head " + p.getYHeadRot() + " view " + p.getViewYRot(1.0F) + "/" + p.getViewXRot(1.0F) + " look " + p.getViewVector(1.0F) + " hit " + net.minecraft.world.entity.projectile.ProjectileUtil.getHitResultOnViewVector(p, net.minecraft.world.entity.EntitySelector.CAN_BE_PICKED, p.blockInteractionRange()).getType() + " " + net.minecraft.world.entity.projectile.ProjectileUtil.getHitResultOnViewVector(p, net.minecraft.world.entity.EntitySelector.CAN_BE_PICKED, p.blockInteractionRange()).getLocation() + " range " + p.blockInteractionRange()
                    + " block " + level.getBlockState(new BlockPos(2, 99, 0)) + " cd " + p.getCooldowns().isOnCooldown(p.getMainHandItem()));
        }
        if (System.getenv("VAULT_DEBUG") != null && c.tickLevel) {
            for (int[] w : c.watch) {
                if (level.getBlockEntity(new BlockPos(w[0], w[1], w[2])) instanceof net.minecraft.world.level.block.entity.vault.VaultBlockEntity vbe)
                    System.out.println("VDEBUG " + c.name + " op " + s.get("op") + " time " + level.getGameTime() + " lastFail " + get(vbe.getServerData(), "lastInsertFailTimestamp")
                            + " key " + vbe.getConfig().keyItem() + " held " + p.getMainHandItem() + " state " + level.getBlockState(new BlockPos(w[0], w[1], w[2])));
            }
        }
        if (c.tickLevel && !"wait".equals(s.get("op"))) levelTick(c);
        // Queued work (the sign text filter completes on the server thread's executor).
        for (int i = 0; i < 3; i++) call(server, "runAllTasks");
        broadcastChanges();
    }

    // ---------------------------------------------------------------- maps (wp49)

    /** `MapDecoration`s as the vectors record them: [type id, x, y, rot, name (hex of its NBT) or null]. */
    static List<Object> decoRows(Iterable<net.minecraft.world.level.saveddata.maps.MapDecoration> decorations) {
        List<Object> rows = new ArrayList<>();
        for (var d : decorations) {
            String name = null;
            if (d.name().isPresent()) {
                try {
                    name = nbtHex(net.minecraft.network.chat.ComponentSerialization.CODEC
                            .encodeStart(server.registryAccess().createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE), d.name().get()).getOrThrow());
                } catch (Exception e) {
                    throw new IllegalStateException(e);
                }
            }
            rows.add(java.util.Arrays.asList(BuiltInRegistries.MAP_DECORATION_TYPE.getId(d.type().value()), (int) d.x(), (int) d.y(), (int) d.rot(), name));
        }
        return rows;
    }

    /** The map ids start from zero in every case (the replay's server is new for each). */
    static void resetMaps() throws Exception {
        var index = server.getDataStorage().computeIfAbsent(net.minecraft.world.level.saveddata.maps.MapIndex.TYPE);
        field(net.minecraft.world.level.saveddata.maps.MapIndex.class, "lastMapId").setInt(index, -1);
    }

    /** wp49: one tick of the level as far as the watched block entities go: the clock moves on, their tickers run. */
    @SuppressWarnings({"unchecked", "rawtypes"})
    static ServerPlayer tickPlayer;

    /** The previous tick's rotations are the current ones (a player who has stood still: `getViewVector(0)` reads them). */
    static void settleRotation(ServerPlayer p) {
        try {
            p.setOldPosAndRot();
            var headO = net.minecraft.world.entity.LivingEntity.class.getDeclaredField("yHeadRotO");
            headO.setAccessible(true);
            headO.setFloat(p, p.getYHeadRot());
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    static void levelTick(Case c) {
        ServerLevel level = server.overworld();
        var data = (net.minecraft.world.level.storage.ServerLevelData) level.getLevelData();
        data.setGameTime(data.getGameTime() + 1);
        if (c.fullTicks) {
            if (System.getenv("BRUSH_DEBUG") != null && level.getBlockEntity(new BlockPos(2, 99, 0)) instanceof net.minecraft.world.level.block.entity.BrushableBlockEntity bbe) {
                try {
                    System.out.println("BTICK " + c.name + " t " + level.getGameTime() + " count " + get(bbe, "brushCount") + " resets " + get(bbe, "brushCountResetsAtTick") + " cool " + get(bbe, "coolDownEndsAtTick")
                            + " using " + (tickPlayer != null && tickPlayer.isUsingItem()) + " block " + level.getBlockState(new BlockPos(2, 99, 0)));
                } catch (Exception e) {
                    throw new IllegalStateException(e);
                }
            }
            try {
                // `ServerLevel.tick`: the scheduled block ticks first, then the entities (the player's use of his item), then the block entities.
                var blockTicks = (net.minecraft.world.ticks.LevelTicks<net.minecraft.world.level.block.Block>) level.getBlockTicks();
                Method tickBlock = ServerLevel.class.getDeclaredMethod("tickBlock", BlockPos.class, net.minecraft.world.level.block.Block.class);
                tickBlock.setAccessible(true);
                blockTicks.tick(level.getGameTime(), 65536, (pos, block) -> {
                    try {
                        tickBlock.invoke(level, pos, block);
                    } catch (ReflectiveOperationException e) {
                        throw new IllegalStateException(e);
                    }
                });
                if (tickPlayer != null) {
                    // (`Entity.baseTick` and `LivingEntity.tick` keep the previous tick's rotations, which `getViewVector(0)` reads.)
                    settleRotation(tickPlayer);
                    Method using = net.minecraft.world.entity.LivingEntity.class.getDeclaredMethod("updatingUsingItem");
                    using.setAccessible(true);
                    using.invoke(tickPlayer);
                }
            } catch (ReflectiveOperationException e) {
                throw new IllegalStateException(e);
            }
        }
        for (int[] w : c.watch) {
            BlockPos wp = new BlockPos(w[0], w[1], w[2]);
            BlockState st = level.getBlockState(wp);
            BlockEntity be = level.getBlockEntity(wp);
            if (be != null && st.getBlock() instanceof net.minecraft.world.level.block.EntityBlock eb) {
                var ticker = eb.getTicker(level, st, (net.minecraft.world.level.block.entity.BlockEntityType) be.getType());
                if (ticker != null) ticker.tick(level, wp, st, be);
            }
        }
    }

    /** One server tick of the player's maps:`Inventory.tick`, `EntityEquipment.tick` and `ServerPlayer.doTick`'s sync. */
    static void mapTick(ServerPlayer p) throws Exception {
        p.containerMenu.broadcastChanges();
        p.getInventory().tick();
        var equipment = (net.minecraft.world.entity.EntityEquipment) field(net.minecraft.world.entity.LivingEntity.class, "equipment").get(p);
        equipment.tick(p);
        Method sync = ServerPlayer.class.getDeclaredMethod("synchronizeSpecialItemUpdates", ItemStack.class);
        sync.setAccessible(true);
        for (int i = 0; i < p.getInventory().getContainerSize(); i++) {
            ItemStack st = p.getInventory().getItem(i);
            if (!st.isEmpty()) sync.invoke(p, st);
        }
        // The level's tracking: `ServerEntity.sendChanges` of the item frames (a map in one is ticked every tenth call).
        var tracked = (it.unimi.dsi.fastutil.ints.Int2ObjectMap<?>) get(server.overworld().getChunkSource().chunkMap, "entityMap");
        for (var frame : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.decoration.ItemFrame.class, new AABB(-128, -64, -128, 128, 320, 128))) {
            Object t = tracked.get(frame.getId());
            if (t == null) continue;
            Object serverEntity = get(t, "serverEntity");
            Method send = serverEntity.getClass().getMethod("sendChanges");
            send.invoke(serverEntity);
        }
    }

    /** The saved data of the maps in the player's inventory. */
    static List<Object> mapsOf(ServerPlayer p) throws Exception {
        // Every map made so far (they all begin at id 0 in a case).
        java.util.TreeSet<Integer> ids = new java.util.TreeSet<>();
        var index = server.getDataStorage().computeIfAbsent(net.minecraft.world.level.saveddata.maps.MapIndex.TYPE);
        for (int i = 0; i <= field(net.minecraft.world.level.saveddata.maps.MapIndex.class, "lastMapId").getInt(index); i++) ids.add(i);
        List<Object> out = new ArrayList<>();
        for (int id : ids) {
            var d = server.overworld().getMapData(new net.minecraft.world.level.saveddata.maps.MapId(id));
            if (d == null) continue;
            out.add(op("id", id, "scale", (int) d.scale, "center", List.of(d.centerX, d.centerZ), "locked", d.locked,
                    "tracking", get(d, "trackingPosition"), "unlimited", get(d, "unlimitedTracking"),
                    "colors", ByteBufUtil.hexDump(d.colors), "decos", decoRows(d.getDecorations())));
        }
        return out;
    }

    /** The tick's end sends the blocks that changed to the players tracking them. */
    static void broadcastChanges() throws Exception {
        ServerLevel level = server.overworld();
        for (Method m : level.getChunkSource().getClass().getDeclaredMethods()) {
            if (m.getName().equals("broadcastChangedChunks") && m.getParameterCount() == 1) {
                m.setAccessible(true);
                m.invoke(level.getChunkSource(), net.minecraft.util.profiling.Profiler.get());
            }
        }
    }

    static String run(Case c) throws Exception {
        // (A level that ticks whole: the ticks the setup schedules count from the case's clock, not the last case's.)
        if (c.fullTicks) ((net.minecraft.world.level.storage.ServerLevelData) server.overworld().getLevelData()).setGameTime(START_TIME);
        for (String cmd : c.commands) command(cmd);
        if (!c.tickLevel) for (String cmd : c.late) command(cmd);
        // The replay's level makes one tick between these commands and the first step: a hive's bees age by it.
        if (!c.late.isEmpty()) {
            for (int[] w : c.watch) {
                BlockPos wp = new BlockPos(w[0], w[1], w[2]);
                if (server.overworld().getBlockEntity(wp) instanceof net.minecraft.world.level.block.entity.BeehiveBlockEntity hb)
                    net.minecraft.world.level.block.entity.BeehiveBlockEntity.serverTick(server.overworld(), wp, server.overworld().getBlockState(wp), hb);
            }
        }
        // The recorded level does not tick: what the commands made has had its first tick for the cases that
        // do something to it (a stand's equipment sounds only after its first tick).
        for (var stand : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.decoration.ArmorStand.class, new AABB(-16, 90, -16, 32, 120, 32))) {
            java.lang.reflect.Field f = field(net.minecraft.world.entity.Entity.class, "firstTick");
            f.set(stand, false);
        }
        if (System.getenv("INTERACT_DEBUG") != null) { command("data get entity @e[type=minecraft:painting,limit=1]"); command("summon minecraft:painting 2 100 0 {facing:1b,variant:\"minecraft:courbet\"}"); }
        players++;
        ServerPlayer p = mockPlayer("Interact");
        tickPlayer = p;
        setup(p, c);
        broadcastChanges();
        drain(p);
        if (c.tickLevel) {
            for (String cmd : c.late) command(cmd);
            levelTick(c);
            broadcastChanges();
            drain(p);
        }
        if (System.getenv("INTERACT_DEBUG") != null) {
            System.out.println("DEBUG " + c.name + " pos " + p.getX() + " " + p.getY() + " " + p.getZ());
            var dbe = server.overworld().getBlockEntity(new BlockPos(4, 100, 4));
            if (dbe instanceof net.minecraft.world.level.block.entity.SignBlockEntity sbe) {
                var sb = (net.minecraft.world.level.block.SignBlock) sbe.getBlockState().getBlock();
                System.out.println("DEBUG yrot " + sb.getYRotationDegrees(sbe.getBlockState()) + " slot " + sbe.getSlotPlayerIsFacing(p)
                        + " center " + sb.getSignHitboxCenterPosition(sbe.getBlockState()));
            }
            System.out.println("DEBUG tracking view " + p.getChunkTrackingView() + " players "
                    + server.overworld().getChunkSource().chunkMap.getPlayers(new net.minecraft.world.level.ChunkPos(0, 0), false).size());
        }
        List<Object> results = new ArrayList<>();
        // The player's statistics outlive the mock player (the stats counter is kept by uuid): what a
        // case used is counted from where the case began.
        Map<String, Integer> usedBefore = new HashMap<>();
        for (String item : c.statItems) {
            usedBefore.put(item, p.getStats().getValue(net.minecraft.stats.Stats.ITEM_USED.get(BuiltInRegistries.ITEM.getValue(Identifier.parse(item)))));
        }
        Map<String, Integer> customBefore = new HashMap<>();
        for (String n : c.customStats) customBefore.put(n, p.getStats().getValue(net.minecraft.stats.Stats.CUSTOM.get(BuiltInRegistries.CUSTOM_STAT.getValue(Identifier.parse(n)))));
        java.util.Set<UUID> seenBees = new java.util.HashSet<>();
        recordMenus = c.watchMenus;
        long startClock = server.overworld().getGameTime();
        mobCase = c.watchMobs;
        if (c.watchAdv) resetAdvancements(p);
        java.util.TreeSet<String> advBefore = c.watchAdv ? doneCriteria(p) : null;
        recordNoPitch = c.noPitch;
        if (c.noPitch) addAttackTicks(p, 100);
        recordMaps = c.watchMaps;
        if (c.watchMaps) resetMaps();
        for (Map<String, Object> s : c.steps) {
            step(p, c, s);
            if (c.tickCushions && !"tick_cushions".equals(s.get("op"))) {
                if (c.noPitch) addAttackTicks(p, 1);
                for (var cu : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.decoration.Cushion.class, new AABB(-16, 60, -16, 32, 330, 32))) cu.tick();
                for (var mq : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.decoration.Mannequin.class, new AABB(-16, 60, -16, 32, 330, 32))) mq.tick();
            }
            if (c.watchMaps) mapTick(p);
            Map<String, Object> r = new LinkedHashMap<>();
            if (c.watchMaps) r.put("maps", mapsOf(p));
            r.put("inv", inventory(p));
            r.put("packets", packets(p));
            if (c.watchMove) {
                r.put("ppos", List.of(p.getX(), p.getY(), p.getZ()));
                r.put("teleports", teleports);
                teleports = 0;
            }
            r.put("blocks", blocks(c));
            if (!c.templates.isEmpty()) r.put("templates", templatesOf(c));
            r.put("entities", itemEntities());
            if (c.watchMobs) r.put("mobs", mobRows());
            Map<String, Object> used = new LinkedHashMap<>();
            for (String item : c.statItems) {
                used.put(item, p.getStats().getValue(net.minecraft.stats.Stats.ITEM_USED.get(BuiltInRegistries.ITEM.getValue(Identifier.parse(item)))) - usedBefore.get(item));
            }
            r.put("used", used);
            if (c.watchAdv) {
                var now = doneCriteria(p);
                var fresh = new ArrayList<String>(now);
                fresh.removeAll(advBefore);
                advBefore = now;
                r.put("adv", fresh);
            }
            if (c.watchHanging) r.put("hangings", hangings());
            if (c.watchStands) r.put("stands", stands());
            if (c.watchBees) {
                List<List<Object>> fresh = new ArrayList<>();
                for (var bee : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.animal.bee.Bee.class, new AABB(-16, 90, -16, 32, 120, 32))) {
                    if (seenBees.add(bee.getUUID())) fresh.add(List.of(bee.getX(), bee.getY(), bee.getZ(), bee.getTargetUnchecked() != null ? 1 : 0));
                }
                fresh.sort(Comparator.<List<Object>>comparingDouble(l -> (Double) l.get(0)).thenComparingDouble(l -> (Double) l.get(1)).thenComparingDouble(l -> (Double) l.get(2)));
                r.put("bees_new", new ArrayList<Object>(fresh));
            }
            if (c.watchFood) r.put("food", List.of(p.getFoodData().getFoodLevel(), p.getFoodData().getSaturationLevel(), (float) get(p.getFoodData(), "exhaustionLevel")));
            if (!c.customStats.isEmpty()) {
                Map<String, Object> cs = new LinkedHashMap<>();
                for (String n : c.customStats) cs.put(n, p.getStats().getValue(net.minecraft.stats.Stats.CUSTOM.get(BuiltInRegistries.CUSTOM_STAT.getValue(Identifier.parse(n)))) - customBefore.get(n));
                r.put("custom", cs);
            }
            results.add(r);
        }
        server.getPlayerList().remove(p);
        if (c.op) command("deop " + p.getGameProfile().name());
        command("fill -4 90 -8 15 110 15 minecraft:air");
        // (Scheduled ticks that were still due when the case ended would block the next case's: the game time goes back.)
        if (c.fullTicks) {
            // (The ticks run out: a few ticks of the clock, scheduled block ticks only.)
            var level = server.overworld();
            var data = (net.minecraft.world.level.storage.ServerLevelData) level.getLevelData();
            var blockTicks = (net.minecraft.world.ticks.LevelTicks<net.minecraft.world.level.block.Block>) level.getBlockTicks();
            Method tickBlock = ServerLevel.class.getDeclaredMethod("tickBlock", BlockPos.class, net.minecraft.world.level.block.Block.class);
            tickBlock.setAccessible(true);
            for (int i = 0; i < 12; i++) {
                data.setGameTime(data.getGameTime() + 1);
                blockTicks.tick(level.getGameTime(), 65536, (pos, block) -> {
                    try {
                        tickBlock.invoke(level, pos, block);
                    } catch (ReflectiveOperationException e) {
                        throw new IllegalStateException(e);
                    }
                });
            }
            command("fill -4 90 -8 15 110 15 minecraft:air");
        }
        command("kill @e[type=minecraft:item]");
        command("kill @e[type=minecraft:item_frame]");
        command("kill @e[type=minecraft:glow_item_frame]");
        command("kill @e[type=minecraft:painting]");
        command("kill @e[type=minecraft:cushion]");
        command("kill @e[type=minecraft:mannequin]");
        for (var mq : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.decoration.Mannequin.class, new AABB(-64, -64, -64, 64, 320, 64))) mq.discard();
        command("kill @e[type=minecraft:armor_stand]");
        command("kill @e[type=minecraft:marker]");
        command("kill @e[type=minecraft:minecart]");
        // (wp52: whatever else a case summoned: carts, boats, crystals, primed TNT...)
        command("kill @e[type=!minecraft:player]");
        command("kill @e[type=minecraft:falling_block]");
        command("kill @e[type=minecraft:item]");
        for (var bee : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.animal.bee.Bee.class, new AABB(-64, -64, -64, 64, 320, 64))) bee.discard();
        for (var mob : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.Mob.class, new AABB(-64, -64, -64, 64, 320, 64))) mob.discard();
        Map<String, Object> line = new LinkedHashMap<>();
        line.put("name", c.name);
        line.put("game_mode", c.gameMode);
        line.put("pos", c.pos);
        line.put("rot", new double[] {c.yaw, c.pitch});
        line.put("sneaking", c.sneaking);
        line.put("selected", c.selected);
        line.put("commands", c.commands);
        line.put("late", c.late);
        Map<String, Object> slots = new LinkedHashMap<>();
        for (var e : c.slots.entrySet()) slots.put(e.getKey(), hex(e.getValue()));
        line.put("slots", slots);
        line.put("steps", c.steps);
        List<Object> watch = new ArrayList<>();
        for (int[] w : c.watch) watch.add(List.of(w[0], w[1], w[2]));
        line.put("watch", watch);
        line.put("stat_items", c.statItems);
        line.put("adv", c.watchAdv);
        line.put("templates", c.templates);
        line.put("food", c.watchFood ? c.food : null);
        line.put("hanging", c.watchHanging);
        line.put("no_pitch", c.noPitch);
        line.put("stands", c.watchStands);
        line.put("bees", c.watchBees);
        line.put("menus", c.watchMenus);
        line.put("maps", c.watchMaps);
        line.put("ticking", c.tickLevel);
        line.put("op", c.op);
        if (c.fullTicks) line.put("clock", startClock);
        line.put("mobs", c.watchMobs);
        line.put("player_uuid", p.getUUID().toString());
        line.put("custom_stats", c.customStats);
        line.put("result", results);
        return toJson(line);
    }

    /** `Entity.getPickResult` of one entity of every type that can be created (middle click on it). */
    static void pickEntityTable(List<String> out) throws Exception {
        ServerLevel level = server.overworld();
        for (var type : BuiltInRegistries.ENTITY_TYPE) {
            String name = BuiltInRegistries.ENTITY_TYPE.getKey(type).toString();
            if (name.equals("minecraft:player")) continue;
            String item;
            try {
                net.minecraft.world.entity.Entity e = type.create(level, net.minecraft.world.entity.EntitySpawnReason.COMMAND);
                if (e == null) {
                    item = "none";
                } else {
                    ItemStack r = e.getPickResult();
                    item = r == null ? "null" : hex(r);
                }
            } catch (Throwable t) {
                item = "error";
            }
            out.add("{\"entity\":\"" + name + "\",\"item\":\"" + item + "\"}");
        }
    }

    /** `BlockState.getCloneItemStack` of every block's states (what middle click picks), without data. */
    static void pickTable(List<String> out) throws Exception {
        ServerLevel level = server.overworld();
        BlockPos pos = new BlockPos(4, 100, 4);
        for (net.minecraft.world.level.block.Block block : BuiltInRegistries.BLOCK) {
            // The class that decides what the block gives (where `getCloneItemStack` is declared).
            String declaring = "Block";
            for (Class<?> k = block.getClass(); k != null && k != net.minecraft.world.level.block.Block.class; k = k.getSuperclass()) {
                boolean found = false;
                for (Method m : k.getDeclaredMethods()) {
                    if (m.getName().equals("getCloneItemStack") && m.getParameterCount() == 4) found = true;
                }
                if (found) {
                    declaring = k.getSimpleName();
                    break;
                }
            }
            out.add("{\"override\":\"" + BuiltInRegistries.BLOCK.getKey(block) + "\",\"class\":\"" + declaring + "\"}");
            for (BlockState st : block.getStateDefinition().getPossibleStates()) {
                String item;
                try {
                    item = hex(st.getCloneItemStack(level, pos, false));
                } catch (Throwable t) {
                    item = "error";
                }
                out.add("{\"block\":\"" + BuiltInRegistries.BLOCK.getKey(block) + "\",\"state\":" + net.minecraft.world.level.block.Block.getId(st) + ",\"item\":\"" + item + "\"}");
            }
        }
    }

    // ---------------------------------------------------------------- main

    public static void main(String[] args) {
        try {
            main0(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    static void main0(String[] args) throws Exception {
        Path outPath = Path.of(args[0]).toAbsolutePath();
        String filter = args.length > 1 ? args[1] : null;
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "InteractVectors main");
        main.start();
        server = awaitServer();
        server.submit(() -> {
            ServerLevel level = server.overworld();
            level.tickRateManager().setFrozen(true);
            // (wp49: a map covers 128 blocks around the origin.)
            for (int cx = -4; cx <= 4; cx++)
                for (int cz = -4; cz <= 4; cz++) {
                    level.setChunkForced(cx, cz, true);
                    level.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(1000);
        server.submit(() -> command("gamerule block_drops true")).get();
        // The mock player earns advancements as it uses things; their announcements are not what these vectors are about.
        server.submit(() -> command("gamerule show_advancement_messages false")).get();
        List<Case> all = new ArrayList<>();
        server.submit(() -> {
            equip(all);
            signs(all);
            books(all);
            picks(all);
            pickEntities52(all);
            placeComponents52(all);
            cakes(all);
            blocks49(all);
            frames49(all);
            stands49(all);
            bells49(all);
            hives49(all);
            pots49(all);
            statues49(all);
            lecterns49(all);
            maps49(all);
            vaults49(all);
            trials49(all);
            crafters49(all);
            brushes49(all);
            banners50(all);
            advancements53(all);
            cushions50(all);
            mannequins50(all);
            structures50(all);
            moves50(all);
            moves52(all);
            cauldrons50(all);
            commandBlocks49(all);
        }).get();
        List<Case> selected = new ArrayList<>();
        for (Case c : all) {
            if (filter == null || c.name.matches(filter) || c.name.contains(filter)) selected.add(c);
        }
        System.out.println("InteractVectors: " + selected.size() + " scenarios");
        List<String> lines = new ArrayList<>();
        for (Case c : selected) {
            server.submit(() -> {
                try {
                    lines.add(run(c));
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("{\"name\":\"" + c.name + "\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
                }
            }).get();
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("InteractVectors: wrote " + lines.size() + " scenarios to " + outPath);
        if (filter == null) {
            List<String> table = new ArrayList<>();
            server.submit(() -> {
                try {
                    pickTable(table);
                    pickEntityTable(table);
                } catch (Throwable t) {
                    t.printStackTrace();
                }
            }).get();
            Path tablePath = outPath.resolveSibling("pick_table.jsonl");
            try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(tablePath))) {
                for (String l : table) w.println(l);
            }
            System.out.println("InteractVectors: wrote " + table.size() + " pick results to " + tablePath);
        }
        server.halt(false);
        System.exit(0);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + harnessPort(),
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:the_void\"}",
                "spawn-protection=0",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "enable-rcon=false",
                "enable-query=false",
                "spawn-monsters=false",
                "enable-command-block=true",
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
            // The server thread died at startup (the port was taken between the check and the bind).
            if (i > 600 && Thread.getAllStackTraces().keySet().stream().noneMatch(t -> t.getName().equals("Server thread"))) break;
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

    // ---------------------------------------------------------------- reflection and JSON

    static Field field(Class<?> c, String name) throws NoSuchFieldException {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                Field f = k.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
                // keep looking
            }
        }
        throw new NoSuchFieldException(name);
    }

    static Object get(Object o, String name) throws Exception {
        return field(o.getClass(), name).get(o);
    }

    static Object call(Object o, String name) throws Exception {
        for (Class<?> k = o.getClass(); k != null; k = k.getSuperclass()) {
            for (Method m : k.getDeclaredMethods()) {
                if (m.getName().equals(name) && m.getParameterCount() == 0) {
                    m.setAccessible(true);
                    return m.invoke(o);
                }
            }
        }
        throw new NoSuchMethodException(name);
    }

    static String toJson(Object o) {
        if (o == null) return "null";
        if (o instanceof String s) {
            StringBuilder b = new StringBuilder("\"");
            for (char ch : s.toCharArray()) {
                switch (ch) {
                    case '"' -> b.append("\\\"");
                    case '\\' -> b.append("\\\\");
                    case '\n' -> b.append("\\n");
                    default -> {
                        if (ch < 0x20 || ch > 0x7e) b.append(String.format("\\u%04x", (int) ch));
                        else b.append(ch);
                    }
                }
            }
            return b.append('"').toString();
        }
        if (o instanceof Boolean || o instanceof Integer || o instanceof Long) return o.toString();
        if (o instanceof Float f) return Float.toString(f);
        if (o instanceof Double d) return Double.toString(d);
        if (o instanceof double[] a) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < a.length; i++) b.append(i > 0 ? "," : "").append(Double.toString(a[i]));
            return b.append("]").toString();
        }
        if (o instanceof Object[] a) return toJson(java.util.Arrays.asList(a));
        if (o instanceof List<?> l) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < l.size(); i++) b.append(i > 0 ? "," : "").append(toJson(l.get(i)));
            return b.append("]").toString();
        }
        if (o instanceof Map<?, ?> m) {
            StringBuilder b = new StringBuilder("{");
            boolean first = true;
            for (var e : m.entrySet()) {
                b.append(first ? "" : ",").append(toJson(String.valueOf(e.getKey()))).append(":").append(toJson(e.getValue()));
                first = false;
            }
            return b.append("}").toString();
        }
        return toJson(String.format(Locale.ROOT, "%s", o));
    }

    /// $KILN_HARNESS_PORT, else the first free port of 25581-25583 (waits while all are busy).
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
