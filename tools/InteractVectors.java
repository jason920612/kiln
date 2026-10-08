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
        // wp49: the custom stats (by name) whose change is recorded.
        List<String> customStats = new ArrayList<>();

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

    static void setup(ServerPlayer p, Case c) throws Exception {
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
            case "select" -> p.connection.handleSetCarriedItem(new ServerboundSetCarriedItemPacket((int) s.get("slot")));
            case "cooldown" -> p.getCooldowns().addCooldown(stack((String) s.get("item")), (int) s.get("ticks"));
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
        for (Map<String, Object> s : c.steps) {
            step(p, c, s);
            Map<String, Object> r = new LinkedHashMap<>();
            r.put("inv", inventory(p));
            r.put("packets", packets(p));
            r.put("blocks", blocks(c));
            r.put("entities", itemEntities());
            Map<String, Object> used = new LinkedHashMap<>();
            for (String item : c.statItems) {
                used.put(item, p.getStats().getValue(net.minecraft.stats.Stats.ITEM_USED.get(BuiltInRegistries.ITEM.getValue(Identifier.parse(item)))) - usedBefore.get(item));
            }
            r.put("used", used);
            if (c.watchFood) r.put("food", List.of(p.getFoodData().getFoodLevel(), p.getFoodData().getSaturationLevel(), (float) get(p.getFoodData(), "exhaustionLevel")));
            if (!c.customStats.isEmpty()) {
                Map<String, Object> cs = new LinkedHashMap<>();
                for (String n : c.customStats) cs.put(n, p.getStats().getValue(net.minecraft.stats.Stats.CUSTOM.get(BuiltInRegistries.CUSTOM_STAT.getValue(Identifier.parse(n)))) - customBefore.get(n));
                r.put("custom", cs);
            }
            results.add(r);
        }
        server.getPlayerList().remove(p);
        command("fill 0 90 0 15 110 15 minecraft:air");
        command("kill @e[type=minecraft:item]");
        Map<String, Object> line = new LinkedHashMap<>();
        line.put("name", c.name);
        line.put("game_mode", c.gameMode);
        line.put("pos", c.pos);
        line.put("rot", new double[] {c.yaw, c.pitch});
        line.put("sneaking", c.sneaking);
        line.put("selected", c.selected);
        line.put("commands", c.commands);
        Map<String, Object> slots = new LinkedHashMap<>();
        for (var e : c.slots.entrySet()) slots.put(e.getKey(), hex(e.getValue()));
        line.put("slots", slots);
        line.put("steps", c.steps);
        List<Object> watch = new ArrayList<>();
        for (int[] w : c.watch) watch.add(List.of(w[0], w[1], w[2]));
        line.put("watch", watch);
        line.put("stat_items", c.statItems);
        line.put("food", c.watchFood ? c.food : null);
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
            for (int cx = -2; cx <= 2; cx++)
                for (int cz = -2; cz <= 2; cz++) {
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
