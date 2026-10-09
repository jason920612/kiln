// Differential test vectors for Kiln's mob effects, fire and air: runs scenarios on a mock
// player in a real vanilla 26.3 dedicated server (started in-process) and records the player's
// state after every tick (ServerLevel's commonTick + ServerPlayer.tick, then the connection's
// tickPlayer, as a server tick runs them).
//
// A scenario sets the player up (position, game mode, health, food, air, fire, armor, blocks
// around it, randoms), then ticks it, running actions before chosen ticks: adding or removing
// effects, taking damage, holding and consuming items. Each tick records health, absorption,
// food, fire ticks, air, the active effects (with hidden ones), attribute values, the destroy
// speed on stone and the effect and entity event packets the player got. One JSON line per
// scenario; the first line holds the mob effect and potion registries.
//
// usage (cwd = a scratch server directory, e.g. work/wp9-effects/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/EffectVectors.java <out.jsonl> [name-filter]
// (tools/effect_vectors.py sets this up)

import com.mojang.authlib.GameProfile;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundBundlePacket;
import net.minecraft.network.protocol.game.ClientboundEntityEventPacket;
import net.minecraft.network.protocol.game.ClientboundRemoveMobEffectPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateMobEffectPacket;
import net.minecraft.network.protocol.game.ServerboundClientTickEndPacket;
import net.minecraft.network.protocol.game.ServerboundMovePlayerPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.alchemy.PotionContents;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

public class EffectVectors {
    static final double BX = 0.5, BY = 100.0, BZ = 0.5;
    static final int[] BASE = {0, 100, 0};

    // ---------------------------------------------------------------- scenario model

    static final class Scenario {
        final String name;
        int ticks = 40;
        String gameMode = "survival";
        String difficulty = "normal";
        float health = 20f;
        int food = 20;
        float saturation = 5f;
        float absorption;
        int air = 300;
        int fire = -20;
        boolean onGround = true;
        boolean sneaking;
        double dx, dy, dz;
        long seed;
        String mainHand;
        String[] armor = new String[4]; // feet, legs, chest, head
        List<Map<String, Integer>> armorEnch = new ArrayList<>(List.of(
                new LinkedHashMap<>(), new LinkedHashMap<>(), new LinkedHashMap<>(), new LinkedHashMap<>()));
        // Blocks relative to BASE: {dx, dy, dz, state}.
        List<Object[]> blocks = new ArrayList<>();
        // Actions before a tick (1-based): each a map with "op".
        TreeMap<Integer, List<Map<String, Object>>> actions = new TreeMap<>();
        // wp44 (player hazards): a shadow client drives the player with move packets, as a real
        // client does; the packets it sent are recorded in `moves` (one per tick).
        boolean client;
        List<Object> moves = new ArrayList<>();
        // Attribute base values set at the start: {attribute id, value}.
        List<Object[]> attrs = new ArrayList<>();
        // wp49: blocks (relative to BASE) whose states are recorded each tick, and whether the boots' wear is.
        List<int[]> watch = new ArrayList<>();
        boolean watchBoots;

        Scenario watch(int x, int y, int z) {
            watch.add(new int[] {x, y, z});
            return this;
        }

        Scenario(String name) {
            this.name = name;
            seed = name.hashCode();
        }

        Scenario at(int tick, Map<String, Object> action) {
            actions.computeIfAbsent(tick, k -> new ArrayList<>()).add(action);
            return this;
        }

        Scenario block(int x, int y, int z, String state) {
            blocks.add(new Object[] {x, y, z, state});
            return this;
        }

        /** A box of `state` from (x0,y0,z0) to (x1,y1,z1), inclusive. */
        Scenario fill(int x0, int y0, int z0, int x1, int y1, int z1, String state) {
            for (int x = x0; x <= x1; x++)
                for (int y = y0; y <= y1; y++)
                    for (int z = z0; z <= z1; z++) block(x, y, z, state);
            return this;
        }

        /** A 1x`height` column of `fluid` at the player's block, walled in by glass (open above,
         *  so the player keeps standing). */
        Scenario pool(String fluid, int height) {
            fill(-1, -1, -1, 1, height - 1, 1, "minecraft:glass");
            fill(0, 0, 0, 0, height - 1, 0, fluid);
            return this;
        }

