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

        Case hanging() {
            watchHanging = true;
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

    /** wp49: lecterns: putting a book on, the menu (pages, jump, take), redstone pulse, breaking. */
    /** wp49: a landscape across the map around the origin, built with commands (the same ones build it in the replay). */
    static Case mapCase(String name) {
        Case c = new Case(name).maps();
        for (int y = 62; y <= 70; y += 2) c.cmd("fill -64 " + y + " -64 63 " + Math.min(y + 1, 70) + " 63 minecraft:air");
        // The ground: stone up to y=61 (lakes and pools are flush with it, so nothing flows).
        for (int y = 56; y <= 60; y += 2) c.cmd("fill -64 " + y + " -64 63 " + (y + 1) + " 63 minecraft:stone");
        c.cmd("fill -64 61 -64 63 61 63 minecraft:stone");
        c.cmd("fill -60 62 -60 -20 63 -20 minecraft:grass_block");
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
        // A banner outside the map's area is not taken.
        c = mapCase("map_banner_outside").cmd("setblock 70 62 70 minecraft:red_banner[rotation=4]");
        c.slot("h0", stack("minecraft:map", 1));
        c.step(op("op", "use", "hand", 0)).step(useOn(70, 62, 70, 1, 0)).step(op("op", "map_wait", "ticks", 6));
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
        p.snapTo(c.pos[0], c.pos[1], c.pos[2], c.yaw, c.pitch);
        p.setDeltaMovement(Vec3.ZERO);
        p.setOnGround(true);
        p.fallDistance = 0.0;
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

    static List<Object> packets(ServerPlayer p) throws Exception {
        List<Object> out = new ArrayList<>();
        for (Object o : drain(p)) {
            if (o instanceof ClientboundSoundPacket s) {
                out.add(op("t", "sound", "name", soundName(s), "source", s.getSource().getName(),
                        "pos", new double[] {s.getX(), s.getY(), s.getZ()}, "volume", s.getVolume(), "pitch", s.getPitch()));
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
        for (var e : level.getEntities((net.minecraft.world.entity.Entity) null, new AABB(x - 2, y - 2, z - 2, x + 2, y + 2, z + 2),
                en -> en instanceof net.minecraft.world.entity.decoration.HangingEntity || en instanceof net.minecraft.world.entity.decoration.ArmorStand)) {
            double d = e.position().distanceToSqr(x, y, z);
            if (d < bd) { bd = d; best = e; }
        }
        return best;
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

    static List<Object> itemEntities() {
        ServerLevel level = server.overworld();
        List<Object> out = new ArrayList<>();
        List<ItemEntity> items = new ArrayList<>(level.getEntitiesOfClass(ItemEntity.class, new AABB(-16, 90, -16, 32, 120, 32)));
        items.sort(Comparator.comparing((ItemEntity e) -> hex(e.getItem())));
        for (ItemEntity e : items) out.add(op("item", hex(e.getItem()), "pos", new double[] {e.getX(), e.getY(), e.getZ()}));
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
            // wp49: the game time moves on (the level itself does not tick here).
            case "wait" -> {
                var data = (net.minecraft.world.level.storage.ServerLevelData) level.getLevelData();
                data.setGameTime(data.getGameTime() + (int) s.get("ticks"));
            }
            case "select" -> p.connection.handleSetCarriedItem(new ServerboundSetCarriedItemPacket((int) s.get("slot")));
            // wp49: a click on a menu button (`ServerboundContainerButtonClickPacket`) of the player's open menu.
            case "menu_button" -> p.connection.handleContainerButtonClick(new ServerboundContainerButtonClickPacket(p.containerMenu.containerId, (int) s.get("button")));
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
        var index = server.overworld().getDataStorage().computeIfAbsent(net.minecraft.world.level.saveddata.maps.MapIndex.TYPE);
        field(net.minecraft.world.level.saveddata.maps.MapIndex.class, "lastMapId").setInt(index, -1);
    }

    /** One server tick of the player's maps: `Inventory.tick`, `EntityEquipment.tick` and `ServerPlayer.doTick`'s sync. */
    static void mapTick(ServerPlayer p) throws Exception {
        p.getInventory().tick();
        var equipment = (net.minecraft.world.entity.EntityEquipment) field(net.minecraft.world.entity.LivingEntity.class, "equipment").get(p);
        equipment.tick(p);
        Method sync = ServerPlayer.class.getDeclaredMethod("synchronizeSpecialItemUpdates", ItemStack.class);
        sync.setAccessible(true);
        for (int i = 0; i < p.getInventory().getContainerSize(); i++) {
            ItemStack st = p.getInventory().getItem(i);
            if (!st.isEmpty()) sync.invoke(p, st);
        }
    }

    /** The saved data of the maps in the player's inventory. */
    static List<Object> mapsOf(ServerPlayer p) throws Exception {
        java.util.TreeSet<Integer> ids = new java.util.TreeSet<>();
        for (int i = 0; i < p.getInventory().getContainerSize(); i++) {
            var id = p.getInventory().getItem(i).get(DataComponents.MAP_ID);
            if (id != null) ids.add(id.id());
        }
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
        for (String cmd : c.commands) command(cmd);
        for (String cmd : c.late) command(cmd);
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
        setup(p, c);
        broadcastChanges();
        drain(p);
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
        recordMaps = c.watchMaps;
        if (c.watchMaps) resetMaps();
        for (Map<String, Object> s : c.steps) {
            step(p, c, s);
            if (c.watchMaps) mapTick(p);
            Map<String, Object> r = new LinkedHashMap<>();
            if (c.watchMaps) r.put("maps", mapsOf(p));
            r.put("inv", inventory(p));
            r.put("packets", packets(p));
            r.put("blocks", blocks(c));
            r.put("entities", itemEntities());
            Map<String, Object> used = new LinkedHashMap<>();
            for (String item : c.statItems) {
                used.put(item, p.getStats().getValue(net.minecraft.stats.Stats.ITEM_USED.get(BuiltInRegistries.ITEM.getValue(Identifier.parse(item)))) - usedBefore.get(item));
            }
            r.put("used", used);
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
        command("fill -4 90 -8 15 110 15 minecraft:air");
        command("kill @e[type=minecraft:item]");
        command("kill @e[type=minecraft:item_frame]");
        command("kill @e[type=minecraft:glow_item_frame]");
        command("kill @e[type=minecraft:painting]");
        command("kill @e[type=minecraft:armor_stand]");
        command("kill @e[type=minecraft:item]");
        for (var bee : server.overworld().getEntitiesOfClass(net.minecraft.world.entity.animal.bee.Bee.class, new AABB(-64, -64, -64, 64, 320, 64))) bee.discard();
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
        line.put("food", c.watchFood ? c.food : null);
        line.put("hanging", c.watchHanging);
        line.put("stands", c.watchStands);
        line.put("bees", c.watchBees);
        line.put("menus", c.watchMenus);
        line.put("maps", c.watchMaps);
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
            cakes(all);
            blocks49(all);
            frames49(all);
            stands49(all);
            bells49(all);
            hives49(all);
            pots49(all);
            lecterns49(all);
            maps49(all);
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
