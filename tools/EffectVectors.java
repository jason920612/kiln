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
            List<Object> bl = new ArrayList<>();
            for (Object[] b : blocks) {
                bl.add(List.of(BASE[0] + (int) b[0], BASE[1] + (int) b[1], BASE[2] + (int) b[2], b[3]));
            }
            m.put("blocks", bl);
            Map<String, Object> acts = new LinkedHashMap<>();
            for (var e : actions.entrySet()) acts.put(String.valueOf(e.getKey()), e.getValue());
            m.put("actions", acts);
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
        // The server moves a player it gets no movement from (gravity, then tickPlayer snaps it
        // back): a floor keeps it standing like a client that reports standing still.
        for (Scenario x : out) {
            boolean floor = x.blocks.stream().anyMatch(b -> (int) b[0] == 0 && (int) b[1] == -1 && (int) b[2] == 0);
            if (!floor) x.blocks.add(0, new Object[] {0, -1, 0, "minecraft:stone"});
        }
        return out;
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
        p.tickCount = 0;
        p.getCombatTracker().recheckStatus();
    }

    static String run(MinecraftServer server, Scenario s) throws Exception {
        ServerLevel level = server.overworld();
        command(server, "difficulty " + s.difficulty);
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
        for (int t = 1; t <= s.ticks; t++) {
            for (Map<String, Object> a : s.actions.getOrDefault(t, List.of())) act(server, p, a);
            p.commonTick();
            p.tick();
            call(p.connection, "tickPlayer");
            ticks.add(state(p));
        }
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
            case "effect" -> p.addEffect(new MobEffectInstance(effectHolder((String) a.get("id")), (Integer) a.get("duration"),
                    (Integer) a.get("amp"), (Boolean) a.get("ambient"), (Boolean) a.get("visible"), (Boolean) a.get("icon")));
            case "remove" -> p.removeEffect(effectHolder((String) a.get("id")));
            case "clear" -> p.removeAllEffects();
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
        List<MobEffectInstance> effects = new ArrayList<>(p.getActiveEffects());
        effects.sort(Comparator.comparingInt(e -> BuiltInRegistries.MOB_EFFECT.getId(e.getEffect().value())));
        List<Object> ej = new ArrayList<>();
        for (MobEffectInstance e : effects) ej.add(effectJson(e));
        m.put("effects", ej);
        Map<String, Object> attrs = new LinkedHashMap<>();
        for (Holder<Attribute> a : List.of(Attributes.MOVEMENT_SPEED, Attributes.ATTACK_DAMAGE, Attributes.ATTACK_SPEED,
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