        Map<String, Object> json() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", name);
            m.put("ticks", ticks);
            m.put("game_mode", gameMode);
            m.put("difficulty", difficulty);
            m.put("health", health);
            m.put("food", food);
            m.put("saturation", saturation);
            m.put("absorption", absorption);
            m.put("air", air);
            m.put("fire", fire);
            m.put("on_ground", onGround);
            m.put("sneaking", sneaking);
            m.put("pos", new double[] {BX + dx, BY + dy, BZ + dz});
            m.put("seed", seed);
            m.put("main_hand", mainHand);
            m.put("armor", armor);
            m.put("armor_enchantments", armorEnch);
            List<Object> watched = new ArrayList<>();
            for (int[] w : watch) watched.add(List.of(w[0], w[1], w[2]));
            m.put("watch", watched);
            m.put("watch_boots", watchBoots);
            List<Object> bl = new ArrayList<>();
            for (Object[] b : blocks) {
                bl.add(List.of(BASE[0] + (int) b[0], BASE[1] + (int) b[1], BASE[2] + (int) b[2], b[3]));
            }
            m.put("blocks", bl);
            Map<String, Object> acts = new LinkedHashMap<>();
            for (var e : actions.entrySet()) acts.put(String.valueOf(e.getKey()), e.getValue());
            m.put("actions", acts);
            if (client) {
                m.put("client", true);
                m.put("moves", moves);
            }
            if (!attrs.isEmpty()) {
                List<Object> al = new ArrayList<>();
                for (Object[] a : attrs) al.add(List.of(a[0], a[1]));
                m.put("attrs", al);
            }
            return m;
        }
    }

    static Map<String, Object> op(Object... kv) {
        Map<String, Object> m = new LinkedHashMap<>();
        for (int i = 0; i < kv.length; i += 2) m.put((String) kv[i], kv[i + 1]);
        return m;
    }

    static Map<String, Object> effect(String id, int duration, int amp) {
        return op("op", "effect", "id", "minecraft:" + id, "duration", duration, "amp", amp,
                "ambient", false, "visible", true, "icon", true);
    }

    static Map<String, Object> effectFlags(String id, int duration, int amp, boolean ambient, boolean visible, boolean icon) {
        return op("op", "effect", "id", "minecraft:" + id, "duration", duration, "amp", amp,
                "ambient", ambient, "visible", visible, "icon", icon);
    }

    static Map<String, Object> hold(String item) {
        return op("op", "hold", "item", "minecraft:" + item);
    }

    static Map<String, Object> holdPotion(String item, String potion) {
        return op("op", "hold", "item", "minecraft:" + item, "potion", "minecraft:" + potion);
    }

    static Map<String, Object> hurt(String type, float amount) {
        return op("op", "hurt", "type", "minecraft:" + type, "amount", amount);
    }

    static List<Scenario> scenarios() {
        List<Scenario> out = new ArrayList<>();
        Scenario s;

        // ---- periodic effects
        for (int amp : new int[] {0, 1, 2, 5}) {
            s = new Scenario("regeneration_" + amp);
            s.health = 8f;
            s.food = 10;
            s.saturation = 0f;
            s.ticks = 130;
            s.at(1, effect("regeneration", 120, amp));
            out.add(s);
        }
        s = new Scenario("regeneration_full_health");
        s.ticks = 60;
        s.at(1, effect("regeneration", 50, 1));
        out.add(s);
        for (int amp : new int[] {0, 1, 3}) {
            s = new Scenario("poison_" + amp);
            s.food = 17;
            s.saturation = 0f;
            s.ticks = 120;
            s.at(1, effect("poison", 110, amp));
            out.add(s);
        }
        s = new Scenario("poison_stops_at_one");
        s.health = 3f;
        s.food = 17;
        s.saturation = 0f;
        s.ticks = 100;
        s.at(1, effect("poison", 90, 2));
        out.add(s);
        for (int amp : new int[] {0, 1}) {
            s = new Scenario("wither_" + amp);
            s.health = 6f;
            s.food = 17;
            s.saturation = 0f;
            s.ticks = 150;
            s.at(1, effect("wither", 140, amp));
            out.add(s);
        }
        s = new Scenario("hunger_0");
        s.ticks = 60;
        s.at(1, effect("hunger", 50, 0));
        out.add(s);
        s = new Scenario("hunger_2_starving");
        s.food = 3;
        s.saturation = 0f;
        s.ticks = 200;
        s.at(1, effect("hunger", 190, 2));
        out.add(s);

        // ---- instantaneous effects
        for (int amp : new int[] {0, 1}) {
            s = new Scenario("instant_health_" + amp);
            s.health = 5f;
            s.ticks = 5;
            s.at(1, effect("instant_health", 1, amp));
            out.add(s);
            s = new Scenario("instant_damage_" + amp);
            s.ticks = 5;
            s.at(1, effect("instant_damage", 1, amp));
            out.add(s);
        }
        s = new Scenario("instant_damage_twice");
        s.ticks = 30;
        s.at(1, effect("instant_damage", 1, 0)).at(5, effect("instant_damage", 1, 0)).at(22, effect("instant_damage", 1, 0));
        out.add(s);
        s = new Scenario("saturation_1");
        s.food = 10;
        s.saturation = 0f;
        s.ticks = 5;
        s.at(1, effect("saturation", 1, 1));
        out.add(s);
        s = new Scenario("saturation_long");
        s.food = 2;
        s.saturation = 0f;
        s.ticks = 12;
        s.at(1, effect("saturation", 8, 0));
        out.add(s);

        // ---- absorption and health boost
        for (int amp : new int[] {0, 3}) {
            s = new Scenario("absorption_" + amp);
            s.ticks = 50;
            s.at(1, effect("absorption", 40, amp));
            out.add(s);
        }
        s = new Scenario("absorption_used_up");
        s.ticks = 40;
        s.at(1, effect("absorption", 400, 0)).at(3, hurt("generic", 3f)).at(25, hurt("generic", 3f));
        out.add(s);
        s = new Scenario("absorption_readded");
        s.ticks = 30;
        s.at(1, effect("absorption", 400, 1)).at(3, hurt("generic", 5f)).at(10, effect("absorption", 100, 0));
        out.add(s);
        s = new Scenario("health_boost");
        s.ticks = 60;
        s.at(1, effect("health_boost", 40, 1)).at(2, effect("instant_health", 1, 2));
        out.add(s);
        s = new Scenario("health_boost_regen");
        s.ticks = 80;
        s.at(1, effect("health_boost", 70, 0)).at(1, effect("regeneration", 60, 3));
        out.add(s);

        // ---- resistance and damage
        for (int amp : new int[] {0, 2, 4}) {
            s = new Scenario("resistance_" + amp);
            s.ticks = 30;
            s.at(1, effect("resistance", 100, amp)).at(2, hurt("generic", 10f)).at(25, hurt("magic", 7f));
            out.add(s);
        }
        s = new Scenario("resistance_vs_starve");
        s.ticks = 5;
        s.at(1, effect("resistance", 100, 1)).at(2, hurt("starve", 6f));
        out.add(s);
        s = new Scenario("fire_resistance_vs_in_fire");
        s.ticks = 5;
        s.at(1, effect("fire_resistance", 100, 0)).at(2, hurt("in_fire", 6f)).at(3, hurt("generic", 2f));
        out.add(s);

        // ---- attribute effects
        String[][] attrEffects = {{"speed", "1"}, {"slowness", "3"}, {"haste", "1"}, {"mining_fatigue", "2"},
                {"strength", "1"}, {"weakness", "0"}, {"luck", "0"}, {"unluck", "2"}, {"jump_boost", "1"},
                {"invisibility", "0"}, {"night_vision", "0"}, {"blindness", "0"}, {"darkness", "0"},
                {"water_breathing", "0"}, {"conduit_power", "1"}, {"slow_falling", "0"}, {"levitation", "1"},
                {"glowing", "0"}, {"dolphins_grace", "0"}, {"nausea", "0"}, {"hero_of_the_village", "2"},
                {"bad_omen", "0"}, {"breath_of_the_nautilus", "0"}};
        for (String[] e : attrEffects) {
            s = new Scenario("attr_" + e[0]);
            s.ticks = 25;
            s.mainHand = "minecraft:iron_pickaxe";
            s.at(1, effect(e[0], 20, Integer.parseInt(e[1])));
            out.add(s);
        }
        s = new Scenario("haste_and_fatigue_dig");
        s.ticks = 12;
        s.mainHand = "minecraft:diamond_pickaxe";
        s.at(1, effect("haste", 100, 1)).at(4, effect("mining_fatigue", 100, 0)).at(7, effect("conduit_power", 100, 3))
                .at(9, op("op", "remove", "id", "minecraft:haste"));
        out.add(s);
        s = new Scenario("speed_and_slowness");
        s.ticks = 20;
        s.at(1, effect("speed", 10, 2)).at(3, effect("slowness", 15, 1));
        out.add(s);

        // ---- instance merging, hidden effects, infinite durations, flags
        s = new Scenario("upgrade_hides_weaker");
        s.ticks = 70;
        s.at(1, effect("speed", 60, 0)).at(5, effect("speed", 20, 2)).at(8, effect("speed", 10, 1));
        out.add(s);
        s = new Scenario("weaker_longer_hidden");
        s.ticks = 60;
        s.at(1, effect("strength", 20, 2)).at(2, effect("strength", 50, 0)).at(3, effect("strength", 40, 1));
        out.add(s);
        s = new Scenario("same_amplifier_longer");
        s.ticks = 40;
        s.at(1, effect("speed", 20, 1)).at(5, effect("speed", 30, 1)).at(6, effect("speed", 10, 1));
        out.add(s);
        s = new Scenario("flags_update");
        s.ticks = 12;
        s.at(1, effectFlags("regeneration", 100, 0, true, false, false)).at(3, effectFlags("regeneration", 5, 0, false, true, true))
                .at(5, effectFlags("regeneration", 5, 0, true, true, false));
        out.add(s);
        s = new Scenario("infinite_regeneration");
        s.health = 10f;
        s.food = 10;
        s.saturation = 0f;
        s.ticks = 60;
        s.at(1, effect("regeneration", -1, 1)).at(50, effect("regeneration", 100, 1)).at(55, effect("regeneration", -1, 1));
        out.add(s);
        s = new Scenario("infinite_over_long");
        s.ticks = 20;
        s.at(1, effect("speed", 600, 0)).at(2, effect("speed", -1, 0)).at(3, effect("speed", 50, 1));
        out.add(s);
        s = new Scenario("refresh_every_600");
        s.ticks = 12;
        s.at(1, effect("speed", 605, 0));
        out.add(s);
        s = new Scenario("remove_and_clear");
        s.ticks = 12;
        s.at(1, effect("speed", 100, 1)).at(1, effect("haste", 100, 0)).at(1, effect("luck", 100, 0))
                .at(4, op("op", "remove", "id", "minecraft:haste")).at(5, op("op", "remove", "id", "minecraft:haste"))
                .at(7, op("op", "clear"));
        out.add(s);

        // ---- fire
        s = new Scenario("burning_in_air");
        s.fire = 100;
        s.food = 17;
        s.saturation = 0f;
        s.ticks = 110;
        out.add(s);
        s = new Scenario("burning_fire_resistance");
        s.fire = 60;
        s.ticks = 70;
        s.at(1, effect("fire_resistance", 30, 0));
        out.add(s);
        s = new Scenario("burning_fire_protection");
        s.fire = 45;
        s.ticks = 50;
        s.armor = new String[] {null, "minecraft:iron_leggings", "minecraft:iron_chestplate", null};
        s.armorEnch.get(2).put("minecraft:fire_protection", 4);
        s.armorEnch.get(1).put("minecraft:fire_protection", 3);
        out.add(s);
        s = new Scenario("burning_creative");
        s.gameMode = "creative";
        s.fire = 100;
        s.ticks = 5;
        out.add(s);
        s = new Scenario("in_fire_block");
        s.food = 17;
        s.saturation = 0f;
        s.ticks = 100;
        s.block(0, -1, 0, "minecraft:netherrack").block(0, 0, 0, "minecraft:fire");
        out.add(s);
        s = new Scenario("in_soul_fire_block");
        s.food = 17;
        s.saturation = 0f;
        s.ticks = 60;
        s.block(0, -1, 0, "minecraft:soul_soil").block(0, 0, 0, "minecraft:soul_fire");
        out.add(s);
        s = new Scenario("in_fire_block_fire_protection");
        s.ticks = 60;
        s.armor = new String[] {"minecraft:diamond_boots", null, null, "minecraft:leather_helmet"};
        s.armorEnch.get(0).put("minecraft:fire_protection", 4);
        s.armorEnch.get(3).put("minecraft:fire_protection", 2);
        s.block(0, -1, 0, "minecraft:netherrack").block(0, 0, 0, "minecraft:fire");
        out.add(s);
        s = new Scenario("in_fire_then_out");
        s.ticks = 90;
        s.block(0, -1, 0, "minecraft:netherrack").block(0, 0, 0, "minecraft:fire");
        s.at(30, op("op", "setblock", "pos", List.of(0, 100, 0), "state", "minecraft:air"));
        out.add(s);
        s = new Scenario("in_lava");
        s.ticks = 60;
        s.pool("minecraft:lava", 2);
        out.add(s);
        s = new Scenario("in_lava_fire_resistance");
        s.ticks = 50;
        s.pool("minecraft:lava", 2);
        s.at(1, effect("fire_resistance", 30, 0));
        out.add(s);
        s = new Scenario("burning_into_water");
        s.fire = 200;
        s.ticks = 20;
        s.pool("minecraft:water", 1);
        out.add(s);
        // Kiln's world runs fluid ticks: the water flows into the fire after 5 ticks.
        s = new Scenario("fire_and_water_same_tick");
        s.ticks = 4;
        s.block(0, -1, 0, "minecraft:netherrack").block(0, 0, 0, "minecraft:fire").block(0, 1, 0, "minecraft:water")
                .fill(-1, 1, -1, 1, 2, -1, "minecraft:glass").fill(-1, 1, 1, 1, 2, 1, "minecraft:glass")
                .block(-1, 1, 0, "minecraft:glass").block(1, 1, 0, "minecraft:glass").block(0, 2, 0, "minecraft:glass");
        out.add(s);
        s = new Scenario("magma_block");
        s.food = 17;
        s.saturation = 0f;
        s.ticks = 50;
        s.block(0, -1, 0, "minecraft:magma_block");
        out.add(s);
        s = new Scenario("magma_block_sneaking");
        s.sneaking = true;
        s.ticks = 20;
        s.block(0, -1, 0, "minecraft:magma_block");
        out.add(s);
        s = new Scenario("campfire");
        s.ticks = 45;
        s.block(0, 0, 0, "minecraft:campfire[lit=true]");
        out.add(s);
        s = new Scenario("soul_campfire_unlit");
        s.ticks = 10;
        s.block(0, 0, 0, "minecraft:soul_campfire[lit=false]");
        out.add(s);

        // ---- air
        s = new Scenario("drowning");
        s.food = 17;
        s.saturation = 0f;
        s.ticks = 380;
        s.pool("minecraft:water", 3);
        out.add(s);
        s = new Scenario("drowning_sneaking");
        s.sneaking = true;
        s.air = 30;
        s.ticks = 90;
        s.pool("minecraft:water", 2);
        out.add(s);
        s = new Scenario("feet_in_water_breathes");
        s.air = 100;
        s.ticks = 60;
        s.pool("minecraft:water", 1);
        out.add(s);
        for (long seed : new long[] {1, 2, 3}) {
            s = new Scenario("respiration_3_" + seed);
            s.air = 40;
            s.ticks = 120;
            s.seed = seed;
            s.armor = new String[] {null, null, null, "minecraft:turtle_helmet"};
            s.armorEnch.get(3).put("minecraft:respiration", 3);
            s.pool("minecraft:water", 3);
            out.add(s);
        }
        s = new Scenario("water_breathing_underwater");
        s.air = 50;
        s.ticks = 70;
        s.pool("minecraft:water", 3);
        s.at(1, effect("water_breathing", 40, 0));
        out.add(s);
        s = new Scenario("nautilus_underwater");
        s.air = 50;
        s.ticks = 30;
        s.pool("minecraft:water", 3);
        s.at(1, effect("breath_of_the_nautilus", 100, 0));
        out.add(s);
        s = new Scenario("air_refill");
        s.air = -5;
        s.ticks = 90;
        out.add(s);
        s = new Scenario("air_refill_nautilus");
        s.air = 10;
        s.ticks = 20;
        s.at(1, effect("breath_of_the_nautilus", 100, 0)).at(10, effect("conduit_power", 100, 0));
        out.add(s);
        s = new Scenario("creative_underwater");
        s.gameMode = "creative";
        s.air = 100;
        s.ticks = 30;
        s.pool("minecraft:water", 3);
        out.add(s);

        // ---- consuming
        s = new Scenario("eat_golden_apple");
        s.health = 10f;
        s.ticks = 110;
        s.at(1, hold("golden_apple")).at(1, op("op", "finish"));
        out.add(s);
        s = new Scenario("eat_enchanted_golden_apple");
        s.health = 4f;
        s.food = 14;
        s.ticks = 50;
        s.at(1, effect("absorption", 100, 0)).at(2, hold("enchanted_golden_apple")).at(2, op("op", "finish"));
        out.add(s);
        s = new Scenario("drink_milk");
        s.ticks = 10;
        s.at(1, effect("speed", 100, 1)).at(1, effect("poison", 100, 0)).at(1, effect("health_boost", 100, 1))
                .at(3, hold("milk_bucket")).at(3, op("op", "finish"));
        out.add(s);
        s = new Scenario("drink_honey");
        s.food = 10;
        s.ticks = 10;
        s.at(1, effect("poison", 100, 0)).at(1, effect("speed", 100, 0)).at(3, hold("honey_bottle")).at(3, op("op", "finish"));
        out.add(s);
        for (long seed : new long[] {1, 2, 3, 4, 5, 6}) {
            s = new Scenario("eat_rotten_flesh_" + seed);
            s.food = 10;
            s.seed = seed;
            s.ticks = 5;
            s.at(1, hold("rotten_flesh")).at(1, op("op", "finish"));
            out.add(s);
        }
        for (long seed : new long[] {1, 7}) {
            s = new Scenario("eat_pufferfish_" + seed);
            s.food = 10;
            s.seed = seed;
            s.ticks = 20;
            s.at(1, hold("pufferfish")).at(1, op("op", "finish"));
            out.add(s);
        }
        for (long seed : new long[] {3, 11, 12}) {
            s = new Scenario("eat_raw_chicken_" + seed);
            s.food = 10;
            s.seed = seed;
            s.ticks = 3;
            s.at(1, hold("chicken")).at(1, op("op", "finish"));
            out.add(s);
        }
        s = new Scenario("eat_over_time_then_flesh");
        s.food = 6;
        s.seed = 42;
        s.ticks = 50;
        s.at(1, hold("cooked_beef")).at(1, op("op", "use")).at(40, hold("rotten_flesh")).at(40, op("op", "finish"));
        out.add(s);
        s = new Scenario("drink_over_time");
        s.seed = 5;
        s.ticks = 40;
        s.at(1, holdPotion("potion", "swiftness")).at(1, op("op", "use"));
        out.add(s);
        String[] potions = {"healing", "strong_healing", "harming", "strong_harming", "swiftness", "long_regeneration",
                "strong_poison", "turtle_master", "strong_turtle_master", "water", "awkward", "fire_resistance",
                "strong_strength", "slow_falling", "long_weakness", "luck", "infested", "oozing", "weaving", "wind_charged"};
        for (String potion : potions) {
            s = new Scenario("drink_" + potion);
            s.health = 12f;
            s.ticks = 30;
            s.at(1, holdPotion("potion", potion)).at(1, op("op", "finish"));
            out.add(s);
        }
        s = new Scenario("drink_harming_absorbed");
        s.ticks = 5;
        s.at(1, effect("absorption", 100, 1)).at(2, holdPotion("potion", "strong_harming")).at(2, op("op", "finish"));
        out.add(s);
        s = new Scenario("eat_suspicious_stew");
        s.food = 10;
        s.ticks = 20;
        s.at(1, op("op", "hold", "item", "minecraft:suspicious_stew", "stew", List.of("minecraft:saturation", 7,
                "minecraft:night_vision", 100)));
        s.at(1, op("op", "finish"));
        out.add(s);
        s = new Scenario("eat_spider_eye");
        s.ticks = 30;
        s.at(1, hold("spider_eye")).at(1, op("op", "finish"));
        out.add(s);
        playerScenarios(out);
        // The server moves a player it gets no movement from (gravity, then tickPlayer snaps it
        // back): a floor keeps it standing like a client that reports standing still.
        for (Scenario x : out) {
            boolean floor = x.blocks.stream().anyMatch(b -> (int) b[0] == 0 && (int) b[1] == -1 && (int) b[2] == 0);
            if (!floor) x.blocks.add(0, new Object[] {0, -1, 0, "minecraft:stone"});
        }
        return out;
    }

    // ---------------------------------------------------------------- wp44: player hazards
    // (scenarios driven by a shadow client; see the "shadow client" section)

    /** Ticks a free fall of `h` blocks from rest takes (client physics: move, then gravity and drag). */
    static int fallTicks(double h) {
        double v = 0, d = 0;
        int n = 0;
        while (d < h && n < 400) {
            d -= v;
            v = (v - 0.08) * 0.98;
            n++;
        }
        return n;
    }

    /** A fall from `h` blocks above the floor surface (y 100), the player starting in the air. */
    static Scenario fall(String name, double h) {
        Scenario s = new Scenario(name);
        s.client = true;
        s.onGround = false;
        s.dy = h;
        s.ticks = fallTicks(h) + 6;
        return s;
    }

    /** The landing block variants: blocks (relative to BASE) replacing the stone floor. */
    static Scenario landing(Scenario s, String kind) {
        switch (kind) {
            case "stone" -> s.block(0, -1, 0, "minecraft:stone");
            case "hay" -> s.block(0, -1, 0, "minecraft:hay_block");
            case "bed" -> s.block(0, -1, 0, "minecraft:white_bed[facing=north,part=foot]")
                    .block(0, -1, -1, "minecraft:white_bed[facing=north,part=head]");
            case "slime" -> s.block(0, -1, 0, "minecraft:slime_block").block(0, -2, 0, "minecraft:stone");
            case "honey" -> s.block(0, -1, 0, "minecraft:honey_block");
            case "cobweb" -> s.block(0, -1, 0, "minecraft:cobweb").block(0, -2, 0, "minecraft:stone");
            case "vine" -> s.block(0, -1, 0, "minecraft:vine[up=true]").block(0, -2, 0, "minecraft:stone");
            case "scaffolding" -> s.block(0, -1, 0, "minecraft:scaffolding[bottom=false,distance=0]").block(0, -2, 0, "minecraft:stone");
            case "powder_snow" -> s.block(0, -1, 0, "minecraft:powder_snow").block(0, -2, 0, "minecraft:powder_snow")
                    .block(0, -3, 0, "minecraft:stone");
            case "water" -> {
                s.fill(-1, -4, -1, 1, -1, 1, "minecraft:glass").fill(0, -3, 0, 0, -1, 0, "minecraft:water");
                s.block(0, -4, 0, "minecraft:stone");
            }
            case "water1" -> {
                s.fill(-1, -2, -1, 1, -1, 1, "minecraft:glass").block(0, -1, 0, "minecraft:water");
                s.block(0, -2, 0, "minecraft:stone");
            }
            case "lava" -> {
                s.fill(-1, -4, -1, 1, -1, 1, "minecraft:glass").fill(0, -3, 0, 0, -1, 0, "minecraft:lava");
                s.block(0, -4, 0, "minecraft:stone");
            }
            case "sweet_berry_bush" -> s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:sweet_berry_bush[age=3]");
            case "dripstone" -> s.block(0, -1, 0, "minecraft:stone")
                    .block(0, 0, 0, "minecraft:pointed_dripstone[vertical_direction=up,thickness=tip,waterlogged=false]");
            case "dripstone_frustum" -> s.block(0, -1, 0, "minecraft:stone")
                    .block(0, 0, 0, "minecraft:pointed_dripstone[vertical_direction=up,thickness=frustum,waterlogged=false]");
            case "dripstone_down" -> s.block(0, -1, 0, "minecraft:stone")
                    .block(0, 0, 0, "minecraft:pointed_dripstone[vertical_direction=down,thickness=tip,waterlogged=false]");
            case "farmland" -> s.block(0, -1, 0, "minecraft:farmland[moisture=0]");
            case "snow" -> s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:snow[layers=3]");
            case "carpet_on_hay" -> s.block(0, -1, 0, "minecraft:hay_block").block(0, 0, 0, "minecraft:white_carpet");
            case "slab_on_hay" -> s.block(0, -1, 0, "minecraft:hay_block").block(0, 0, 0, "minecraft:stone_slab[type=bottom]");
            case "fence_on_hay" -> s.block(0, -1, 0, "minecraft:hay_block").block(0, 0, 0, "minecraft:oak_fence");
            case "soul_sand" -> s.block(0, -1, 0, "minecraft:soul_sand");
            case "ice" -> s.block(0, -1, 0, "minecraft:blue_ice");
            case "bubble" -> {
                s.fill(-1, -3, -1, 1, -1, 1, "minecraft:glass").fill(0, -2, 0, 0, -1, 0, "minecraft:water");
                s.block(0, -3, 0, "minecraft:soul_sand").block(0, -2, 0, "minecraft:bubble_column[drag=false]")
                        .block(0, -1, 0, "minecraft:bubble_column[drag=false]");
            }
            case "twisting_vines" -> s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:twisting_vines[age=1]");
            case "ladder" -> s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:ladder[facing=south]")
                    .block(0, 0, -1, "minecraft:stone");
            case "cactus" -> s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
            case "magma" -> s.block(0, -1, 0, "minecraft:magma_block");
            case "stairs" -> s.block(0, -1, 0, "minecraft:stone")
                    .block(0, 0, 0, "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]");
            default -> throw new IllegalArgumentException(kind);
        }
        return s;
    }

    static void playerScenarios(List<Scenario> out) {
        Scenario s;
        // ---- fall heights on stone: every height from 2 to 40
        for (int h = 2; h <= 40; h++) {
            out.add(landing(fall("fall_stone_" + h, h), "stone"));
        }
        for (double h : new double[] {1.0, 1.4, 1.62, 2.5, 3.2, 3.9, 4.5, 6.3}) {
            out.add(landing(fall("fall_stone_frac_" + h, h), "stone"));
        }
        // ---- landing blocks
        String[] blocks = {"hay", "bed", "slime", "honey", "cobweb", "vine", "scaffolding", "powder_snow", "water", "water1",
                "lava", "sweet_berry_bush", "dripstone", "dripstone_frustum", "dripstone_down", "farmland", "snow",
                "carpet_on_hay", "slab_on_hay", "fence_on_hay", "soul_sand", "ice", "bubble", "twisting_vines", "ladder",
                "cactus", "magma", "stairs"};
        for (String b : blocks) {
            for (int h : new int[] {3, 4, 6, 10, 20, 40}) {
                out.add(landing(fall("fall_" + b + "_" + h, h), b));
            }
        }
        // ---- bouncing and sneaking
        for (String b : new String[] {"slime", "bed"}) {
            for (int h : new int[] {5, 12}) {
                s = landing(fall("fall_" + b + "_sneak_" + h, h), b);
                s.sneaking = true;
                s.ticks += 30;
                out.add(s);
                s = landing(fall("fall_" + b + "_bounce_" + h, h), b);
                s.ticks += 90;
                out.add(s);
            }
        }
        // ---- effects, enchantments and attributes on stone and hay
        int[] heights = {4, 6, 10, 20};
        for (int h : heights) {
            for (int amp = 0; amp <= 3; amp++) {
                s = landing(fall("fall_jump_boost" + amp + "_" + h, h), "stone");
                s.at(1, effect("jump_boost", 600, amp));
                out.add(s);
            }
            s = landing(fall("fall_slow_falling_" + h, h), "stone");
            s.ticks += 80;
            s.at(1, effect("slow_falling", 600, 0));
            out.add(s);
            s = landing(fall("fall_slow_falling_ends_" + h, h), "stone");
            s.ticks += 80;
            s.at(1, effect("slow_falling", 12, 0));
            out.add(s);
            for (int amp = 0; amp <= 2; amp++) {
                s = landing(fall("fall_resistance" + amp + "_" + h, h), "stone");
                s.at(1, effect("resistance", 600, amp));
                out.add(s);
            }
            s = landing(fall("fall_absorption_" + h, h), "stone");
            s.at(1, effect("absorption", 600, 1));
            out.add(s);
            for (int lvl = 1; lvl <= 4; lvl++) {
                s = landing(fall("fall_feather_falling" + lvl + "_" + h, h), "stone");
                s.armor = new String[] {"minecraft:diamond_boots", null, null, null};
                s.armorEnch.get(0).put("minecraft:feather_falling", lvl);
                out.add(s);
            }
            s = landing(fall("fall_protection4_" + h, h), "stone");
            s.armor = new String[] {"minecraft:diamond_boots", "minecraft:diamond_leggings", "minecraft:diamond_chestplate", "minecraft:diamond_helmet"};
            for (int i = 0; i < 4; i++) s.armorEnch.get(i).put("minecraft:protection", 4);
            out.add(s);
            s = landing(fall("fall_ff4_prot4_all_" + h, h), "stone");
            s.armor = new String[] {"minecraft:netherite_boots", "minecraft:netherite_leggings", "minecraft:netherite_chestplate", "minecraft:netherite_helmet"};
            s.armorEnch.get(0).put("minecraft:feather_falling", 4);
            for (int i = 0; i < 4; i++) s.armorEnch.get(i).put("minecraft:protection", 4);
            out.add(s);
            s = landing(fall("fall_ff4_hay_" + h, h), "hay");
            s.armor = new String[] {"minecraft:diamond_boots", null, null, null};
            s.armorEnch.get(0).put("minecraft:feather_falling", 4);
            out.add(s);
            for (double m : new double[] {0.0, 0.5, 2.0, 5.0}) {
                s = landing(fall("fall_multiplier_" + m + "_" + h, h), "stone");
                s.attrs.add(new Object[] {"minecraft:fall_damage_multiplier", m});
                out.add(s);
            }
            for (double m : new double[] {0.0, 1.0, 5.5, 20.0, -1.0}) {
                s = landing(fall("fall_safe_distance_" + m + "_" + h, h), "stone");
                s.attrs.add(new Object[] {"minecraft:safe_fall_distance", m});
                out.add(s);
            }
            s = landing(fall("fall_multiplier_resistance_jump_" + h, h), "stone");
            s.attrs.add(new Object[] {"minecraft:fall_damage_multiplier", 1.5});
            s.at(1, effect("jump_boost", 600, 1)).at(1, effect("resistance", 600, 0));
            out.add(s);
        }
        // ---- game modes, game rule, difficulty
        for (String mode : new String[] {"creative", "adventure", "spectator"}) {
            for (int h : new int[] {6, 40}) {
                s = landing(fall("fall_mode_" + mode + "_" + h, h), "stone");
                s.gameMode = mode;
                out.add(s);
            }
        }
        for (String diff : new String[] {"peaceful", "easy", "hard"}) {
            for (int h : new int[] {6, 20}) {
                s = landing(fall("fall_difficulty_" + diff + "_" + h, h), "stone");
                s.difficulty = diff;
                out.add(s);
            }
        }
        s = landing(fall("fall_rule_off_10", 10), "stone");
        s.at(1, op("op", "gamerule", "name", "fall_damage", "value", "false"));
        s.at(s.ticks - 1, op("op", "gamerule", "name", "fall_damage", "value", "true"));
        out.add(s);
        // ---- low health, death
        s = landing(fall("fall_low_health_10", 10), "stone");
        s.health = 4f;
        out.add(s);
        s = landing(fall("fall_death_40", 40), "stone");
        s.health = 10f;
        out.add(s);
        // ---- jumps from the ground
        for (int amp = -1; amp <= 3; amp++) {
            s = new Scenario("jump_" + amp);
            s.client = true;
            s.ticks = 40;
            if (amp >= 0) s.at(1, effect("jump_boost", 600, amp));
            s.at(2, op("op", "jump"));
            out.add(s);
        }
        // ---- landing on the edge of two blocks: the supporting block is the nearer one
        for (double dx : new double[] {0.29, 0.49, 0.51, 0.71, 0.99}) {
            for (String b : new String[] {"hay", "slime", "bed"}) {
                s = fall("fall_edge_" + b + "_" + dx, 8);
                s.dx = dx;
                s.block(0, -1, 0, b.equals("hay") ? "minecraft:hay_block" : b.equals("slime") ? "minecraft:slime_block"
                        : "minecraft:white_bed[facing=east,part=foot]");
                if (b.equals("bed")) s.block(1, -1, 0, "minecraft:white_bed[facing=east,part=head]");
                else s.block(1, -1, 0, "minecraft:stone");
                out.add(s);
            }
        }
        // ---- stepping off a ledge (the client keeps its momentum)
        for (int drop : new int[] {2, 3, 4, 5, 9}) {
            s = new Scenario("ledge_" + drop);
            s.client = true;
            s.ticks = 30 + fallTicks(drop);
            s.block(0, -1, 0, "minecraft:stone").fill(1, -1 - drop, -1, 3, -1 - drop, 1, "minecraft:stone");
            s.at(2, op("op", "velocity", "x", 0.2, "y", 0.0, "z", 0.0));
            out.add(s);
        }
        hazardScenarios(out);
        survivalScenarios(out);
        enchantLocationScenarios(out);
    }

    /** wp49: soul speed and frost walker boots (`location_changed`, `tick`). */
    static Scenario boots(String name, int ticks, String enchantment, int level) {
        Scenario s = hazard(name, ticks);
        s.armor[0] = "minecraft:netherite_boots";
        s.armorEnch.get(0).put("minecraft:" + enchantment, level);
        s.watchBoots = true;
        return s;
    }

    static void enchantLocationScenarios(List<Scenario> out) {
        Scenario s;
        // ---- soul speed: standing on soul sand, soul soil, stone; walking on and off; jumping; the wear of the boots
        for (int level = 1; level <= 3; level++) {
            s = boots("ench_soul_speed_" + level + "_stand", 30, "soul_speed", level);
            s.block(0, -1, 0, "minecraft:soul_sand");
            out.add(s);
        }
        s = boots("ench_soul_speed_soil", 20, "soul_speed", 2);
        s.block(0, -1, 0, "minecraft:soul_soil");
        out.add(s);
        s = boots("ench_soul_speed_stone", 20, "soul_speed", 3);
        s.block(0, -1, 0, "minecraft:stone");
        out.add(s);
        s = boots("ench_soul_speed_walk", 70, "soul_speed", 2);
        s.block(0, -1, 0, "minecraft:stone").fill(1, -1, 0, 3, -1, 0, "minecraft:soul_sand").fill(4, -1, 0, 8, -1, 0, "minecraft:stone");
        walkInto(s, 0.25, 3, 60, 2);
        out.add(s);
        s = boots("ench_soul_speed_jump", 60, "soul_speed", 3);
        s.fill(0, -1, 0, 2, -1, 0, "minecraft:soul_sand");
        s.at(10, op("op", "jump"));
        s.at(30, op("op", "jump"));
        walkInto(s, 0.1, 10, 50, 3);
        out.add(s);
        s = boots("ench_soul_speed_wear", 160, "soul_speed", 3);
        s.fill(0, -1, 0, 12, -1, 0, "minecraft:soul_sand");
        walkInto(s, 0.2, 3, 150, 2);
        out.add(s);
        s = boots("ench_soul_speed_flying", 30, "soul_speed", 3);
        s.gameMode = "creative";
        s.block(0, -1, 0, "minecraft:soul_sand");
        out.add(s);
        // The boots come off and go on again.
        s = boots("ench_soul_speed_swap", 40, "soul_speed", 3);
        s.block(0, -1, 0, "minecraft:soul_sand");
        s.at(10, op("op", "armor", "slot", 0, "item", ""));
        s.at(20, op("op", "armor", "slot", 0, "item", "minecraft:netherite_boots", "enchant", "minecraft:soul_speed", "level", 2));
        out.add(s);
        // ---- frost walker
        for (int level = 1; level <= 2; level++) {
            s = boots("ench_frost_walker_" + level + "_pool", 40, "frost_walker", level);
            s.block(0, -1, 0, "minecraft:stone").fill(1, -2, -5, 7, -2, 5, "minecraft:stone").fill(1, -1, -5, 7, -1, 5, "minecraft:water[level=0]");
            for (int x = 1; x <= 6; x++) s.watch(x, -1, 0);
            s.watch(1, -1, 3).watch(3, -1, 2).watch(4, -1, 1);
            out.add(s);
        }
        s = boots("ench_frost_walker_walk", 90, "frost_walker", 1);
        s.fill(0, -1, 0, 2, -1, 0, "minecraft:stone").fill(3, -2, -3, 14, -2, 3, "minecraft:stone").fill(3, -1, -3, 14, -1, 3, "minecraft:water[level=0]");
        for (int x = 3; x <= 12; x++) s.watch(x, -1, 0);
        walkInto(s, 0.2, 3, 80, 2);
        out.add(s);
        s = boots("ench_frost_walker_flowing", 30, "frost_walker", 2);
        s.block(0, -1, 0, "minecraft:stone").fill(1, -2, -2, 4, -2, 2, "minecraft:stone").fill(1, -1, -2, 4, -1, 2, "minecraft:water[level=2]");
        s.block(2, -1, 0, "minecraft:water[level=0]").block(3, 0, 0, "minecraft:stone").block(2, 0, 1, "minecraft:glass");
        for (int x = 1; x <= 4; x++) s.watch(x, -1, 0);
        s.watch(2, -1, 1).watch(2, -1, -1);
        out.add(s);
        s = boots("ench_frost_walker_air", 30, "frost_walker", 2);
        s.onGround = false;
        s.dy = 6;
        s.fill(0, -2, -3, 6, -2, 3, "minecraft:stone").fill(0, -1, -3, 6, -1, 3, "minecraft:water[level=0]");
        for (int x = 0; x <= 4; x++) s.watch(x, -1, 0);
        out.add(s);
        // Frost walker keeps the feet off magma, and a campfire.
        s = boots("ench_frost_walker_magma", 60, "frost_walker", 1);
        s.block(0, -1, 0, "minecraft:magma_block");
        out.add(s);
        s = hazard("ench_no_frost_walker_magma", 60);
        s.armor[0] = "minecraft:netherite_boots";
        s.block(0, -1, 0, "minecraft:magma_block");
        out.add(s);
    }

    // ---------------------------------------------------------------- wp44: block hazards
    // Cactus, sweet berries, wither roses, powder snow (freezing) and suffocation in walls,
    // with and without protections, per difficulty and game mode.

    static final String[] ARMOR_FULL_DIAMOND = {"minecraft:diamond_boots", "minecraft:diamond_leggings",
            "minecraft:diamond_chestplate", "minecraft:diamond_helmet"};

    static Scenario hazard(String name, int ticks) {
        Scenario s = new Scenario(name);
        s.client = true;
        s.ticks = ticks;
        return s;
    }

    /** Pushes the client toward +x now and then (a player walking into something). */
    static Scenario walkInto(Scenario s, double v, int from, int to, int every) {
        for (int t = from; t <= to; t += every) s.at(t, op("op", "velocity", "x", v, "y", 0.0, "z", 0.0));
        return s;
    }

    static void hazardScenarios(List<Scenario> out) {
        Scenario s;
        // ---- cactus: standing in it, and walking into one
        s = hazard("haz_cactus_inside", 60);
        s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
        out.add(s);
        s = hazard("haz_cactus_walk", 80);
        s.block(0, -1, 0, "minecraft:stone").block(1, -2, 0, "minecraft:stone").block(1, -1, 0, "minecraft:sand").block(1, 0, 0, "minecraft:cactus[age=3]");
        walkInto(s, 0.4, 3, 40, 2);
        out.add(s);
        s = hazard("haz_cactus_walk_sneaking", 80);
        s.sneaking = true;
        s.block(0, -1, 0, "minecraft:stone").block(1, -2, 0, "minecraft:stone").block(1, -1, 0, "minecraft:sand").block(1, 0, 0, "minecraft:cactus[age=3]");
        walkInto(s, 0.2, 3, 40, 2);
        out.add(s);
        s = hazard("haz_cactus_armor", 60);
        s.armor = ARMOR_FULL_DIAMOND;
        s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
        out.add(s);
        s = hazard("haz_cactus_protection4", 60);
        s.armor = ARMOR_FULL_DIAMOND;
        for (int i = 0; i < 4; i++) s.armorEnch.get(i).put("minecraft:protection", 4);
        s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
        out.add(s);
        s = hazard("haz_cactus_resistance", 60);
        s.at(1, effect("resistance", 400, 1));
        s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
        out.add(s);
        s = hazard("haz_cactus_absorption", 60);
        s.at(1, effect("absorption", 400, 0));
        s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
        out.add(s);
        for (String mode : new String[] {"creative", "adventure", "spectator"}) {
            s = hazard("haz_cactus_" + mode, 40);
            s.gameMode = mode;
            s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
            out.add(s);
        }
        for (String diff : new String[] {"peaceful", "easy", "hard"}) {
            s = hazard("haz_cactus_" + diff, 40);
            s.difficulty = diff;
            s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
            out.add(s);
        }
        s = hazard("haz_cactus_low_health", 60);
        s.health = 3f;
        s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]");
        out.add(s);
        s = hazard("haz_cactus_two_tall", 40);
        s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand").block(0, 0, 0, "minecraft:cactus[age=0]").block(0, 1, 0, "minecraft:cactus[age=0]");
        out.add(s);

        // ---- sweet berry bushes: slow, and hurt a player that moves in them
        for (int age = 0; age <= 3; age++) {
            s = hazard("haz_berry_still_" + age, 40);
            s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:sweet_berry_bush[age=" + age + "]");
            out.add(s);
            s = hazard("haz_berry_walk_" + age, 80);
            s.block(0, -1, 0, "minecraft:stone").block(1, -1, 0, "minecraft:stone").block(-1, -1, 0, "minecraft:stone")
                    .block(0, 0, 0, "minecraft:sweet_berry_bush[age=" + age + "]");
            s.dx = -0.4;
            walkInto(s, 0.1, 3, 60, 3);
            out.add(s);
        }
        s = hazard("haz_berry_walk_armor", 80);
        s.armor = ARMOR_FULL_DIAMOND;
        s.block(0, -1, 0, "minecraft:stone").block(1, -1, 0, "minecraft:stone").block(-1, -1, 0, "minecraft:stone")
                .block(0, 0, 0, "minecraft:sweet_berry_bush[age=3]");
        s.dx = -0.4;
        walkInto(s, 0.1, 3, 60, 3);
        out.add(s);
        s = hazard("haz_berry_walk_sneaking", 80);
        s.sneaking = true;
        s.block(0, -1, 0, "minecraft:stone").block(1, -1, 0, "minecraft:stone").block(-1, -1, 0, "minecraft:stone")
                .block(0, 0, 0, "minecraft:sweet_berry_bush[age=3]");
        s.dx = -0.4;
        walkInto(s, 0.1, 3, 60, 3);
        out.add(s);
        s = hazard("haz_berry_walk_creative", 60);
        s.gameMode = "creative";
        s.block(0, -1, 0, "minecraft:stone").block(1, -1, 0, "minecraft:stone").block(-1, -1, 0, "minecraft:stone")
                .block(0, 0, 0, "minecraft:sweet_berry_bush[age=3]");
        s.dx = -0.4;
        walkInto(s, 0.1, 3, 40, 3);
        out.add(s);
        s = hazard("haz_berry_jump_in", 60);
        s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:sweet_berry_bush[age=3]");
        s.dy = 4;
        s.onGround = false;
        out.add(s);

        // ---- wither rose: wither for 8 seconds unless peaceful or invulnerable
        for (String diff : new String[] {"peaceful", "easy", "normal", "hard"}) {
            s = hazard("haz_wither_rose_" + diff, 120);
            s.difficulty = diff;
            s.block(0, -1, 0, "minecraft:dirt").block(0, 0, 0, "minecraft:wither_rose");
            out.add(s);
        }
        for (String mode : new String[] {"creative", "spectator", "adventure"}) {
            s = hazard("haz_wither_rose_" + mode, 60);
            s.gameMode = mode;
            s.block(0, -1, 0, "minecraft:dirt").block(0, 0, 0, "minecraft:wither_rose");
            out.add(s);
        }
        s = hazard("haz_wither_rose_walk", 200);
        s.fill(-1, -1, -1, 4, -1, 1, "minecraft:dirt").block(1, 0, 0, "minecraft:wither_rose");
        walkInto(s, 0.25, 3, 6, 1);
        out.add(s);
        s = hazard("haz_wither_rose_long", 400);
        s.health = 20f;
        s.food = 18;
        s.block(0, -1, 0, "minecraft:dirt").block(0, 0, 0, "minecraft:wither_rose");
        out.add(s);
        s = hazard("haz_wither_rose_potted", 60);
        s.block(0, -1, 0, "minecraft:dirt").block(0, 0, 0, "minecraft:potted_wither_rose");
        out.add(s);

        // ---- powder snow: freezing
        String[][] snowLayouts = {{"sink", "none"}, {"boots_top", "leather_boots"}, {"helmet", "leather_helmet"},
                {"leggings", "leather_leggings"}, {"chest", "leather_chestplate"}, {"iron_boots", "iron_boots"}};
        for (String[] l : snowLayouts) {
            s = hazard("haz_snow_" + l[0], 360);
            snowColumn(s);
            if (l[0].equals("boots_top")) s.dy = 1;
            if (!l[1].equals("none")) {
                String slot = l[1].contains("boots") ? "boots" : l[1].contains("helmet") ? "helmet" : l[1].contains("leggings") ? "leggings" : "chestplate";
                s.armor = new String[] {slot.equals("boots") ? "minecraft:" + l[1] : null, slot.equals("leggings") ? "minecraft:" + l[1] : null,
                        slot.equals("chestplate") ? "minecraft:" + l[1] : null, slot.equals("helmet") ? "minecraft:" + l[1] : null};
            }
            out.add(s);
        }
        s = hazard("haz_snow_thaw", 260);
        snowColumn(s);
        s.at(100, op("op", "setblock", "pos", List.of(0, 100, 0), "state", "minecraft:air"));
        s.at(100, op("op", "setblock", "pos", List.of(0, 99, 0), "state", "minecraft:air"));
        out.add(s);
        s = hazard("haz_snow_leave_early", 200);
        snowColumn(s);
        s.at(60, op("op", "setblock", "pos", List.of(0, 100, 0), "state", "minecraft:air"));
        s.at(60, op("op", "setblock", "pos", List.of(0, 99, 0), "state", "minecraft:air"));
        out.add(s);
        s = hazard("haz_snow_rule_off", 360);
        snowColumn(s);
        s.at(1, op("op", "gamerule", "name", "freeze_damage", "value", "false"));
        out.add(s);
        for (String mode : new String[] {"creative", "adventure", "spectator"}) {
            s = hazard("haz_snow_" + mode, 300);
            s.gameMode = mode;
            snowColumn(s);
            out.add(s);
        }
        s = hazard("haz_snow_burning", 60);
        snowColumn(s);
        s.fire = 200;
        out.add(s);
        s = hazard("haz_snow_fire_resistance_burning", 60);
        snowColumn(s);
        s.fire = 200;
        s.at(1, effect("fire_resistance", 400, 0));
        out.add(s);
        s = hazard("haz_snow_armor", 360);
        s.armor = ARMOR_FULL_DIAMOND;
        snowColumn(s);
        out.add(s);
        s = hazard("haz_snow_hard", 360);
        s.difficulty = "hard";
        snowColumn(s);
        out.add(s);
        s = hazard("haz_snow_low_health", 360);
        s.health = 3f;
        snowColumn(s);
        out.add(s);
        s = hazard("haz_snow_stand_on_top_no_boots", 60);
        s.dy = 3;
        s.onGround = false;
        snowColumn(s);
        out.add(s);
        s = hazard("haz_snow_fall_in", 200);
        s.dy = 8;
        s.onGround = false;
        snowColumn(s);
        out.add(s);
        s = hazard("haz_snow_lava_clears", 80);
        snowColumn(s);
        s.at(30, op("op", "setblock", "pos", List.of(0, 99, 0), "state", "minecraft:lava"));
        out.add(s);

        // ---- suffocation in walls
        String[] walls = {"stone", "dirt", "sand", "gravel", "glass", "oak_leaves", "stone_slab[type=top]", "stone_slab[type=bottom]",
                "oak_stairs[facing=north,half=top,shape=straight]", "iron_bars", "barrier", "honey_block", "slime_block", "tinted_glass",
                "ice", "soul_sand", "anvil", "chest[facing=north]", "bookshelf", "snow[layers=8]", "snow[layers=3]", "oak_trapdoor[facing=north,half=top,open=false]",
                "oak_fence", "cobblestone_wall", "farmland[moisture=0]", "dirt_path", "oak_door[facing=north,half=lower]", "white_bed[facing=north,part=foot]",
                "cactus", "magma_block", "composter", "cauldron", "hopper", "oak_log", "spawner", "sculk_sensor", "campfire[lit=false]"};
        for (String w : walls) {
            String id = w.contains("[") ? w.substring(0, w.indexOf('[')) : w;
            s = hazard("haz_wall_" + id + (w.contains("=") ? "_" + Math.abs(w.hashCode() % 1000) : ""), 45);
            s.onGround = true;
            // (A cactus needs sand under it, and the sand something under that.)
            if (w.equals("cactus")) s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:sand");
            else s.block(0, -1, 0, "minecraft:stone");
            s.block(0, 0, 0, "minecraft:" + w).block(0, 1, 0, "minecraft:" + w);
            out.add(s);
        }
        s = hazard("haz_wall_head_only", 45);
        s.block(0, -1, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone");
        out.add(s);
        s = hazard("haz_wall_head_only_sneaking", 45);
        s.sneaking = true;
        s.block(0, -1, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone");
        out.add(s);
        s = hazard("haz_wall_ceiling_slab", 45);
        s.block(0, -1, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone_slab[type=bottom]");
        out.add(s);
        s = hazard("haz_wall_ceiling_slab_top", 45);
        s.block(0, -1, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone_slab[type=top]");
        out.add(s);
        s = hazard("haz_wall_edge", 45);
        s.dx = 0.49;
        s.block(0, -1, 0, "minecraft:stone").block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:stone").block(1, 1, 0, "minecraft:stone");
        out.add(s);
        s = hazard("haz_wall_edge_in", 45);
        s.dx = 0.45;
        s.block(0, -1, 0, "minecraft:stone").block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:stone").block(1, 1, 0, "minecraft:stone");
        out.add(s);
        for (String mode : new String[] {"creative", "adventure", "spectator"}) {
            s = hazard("haz_wall_stone_" + mode, 45);
            s.gameMode = mode;
            s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone");
            out.add(s);
        }
        for (String diff : new String[] {"peaceful", "easy", "hard"}) {
            s = hazard("haz_wall_stone_" + diff, 45);
            s.difficulty = diff;
            s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone");
            out.add(s);
        }
        s = hazard("haz_wall_stone_armor", 45);
        s.armor = ARMOR_FULL_DIAMOND;
        s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone");
        out.add(s);
        s = hazard("haz_wall_stone_protection4", 45);
        s.armor = ARMOR_FULL_DIAMOND;
        for (int i = 0; i < 4; i++) s.armorEnch.get(i).put("minecraft:protection", 4);
        s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone");
        out.add(s);
        s = hazard("haz_wall_stone_low_health", 60);
        s.health = 3f;
        s.block(0, -1, 0, "minecraft:stone").block(0, 0, 0, "minecraft:stone").block(0, 1, 0, "minecraft:stone");
        out.add(s);
        s = hazard("haz_wall_placed_over_player", 60);
        s.block(0, -1, 0, "minecraft:stone");
        s.at(10, op("op", "setblock", "pos", List.of(0, 101, 0), "state", "minecraft:stone"));
        s.at(30, op("op", "setblock", "pos", List.of(0, 101, 0), "state", "minecraft:air"));
        out.add(s);
    }

    // ---------------------------------------------------------------- wp44: totems and regeneration

    static Map<String, Object> offhand(String item) {
        return op("op", "offhand", "item", "minecraft:" + item);
    }

    static void survivalScenarios(List<Scenario> out) {
        Scenario s;
        String totem = "totem_of_undying";
        // ---- the totem of undying: lethal damage in either hand
        s = new Scenario("totem_main_hand");
        s.ticks = 60;
        s.at(1, hold(totem)).at(2, hurt("generic", 30f));
        out.add(s);
        s = new Scenario("totem_off_hand");
        s.ticks = 60;
        s.at(1, offhand(totem)).at(2, hurt("generic", 30f));
        out.add(s);
        s = new Scenario("totem_both_hands");
        s.ticks = 60;
        s.at(1, hold(totem)).at(1, offhand(totem)).at(2, hurt("generic", 30f)).at(10, hurt("generic", 30f)).at(25, hurt("generic", 30f));
        out.add(s);
        s = new Scenario("totem_none");
        s.ticks = 20;
        s.at(2, hurt("generic", 30f));
        out.add(s);
        s = new Scenario("totem_not_lethal");
        s.ticks = 20;
        s.at(1, hold(totem)).at(2, hurt("generic", 5f));
        out.add(s);
        s = new Scenario("totem_void");
        s.ticks = 20;
        s.at(1, hold(totem)).at(2, hurt("out_of_world", 30f));
        out.add(s);
        s = new Scenario("totem_kill");
        s.ticks = 20;
        s.at(1, hold(totem)).at(2, hurt("generic_kill", 30f));
        out.add(s);
        s = new Scenario("totem_fire");
        s.ticks = 40;
        s.at(1, hold(totem)).at(2, hurt("in_fire", 30f)).at(3, hurt("magic", 30f));
        out.add(s);
        s = new Scenario("totem_with_effects");
        s.ticks = 80;
        s.at(1, effect("poison", 400, 1)).at(1, effect("speed", 400, 1)).at(1, effect("absorption", 400, 0)).at(1, hold(totem))
                .at(5, hurt("generic", 40f));
        out.add(s);
        s = new Scenario("totem_regeneration_window");
        s.ticks = 1000;
        s.at(1, hold(totem)).at(2, hurt("generic", 30f));
        out.add(s);
        s = new Scenario("totem_creative");
        s.gameMode = "creative";
        s.ticks = 20;
        s.at(1, hold(totem)).at(2, hurt("generic", 30f));
        out.add(s);
        s = new Scenario("totem_low_health_effect_tick");
        s.health = 2f;
        s.ticks = 60;
        s.at(1, effect("poison", 400, 4)).at(1, effect("wither", 400, 2)).at(1, hold(totem));
        out.add(s);
        s = new Scenario("totem_resistance");
        s.ticks = 20;
        s.at(1, effect("resistance", 400, 4)).at(1, hold(totem)).at(2, hurt("generic", 200f));
        out.add(s);
        s = fall("totem_fall", 30);
        s.at(1, hold(totem));
        out.add(s);
        s = new Scenario("totem_drowning");
        s.air = 5;
        s.ticks = 120;
        s.pool("minecraft:water", 3);
        s.health = 2f;
        s.at(1, hold(totem));
        out.add(s);

        // ---- natural regeneration, saturation and starvation
        for (String diff : new String[] {"peaceful", "easy", "normal", "hard"}) {
            for (int food : new int[] {20, 19, 18, 17, 0}) {
                for (float sat : new float[] {0f, 5f}) {
                    if (food != 20 && sat > 0f && food != 18) continue;
                    s = new Scenario("regen_" + diff + "_f" + food + "_s" + (int) sat);
                    s.difficulty = diff;
                    s.food = food;
                    s.saturation = sat;
                    s.health = food == 0 ? 14f : 6f;
                    s.ticks = food == 0 ? 400 : 260;
                    out.add(s);
                }
            }
        }
        s = new Scenario("regen_hard_starve_to_death");
        s.difficulty = "hard";
        s.food = 0;
        s.saturation = 0f;
        s.health = 5f;
        s.ticks = 600;
        out.add(s);
        s = new Scenario("regen_easy_starve_floor");
        s.difficulty = "easy";
        s.food = 0;
        s.saturation = 0f;
        s.health = 11f;
        s.ticks = 800;
        out.add(s);
        s = new Scenario("regen_normal_starve_floor");
        s.difficulty = "normal";
        s.food = 0;
        s.saturation = 0f;
        s.health = 4f;
        s.ticks = 800;
        out.add(s);
        s = new Scenario("regen_full_saturation_burst");
        s.food = 20;
        s.saturation = 20f;
        s.health = 1f;
        s.ticks = 300;
        out.add(s);
        s = new Scenario("regen_hunger_effect");
        s.food = 20;
        s.saturation = 0f;
        s.health = 10f;
        s.ticks = 400;
        s.at(1, effect("hunger", 400, 3));
        out.add(s);
        s = new Scenario("regen_rule_off");
        s.food = 20;
        s.saturation = 5f;
        s.health = 10f;
        s.ticks = 200;
        s.at(1, op("op", "gamerule", "name", "natural_health_regeneration", "value", "false"));
        out.add(s);
        s = new Scenario("regen_creative");
        s.gameMode = "creative";
        s.health = 10f;
        s.ticks = 200;
        out.add(s);
        s = new Scenario("regen_exhaustion_damage");
        s.food = 20;
        s.saturation = 0f;
        s.ticks = 300;
        s.at(5, hurt("generic", 4f)).at(60, hurt("in_fire", 2f)).at(120, hurt("magic", 3f));
        out.add(s);
    }

    /** Two blocks of powder snow over a stone floor, the player at the top. */
    static void snowColumn(Scenario s) {
        s.block(0, -2, 0, "minecraft:stone").block(0, -1, 0, "minecraft:powder_snow").block(0, 0, 0, "minecraft:powder_snow");
    }

    // ---------------------------------------------------------------- registries

    static Map<String, Object> registries() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", "@registries");
        List<Object> effects = new ArrayList<>();
        for (MobEffect e : BuiltInRegistries.MOB_EFFECT) {
            Map<String, Object> x = new LinkedHashMap<>();
            x.put("id", BuiltInRegistries.MOB_EFFECT.getKey(e).toString());
            x.put("raw", BuiltInRegistries.MOB_EFFECT.getId(e));
            x.put("class", e.getClass().getSimpleName());
            x.put("category", e.getCategory().name());
            x.put("color", e.getColor());
            x.put("instantaneous", e.isInstantaneous());
            List<Object> mods = new ArrayList<>();
            e.createModifiers(0, (attr, mod) -> mods.add(List.of(attr.unwrapKey().orElseThrow().identifier().toString(),
                    mod.id().toString(), mod.amount(), mod.operation().name())));
            x.put("modifiers", mods);
            effects.add(x);
        }
        m.put("effects", effects);
        List<Object> potions = new ArrayList<>();
        for (var p : BuiltInRegistries.POTION) {
            Map<String, Object> x = new LinkedHashMap<>();
            x.put("id", BuiltInRegistries.POTION.getKey(p).toString());
            List<Object> list = new ArrayList<>();
            for (MobEffectInstance i : p.getEffects()) {
                list.add(List.of(i.getEffect().unwrapKey().orElseThrow().identifier().toString(), i.getDuration(), i.getAmplifier(),
                        i.isAmbient(), i.isVisible(), i.showIcon()));
            }
            x.put("effects", list);
            potions.add(x);
        }
        m.put("potions", potions);
        return m;
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
        String filter = args.length > 1 ? args[1] : null;
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "EffectVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        List<Scenario> selected = new ArrayList<>();
        for (Scenario s : scenarios()) {
            if (filter == null || s.name.contains(filter)) selected.add(s);
        }
        System.out.println("EffectVectors: " + selected.size() + " scenarios");
        server.submit(() -> {
            ServerLevel level = server.overworld();
            level.tickRateManager().setFrozen(true);
            for (int cx = -2; cx <= 2; cx++)
                for (int cz = -2; cz <= 2; cz++) {
                    level.setChunkForced(cx, cz, true);
                    level.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(2000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            lines.add(toJson(registries()));
            for (Scenario s : selected) {
                try {
                    lines.add(run(server, s));
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("{\"name\":\"" + s.name + "\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
                }
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("EffectVectors: wrote " + (lines.size() - 1) + " scenarios to " + outPath);
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

    // ---------------------------------------------------------------- one scenario

    static int players;

    static ServerPlayer mockPlayer(MinecraftServer server, String name) {
        CommonListenerCookie cookie = CommonListenerCookie.createInitial(
                new GameProfile(UUID.nameUUIDFromBytes(name.getBytes()), name), false);
        ServerPlayer p = new ServerPlayer(server, server.overworld(), cookie.gameProfile(), cookie.clientInformation());
        Connection connection = new Connection(PacketFlow.SERVERBOUND);
        new EmbeddedChannel(connection);
        server.getPlayerList().placeNewPlayer(connection, p, cookie);
        return p;
    }

    static EmbeddedChannel channel(ServerPlayer p) throws Exception {
        Object connection = get(p.connection, "connection");
        return (EmbeddedChannel) get(connection, "channel");
    }

    static List<Object> drain(ServerPlayer p) throws Exception {
        List<Object> out = new ArrayList<>();
        EmbeddedChannel ch = channel(p);
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

    static Holder<MobEffect> effectHolder(String id) {
        return BuiltInRegistries.MOB_EFFECT.getOrThrow(ResourceKey.create(Registries.MOB_EFFECT, Identifier.parse(id)));
    }

    static ItemStack stack(MinecraftServer server, String item, Map<String, Integer> ench) {
        ItemStack s = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(item)));
        for (var e : ench.entrySet()) {
            var holder = server.registryAccess().lookupOrThrow(Registries.ENCHANTMENT)
                    .getOrThrow(ResourceKey.create(Registries.ENCHANTMENT, Identifier.parse(e.getKey())));
            s.enchant(holder, e.getValue());
        }
        return s;
    }

    static void command(MinecraftServer server, String cmd) {
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), cmd);
    }

    static void setup(MinecraftServer server, ServerPlayer p, Scenario s) throws Exception {
        p.setGameMode(GameType.byName(s.gameMode));
        call(p.connection, "markClientLoaded");
        p.snapTo(BX + s.dx, BY + s.dy, BZ + s.dz, 0f, 0f);
        p.setDeltaMovement(Vec3.ZERO);
        p.setOnGround(s.onGround);
        p.fallDistance = 0.0;
        if (s.sneaking) {
            p.setShiftKeyDown(true);
            p.setPose(Pose.CROUCHING);
        }
        p.getInventory().clearContent();
        p.getInventory().setSelectedSlot(0);
        ItemStack main = s.mainHand == null ? ItemStack.EMPTY : stack(server, s.mainHand, Map.of());
        p.setItemSlot(EquipmentSlot.MAINHAND, main);
        EquipmentSlot[] slots = {EquipmentSlot.FEET, EquipmentSlot.LEGS, EquipmentSlot.CHEST, EquipmentSlot.HEAD};
        for (int i = 0; i < 4; i++) {
            p.setItemSlot(slots[i], s.armor[i] == null ? ItemStack.EMPTY : stack(server, s.armor[i], s.armorEnch.get(i)));
        }
        call(p, "detectEquipmentUpdates");
        p.removeAllEffects();
        p.setHealth(s.health);
        p.setAbsorptionAmount(s.absorption);
        p.damageCooldownTime = 0;
        set(p, "lastHurt", 0f);
        p.getFoodData().setFoodLevel(s.food);
        p.getFoodData().setSaturation(s.saturation);
        set(p.getFoodData(), "exhaustionLevel", 0f);
        set(p.getFoodData(), "tickTimer", 0);
        p.setAirSupply(s.air);
        p.setRemainingFireTicks(s.fire);
        for (Object[] at : s.attrs) setAttribute(p, (String) at[0], ((Number) at[1]).doubleValue());
        p.tickCount = 0;
        p.getCombatTracker().recheckStatus();
    }

    static String run(MinecraftServer server, Scenario s) throws Exception {
        ServerLevel level = server.overworld();
        command(server, "difficulty " + s.difficulty);
        // Rules scenarios toggle go back to their defaults.
        command(server, "gamerule fall_damage true");
        command(server, "gamerule freeze_damage true");
        command(server, "gamerule natural_health_regeneration true");
        for (Object[] b : s.blocks) {
            command(server, String.format(Locale.ROOT, "setblock %d %d %d %s", BASE[0] + (int) b[0], BASE[1] + (int) b[1],
                    BASE[2] + (int) b[2], b[3]));
        }
        ServerPlayer p = mockPlayer(server, "Effects" + players++);
        setup(server, p, s);
        drain(p);
        level.getRandom().setSeed(s.seed);
        p.getRandom().setSeed(s.seed + 1);
        List<Object> ticks = new ArrayList<>();
        ServerPlayer shadowPlayer = s.client ? startClient(server, p, s) : null;
        shadow = shadowPlayer;
        ClientState cs = new ClientState();
        for (int t = 1; t <= s.ticks; t++) {
            for (Map<String, Object> a : s.actions.getOrDefault(t, List.of())) act(server, p, a);
            if (shadowPlayer != null) clientTick(p, shadowPlayer, s, cs);
            if (System.getenv("KILN_DEBUG_MOVES") != null && s.name.contains(System.getenv("KILN_DEBUG_MOVES"))) {
                System.err.println("DBG " + s.name + " tick " + t + " pos " + p.position() + " delta " + p.getDeltaMovement() + " onGround " + p.onGround() + " shadowGround " + (shadow == null ? null : shadow.onGround()) + " moves " + get(p, "movementThisTick"));
            }
            boolean dbg = System.getenv("KILN_DEBUG_MOVES") != null && s.name.contains(System.getenv("KILN_DEBUG_MOVES"));
            if (dbg) System.err.println("DBG   fire before commonTick " + p.getRemainingFireTicks());
            p.commonTick();
            if (dbg) System.err.println("DBG   fire after commonTick " + p.getRemainingFireTicks());
            p.tick();
            if (dbg) System.err.println("DBG   fire after tick " + p.getRemainingFireTicks());
            call(p.connection, "tickPlayer");
            if (System.getenv("KILN_DEBUG_MOVES") != null && s.name.contains(System.getenv("KILN_DEBUG_MOVES"))) {
                System.err.println("DBG   after tick " + t + " pos " + p.position() + " final " + get(p, "finalMovementsThisTick") + " frozen " + p.getTicksFrozen());
            }
            if (shadowPlayer != null) p.connection.handleClientTickEnd(ServerboundClientTickEndPacket.INSTANCE);
            Map<String, Object> st = state(p);
            if (!s.watch.isEmpty()) {
                List<Object> seen = new ArrayList<>();
                for (int[] w : s.watch) seen.add(blockString(level.getBlockState(new net.minecraft.core.BlockPos(BASE[0] + w[0], BASE[1] + w[1], BASE[2] + w[2]))));
                st.put("blocks", seen);
            }
            if (s.watchBoots) st.put("boots_damage", p.getItemBySlot(EquipmentSlot.FEET).getDamageValue());
            ticks.add(st);
        }
        if (shadowPlayer != null) server.getPlayerList().remove(shadowPlayer);
        shadow = null;
        for (Object[] b : s.blocks) {
            command(server, String.format(Locale.ROOT, "setblock %d %d %d air", BASE[0] + (int) b[0], BASE[1] + (int) b[1],
                    BASE[2] + (int) b[2]));
        }
        server.getPlayerList().remove(p);
        Map<String, Object> line = s.json();
        line.put("result", ticks);
        return toJson(line);
    }

    @SuppressWarnings("unchecked")
    static void act(MinecraftServer server, ServerPlayer p, Map<String, Object> a) throws Exception {
        ServerLevel level = server.overworld();
        switch ((String) a.get("op")) {
            case "effect" -> {
                for (ServerPlayer q : actors(p)) {
                    q.addEffect(new MobEffectInstance(effectHolder((String) a.get("id")), (Integer) a.get("duration"),
                            (Integer) a.get("amp"), (Boolean) a.get("ambient"), (Boolean) a.get("visible"), (Boolean) a.get("icon")));
                }
            }
            case "remove" -> {
                for (ServerPlayer q : actors(p)) q.removeEffect(effectHolder((String) a.get("id")));
            }
            case "clear" -> {
                for (ServerPlayer q : actors(p)) q.removeAllEffects();
            }
            // wp44: the shadow client jumps; attributes and sneaking apply to both; a game rule.
            case "jump" -> {
                if (shadow != null) shadow.jumpFromGround();
            }
            case "attribute" -> {
                for (ServerPlayer q : actors(p)) setAttribute(q, (String) a.get("id"), ((Number) a.get("value")).doubleValue());
            }
            case "sneak" -> {
                for (ServerPlayer q : actors(p)) {
                    q.setShiftKeyDown((Boolean) a.get("on"));
                    q.setPose((Boolean) a.get("on") ? Pose.CROUCHING : Pose.STANDING);
                }
            }
            case "gamerule" -> command(server, "gamerule " + a.get("name") + " " + a.get("value"));
            // wp49: the boots change.
            case "armor" -> {
                String item = (String) a.get("item");
                Map<String, Integer> ench = new LinkedHashMap<>();
                if (a.containsKey("enchant")) ench.put((String) a.get("enchant"), (Integer) a.get("level"));
                p.setItemSlot(EquipmentSlot.FEET, item.isEmpty() ? ItemStack.EMPTY : stack(server, item, ench));
            }
            case "velocity" -> {
                if (shadow != null) {
                    shadow.setDeltaMovement(((Number) a.get("x")).doubleValue(), ((Number) a.get("y")).doubleValue(),
                            ((Number) a.get("z")).doubleValue());
                }
            }
            case "hurt" -> {
                var type = level.registryAccess().lookupOrThrow(Registries.DAMAGE_TYPE)
                        .getOrThrow(ResourceKey.create(Registries.DAMAGE_TYPE, Identifier.parse((String) a.get("type"))));
                p.hurtServer(level, new DamageSource(type), (Float) a.get("amount"));
            }
            case "hold" -> {
                ItemStack st;
                if (a.containsKey("potion")) {
                    var potion = BuiltInRegistries.POTION.getOrThrow(ResourceKey.create(Registries.POTION,
                            Identifier.parse((String) a.get("potion"))));
                    st = PotionContents.createItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse((String) a.get("item"))), potion);
                } else {
                    st = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse((String) a.get("item"))));
                }
                if (a.containsKey("stew")) {
                    List<Object> kv = (List<Object>) a.get("stew");
                    List<net.minecraft.world.item.component.SuspiciousStewEffects.Entry> entries = new ArrayList<>();
                    for (int i = 0; i < kv.size(); i += 2) {
                        entries.add(new net.minecraft.world.item.component.SuspiciousStewEffects.Entry(
                                effectHolder((String) kv.get(i)), (Integer) kv.get(i + 1)));
                    }
                    st.set(net.minecraft.core.component.DataComponents.SUSPICIOUS_STEW_EFFECTS,
                            new net.minecraft.world.item.component.SuspiciousStewEffects(entries));
                }
                p.setItemInHand(InteractionHand.MAIN_HAND, st);
            }
            case "offhand" -> p.setItemInHand(InteractionHand.OFF_HAND,
                    new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse((String) a.get("item")))));
            case "finish" -> {
                ItemStack st = p.getMainHandItem();
                ItemStack rest = st.finishUsingItem(level, p);
                if (rest != st) p.setItemInHand(InteractionHand.MAIN_HAND, rest);
            }
            case "use" -> p.gameMode.useItem(p, level, p.getMainHandItem(), InteractionHand.MAIN_HAND);
            case "setblock" -> {
                List<Integer> pos = (List<Integer>) a.get("pos");
                command(server, String.format(Locale.ROOT, "setblock %d %d %d %s", pos.get(0), pos.get(1), pos.get(2), a.get("state")));
            }
            default -> throw new IllegalArgumentException("unknown op " + a.get("op"));
        }
    }

    static String blockString(net.minecraft.world.level.block.state.BlockState s) {
        String name = BuiltInRegistries.BLOCK.getKey(s.getBlock()).toString();
        List<String> props = new ArrayList<>();
        for (net.minecraft.world.level.block.state.properties.Property<?> p : s.getProperties()) props.add(p.getName() + "=" + propValue(s, p));
        props.sort(null);
        return props.isEmpty() ? name : name + "[" + String.join(",", props) + "]";
    }

    static <T extends Comparable<T>> String propValue(net.minecraft.world.level.block.state.BlockState s, net.minecraft.world.level.block.state.properties.Property<T> p) {
        return p.getName(s.getValue(p));
    }

    static Object effectJson(MobEffectInstance e) throws Exception {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", e.getEffect().unwrapKey().orElseThrow().identifier().toString());
        m.put("amp", e.getAmplifier());
        m.put("duration", e.getDuration());
        m.put("ambient", e.isAmbient());
        m.put("visible", e.isVisible());
        m.put("icon", e.showIcon());
        MobEffectInstance hidden = (MobEffectInstance) get(e, "hiddenEffect");
        m.put("hidden", hidden == null ? null : effectJson(hidden));
        return m;
    }

    @SuppressWarnings("unchecked")
    static Map<String, Object> state(ServerPlayer p) throws Exception {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("health", p.getHealth());
        m.put("absorption", p.getAbsorptionAmount());
        m.put("food", p.getFoodData().getFoodLevel());
        m.put("saturation", p.getFoodData().getSaturationLevel());
        m.put("exhaustion", (Float) get(p.getFoodData(), "exhaustionLevel"));
        m.put("fire", p.getRemainingFireTicks());
        m.put("on_fire", (Boolean) callWithInt(p, "getSharedFlag", 0));
        m.put("air", p.getAirSupply());
        m.put("hurt_cooldown", p.damageCooldownTime);
        m.put("dead", p.isDeadOrDying());
        m.put("y", p.getY());
        m.put("on_ground", p.onGround());
        m.put("fall_distance", p.fallDistance);
        m.put("frozen", p.getTicksFrozen());
        List<MobEffectInstance> effects = new ArrayList<>(p.getActiveEffects());
        effects.sort(Comparator.comparingInt(e -> BuiltInRegistries.MOB_EFFECT.getId(e.getEffect().value())));
        List<Object> ej = new ArrayList<>();
        for (MobEffectInstance e : effects) ej.add(effectJson(e));
        m.put("effects", ej);
        Map<String, Object> attrs = new LinkedHashMap<>();
        for (Holder<Attribute> a : List.of(Attributes.MOVEMENT_SPEED, Attributes.MOVEMENT_EFFICIENCY, Attributes.ATTACK_DAMAGE, Attributes.ATTACK_SPEED,
                Attributes.MAX_HEALTH, Attributes.MAX_ABSORPTION, Attributes.LUCK, Attributes.SAFE_FALL_DISTANCE,
                Attributes.OXYGEN_BONUS, Attributes.BURNING_TIME, Attributes.WAYPOINT_TRANSMIT_RANGE)) {
            attrs.put(a.unwrapKey().orElseThrow().identifier().toString(), p.getAttributeValue(a));
        }
        m.put("attributes", attrs);
        m.put("destroy_speed", p.getDestroySpeed(Blocks.STONE.defaultBlockState()));
        List<Object> packets = new ArrayList<>();
        for (Object pkt : drain(p)) {
            if (pkt instanceof ClientboundUpdateMobEffectPacket u && u.getEntityId() == p.getId()) {
                packets.add(op("t", "effect", "id", u.getEffect().unwrapKey().orElseThrow().identifier().toString(),
                        "amp", u.getEffectAmplifier(), "duration", u.getEffectDurationTicks(), "flags", (int) (Byte) get(u, "flags")));
            } else if (pkt instanceof ClientboundRemoveMobEffectPacket r && r.entityId() == p.getId()) {
                packets.add(op("t", "remove", "id", r.effect().unwrapKey().orElseThrow().identifier().toString()));
            } else if (pkt instanceof ClientboundEntityEventPacket ev && (Integer) get(ev, "entityId") == p.getId()) {
                packets.add(op("t", "event", "event", (int) ev.getEventId()));
            }
        }
        m.put("packets", packets);
        return m;
    }

    // ---------------------------------------------------------------- wp44: the shadow client

    // A second mock player plays the client: it runs the movement (gravity, collisions, bounces,
    // block effects) as `ServerPlayer.doTick` does without the connection's snap back, and its
    // position after each tick goes to the real mock player as a move packet, the way
    // `LocalPlayer.sendPosition` sends it (a position when it moved or every 20 ticks, else only
    // the on-ground and collision flags). The packets are recorded in the scenario.

    static ServerPlayer shadow;

    static final class ClientState {
        Vec3 last;
        int reminder;
    }

    static List<ServerPlayer> actors(ServerPlayer p) {
        return shadow == null ? List.of(p) : List.of(p, shadow);
    }

    static void setAttribute(ServerPlayer p, String id, double value) {
        Holder<Attribute> h = BuiltInRegistries.ATTRIBUTE.getOrThrow(ResourceKey.create(Registries.ATTRIBUTE, Identifier.parse(id)));
        p.getAttribute(h).setBaseValue(value);
    }

    static ServerPlayer startClient(MinecraftServer server, ServerPlayer p, Scenario s) throws Exception {
        ServerPlayer c = mockPlayer(server, "Shadow" + players++);
        setup(server, c, s);
        drain(c);
        // The join teleport is long confirmed on a real connection.
        set(p.connection, "awaitingPositionFromClient", null);
        p.connection.resetPosition();
        return c;
    }

    static void clientTick(ServerPlayer p, ServerPlayer c, Scenario s, ClientState cs) throws Exception {
        if (cs.last == null) cs.last = c.position();
        c.setHealth(c.getMaxHealth());
        // The shadow burns nothing down: a burning body melts the powder snow it stands in.
        c.setRemainingFireTicks(-20);
        c.commonTick();
        c.tick();
        c.doTick();
        drain(c);
        Vec3 pos = c.position();
        cs.reminder++;
        boolean moved = pos.subtract(cs.last).lengthSqr() > 2.0E-4 * 2.0E-4 || cs.reminder >= 20;
        boolean onGround = c.onGround();
        boolean hcol = c.horizontalCollision;
        Map<String, Object> rec = new LinkedHashMap<>();
        if (moved) {
            rec.put("pos", new double[] {pos.x, pos.y, pos.z});
            cs.last = pos;
            cs.reminder = 0;
        } else {
            rec.put("pos", null);
        }
        rec.put("on_ground", onGround);
        rec.put("hcol", hcol);
        s.moves.add(rec);
        ServerboundMovePlayerPacket pkt = moved
                ? new ServerboundMovePlayerPacket.Pos(pos.x, pos.y, pos.z, onGround, hcol)
                : new ServerboundMovePlayerPacket.StatusOnly(onGround, hcol);
        p.connection.handleMovePlayer(pkt);
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

    static void set(Object o, String name, Object v) throws Exception {
        field(o.getClass(), name).set(o, v);
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

    static Object callWithInt(Object o, String name, int arg) throws Exception {
        for (Class<?> k = o.getClass(); k != null; k = k.getSuperclass()) {
            for (Method m : k.getDeclaredMethods()) {
                if (m.getName().equals(name) && m.getParameterCount() == 1 && m.getParameterTypes()[0] == int.class) {
                    m.setAccessible(true);
                    return m.invoke(o, arg);
                }
            }
        }
        throw new NoSuchMethodException(name);
    }

    static String toJson(Object o) {
        if (o == null) return "null";
        if (o instanceof String s) return "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
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
