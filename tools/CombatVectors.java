// Differential test vectors for Kiln's player combat: runs melee scenarios between mock players
// in a real vanilla 26.3 dedicated server (started in-process) and records the outcome of each
// attack (Player.attack called directly, as ServerGamePacketListenerImpl.handleAttack does).
//
// Each scenario sets up an attacker, a target and optionally a bystander (items, armor, attack
// strength ticker, fall distance, sprinting, hurt cooldown, ...), performs one attack and
// dumps one JSON line with the setup (so Rust can replay it) and the result: health,
// absorption, the motion packet the target got, exhaustion, item and armor damage, the death
// message. Floats and doubles are printed with Float/Double.toString (exact round trip).
//
// usage (cwd = a scratch server directory, e.g. work/m6-combat/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/CombatVectors.java <out.jsonl> [name-filter]
// (tools/combat_vectors.py sets this up)

import com.mojang.authlib.GameProfile;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.Connection;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundBundlePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerCombatKillPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.enchantment.EnchantmentHelper;
import net.minecraft.world.level.GameType;
import net.minecraft.world.phys.Vec3;

public class CombatVectors {
    static final double BX = 0.5, BY = 100.0, BZ = 0.5;

    // ---------------------------------------------------------------- scenario model

    static final class Side {
        String mainHand;
        int mainHandDamage;
        String customName;
        String[] armor = new String[4]; // feet, legs, chest, head
        int[] armorDamage = new int[4];
        double dx, dy, dz;
        float yaw;
        boolean onGround = true, sprinting;
        double fallDistance;
        int ticker = 100;
        String gameMode = "survival";
        float health = 20.0f;
        float absorption;
        int hurtCooldown;
        float lastHurt;
        double kmx, kmz; // known movement
        // Enchantments (id -> level) on the main hand item and on each armor piece.
        Map<String, Integer> mainEnch = new LinkedHashMap<>();
        List<Map<String, Integer>> armorEnch = new ArrayList<>(List.of(
                new LinkedHashMap<>(), new LinkedHashMap<>(), new LinkedHashMap<>(), new LinkedHashMap<>()));

        Side ench(String id, int level) {
            mainEnch.put(id, level);
            return this;
        }

        Side armorEnch(int slot, String id, int level) {
            armorEnch.get(slot).put(id, level);
            return this;
        }

        Map<String, Object> json() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("main_hand", mainHand);
            m.put("main_hand_damage", mainHandDamage);
            m.put("main_hand_enchantments", mainEnch);
            m.put("armor_enchantments", armorEnch);
            m.put("custom_name", customName);
            m.put("armor", armor);
            m.put("armor_damage", armorDamage);
            m.put("pos", new double[] {dx, dy, dz});
            m.put("yaw", yaw);
            m.put("on_ground", onGround);
            m.put("sprinting", sprinting);
            m.put("fall_distance", fallDistance);
            m.put("ticker", ticker);
            m.put("game_mode", gameMode);
            m.put("health", health);
            m.put("absorption", absorption);
            m.put("hurt_cooldown", hurtCooldown);
            m.put("last_hurt", lastHurt);
            m.put("known_movement", new double[] {kmx, 0.0, kmz});
            return m;
        }
    }

    static final class Scenario {
        final String name;
        final Side attacker = new Side(), target = new Side();
        Side bystander;
        /** The attacker is in a riptide spin (`startAutoSpinAttack(20, 8.0F, main hand)`). */
        boolean spin;
        boolean pvp = true;
        String difficulty = "normal";
        // The level's random and each player's entity random are reseeded before the attack.
        long levelSeed;

        Scenario(String name) {
            this.name = name;
            target.dz = 2.0;
            target.yaw = 180f;
            levelSeed = name.hashCode();
        }
    }

    /** Enchanted-combat scenarios (sharpness, knockback, fire aspect, protection, unbreaking...). */
    static void enchantedScenarios(List<Scenario> out) {
        Scenario s;
        for (int l : new int[] {1, 3, 5}) {
            s = new Scenario("ench_sharpness_" + l);
            s.attacker.mainHand = "minecraft:diamond_sword";
            s.attacker.ench("minecraft:sharpness", l);
            out.add(s);
        }
        s = new Scenario("ench_sharpness_partial");
        s.attacker.mainHand = "minecraft:iron_axe";
        s.attacker.ench("minecraft:sharpness", 4);
        s.attacker.ticker = 7;
        out.add(s);
        s = new Scenario("ench_sharpness_fist_weak");
        s.attacker.mainHand = "minecraft:stick";
        s.attacker.ench("minecraft:sharpness", 2);
        s.attacker.ticker = 1;
        out.add(s);
        s = new Scenario("ench_sharpness_crit");
        s.attacker.mainHand = "minecraft:netherite_sword";
        s.attacker.ench("minecraft:sharpness", 5);
        s.attacker.onGround = false;
        s.attacker.fallDistance = 1.0;
        out.add(s);
        s = new Scenario("ench_smite_vs_player");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.ench("minecraft:smite", 5);
        out.add(s);
        s = new Scenario("ench_bane_vs_player");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.ench("minecraft:bane_of_arthropods", 5);
        out.add(s);
        s = new Scenario("ench_knockback_2");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.ench("minecraft:knockback", 2);
        out.add(s);
        s = new Scenario("ench_knockback_1_sprint_yaw");
        s.attacker.mainHand = "minecraft:stone_sword";
        s.attacker.ench("minecraft:knockback", 1);
        s.attacker.sprinting = true;
        s.attacker.yaw = 30f;
        s.target.dx = -1.0;
        s.target.dz = 1.7;
        out.add(s);
        s = new Scenario("ench_fire_aspect_2");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.ench("minecraft:fire_aspect", 2);
        out.add(s);
        s = new Scenario("ench_fire_aspect_vs_fire_protection");
        s.attacker.mainHand = "minecraft:golden_sword";
        s.attacker.ench("minecraft:fire_aspect", 1).ench("minecraft:sharpness", 2);
        s.target.armor = new String[] {null, "minecraft:iron_leggings", "minecraft:iron_chestplate", null};
        s.target.armorEnch(2, "minecraft:fire_protection", 4).armorEnch(1, "minecraft:fire_protection", 2);
        out.add(s);
        s = new Scenario("ench_fire_aspect_no_weapon_component");
        s.attacker.mainHand = "minecraft:stick";
        s.attacker.ench("minecraft:fire_aspect", 2);
        out.add(s);
        s = new Scenario("ench_protection_4_full");
        s.attacker.mainHand = "minecraft:netherite_sword";
        s.target.armor = new String[] {"minecraft:diamond_boots", "minecraft:diamond_leggings",
                "minecraft:diamond_chestplate", "minecraft:diamond_helmet"};
        for (int i = 0; i < 4; i++) s.target.armorEnch(i, "minecraft:protection", 4);
        out.add(s);
        s = new Scenario("ench_protection_mixed");
        s.attacker.mainHand = "minecraft:iron_sword";
        s.attacker.ench("minecraft:sharpness", 3);
        s.target.armor = new String[] {"minecraft:leather_boots", null, "minecraft:chainmail_chestplate", "minecraft:iron_helmet"};
        s.target.armorEnch(0, "minecraft:feather_falling", 4).armorEnch(0, "minecraft:protection", 1)
                .armorEnch(2, "minecraft:protection", 3).armorEnch(3, "minecraft:projectile_protection", 4);
        out.add(s);
        s = new Scenario("ench_protection_cap");
        s.attacker.mainHand = "minecraft:diamond_axe";
        s.target.armor = new String[] {"minecraft:iron_boots", "minecraft:iron_leggings", "minecraft:iron_chestplate", "minecraft:iron_helmet"};
        for (int i = 0; i < 4; i++) s.target.armorEnch(i, "minecraft:protection", 10);
        out.add(s);
        for (int l : new int[] {1, 4}) {
            s = new Scenario("ench_breach_" + l);
            s.attacker.mainHand = "minecraft:diamond_sword";
            s.attacker.ench("minecraft:breach", l);
            s.target.armor = new String[] {"minecraft:diamond_boots", "minecraft:diamond_leggings",
                    "minecraft:diamond_chestplate", "minecraft:diamond_helmet"};
            out.add(s);
        }
        s = new Scenario("ench_sweeping_edge_3");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.ench("minecraft:sweeping_edge", 3);
        s.bystander = new Side();
        s.bystander.dx = 1.0;
        s.bystander.dz = 2.2;
        out.add(s);
        s = new Scenario("ench_sweep_sharpness_fire");
        s.attacker.mainHand = "minecraft:iron_sword";
        s.attacker.ench("minecraft:sweeping_edge", 1).ench("minecraft:sharpness", 5).ench("minecraft:fire_aspect", 1);
        s.bystander = new Side();
        s.bystander.dx = -0.9;
        s.bystander.dz = 2.0;
        s.bystander.armor = new String[] {null, null, "minecraft:golden_chestplate", null};
        s.bystander.armorEnch(2, "minecraft:protection", 2);
        out.add(s);
        for (long seed : new long[] {1, 2, 3, 4, 5, 6, 7, 8, 12345}) {
            s = new Scenario("ench_unbreaking_sword_" + seed);
            s.attacker.mainHand = "minecraft:diamond_sword";
            s.attacker.mainHandDamage = 100;
            s.attacker.ench("minecraft:unbreaking", 3);
            s.levelSeed = seed;
            out.add(s);
        }
        s = new Scenario("ench_unbreaking_sword_breaks");
        s.attacker.mainHand = "minecraft:wooden_sword";
        s.attacker.mainHandDamage = 58;
        s.attacker.ench("minecraft:unbreaking", 1);
        s.levelSeed = 7;
        out.add(s);
        for (long seed : new long[] {5, 99, 2024}) {
            s = new Scenario("ench_unbreaking_armor_" + seed);
            s.attacker.mainHand = "minecraft:netherite_axe";
            s.attacker.ench("minecraft:unbreaking", 2);
            s.target.armor = new String[] {"minecraft:iron_boots", "minecraft:iron_leggings", "minecraft:iron_chestplate", "minecraft:iron_helmet"};
            for (int i = 0; i < 4; i++) s.target.armorEnch(i, "minecraft:unbreaking", 3);
            s.levelSeed = seed;
            out.add(s);
        }
        for (long seed : new long[] {1, 4, 8, 31, 2, 3, 5, 6, 7, 9, 10}) {
            s = new Scenario("ench_thorns_" + seed);
            s.attacker.mainHand = "minecraft:iron_sword";
            s.target.armor = new String[] {null, null, "minecraft:diamond_chestplate", "minecraft:iron_helmet"};
            s.target.armorEnch(2, "minecraft:thorns", 3).armorEnch(3, "minecraft:thorns", 1);
            s.levelSeed = seed;
            out.add(s);
        }
        for (long seed : new long[] {1, 2, 3, 4, 5}) {
            s = new Scenario("ench_thorns_fist_" + seed);
            s.target.armor = new String[] {null, null, "minecraft:diamond_chestplate", null};
            s.target.armorEnch(2, "minecraft:thorns", 3).armorEnch(2, "minecraft:unbreaking", 1);
            s.levelSeed = seed;
            out.add(s);
        }
        s = new Scenario("ench_thorns_kills_attacker");
        s.attacker.mainHand = "minecraft:wooden_sword";
        s.attacker.health = 1.0f;
        s.target.armor = new String[] {"minecraft:iron_boots", "minecraft:iron_leggings", "minecraft:iron_chestplate", "minecraft:iron_helmet"};
        for (int i = 0; i < 4; i++) s.target.armorEnch(i, "minecraft:thorns", 3);
        s.levelSeed = 1;
        out.add(s);
    }

    static List<Scenario> scenarios() {
        List<Scenario> out = new ArrayList<>();
        Scenario s;

        out.add(new Scenario("fist_full"));
        s = new Scenario("fist_weak");
        s.attacker.ticker = 0;
        out.add(s);
        s = new Scenario("fist_partial");
        s.attacker.ticker = 2;
        out.add(s);

        s = new Scenario("diamond_sword_full");
        s.attacker.mainHand = "minecraft:diamond_sword";
        out.add(s);
        for (int t : new int[] {0, 3, 5, 8, 10, 11, 12}) {
            s = new Scenario("diamond_sword_ticker_" + t);
            s.attacker.mainHand = "minecraft:diamond_sword";
            s.attacker.ticker = t;
            out.add(s);
        }
        s = new Scenario("iron_axe_full");
        s.attacker.mainHand = "minecraft:iron_axe";
        out.add(s);
        s = new Scenario("netherite_axe_partial");
        s.attacker.mainHand = "minecraft:netherite_axe";
        s.attacker.ticker = 10;
        out.add(s);
        s = new Scenario("wooden_pickaxe");
        s.attacker.mainHand = "minecraft:wooden_pickaxe";
        out.add(s);
        s = new Scenario("mace_standing");
        s.attacker.mainHand = "minecraft:mace";
        out.add(s);
        s = new Scenario("stick");
        s.attacker.mainHand = "minecraft:stick";
        out.add(s);

        s = new Scenario("crit_sword");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.onGround = false;
        s.attacker.fallDistance = 0.5;
        out.add(s);
        s = new Scenario("crit_fist");
        s.attacker.onGround = false;
        s.attacker.fallDistance = 1.25;
        out.add(s);
        s = new Scenario("no_crit_while_sprinting");
        s.attacker.mainHand = "minecraft:iron_sword";
        s.attacker.onGround = false;
        s.attacker.fallDistance = 0.5;
        s.attacker.sprinting = true;
        out.add(s);
        s = new Scenario("no_crit_when_weak");
        s.attacker.mainHand = "minecraft:iron_sword";
        s.attacker.onGround = false;
        s.attacker.fallDistance = 0.5;
        s.attacker.ticker = 6;
        out.add(s);

        s = new Scenario("sprint_knockback");
        s.attacker.mainHand = "minecraft:stone_sword";
        s.attacker.sprinting = true;
        out.add(s);
        s = new Scenario("sprint_knockback_yaw");
        s.attacker.sprinting = true;
        s.attacker.yaw = 37.5f;
        s.target.dx = -1.2;
        s.target.dz = 1.6;
        out.add(s);
        s = new Scenario("sprint_weak_no_knockback");
        s.attacker.sprinting = true;
        s.attacker.ticker = 3;
        out.add(s);
        s = new Scenario("target_in_air");
        s.attacker.sprinting = true;
        s.target.onGround = false;
        s.target.dy = 0.8;
        out.add(s);

        for (String mat : new String[] {"leather", "golden", "chainmail", "iron", "diamond", "netherite"}) {
            s = new Scenario("armor_" + mat);
            s.attacker.mainHand = "minecraft:diamond_sword";
            s.target.armor = new String[] {"minecraft:" + mat + "_boots", "minecraft:" + mat + "_leggings",
                    "minecraft:" + mat + "_chestplate", "minecraft:" + mat + "_helmet"};
            s.attacker.sprinting = mat.equals("netherite");
            out.add(s);
        }
        s = new Scenario("armor_mixed_crit");
        s.attacker.mainHand = "minecraft:netherite_sword";
        s.attacker.onGround = false;
        s.attacker.fallDistance = 2.0;
        s.target.armor = new String[] {null, "minecraft:iron_leggings", "minecraft:diamond_chestplate", "minecraft:turtle_helmet"};
        out.add(s);
        s = new Scenario("armor_breaks");
        s.attacker.mainHand = "minecraft:netherite_axe";
        s.target.armor = new String[] {"minecraft:leather_boots", null, null, null};
        s.target.armorDamage = new int[] {64, 0, 0, 0};
        out.add(s);

        s = new Scenario("cooldown_excess");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.target.hurtCooldown = 15;
        s.target.lastHurt = 5.0f;
        out.add(s);
        s = new Scenario("cooldown_blocked");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.target.hurtCooldown = 15;
        s.target.lastHurt = 7.0f;
        out.add(s);
        s = new Scenario("cooldown_half");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.target.hurtCooldown = 11;
        s.target.lastHurt = 3.0f;
        s.attacker.sprinting = true;
        out.add(s);
        s = new Scenario("cooldown_over");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.target.hurtCooldown = 10;
        s.target.lastHurt = 30.0f;
        out.add(s);

        s = new Scenario("absorption");
        s.attacker.mainHand = "minecraft:iron_sword";
        s.target.absorption = 4.0f;
        out.add(s);
        s = new Scenario("absorption_partial");
        s.attacker.mainHand = "minecraft:iron_sword";
        s.target.absorption = 2.5f;
        s.target.armor = new String[] {null, null, "minecraft:iron_chestplate", null};
        out.add(s);

        s = new Scenario("pvp_off");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.pvp = false;
        out.add(s);
        s = new Scenario("target_creative");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.target.gameMode = "creative";
        out.add(s);
        s = new Scenario("attacker_creative");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.gameMode = "creative";
        s.target.dz = 4.5;
        out.add(s);
        s = new Scenario("hard_difficulty");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.difficulty = "hard";
        out.add(s);

        s = new Scenario("sweep");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.bystander = new Side();
        s.bystander.dx = 1.0;
        s.bystander.dz = 2.2;
        out.add(s);
        s = new Scenario("sweep_too_fast");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.kmz = 0.3;
        s.bystander = new Side();
        s.bystander.dx = 1.0;
        s.bystander.dz = 2.2;
        out.add(s);
        s = new Scenario("sweep_armored_bystander");
        s.attacker.mainHand = "minecraft:golden_sword";
        s.attacker.yaw = -20f;
        s.bystander = new Side();
        s.bystander.dx = -0.8;
        s.bystander.dz = 1.4;
        s.bystander.armor = new String[] {"minecraft:iron_boots", null, "minecraft:iron_chestplate", null};
        out.add(s);
        s = new Scenario("no_sweep_axe");
        s.attacker.mainHand = "minecraft:diamond_axe";
        s.bystander = new Side();
        s.bystander.dx = 1.0;
        s.bystander.dz = 2.2;
        out.add(s);

        s = new Scenario("kill");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.target.health = 3.0f;
        out.add(s);
        s = new Scenario("kill_named_weapon");
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.attacker.customName = "Excalibur";
        s.target.health = 3.0f;
        out.add(s);
        s = new Scenario("sword_breaks");
        s.attacker.mainHand = "minecraft:wooden_sword";
        s.attacker.mainHandDamage = 58;
        out.add(s);
        enchantedScenarios(out);
        spinScenarios(out);
        return out;
    }

    /**
     * Riptide spin attacks: `Player.attack` while `startAutoSpinAttack(20, 8.0F, trident)` is on,
     * so the base damage is the spin's 8 (attack cooldown and armor still apply) and the weapon
     * is the trident (its enchantments, its durability).
     */
    static void spinScenarios(List<Scenario> out) {
        Scenario s;
        s = new Scenario("spin_full");
        s.attacker.mainHand = "minecraft:trident";
        s.spin = true;
        out.add(s);
        for (int t : new int[] {0, 4, 9, 14, 18}) {
            s = new Scenario("spin_ticker_" + t);
            s.attacker.mainHand = "minecraft:trident";
            s.attacker.ticker = t;
            s.spin = true;
            out.add(s);
        }
        s = new Scenario("spin_armor");
        s.attacker.mainHand = "minecraft:trident";
        s.target.armor = new String[] {"minecraft:iron_boots", "minecraft:iron_leggings", "minecraft:diamond_chestplate", "minecraft:iron_helmet"};
        s.spin = true;
        out.add(s);
        s = new Scenario("spin_crit");
        s.attacker.mainHand = "minecraft:trident";
        s.attacker.onGround = false;
        s.attacker.fallDistance = 1.0;
        s.spin = true;
        out.add(s);
        s = new Scenario("spin_sprinting");
        s.attacker.mainHand = "minecraft:trident";
        s.attacker.sprinting = true;
        s.spin = true;
        out.add(s);
        s = new Scenario("spin_sharpness_knockback");
        s.attacker.mainHand = "minecraft:trident";
        s.attacker.ench("minecraft:sharpness", 5).ench("minecraft:knockback", 2);
        s.spin = true;
        out.add(s);
        s = new Scenario("spin_fire_aspect");
        s.attacker.mainHand = "minecraft:trident";
        s.attacker.ench("minecraft:fire_aspect", 2);
        s.spin = true;
        out.add(s);
        s = new Scenario("spin_kills_target");
        s.attacker.mainHand = "minecraft:trident";
        s.target.health = 6.0f;
        s.spin = true;
        out.add(s);
        s = new Scenario("spin_worn_trident");
        s.attacker.mainHand = "minecraft:trident";
        s.attacker.mainHandDamage = 249;
        s.spin = true;
        out.add(s);
        s = new Scenario("spin_thorns");
        s.attacker.mainHand = "minecraft:trident";
        s.target.armor = new String[] {null, null, "minecraft:diamond_chestplate", null};
        s.target.armorEnch(2, "minecraft:thorns", 3);
        s.levelSeed = 2;
        s.spin = true;
        out.add(s);
        s = new Scenario("spin_with_a_sword");
        // Whatever is in hand when the spin starts is its weapon (a sword's enchantments and wear).
        s.attacker.mainHand = "minecraft:diamond_sword";
        s.spin = true;
        out.add(s);
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
        }, "CombatVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        List<Scenario> selected = new ArrayList<>();
        for (Scenario s : scenarios()) {
            if (filter == null || s.name.contains(filter)) selected.add(s);
        }
        System.out.println("CombatVectors: " + selected.size() + " scenarios");
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
        if ("spear".equals(filter)) {
            // wp34: only the spear vectors (spear.jsonl beside the output file).
            List<String> spearLines = new ArrayList<>();
            server.submit(() -> {
                try {
                    SpearVectors.run(server, spearLines);
                } catch (Throwable t) {
                    t.printStackTrace();
                    spearLines.add("{\"kind\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
                }
            }).get();
            Path spearPath = outPath.resolveSibling("spear.jsonl");
            try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(spearPath))) {
                for (String l : spearLines) w.println(l);
            }
            System.out.println("CombatVectors: wrote " + spearLines.size() + " spear vectors to " + spearPath);
            server.halt(false);
            System.exit(0);
        }
        // wp44: the melee vectors (melee.jsonl beside the output file); `--filter melee...` writes only those.
        List<String> meleeLines = new ArrayList<>();
        if (filter == null || filter.startsWith("melee") || filter.startsWith("mace") || filter.startsWith("sweep")) {
            String meleeFilter = filter == null ? null : filter;
            server.submit(() -> {
                try {
                    MeleeVectors.run(server, meleeLines, meleeFilter == null ? null : (meleeFilter.startsWith("melee/") ? meleeFilter : meleeFilter.equals("melee") ? "melee" : "melee/" + meleeFilter));
                } catch (Throwable t) {
                    t.printStackTrace();
                    meleeLines.add("{\"kind\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
                }
            }).get();
            Path meleePath = outPath.resolveSibling("melee.jsonl");
            try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(meleePath))) {
                for (String l : meleeLines) w.println(l);
            }
            System.out.println("CombatVectors: wrote " + meleeLines.size() + " melee vectors to " + meleePath);
        }
        if (filter != null && (filter.startsWith("melee") || filter.startsWith("mace") || filter.startsWith("sweep"))) {
            server.halt(false);
            System.exit(0);
        }
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
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
        System.out.println("CombatVectors: wrote " + lines.size() + " scenarios to " + outPath);
        List<String> helperLines = new ArrayList<>();
        server.submit(() -> {
            try {
                EnchantHelperVectors.run(server, helperLines);
            } catch (Throwable t) {
                t.printStackTrace();
                helperLines.add("{\"kind\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
            }
        }).get();
        Path helperPath = outPath.resolveSibling("enchant_helpers.jsonl");
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(helperPath))) {
            for (String l : helperLines) w.println(l);
        }
        System.out.println("CombatVectors: wrote " + helperLines.size() + " EnchantmentHelper vectors to " + helperPath);
        List<String> riptideLines = new ArrayList<>();
        server.submit(() -> {
            try {
                RiptideVectors.run(server, riptideLines);
            } catch (Throwable t) {
                t.printStackTrace();
                riptideLines.add("{\"kind\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
            }
        }).get();
        Path riptidePath = outPath.resolveSibling("riptide.jsonl");
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(riptidePath))) {
            for (String l : riptideLines) w.println(l);
        }
        System.out.println("CombatVectors: wrote " + riptideLines.size() + " riptide vectors to " + riptidePath);
        List<String> mountLines = new ArrayList<>();
        server.submit(() -> {
            try {
                MountVectors.run(server, mountLines, 240, 11L);
            } catch (Throwable t) {
                t.printStackTrace();
                mountLines.add("{\"kind\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
            }
        }).get();
        Path mountPath = outPath.resolveSibling("mount.jsonl");
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(mountPath))) {
            for (String l : mountLines) w.println(l);
        }
        System.out.println("CombatVectors: wrote " + mountLines.size() + " mount vectors to " + mountPath);
        server.halt(false);
        System.exit(0);
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
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    /** Finds the server through the "Server thread" task (MinecraftServer.spin's AtomicReference). */
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

    // ---------------------------------------------------------------- one scenario

    static int players;

    /** A mock player with an embedded channel (GameTestHelper.makeMockServerPlayerInLevel). */
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

    static void setup(MinecraftServer server, ServerPlayer p, Side side) throws Exception {
        p.setGameMode(GameType.byName(side.gameMode));
        call(p.connection, "markClientLoaded");
        p.setPos(BX + side.dx, BY + side.dy, BZ + side.dz);
        p.setYRot(side.yaw);
        p.setYHeadRot(side.yaw);
        p.setXRot(0f);
        p.setDeltaMovement(Vec3.ZERO);
        p.setOnGround(side.onGround);
        p.fallDistance = side.fallDistance;
        p.setSprinting(side.sprinting);
        ItemStack main = ItemStack.EMPTY;
        if (side.mainHand != null) {
            main = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(side.mainHand)));
            if (side.mainHandDamage > 0) main.setDamageValue(side.mainHandDamage);
            if (side.customName != null) main.set(net.minecraft.core.component.DataComponents.CUSTOM_NAME, Component.literal(side.customName));
            enchant(server, main, side.mainEnch);
        }
        p.getInventory().clearContent();
        p.getInventory().setSelectedSlot(0);
        p.setItemSlot(EquipmentSlot.MAINHAND, main);
        EquipmentSlot[] slots = {EquipmentSlot.FEET, EquipmentSlot.LEGS, EquipmentSlot.CHEST, EquipmentSlot.HEAD};
        for (int i = 0; i < 4; i++) {
            ItemStack a = ItemStack.EMPTY;
            if (side.armor[i] != null) {
                a = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(side.armor[i])));
                if (side.armorDamage[i] > 0) a.setDamageValue(side.armorDamage[i]);
                enchant(server, a, side.armorEnch.get(i));
            }
            p.setItemSlot(slots[i], a);
        }
        // Attributes from the equipment, as the player's tick would collect them.
        call(p, "detectEquipmentUpdates");
        set(p, "attackStrengthTicker", side.ticker);
        p.getAttribute(Attributes.MAX_ABSORPTION).setBaseValue(20.0);
        p.setHealth(side.health);
        p.setAbsorptionAmount(side.absorption);
        p.damageCooldownTime = side.hurtCooldown;
        set(p, "lastHurt", side.lastHurt);
        p.setKnownMovement(new Vec3(side.kmx, 0.0, side.kmz));
        p.getFoodData().setFoodLevel(17);
        p.getFoodData().setSaturation(0f);
        set(p.getFoodData(), "exhaustionLevel", 0f);
        p.getCombatTracker().recheckStatus();
        p.setRemainingFireTicks(0);
    }

    static net.minecraft.core.Holder<net.minecraft.world.item.enchantment.Enchantment> enchantment(MinecraftServer server, String id) {
        return server.registryAccess().lookupOrThrow(net.minecraft.core.registries.Registries.ENCHANTMENT)
                .getOrThrow(net.minecraft.resources.ResourceKey.create(net.minecraft.core.registries.Registries.ENCHANTMENT, Identifier.parse(id)));
    }

    static void enchant(MinecraftServer server, ItemStack stack, Map<String, Integer> enchantments) {
        for (var e : enchantments.entrySet()) stack.enchant(enchantment(server, e.getKey()), e.getValue());
    }

    static String run(MinecraftServer server, Scenario s) throws Exception {
        var cmd = server.createCommandSourceStack();
        server.getCommands().performPrefixedCommand(cmd, "gamerule minecraft:pvp " + s.pvp);
        server.getCommands().performPrefixedCommand(cmd, "difficulty " + s.difficulty);
        ServerPlayer attacker = mockPlayer(server, "Attacker" + players);
        ServerPlayer target = mockPlayer(server, "Target" + players);
        ServerPlayer bystander = s.bystander != null ? mockPlayer(server, "Bystander" + players) : null;
        players++;
        setup(server, attacker, s.attacker);
        setup(server, target, s.target);
        if (bystander != null) setup(server, bystander, s.bystander);
        drain(attacker);
        drain(target);
        if (bystander != null) drain(bystander);
        // Enchantment requirements and unbreaking draw from the level's random, thorns damage
        // from the attacker's.
        server.overworld().getRandom().setSeed(s.levelSeed);
        attacker.getRandom().setSeed(s.levelSeed + 1);
        target.getRandom().setSeed(s.levelSeed + 2);
        if (bystander != null) bystander.getRandom().setSeed(s.levelSeed + 3);

        if (s.spin) attacker.startAutoSpinAttack(20, 8.0f, attacker.getMainHandItem());
        attacker.attack(target);

        Map<String, Object> result = new LinkedHashMap<>();
        Map<String, Object> a = outcome(attacker, target);
        // Thorns hurts the attacker: its motion goes out when the server entity syncs.
        a.put("pending_motion", attacker.syncVelocity ? vec(attacker.getDeltaMovement()) : null);
        result.put("attacker", a);
        result.put("target", outcome(target, target));
        if (bystander != null) {
            // Swept players get their motion when the server entity syncs (end of tick).
            Map<String, Object> b = outcome(bystander, bystander);
            b.put("pending_motion", bystander.syncVelocity ? vec(bystander.getDeltaMovement()) : null);
            result.put("bystander", b);
        }
        Map<String, Object> line = new LinkedHashMap<>();
        line.put("name", s.name);
        line.put("pvp", s.pvp);
        line.put("difficulty", s.difficulty);
        line.put("level_seed", s.levelSeed);
        line.put("spin", s.spin);
        line.put("attacker", s.attacker.json());
        line.put("target", s.target.json());
        line.put("bystander", s.bystander != null ? s.bystander.json() : null);
        line.put("result", result);
        for (ServerPlayer p : new ServerPlayer[] {attacker, target, bystander}) {
            if (p != null) server.getPlayerList().remove(p);
        }
        return toJson(line);
    }

    static Map<String, Object> outcome(ServerPlayer p, ServerPlayer target) throws Exception {
        return outcome(p, drain(p));
    }

    /** The outcome of `p` from the packets it was sent (already drained). */
    static Map<String, Object> outcome(ServerPlayer p, List<Object> packets) throws Exception {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("health", p.getHealth());
        m.put("absorption", p.getAbsorptionAmount());
        m.put("exhaustion", (Float) get(p.getFoodData(), "exhaustionLevel"));
        m.put("hurt_cooldown", p.damageCooldownTime);
        m.put("last_hurt", (Float) get(p, "lastHurt"));
        m.put("sprinting", p.isSprinting());
        m.put("fire_ticks", p.getRemainingFireTicks());
        m.put("ticker", (Integer) get(p, "attackStrengthTicker"));
        ItemStack main = p.getMainHandItem();
        m.put("main_hand", main.isEmpty() ? null : BuiltInRegistries.ITEM.getKey(main.getItem()).toString());
        m.put("main_hand_damage", main.getDamageValue());
        EquipmentSlot[] slots = {EquipmentSlot.FEET, EquipmentSlot.LEGS, EquipmentSlot.CHEST, EquipmentSlot.HEAD};
        List<Object> armor = new ArrayList<>();
        for (EquipmentSlot slot : slots) {
            ItemStack a = p.getItemBySlot(slot);
            armor.add(a.isEmpty() ? null : a.getDamageValue());
        }
        m.put("armor_damage", armor);
        Object motion = null;
        String death = null;
        List<Object> deathArgs = new ArrayList<>();
        // wp36: what the player heard of the hurt and death sounds (`Player.playSound` excludes the hurt
        // player himself), and the tilt his own client got (`ServerPlayer.indicateDamage`).
        List<Object> hurtSounds = new ArrayList<>();
        Object hurtAnimation = null;
        for (Object pkt : packets) {
            if (pkt instanceof net.minecraft.network.protocol.game.ClientboundSoundPacket sp) {
                String sound = sp.getSound().value().location().toString();
                if (sound.startsWith("minecraft:entity.player.hurt") || sound.equals("minecraft:entity.player.death") || sound.equals("minecraft:enchant.thorns.hit")) {
                    Map<String, Object> hs = new LinkedHashMap<>();
                    hs.put("sound", sound);
                    hs.put("source", sp.getSource().getName());
                    hs.put("volume", sp.getVolume());
                    hs.put("pitch", sp.getPitch());
                    hurtSounds.add(hs);
                }
            }
            if (pkt instanceof net.minecraft.network.protocol.game.ClientboundHurtAnimationPacket ha && ha.id() == p.getId()) {
                hurtAnimation = ha.yaw();
            }
            if (pkt instanceof ClientboundSetEntityMotionPacket mp && mp.id() == p.getId()) {
                motion = vec(mp.movement());
            }
            if (pkt instanceof ClientboundPlayerCombatKillPacket kp && kp.message().getContents() instanceof TranslatableContents tc) {
                death = tc.getKey();
                for (Object a : tc.getArgs()) deathArgs.add(a instanceof Component c ? c.getString() : String.valueOf(a));
            }
        }
        m.put("motion", motion);
        m.put("death", death);
        m.put("death_args", deathArgs);
        m.put("hurt_sounds", hurtSounds);
        m.put("hurt_animation", hurtAnimation);
        m.put("velocity", vec(p.getDeltaMovement()));
        return m;
    }

    static double[] vec(Vec3 v) {
        return new double[] {v.x, v.y, v.z};
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

    static void call(Object o, String name) throws Exception {
        for (Class<?> k = o.getClass(); k != null; k = k.getSuperclass()) {
            for (Method m : k.getDeclaredMethods()) {
                if (m.getName().equals(name) && m.getParameterCount() == 0) {
                    m.setAccessible(true);
                    m.invoke(o);
                    return;
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
        if (o instanceof int[] a) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < a.length; i++) b.append(i > 0 ? "," : "").append(a[i]);
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
}

// EnchantmentHelper-level vectors: modifyDamage, modifyKnockback, modifyArmorEffectiveness,
// getDamageProtection / isImmuneToDamage, processDurabilityChange (seeded level random),
// forEachModifier and Player.getDestroySpeed, each as one JSON line with its inputs.
class EnchantHelperVectors {
    static final String[] TARGETS = {"minecraft:player", "minecraft:zombie", "minecraft:skeleton", "minecraft:spider",
            "minecraft:cave_spider", "minecraft:cow", "minecraft:drowned", "minecraft:guardian", "minecraft:wither_skeleton"};

    static Map<String, Object> line(String kind) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("kind", kind);
        return m;
    }

    static ItemStack stack(MinecraftServer server, String item, Map<String, Integer> ench) {
        ItemStack s = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(item)));
        CombatVectors.enchant(server, s, ench);
        return s;
    }

    static Map<String, Integer> ench(Object... kv) {
        Map<String, Integer> m = new LinkedHashMap<>();
        for (int i = 0; i < kv.length; i += 2) m.put((String) kv[i], (Integer) kv[i + 1]);
        return m;
    }

    static net.minecraft.world.entity.Entity entity(MinecraftServer server, ServerPlayer player, String type) {
        if (type.equals("minecraft:player")) return player;
        var e = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(type))
                .create(server.overworld(), net.minecraft.world.entity.EntitySpawnReason.COMMAND);
        e.setPos(0.5, 100.0, 2.5);
        return e;
    }

    static net.minecraft.world.damagesource.DamageSource source(ServerLevel level, ServerPlayer attacker, String name) {
        var d = level.damageSources();
        return switch (name) {
            case "player_attack" -> d.playerAttack(attacker);
            case "fall" -> d.fall();
            case "in_fire" -> d.inFire();
            case "hot_floor" -> d.hotFloor();
            case "explosion" -> d.explosion(null, null);
            case "thrown" -> d.thrown(null, null);
            case "magic" -> d.magic();
            case "out_of_world" -> d.fellOutOfWorld();
            default -> d.generic();
        };
    }

    static void run(MinecraftServer server, List<String> out) throws Exception {
        ServerLevel level = server.overworld();
        ServerPlayer attacker = CombatVectors.mockPlayer(server, "EnchHelperA");
        ServerPlayer wearer = CombatVectors.mockPlayer(server, "EnchHelperB");
        attacker.setPos(0.5, 100.0, 0.5);
        wearer.setPos(0.5, 100.0, 2.5);
        var melee = level.damageSources().playerAttack(attacker);

        // modifyDamage / modifyKnockback / modifyArmorEffectiveness.
        List<Map<String, Integer>> weapons = new ArrayList<>();
        for (String e : new String[] {"minecraft:sharpness", "minecraft:smite", "minecraft:bane_of_arthropods", "minecraft:impaling"})
            for (int l = 1; l <= 5; l++) weapons.add(ench(e, l));
        weapons.add(ench("minecraft:sharpness", 5, "minecraft:smite", 3));
        weapons.add(ench("minecraft:knockback", 2, "minecraft:sharpness", 1));
        weapons.add(ench("minecraft:sharpness", 255));
        weapons.add(ench("minecraft:fire_aspect", 2));
        for (Map<String, Integer> w : weapons) {
            ItemStack stack = stack(server, "minecraft:diamond_sword", w);
            for (String t : TARGETS) {
                var target = entity(server, wearer, t);
                for (float base : new float[] {1.0f, 7.0f, 0.3f}) {
                    Map<String, Object> m = line("damage");
                    m.put("item", "minecraft:diamond_sword");
                    m.put("enchantments", w);
                    m.put("target", t);
                    m.put("base", base);
                    m.put("result", EnchantmentHelper.modifyDamage(level, stack, target, melee, base));
                    out.add(CombatVectors.toJson(m));
                }
            }
        }
        for (Map<String, Integer> w : List.of(ench("minecraft:knockback", 1), ench("minecraft:knockback", 2), ench("minecraft:knockback", 7),
                ench("minecraft:sharpness", 3))) {
            ItemStack stack = stack(server, "minecraft:stone_sword", w);
            for (float base : new float[] {0.0f, 1.0f, 2.5f}) {
                Map<String, Object> m = line("knockback");
                m.put("item", "minecraft:stone_sword");
                m.put("enchantments", w);
                m.put("target", "minecraft:player");
                m.put("base", base);
                m.put("result", EnchantmentHelper.modifyKnockback(level, stack, wearer, melee, base));
                out.add(CombatVectors.toJson(m));
            }
        }
        for (int l = 0; l <= 5; l++) {
            ItemStack stack = stack(server, "minecraft:mace", l == 0 ? ench() : ench("minecraft:breach", l));
            for (float h : new float[] {0.008f, 0.2f, 0.5f, 0.8f}) {
                Map<String, Object> m = line("armor_effectiveness");
                m.put("item", "minecraft:mace");
                m.put("enchantments", l == 0 ? ench() : ench("minecraft:breach", l));
                m.put("target", "minecraft:player");
                m.put("base", h);
                m.put("result", EnchantmentHelper.modifyArmorEffectiveness(level, stack, wearer, melee, h));
                out.add(CombatVectors.toJson(m));
            }
        }

        // getDamageProtection / isImmuneToDamage for armor sets.
        String[] armorItems = {"minecraft:iron_boots", "minecraft:iron_leggings", "minecraft:iron_chestplate", "minecraft:iron_helmet"};
        EquipmentSlot[] armorSlots = {EquipmentSlot.FEET, EquipmentSlot.LEGS, EquipmentSlot.CHEST, EquipmentSlot.HEAD};
        List<List<Map<String, Integer>>> sets = List.of(
                List.of(ench("minecraft:protection", 4), ench("minecraft:protection", 4), ench("minecraft:protection", 4), ench("minecraft:protection", 4)),
                List.of(ench("minecraft:feather_falling", 4, "minecraft:frost_walker", 2), ench(), ench("minecraft:fire_protection", 3), ench("minecraft:blast_protection", 2)),
                List.of(ench("minecraft:protection", 2), ench("minecraft:projectile_protection", 4), ench("minecraft:protection", 1, "minecraft:thorns", 3), ench()),
                List.of(ench("minecraft:protection", 30), ench(), ench(), ench()),
                List.of(ench("minecraft:depth_strider", 3), ench("minecraft:swift_sneak", 3), ench("minecraft:unbreaking", 3), ench("minecraft:respiration", 3, "minecraft:aqua_affinity", 1)));
        String[] sources = {"player_attack", "fall", "in_fire", "hot_floor", "explosion", "thrown", "magic", "out_of_world", "generic"};
        for (List<Map<String, Integer>> set : sets) {
            for (int i = 0; i < 4; i++) wearer.setItemSlot(armorSlots[i], stack(server, armorItems[i], set.get(i)));
            for (String src : sources) {
                var source = source(level, attacker, src);
                Map<String, Object> m = line("protection");
                m.put("armor", armorItems);
                m.put("armor_enchantments", set);
                m.put("source", src);
                m.put("result", EnchantmentHelper.getDamageProtection(level, wearer, source));
                m.put("immune", EnchantmentHelper.isImmuneToDamage(level, wearer, source));
                out.add(CombatVectors.toJson(m));
            }
        }
        for (EquipmentSlot s : armorSlots) wearer.setItemSlot(s, ItemStack.EMPTY);

        // processDurabilityChange with a seeded level random.
        String[][] durabilityItems = {{"minecraft:diamond_sword", "minecraft:unbreaking"}, {"minecraft:diamond_chestplate", "minecraft:unbreaking"},
                {"minecraft:elytra", "minecraft:unbreaking"}, {"minecraft:iron_pickaxe", "minecraft:efficiency"}};
        for (String[] di : durabilityItems) {
            for (int l = 1; l <= 3; l++) {
                ItemStack stack = stack(server, di[0], ench(di[1], l));
                for (int amount : new int[] {1, 2, 5, 13, 40}) {
                    for (long seed : new long[] {0, 1, 42, 1234567, -99}) {
                        level.getRandom().setSeed(seed);
                        Map<String, Object> m = line("durability");
                        m.put("item", di[0]);
                        m.put("enchantments", ench(di[1], l));
                        m.put("amount", amount);
                        m.put("seed", seed);
                        m.put("result", EnchantmentHelper.processDurabilityChange(level, stack, amount));
                        m.put("next_int", level.getRandom().nextInt());
                        out.add(CombatVectors.toJson(m));
                    }
                }
            }
        }

        // forEachModifier per slot.
        Object[][] modItems = {
                {"minecraft:diamond_pickaxe", ench("minecraft:efficiency", 1)}, {"minecraft:diamond_pickaxe", ench("minecraft:efficiency", 5)},
                {"minecraft:diamond_sword", ench("minecraft:sweeping_edge", 1)}, {"minecraft:diamond_sword", ench("minecraft:sweeping_edge", 2)},
                {"minecraft:diamond_sword", ench("minecraft:sweeping_edge", 3)}, {"minecraft:iron_helmet", ench("minecraft:aqua_affinity", 1, "minecraft:respiration", 3)},
                {"minecraft:iron_chestplate", ench("minecraft:fire_protection", 4, "minecraft:blast_protection", 3)},
                {"minecraft:iron_boots", ench("minecraft:depth_strider", 3, "minecraft:soul_speed", 2)},
                {"minecraft:iron_leggings", ench("minecraft:swift_sneak", 3)}};
        for (Object[] mi : modItems) {
            @SuppressWarnings("unchecked")
            Map<String, Integer> e = (Map<String, Integer>) mi[1];
            ItemStack stack = stack(server, (String) mi[0], e);
            for (EquipmentSlot slot : EquipmentSlot.values()) {
                List<Object> mods = new ArrayList<>();
                EnchantmentHelper.forEachModifier(stack, slot, (attr, mod) -> mods.add(List.of(
                        attr.unwrapKey().orElseThrow().identifier().toString(), mod.id().toString(), mod.amount(),
                        mod.operation().getSerializedName())));
                Map<String, Object> m = line("modifiers");
                m.put("item", mi[0]);
                m.put("enchantments", e);
                m.put("slot", slot.getSerializedName());
                m.put("result", mods);
                out.add(CombatVectors.toJson(m));
            }
        }

        // Player.getDestroySpeed with efficiency and aqua affinity.
        String[] blocks = {"minecraft:stone", "minecraft:dirt", "minecraft:obsidian", "minecraft:oak_log", "minecraft:cobweb"};
        Object[][] tools = {{null, ench()}, {"minecraft:wooden_pickaxe", ench()}, {"minecraft:iron_pickaxe", ench("minecraft:efficiency", 1)},
                {"minecraft:diamond_pickaxe", ench("minecraft:efficiency", 5)}, {"minecraft:diamond_shovel", ench("minecraft:efficiency", 3)},
                {"minecraft:golden_axe", ench("minecraft:efficiency", 2)}, {"minecraft:shears", ench("minecraft:efficiency", 4)}};
        net.minecraft.core.BlockPos base = net.minecraft.core.BlockPos.containing(8.5, 100, 8.5);
        for (boolean water : new boolean[] {false, true}) {
            var fill = water ? net.minecraft.world.level.block.Blocks.WATER.defaultBlockState() : net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
            for (int dy = 0; dy < 3; dy++) level.setBlock(base.above(dy), fill, 2);
            for (boolean helmet : new boolean[] {false, true}) {
                wearer.setItemSlot(EquipmentSlot.HEAD, helmet ? stack(server, "minecraft:turtle_helmet", ench("minecraft:aqua_affinity", 1)) : ItemStack.EMPTY);
                for (Object[] tool : tools) {
                    @SuppressWarnings("unchecked")
                    Map<String, Integer> e = (Map<String, Integer>) tool[1];
                    wearer.getInventory().setSelectedSlot(0);
                    wearer.setItemSlot(EquipmentSlot.MAINHAND, tool[0] == null ? ItemStack.EMPTY : stack(server, (String) tool[0], e));
                    CombatVectors.call(wearer, "detectEquipmentUpdates");
                    for (boolean onGround : new boolean[] {true, false}) {
                        wearer.setPos(8.5, 100.0, 8.5);
                        wearer.setOnGround(onGround);
                        CombatVectors.call(wearer, "updateFluidInteraction");
                        for (String b : blocks) {
                            var state = BuiltInRegistries.BLOCK.getValue(Identifier.parse(b)).defaultBlockState();
                            Map<String, Object> m = line("destroy_speed");
                            m.put("item", tool[0]);
                            m.put("enchantments", e);
                            m.put("helmet_aqua_affinity", helmet);
                            m.put("on_ground", onGround);
                            m.put("eye_in_water", wearer.isEyeInFluid(net.minecraft.tags.FluidTags.WATER));
                            m.put("block", b);
                            m.put("mining_efficiency", wearer.getAttributeValue(net.minecraft.world.entity.ai.attributes.Attributes.MINING_EFFICIENCY));
                            m.put("submerged_mining_speed", wearer.getAttributeValue(net.minecraft.world.entity.ai.attributes.Attributes.SUBMERGED_MINING_SPEED));
                            m.put("result", wearer.getDestroySpeed(state));
                            out.add(CombatVectors.toJson(m));
                        }
                    }
                }
            }
        }
        for (int dy = 0; dy < 3; dy++) level.setBlock(base.above(dy), net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(), 2);
        server.getPlayerList().remove(attacker);
        server.getPlayerList().remove(wearer);
    }
}

// Riptide vectors: TridentItem.releaseUsing from the ground under ceilings of every height (the
// 1.2 lift and what stops it), in the air, crouching and at an angle; and the spin's check
// (LivingEntity.checkAutoSpinAttack) against walls and the entities around: a horizontal
// collision ends the spin only when no entity at all is touched.
class RiptideVectors {
    static Map<String, Object> line(String kind) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("kind", kind);
        return m;
    }

    static void clear(ServerLevel level) {
        var air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
        for (int x = -4; x <= 4; x++)
            for (int y = -2; y <= 8; y++)
                for (int z = -4; z <= 4; z++)
                    level.setBlock(new net.minecraft.core.BlockPos((int) Math.floor(CombatVectors.BX) + x, (int) CombatVectors.BY + y, (int) Math.floor(CombatVectors.BZ) + z), air, 2);
        for (var e : new ArrayList<>(level.getEntities((net.minecraft.world.entity.Entity) null, new net.minecraft.world.phys.AABB(-8, 90, -8, 8, 120, 8), e -> !(e instanceof ServerPlayer)))) e.discard();
    }

    static void block(ServerLevel level, int x, int y, int z, String state) throws Exception {
        var st = net.minecraft.commands.arguments.blocks.BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, state, false).blockState();
        level.setBlock(new net.minecraft.core.BlockPos((int) Math.floor(CombatVectors.BX) + x, (int) CombatVectors.BY + y, (int) Math.floor(CombatVectors.BZ) + z), st, 2);
    }

    static void run(MinecraftServer server, List<String> out) throws Exception {
        ServerLevel level = server.overworld();
        // ---- the lift
        String[][] ceilings = {
            {"open", ""}, {"c2", "2:minecraft:stone"}, {"c3", "3:minecraft:stone"}, {"c4", "4:minecraft:stone"},
            {"slab_top_2", "2:minecraft:stone_slab[type=top]"}, {"slab_bottom_2", "2:minecraft:stone_slab[type=bottom]"},
            {"slab_top_3", "3:minecraft:stone_slab[type=top]"}, {"trapdoor_2", "2:minecraft:oak_trapdoor[half=bottom,open=false]"},
            {"c3_edge", "3:minecraft:stone:edge"}, {"c2_edge", "2:minecraft:stone:edge"}};
        int n = 0;
        for (String[] ceil : ceilings) {
            for (int lvl : new int[] {1, 3}) {
                for (int variant = 0; variant < 4; variant++) {
                    boolean onGround = variant != 3;
                    boolean sneak = variant == 2;
                    float pitch = new float[] {0f, -40f, 25f, 10f}[variant];
                    float yaw = new float[] {0f, 90f, -135f, 30f}[variant];
                    double dx = variant == 1 ? 0.3 : 0.0;
                    Map<String, Object> m = line("lift");
                    m.put("name", "riptide_lift/" + ceil[0] + "/" + lvl + "/" + variant);
                    m.put("ceiling", ceil[1]);
                    m.put("level", lvl);
                    m.put("on_ground", onGround);
                    m.put("sneak", sneak);
                    m.put("pitch", pitch);
                    m.put("yaw", yaw);
                    m.put("dx", dx);
                    clear(level);
                    ServerPlayer p = CombatVectors.mockPlayer(server, "Rip" + (n++));
                    for (int x = -2; x <= 2; x++) for (int z = -2; z <= 2; z++) block(level, x, -1, z, "minecraft:stone");
                    block(level, 0, 0, 0, "minecraft:water");
                    block(level, 1, 0, 0, "minecraft:water");
                    block(level, 0, 0, 1, "minecraft:water");
                    block(level, 1, 0, 1, "minecraft:water");
                    if (!ceil[1].isEmpty()) {
                        String[] c = ceil[1].split(":", 2);
                        int y = Integer.parseInt(c[0]);
                        String rest = c[1];
                        boolean edge = rest.endsWith(":edge");
                        if (edge) rest = rest.substring(0, rest.length() - 5);
                        // The ceiling covers the box fully, or only the half of the box east of x = 0.5.
                        for (int x = edge ? 1 : -1; x <= 1; x++) for (int z = -1; z <= 1; z++) block(level, x, y, z, rest);
                    }
                    p.setGameMode(net.minecraft.world.level.GameType.SURVIVAL);
                    CombatVectors.call(p.connection, "markClientLoaded");
                    p.setPos(CombatVectors.BX + dx, CombatVectors.BY, CombatVectors.BZ);
                    p.setYRot(yaw);
                    p.setXRot(pitch);
                    p.setDeltaMovement(Vec3.ZERO);
                    p.setOnGround(onGround);
                    p.setShiftKeyDown(sneak);
                    if (sneak) p.setPose(net.minecraft.world.entity.Pose.CROUCHING);
                    p.fallDistance = 0.0;
                    ItemStack trident = EnchantHelperVectors.stack(server, "minecraft:trident", EnchantHelperVectors.ench("minecraft:riptide", lvl));
                    p.getInventory().clearContent();
                    p.setItemSlot(EquipmentSlot.MAINHAND, trident);
                    CombatVectors.call(p, "detectEquipmentUpdates");
                    CombatVectors.call(p, "updateFluidInteraction");
                    p.verticalCollision = false;
                    boolean wet = p.isInWaterOrRain();
                    CombatVectors.drain(p);
                    boolean released = trident.getItem().releaseUsing(trident, level, p, 72000 - 20);
                    m.put("wet", wet);
                    Map<String, Object> r = new LinkedHashMap<>();
                    r.put("released", released);
                    r.put("pos", new double[] {p.getX() - CombatVectors.BX, p.getY() - CombatVectors.BY, p.getZ() - CombatVectors.BZ});
                    r.put("delta", CombatVectors.vec(p.getDeltaMovement()));
                    r.put("on_ground", p.onGround());
                    r.put("spin", p.isAutoSpinAttack());
                    r.put("needs_sync", p.needsSync);
                    r.put("trident_damage", p.getMainHandItem().getDamageValue());
                    m.put("result", r);
                    out.add(CombatVectors.toJson(m));
                    server.getPlayerList().remove(p);
                }
            }
        }
        // ---- the spin's touch check
        String[] around = {"nothing", "item", "pig", "minecart", "pig_and_item"};
        for (String what : around) {
            for (boolean collision : new boolean[] {false, true}) {
                for (int ticks : new int[] {20, 5, 1}) {
                    Map<String, Object> m = line("touch");
                    m.put("name", "riptide_touch/" + what + "/" + (collision ? "wall" : "free") + "/" + ticks);
                    m.put("around", what);
                    m.put("collision", collision);
                    m.put("ticks", ticks);
                    clear(level);
                    ServerPlayer p = CombatVectors.mockPlayer(server, "Touch" + (n++));
                    for (int x = -2; x <= 2; x++) for (int z = -2; z <= 2; z++) block(level, x, -1, z, "minecraft:stone");
                    p.setGameMode(net.minecraft.world.level.GameType.SURVIVAL);
                    CombatVectors.call(p.connection, "markClientLoaded");
                    p.setPos(CombatVectors.BX, CombatVectors.BY, CombatVectors.BZ);
                    p.setYRot(0f);
                    p.setXRot(0f);
                    p.setOnGround(true);
                    p.setDeltaMovement(new Vec3(0.4, 0.1, -0.3));
                    p.getInventory().clearContent();
                    ItemStack trident = new ItemStack(net.minecraft.world.item.Items.TRIDENT);
                    p.setItemSlot(EquipmentSlot.MAINHAND, trident);
                    CombatVectors.call(p, "detectEquipmentUpdates");
                    CombatVectors.set(p, "attackStrengthTicker", 100);
                    p.getAttribute(net.minecraft.world.entity.ai.attributes.Attributes.MAX_ABSORPTION).setBaseValue(20.0);
                    List<net.minecraft.world.entity.Entity> made = new ArrayList<>();
                    if (what.equals("item") || what.equals("pig_and_item")) {
                        var item = new net.minecraft.world.entity.item.ItemEntity(level, CombatVectors.BX + 0.2, CombatVectors.BY, CombatVectors.BZ, new ItemStack(net.minecraft.world.item.Items.STONE));
                        item.setDeltaMovement(Vec3.ZERO);
                        item.setNoGravity(true);
                        item.setPickUpDelay(1000);
                        level.addFreshEntity(item);
                        made.add(item);
                    }
                    if (what.equals("pig") || what.equals("pig_and_item")) {
                        var pig = net.minecraft.world.entity.EntityTypes.PIG.create(level, net.minecraft.world.entity.EntitySpawnReason.COMMAND);
                        pig.setPos(CombatVectors.BX - 0.3, CombatVectors.BY, CombatVectors.BZ + 0.2);
                        pig.setNoAi(true);
                        pig.setYRot(0f);
                        level.addFreshEntity(pig);
                        made.add(pig);
                    }
                    if (what.equals("minecart")) {
                        var cart = net.minecraft.world.entity.EntityTypes.MINECART.create(level, net.minecraft.world.entity.EntitySpawnReason.COMMAND);
                        cart.setPos(CombatVectors.BX + 0.1, CombatVectors.BY, CombatVectors.BZ);
                        cart.setNoGravity(true);
                        level.addFreshEntity(cart);
                        made.add(cart);
                    }
                    level.getRandom().setSeed(77);
                    p.getRandom().setSeed(78);
                    p.horizontalCollision = collision;
                    p.startAutoSpinAttack(ticks, 8.0f, trident);
                    CombatVectors.drain(p);
                    // `aiStep`: the count goes down, then the check.
                    int now = (Integer) CombatVectors.get(p, "autoSpinAttackTicks") - 1;
                    CombatVectors.set(p, "autoSpinAttackTicks", now);
                    var box = p.getBoundingBox();
                    for (Class<?> k = p.getClass(); k != null; k = k.getSuperclass()) {
                        try {
                            var meth = k.getDeclaredMethod("checkAutoSpinAttack", net.minecraft.world.phys.AABB.class, net.minecraft.world.phys.AABB.class);
                            meth.setAccessible(true);
                            meth.invoke(p, box, box);
                            break;
                        } catch (NoSuchMethodException e) {
                            // up
                        }
                    }
                    Map<String, Object> r = new LinkedHashMap<>();
                    r.put("spin_ticks", (Integer) CombatVectors.get(p, "autoSpinAttackTicks"));
                    r.put("spin", p.isAutoSpinAttack());
                    r.put("delta", CombatVectors.vec(p.getDeltaMovement()));
                    List<Object> hp = new ArrayList<>();
                    for (var e : made) {
                        if (e instanceof net.minecraft.world.entity.LivingEntity le && !(e instanceof net.minecraft.world.entity.decoration.ArmorStand)) hp.add(le.getHealth());
                    }
                    r.put("health", hp);
                    m.put("result", r);
                    out.add(CombatVectors.toJson(m));
                    for (var e : made) e.discard();
                    server.getPlayerList().remove(p);
                }
            }
        }
        clear(level);
    }
}

// Mount vectors: HorseInventoryMenu over horses, donkeys, mules (and llamas): saddle and armor
// slots that take only what the animal can wear when it is tame and grown, a chest's slots,
// random click sequences through AbstractContainerMenu.clicked, and what each leaves in the
// menu's slots, the carried stack and on the ground.
class MountVectors {
    static final String[] POOL = {"minecraft:saddle", "minecraft:iron_horse_armor", "minecraft:diamond_horse_armor", "minecraft:leather_horse_armor",
        "minecraft:golden_horse_armor", "minecraft:copperhorse_armor", "minecraft:netherite_horse_armor", "minecraft:white_carpet", "minecraft:red_carpet", "minecraft:wolf_armor",
        "minecraft:stone", "minecraft:apple", "minecraft:golden_apple", "minecraft:hay_block", "minecraft:chest", "minecraft:diamond_chestplate",
        "minecraft:elytra", "minecraft:carrot_on_a_stick", "minecraft:oak_boat"};

    static String pick(Random r) {
        String s = POOL[r.nextInt(POOL.length)];
        return s.equals("minecraft:copperhorse_armor") ? "minecraft:copper_horse_armor" : s;
    }

    static ItemStack stack(String item, int count) {
        var it = BuiltInRegistries.ITEM.getValue(Identifier.parse(item));
        return new ItemStack(it, count);
    }

    static Object[] snapshot(net.minecraft.world.inventory.AbstractContainerMenu menu) {
        List<Object> slots = new ArrayList<>();
        for (var slot : menu.slots) {
            ItemStack st = slot.getItem();
            slots.add(st.isEmpty() ? null : List.of(BuiltInRegistries.ITEM.getKey(st.getItem()).toString(), st.getCount(), st.getDamageValue()));
        }
        ItemStack c = menu.getCarried();
        return new Object[] {slots, c.isEmpty() ? null : List.of(BuiltInRegistries.ITEM.getKey(c.getItem()).toString(), c.getCount(), c.getDamageValue())};
    }

    static void run(MinecraftServer server, List<String> out, int sequences, long seed) throws Exception {
        ServerLevel level = server.overworld();
        String[] kinds = {"horse", "donkey", "mule", "skeleton_horse", "donkey", "mule", "horse"};
        Random rng = new Random(seed);
        int n = 0;
        for (int seq = 0; seq < sequences; seq++) {
            String kind = kinds[rng.nextInt(kinds.length)];
            boolean tamed = true; // (the screen only opens for a tame animal)
            boolean baby = false; // (nor for a foal)
            boolean chest = rng.nextBoolean();
            boolean creative = rng.nextInt(6) == 0;
            RiptideVectors.clear(level);
            ServerPlayer p = CombatVectors.mockPlayer(server, "Mount" + (n++));
            p.setGameMode(creative ? net.minecraft.world.level.GameType.CREATIVE : net.minecraft.world.level.GameType.SURVIVAL);
            CombatVectors.call(p.connection, "markClientLoaded");
            p.setPos(CombatVectors.BX, CombatVectors.BY, CombatVectors.BZ);
            p.getInventory().clearContent();
            var type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse("minecraft:" + kind));
            var horse = (net.minecraft.world.entity.animal.equine.AbstractHorse) type.create(level, net.minecraft.world.entity.EntitySpawnReason.COMMAND);
            horse.setPos(CombatVectors.BX + 1.5, CombatVectors.BY, CombatVectors.BZ);
            horse.setTamed(tamed);
            if (baby) horse.setBaby(true);
            if (horse instanceof net.minecraft.world.entity.animal.equine.AbstractChestedHorse ch) {
                ch.setChest(chest);
                CombatVectors.call(ch, "createInventory");
            }
            var inv = (net.minecraft.world.SimpleContainer) CombatVectors.get(horse, "inventory");
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("kind", "mount");
            m.put("name", "mount/" + seq);
            m.put("horse", kind);
            m.put("tamed", tamed);
            m.put("baby", baby);
            m.put("chest", chest);
            m.put("creative", creative);
            List<Object> invList = new ArrayList<>();
            double fill = rng.nextDouble();
            for (int i = 0; i < inv.getContainerSize(); i++) {
                if (rng.nextDouble() < fill * 0.6) {
                    String it = pick(rng);
                    int count = Math.min(1 + rng.nextInt(20), stack(it, 1).getMaxStackSize());
                    inv.setItem(i, stack(it, count));
                    invList.add(List.of(i, it, count));
                }
            }
            m.put("inventory", invList);
            if (rng.nextInt(3) == 0) {
                String it = rng.nextBoolean() ? "minecraft:saddle" : pick(rng);
                if (horse.isEquippableInSlot(stack(it, 1), EquipmentSlot.SADDLE)) {
                    horse.setItemSlot(EquipmentSlot.SADDLE, stack(it, 1));
                    m.put("saddle", it);
                }
            }
            if (rng.nextInt(3) == 0) {
                String it = pick(rng);
                if (horse.isEquippableInSlot(stack(it, 1), EquipmentSlot.BODY)) {
                    horse.setItemSlot(EquipmentSlot.BODY, stack(it, 1));
                    m.put("body", it);
                }
            }
            level.addFreshEntity(horse);
            List<Object> player = new ArrayList<>();
            for (int i = 0; i < 36; i++) {
                if (rng.nextDouble() < fill * 0.7) {
                    String it = pick(rng);
                    int count = Math.min(1 + rng.nextInt(30), stack(it, 1).getMaxStackSize());
                    p.getInventory().setItem(i, stack(it, count));
                    player.add(List.of(i, it, count));
                }
            }
            m.put("player_inventory", player);
            m.put("columns", horse.getInventoryColumns());
            p.openHorseInventory(horse, inv);
            var menu = p.containerMenu;
            CombatVectors.drain(p);
            m.put("open", snapshot(menu));
            CombatVectors.field(net.minecraft.world.inventory.AbstractContainerMenu.class, "quickcraftSlots").set(menu, new java.util.LinkedHashSet<>());
            List<Object> steps = new ArrayList<>();
            int count = 5 + rng.nextInt(30);
            int drag = -1;
            for (int k = 0; k < count; k++) {
                int size = menu.slots.size();
                int slot, button, input;
                if (drag >= 0) {
                    if (rng.nextInt(4) == 0) { slot = -999; button = net.minecraft.world.inventory.AbstractContainerMenu.getQuickcraftMask(2, drag); input = 5; drag = -1; }
                    else { slot = rng.nextInt(size); button = net.minecraft.world.inventory.AbstractContainerMenu.getQuickcraftMask(1, drag); input = 5; }
                } else {
                    slot = rng.nextInt(25) == 0 ? -999 : rng.nextInt(size);
                    if (rng.nextInt(3) == 0) slot = rng.nextInt(Math.min(size, 2 + 3 * horse.getInventoryColumns()));
                    int r = rng.nextInt(100);
                    if (r < 45) { button = rng.nextInt(2); input = 0; }
                    else if (r < 65) { button = rng.nextInt(2); input = 1; }
                    else if (r < 75) { button = rng.nextInt(12) == 0 ? 40 : rng.nextInt(9); input = 2; }
                    else if (r < 80) { button = rng.nextInt(2); input = 4; }
                    else if (r < 90) { drag = rng.nextInt(2); slot = -999; button = net.minecraft.world.inventory.AbstractContainerMenu.getQuickcraftMask(0, drag); input = 5; }
                    else if (r < 96) { button = rng.nextInt(2); input = 6; }
                    else { button = 2; input = creative ? 3 : 0; }
                }
                var ci = net.minecraft.world.inventory.ContainerInput.values()[input];
                boolean crash = false;
                try {
                    menu.clicked(slot, button, ci, p);
                    menu.broadcastChanges();
                } catch (RuntimeException e) {
                    crash = true;
                }
                Map<String, Object> step = new LinkedHashMap<>();
                step.put("slot", slot);
                step.put("button", button);
                step.put("input", input);
                if (crash) {
                    step.put("crash", true);
                    steps.add(step);
                    break;
                }
                Object[] snap = snapshot(menu);
                step.put("slots", snap[0]);
                step.put("carried", snap[1]);
                steps.add(step);
                if (!horse.isAlive() || p.containerMenu != menu) break;
            }
            m.put("steps", steps);
            // What lay on the ground afterwards (items by name and count).
            Map<String, Integer> dropped = new java.util.TreeMap<>();
            for (var e : level.getEntities((net.minecraft.world.entity.Entity) null, new net.minecraft.world.phys.AABB(-6, 90, -6, 6, 120, 6), e -> e instanceof net.minecraft.world.entity.item.ItemEntity)) {
                var st = ((net.minecraft.world.entity.item.ItemEntity) e).getItem();
                dropped.merge(BuiltInRegistries.ITEM.getKey(st.getItem()).toString(), st.getCount(), Integer::sum);
            }
            m.put("dropped", dropped);
            // The horse's own state afterwards.
            Map<String, Object> state = new LinkedHashMap<>();
            for (var slot : new EquipmentSlot[] {EquipmentSlot.SADDLE, EquipmentSlot.BODY}) {
                ItemStack st = horse.getItemBySlot(slot);
                state.put(slot.getName(), st.isEmpty() ? null : BuiltInRegistries.ITEM.getKey(st.getItem()).toString());
            }
            m.put("horse_state", state);
            out.add(CombatVectors.toJson(m));
            horse.discard();
            for (var e : new ArrayList<>(level.getEntities((net.minecraft.world.entity.Entity) null, new net.minecraft.world.phys.AABB(-6, 90, -6, 6, 120, 6), e -> e instanceof net.minecraft.world.entity.item.ItemEntity))) e.discard();
            server.getPlayerList().remove(p);
        }
    }
}

// Spear vectors (wp34): the piercing attack (`PiercingWeapon.attack` through the STAB player
// action) and charging with a kinetic weapon (`KineticWeapon.damageEntities` every tick of use),
// by mock players against players and mobs: damage, knockback, dismounting, the lunge
// enchantment, durability, exhaustion and the attack cooldown, and the sounds and swing the
// target hears and sees.
class SpearVectors {
    static Map<String, Object> line(String kind) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("kind", kind);
        return m;
    }

    static net.minecraft.core.BlockPos at(int x, int y, int z) {
        return new net.minecraft.core.BlockPos(x, (int) CombatVectors.BY + y, z);
    }

    static void clear(ServerLevel level) {
        var air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
        for (int x = -4; x <= 4; x++)
            for (int y = -3; y <= 6; y++)
                for (int z = -4; z <= 14; z++) level.setBlock(at(x, y, z), air, 2);
        for (var e : new ArrayList<>(level.getEntities((net.minecraft.world.entity.Entity) null, new net.minecraft.world.phys.AABB(-10, 90, -10, 10, 120, 20), e -> !(e instanceof ServerPlayer)))) e.discard();
    }

    static void gameTime(ServerLevel level, long t) throws Exception {
        var data = (net.minecraft.world.level.storage.ServerLevelData) CombatVectors.get(level, "serverLevelData");
        data.setGameTime(t);
    }

    static CombatVectors.Side player(String spear, double dz, float yaw) {
        CombatVectors.Side s = new CombatVectors.Side();
        s.mainHand = spear;
        s.dz = dz;
        s.yaw = yaw;
        return s;
    }

    static net.minecraft.world.entity.LivingEntity mob(ServerLevel level, String type, double dx, double dy, double dz, float yaw) {
        var e = (net.minecraft.world.entity.LivingEntity) BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(type))
                .create(level, net.minecraft.world.entity.EntitySpawnReason.COMMAND);
        e.setPos(CombatVectors.BX + dx, CombatVectors.BY + dy, CombatVectors.BZ + dz);
        e.setYRot(yaw);
        if (e instanceof net.minecraft.world.entity.Mob m) m.setNoAi(true);
        if (e instanceof net.minecraft.world.entity.Mob m && m.getAttribute(Attributes.MAX_HEALTH) != null) m.setPersistenceRequired();
        level.addFreshEntity(e);
        return e;
    }

    static List<Object> sounds(List<Object> packets) {
        List<Object> out = new ArrayList<>();
        for (Object p : packets) {
            if (p instanceof net.minecraft.network.protocol.game.ClientboundSoundPacket s) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("sound", s.getSound().value().location().toString());
                m.put("source", s.getSource().getName());
                m.put("volume", s.getVolume());
                m.put("pitch", s.getPitch());
                out.add(m);
            }
        }
        return out;
    }

    static Object swing(List<Object> packets, int entityId) {
        for (Object p : packets) {
            if (p instanceof net.minecraft.network.protocol.game.ClientboundSwingAnimationPacket s && s.entityId() == entityId) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("hand", s.hand().name());
                m.put("type", s.animation().type().name());
                m.put("duration", s.animation().duration());
                return m;
            }
        }
        return null;
    }

    static Map<String, Object> playerOutcome(ServerPlayer p, int attackerId) throws Exception {
        List<Object> packets = CombatVectors.drain(p);
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("health", p.getHealth());
        m.put("absorption", p.getAbsorptionAmount());
        m.put("exhaustion", (Float) CombatVectors.get(p.getFoodData(), "exhaustionLevel"));
        m.put("ticker", (Integer) CombatVectors.get(p, "attackStrengthTicker"));
        ItemStack main = p.getMainHandItem();
        m.put("main_hand_damage", main.getDamageValue());
        m.put("main_hand_count", main.getCount());
        EquipmentSlot[] slots = {EquipmentSlot.FEET, EquipmentSlot.LEGS, EquipmentSlot.CHEST, EquipmentSlot.HEAD};
        List<Object> armor = new ArrayList<>();
        for (EquipmentSlot slot : slots) {
            ItemStack a = p.getItemBySlot(slot);
            armor.add(a.isEmpty() ? null : a.getDamageValue());
        }
        m.put("armor_damage", armor);
        Object motion = null;
        for (Object pkt : packets) {
            if (pkt instanceof net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket mp && mp.id() == p.getId()) motion = CombatVectors.vec(mp.movement());
        }
        m.put("motion", motion);
        m.put("velocity", CombatVectors.vec(p.getDeltaMovement()));
        m.put("pending_motion", p.syncVelocity ? CombatVectors.vec(p.getDeltaMovement()) : null);
        m.put("sounds", sounds(packets));
        m.put("swing", swing(packets, attackerId));
        m.put("fire_ticks", p.getRemainingFireTicks());
        m.put("on_ground", p.onGround());
        m.put("vehicle", p.getVehicle() == null ? null : BuiltInRegistries.ENTITY_TYPE.getKey(p.getVehicle().getType()).toString());
        return m;
    }

    /** A fireball or wind charge hovering at an offset of the attacker (it does not tick here). */
    static net.minecraft.world.entity.Entity projectile(ServerLevel level, String type, double dx, double dy, double dz) {
        var e = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(type)).create(level, net.minecraft.world.entity.EntitySpawnReason.COMMAND);
        e.setPos(CombatVectors.BX + dx, CombatVectors.BY + dy, CombatVectors.BZ + dz);
        if (type.endsWith("fireball") || type.endsWith("wind_charge")) e.setDeltaMovement(0.0, 0.0, 0.5);
        level.addFreshEntity(e);
        return e;
    }

    static Map<String, Object> mobOutcome(net.minecraft.world.entity.Entity e) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("type", BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
        if (e instanceof net.minecraft.world.entity.projectile.Projectile p) {
            m.put("owner", p.getOwner() == null ? null : p.getOwner().getId());
            if (e instanceof net.minecraft.world.entity.projectile.hurtingprojectile.AbstractHurtingProjectile h) m.put("acceleration", h.accelerationPower);
        }
        m.put("health", e instanceof net.minecraft.world.entity.LivingEntity l ? l.getHealth() : 0.0f);
        m.put("alive", e.isAlive());
        m.put("velocity", CombatVectors.vec(e.getDeltaMovement()));
        m.put("vehicle", e.getVehicle() == null ? null : BuiltInRegistries.ENTITY_TYPE.getKey(e.getVehicle().getType()).toString());
        m.put("fire_ticks", e.getRemainingFireTicks());
        m.put("hurt_time", e instanceof net.minecraft.world.entity.LivingEntity l ? l.hurtTime : 0);
        return m;
    }

    static void stab(ServerPlayer a) {
        a.connection.handlePlayerAction(new net.minecraft.network.protocol.game.ServerboundPlayerActionPacket(
                net.minecraft.network.protocol.game.ServerboundPlayerActionPacket.Action.STAB, net.minecraft.core.BlockPos.ZERO, net.minecraft.core.Direction.DOWN, 0));
    }

    /** One stab by an attacker holding a spear at a target player (`tgt` null) or mobs. */
    static void stabCase(MinecraftServer server, List<String> out, String name, CombatVectors.Side att, CombatVectors.Side tgt, String[][] mobs,
                         float pitch, int food, String twist, int n) throws Exception {
        ServerLevel level = server.overworld();
        clear(level);
        gameTime(level, 5000);
        var cmd = server.createCommandSourceStack();
        server.getCommands().performPrefixedCommand(cmd, "gamerule minecraft:pvp true");
        server.getCommands().performPrefixedCommand(cmd, "difficulty normal");
        ServerPlayer a = CombatVectors.mockPlayer(server, "Stab" + n);
        ServerPlayer t = tgt != null ? CombatVectors.mockPlayer(server, "Mark" + n) : null;
        CombatVectors.setup(server, a, att);
        if (t != null) CombatVectors.setup(server, t, tgt);
        a.setXRot(pitch);
        a.getFoodData().setFoodLevel(food);
        List<net.minecraft.world.entity.Entity> made = new ArrayList<>();
        if (mobs != null) {
            for (String[] mb : mobs) {
                if (mb[0].endsWith("fireball") || mb[0].endsWith("wind_charge") || mb[0].endsWith("boat") || mb[0].endsWith("raft") || mb[0].endsWith("minecart")) {
                    made.add(projectile(level, mb[0], Double.parseDouble(mb[1]), Double.parseDouble(mb[2]), Double.parseDouble(mb[3])));
                    continue;
                }
                var e = mob(level, mb[0], Double.parseDouble(mb[1]), Double.parseDouble(mb[2]), Double.parseDouble(mb[3]), 180f);
                if (mb.length > 4 && !mb[4].isEmpty()) {
                    for (String piece : mb[4].split(",")) {
                        String[] kv = piece.split("=");
                        e.setItemSlot(EquipmentSlot.valueOf(kv[0]), new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(kv[1]))));
                    }
                }
                made.add(e);
            }
        }
        // Twists: a wall between them, the attacker mounted, in water, gliding, a shield up.
        switch (twist) {
            case "wall" -> level.setBlock(at(0, 1, 1), net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(), 2);
            case "wall_close" -> level.setBlock(at(0, 1, 1), net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(), 2);
            case "wall_high" -> level.setBlock(at(0, 1, 1), net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(), 2);
            case "mounted" -> {
                var pig = mob(level, "minecraft:pig", 0.0, 0.0, 0.0, 0f);
                a.setPos(CombatVectors.BX + att.dx, CombatVectors.BY + att.dy, CombatVectors.BZ + att.dz);
                a.startRiding(pig, true, false);
                made.add(pig);
            }
            case "water" -> {
                level.setBlock(at(0, 0, 0), net.minecraft.world.level.block.Blocks.WATER.defaultBlockState(), 2);
                level.setBlock(at(0, 1, 0), net.minecraft.world.level.block.Blocks.WATER.defaultBlockState(), 2);
                CombatVectors.call(a, "updateFluidInteraction");
            }
            case "glide" -> a.startFallFlying();
            case "shield" -> {
                t.setItemSlot(EquipmentSlot.OFFHAND, new ItemStack(net.minecraft.world.item.Items.SHIELD));
                t.startUsingItem(net.minecraft.world.InteractionHand.OFF_HAND);
            }
            default -> {}
        }
        CombatVectors.drain(a);
        if (t != null) CombatVectors.drain(t);
        long seed = name.hashCode();
        level.getRandom().setSeed(seed);
        a.getRandom().setSeed(seed + 1);
        if (t != null) t.getRandom().setSeed(seed + 2);
        if (twist.equals("melee")) a.attack(made.get(0)); else stab(a);
        Map<String, Object> m = line("stab");
        m.put("name", name);
        m.put("level_seed", seed);
        m.put("attacker", att.json());
        m.put("attacker_pitch", pitch);
        m.put("food", food);
        m.put("twist", twist);
        m.put("target", tgt != null ? tgt.json() : null);
        List<Object> ms = new ArrayList<>();
        if (mobs != null) for (String[] mb : mobs) ms.add(mb);
        m.put("mobs", ms);
        Map<String, Object> r = new LinkedHashMap<>();
        r.put("attacker", playerOutcome(a, a.getId()));
        if (t != null) r.put("target", playerOutcome(t, a.getId()));
        List<Object> mo = new ArrayList<>();
        for (var e : made) mo.add(mobOutcome(e));
        r.put("mobs", mo);
        m.put("result", r);
        out.add(CombatVectors.toJson(m));
        for (var e : made) e.discard();
        server.getPlayerList().remove(a);
        if (t != null) server.getPlayerList().remove(t);
        clear(level);
    }

    static void stabs(MinecraftServer server, List<String> out) throws Exception {
        int n = 0;
        String[] spears = {"minecraft:wooden_spear", "minecraft:iron_spear", "minecraft:netherite_spear", "minecraft:golden_spear", "minecraft:copper_spear"};
        double[] dists = {1.0, 2.0, 2.5, 3.5, 4.0, 4.5, 5.0, 6.5};
        for (String spear : spears) {
            for (double d : dists) {
                CombatVectors.Side a = player(spear, 0.0, 0f);
                CombatVectors.Side t = player(null, d, 180f);
                stabCase(server, out, "stab/" + spear + "/" + d, a, t, null, 0f, 17, "", n++);
            }
        }
        // The attack cooldown: a spear wants its charge.
        for (int ticker : new int[] {0, 5, 10, 13, 14, 15, 20, 100}) {
            CombatVectors.Side a = player("minecraft:iron_spear", 0.0, 0f);
            a.ticker = ticker;
            stabCase(server, out, "stab_ticker/" + ticker, a, player(null, 3.0, 180f), null, 0f, 17, "", n++);
        }
        // Enchantments of the weapon.
        Object[][] enchants = {
            {"lunge1", "minecraft:lunge", 1}, {"lunge2", "minecraft:lunge", 2}, {"lunge3", "minecraft:lunge", 3},
            {"sharpness3", "minecraft:sharpness", 3}, {"fire2", "minecraft:fire_aspect", 2}, {"knockback2", "minecraft:knockback", 2},
            {"unbreaking3", "minecraft:unbreaking", 3}, {"looting3", "minecraft:looting", 3}};
        for (Object[] e : enchants) {
            CombatVectors.Side a = player("minecraft:iron_spear", 0.0, 0f);
            a.ench((String) e[1], (Integer) e[2]);
            a.mainHandDamage = 100;
            stabCase(server, out, "stab_ench/" + e[0], a, player(null, 3.0, 180f), null, 0f, 17, "", n++);
        }
        // Lunge's conditions: hunger, creative, riding, water, gliding.
        for (int food : new int[] {20, 7, 6, 0}) {
            CombatVectors.Side a = player("minecraft:iron_spear", 0.0, 0f);
            a.ench("minecraft:lunge", 2);
            stabCase(server, out, "stab_lunge_food/" + food, a, player(null, 3.0, 180f), null, 0f, food, "", n++);
        }
        for (String twist : new String[] {"mounted", "water", "glide", "wall", "shield"}) {
            CombatVectors.Side a = player("minecraft:iron_spear", 0.0, 0f);
            a.ench("minecraft:lunge", 1);
            CombatVectors.Side t = player(null, 3.0, 180f);
            stabCase(server, out, "stab_twist/" + twist, a, t, null, 0f, 17, twist, n++);
        }
        {
            CombatVectors.Side a = player("minecraft:iron_spear", 0.0, 0f);
            a.ench("minecraft:lunge", 1);
            a.gameMode = "creative";
            stabCase(server, out, "stab_lunge_creative", a, player(null, 3.0, 180f), null, 0f, 3, "", n++);
        }
        // Looking up and down, with the target higher and lower.
        for (float pitch : new float[] {-30f, -10f, 15f, 40f}) {
            for (double dy : new double[] {0.0, 1.5, -1.0}) {
                CombatVectors.Side t = player(null, 3.0, 180f);
                t.dy = dy;
                stabCase(server, out, "stab_pitch/" + pitch + "/" + dy, player("minecraft:iron_spear", 0.0, 0f), t, null, pitch, 17, "", n++);
            }
        }
        // Targets with armor, absorption, an old hurt, sneaking away.
        {
            CombatVectors.Side t = player(null, 3.0, 180f);
            t.armor = new String[] {"minecraft:iron_boots", "minecraft:iron_leggings", "minecraft:iron_chestplate", "minecraft:iron_helmet"};
            stabCase(server, out, "stab_armor", player("minecraft:diamond_spear", 0.0, 0f), t, null, 0f, 17, "", n++);
            CombatVectors.Side t2 = player(null, 3.0, 180f);
            t2.absorption = 4.0f;
            t2.health = 5.0f;
            stabCase(server, out, "stab_absorption", player("minecraft:diamond_spear", 0.0, 0f), t2, null, 0f, 17, "", n++);
            CombatVectors.Side t3 = player(null, 3.0, 180f);
            t3.hurtCooldown = 15;
            t3.lastHurt = 3.0f;
            stabCase(server, out, "stab_cooldown", player("minecraft:diamond_spear", 0.0, 0f), t3, null, 0f, 17, "", n++);
            CombatVectors.Side t4 = player(null, 3.0, 180f);
            t4.gameMode = "creative";
            stabCase(server, out, "stab_creative_target", player("minecraft:diamond_spear", 0.0, 0f), t4, null, 0f, 17, "", n++);
            CombatVectors.Side t5 = player(null, 3.0, 180f);
            t5.health = 0.5f;
            stabCase(server, out, "stab_kill", player("minecraft:diamond_spear", 0.0, 0f), t5, null, 0f, 17, "", n++);
        }
        // Mobs: one, two in a line (both are pierced), behind a wall, riding.
        stabCase(server, out, "stab_mob/pig", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {{"minecraft:pig", "0.0", "0.0", "3.0"}}, 0f, 17, "", n++);
        stabCase(server, out, "stab_mob/zombie_armor", player("minecraft:iron_spear", 0.0, 0f), null,
                new String[][] {{"minecraft:zombie", "0.0", "0.0", "3.0", "HEAD=minecraft:iron_helmet,CHEST=minecraft:iron_chestplate"}}, 0f, 17, "", n++);
        stabCase(server, out, "stab_mob/two", player("minecraft:iron_spear", 0.0, 0f), null,
                new String[][] {{"minecraft:pig", "0.0", "0.0", "3.0"}, {"minecraft:zombie", "0.0", "0.0", "4.0"}}, 0f, 17, "", n++);
        stabCase(server, out, "stab_mob/wall", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {{"minecraft:pig", "0.0", "0.0", "3.0"}}, 0f, 17, "wall", n++);
        stabCase(server, out, "stab_mob/fire", ench(player("minecraft:iron_spear", 0.0, 0f), "minecraft:fire_aspect", 1), null,
                new String[][] {{"minecraft:cow", "0.0", "0.0", "3.0"}}, 0f, 17, "", n++);
        stabCase(server, out, "stab_mob/smite", ench(player("minecraft:iron_spear", 0.0, 0f), "minecraft:smite", 4), null,
                new String[][] {{"minecraft:zombie", "0.0", "0.0", "3.0"}}, 0f, 17, "", n++);
        stabCase(server, out, "stab_mob/baby_far", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {{"minecraft:chicken", "0.0", "0.0", "4.4"}}, 0f, 17, "", n++);
        // wp41: `Player.deflectProjectile`: a fireball or wind charge hit with a knockback stab (or
        // a melee attack) flies on along the attacker's look with the attacker as its owner.
        String[] fb = {"minecraft:fireball", "0.0", "1.0", "3.0"};
        stabCase(server, out, "stab_projectile/fireball", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {fb}, 0f, 17, "", n++);
        stabCase(server, out, "stab_projectile/fireball_pitch", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {{"minecraft:fireball", "0.0", "0.0", "3.0"}}, 15f, 17, "", n++);
        stabCase(server, out, "stab_projectile/fireball_up", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {{"minecraft:fireball", "0.0", "3.0", "2.0"}}, -50f, 17, "", n++);
        stabCase(server, out, "stab_projectile/breeze_wind_charge", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {{"minecraft:breeze_wind_charge", "0.0", "1.6", "3.0"}}, 0f, 17, "", n++);
        stabCase(server, out, "stab_projectile/small_fireball", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {{"minecraft:small_fireball", "0.0", "1.4", "3.0"}}, 0f, 17, "", n++);
        stabCase(server, out, "stab_projectile/fireball_and_pig", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {fb, {"minecraft:pig", "0.0", "0.0", "4.0"}}, 0f, 17, "", n++);
        stabCase(server, out, "stab_projectile/fireball_wall", player("minecraft:iron_spear", 0.0, 0f), null, new String[][] {fb}, 0f, 17, "wall_high", n++);
        stabCase(server, out, "melee_projectile/fireball", player("minecraft:iron_sword", 0.0, 0f), null, new String[][] {{"minecraft:fireball", "0.0", "1.0", "2.0"}}, 0f, 17, "melee", n++);
        stabCase(server, out, "melee_projectile/fireball_pitch", player("minecraft:iron_sword", 0.0, 0f), null, new String[][] {{"minecraft:fireball", "0.0", "1.0", "2.0"}}, -30f, 17, "melee", n++);
        stabCase(server, out, "melee_projectile/breeze_wind_charge", player("minecraft:iron_sword", 0.0, 0f), null, new String[][] {{"minecraft:breeze_wind_charge", "0.0", "1.6", "2.0"}}, 0f, 17, "melee", n++);
        // Vehicles (boats and carts are pickable and attackable): stabbed, pushed, broken.
        for (String vehicle : new String[] {"minecraft:oak_boat", "minecraft:bamboo_raft", "minecraft:minecart", "minecraft:chest_minecart", "minecraft:tnt_minecart"}) {
            for (String spear : new String[] {"minecraft:wooden_spear", "minecraft:iron_spear"}) {
                stabCase(server, out, "stab_vehicle/" + vehicle.substring(10) + "/" + spear.substring(10), player(spear, 0.0, 0f), null, new String[][] {{vehicle, "0.0", "0.0", "3.0"}}, 30f, 17, "", n++);
            }
        }
        stabCase(server, out, "melee_projectile/fist", player(null, 0.0, 0f), null, new String[][] {{"minecraft:fireball", "0.0", "1.0", "2.0"}}, 0f, 17, "melee", n++);
        stabCase(server, out, "stab_mob/player_and_mob", player("minecraft:iron_spear", 0.0, 0f), player(null, 3.0, 180f), new String[][] {{"minecraft:pig", "0.0", "0.0", "4.0"}}, 0f, 17, "", n++);
    }

    static CombatVectors.Side ench(CombatVectors.Side s, String id, int level) {
        s.ench(id, level);
        return s;
    }

    /**
     * Charging: the attacker starts using the spear and walks (or rides) at `speed` blocks per tick
     * along its look direction for `ticks` ticks, its position and known movement updated like a
     * client's; every tick `ItemStack.onUseTick`. The target is a player or mobs that stand or walk.
     */
    static void charge(MinecraftServer server, List<String> out, String name, CombatVectors.Side att, double speed, double targetStep, String targetKind,
                       int ticks, int n) throws Exception {
        ServerLevel level = server.overworld();
        clear(level);
        long start = 7000;
        gameTime(level, start);
        var cmd = server.createCommandSourceStack();
        server.getCommands().performPrefixedCommand(cmd, "gamerule minecraft:pvp true");
        server.getCommands().performPrefixedCommand(cmd, "difficulty normal");
        ServerPlayer a = CombatVectors.mockPlayer(server, "Charge" + n);
        CombatVectors.setup(server, a, att);
        ServerPlayer tp = null;
        net.minecraft.world.entity.LivingEntity tm = null;
        net.minecraft.world.entity.Entity mount = null;
        double tz = 7.0;
        if (targetKind.equals("player")) {
            tp = CombatVectors.mockPlayer(server, "Mark" + n);
            CombatVectors.setup(server, tp, player(null, tz, 180f));
        } else if (targetKind.equals("pig")) {
            tm = mob(level, "minecraft:pig", 0.0, 0.0, tz, 180f);
        } else if (targetKind.equals("mounted_zombie")) {
            mount = mob(level, "minecraft:pig", 0.0, 0.0, tz, 180f);
            tm = mob(level, "minecraft:zombie", 0.0, 0.0, tz, 180f);
            tm.startRiding(mount, true, false);
        }
        long seed = name.hashCode();
        level.getRandom().setSeed(seed);
        a.getRandom().setSeed(seed + 1);
        CombatVectors.drain(a);
        if (tp != null) CombatVectors.drain(tp);
        // `Item.use`: the use begins and its sound goes to the others.
        a.getMainHandItem().use(level, a, net.minecraft.world.InteractionHand.MAIN_HAND);
        CombatVectors.drain(a);
        Map<String, Object> m = line("charge");
        m.put("name", name);
        m.put("level_seed", seed);
        m.put("attacker", att.json());
        m.put("speed", speed);
        m.put("target_step", targetStep);
        m.put("target_kind", targetKind);
        m.put("ticks", ticks);
        m.put("start_time", start);
        m.put("use_duration", a.getUseItem().getUseDuration(a));
        List<Object> states = new ArrayList<>();
        Vec3 step = new Vec3(0.0, 0.0, speed);
        for (int t = 0; t < ticks; t++) {
            gameTime(level, start + t);
            a.setPos(a.getX(), a.getY(), a.getZ() + step.z);
            a.setKnownMovement(step);
            if (tp != null) {
                tp.setPos(tp.getX(), tp.getY(), tp.getZ() + targetStep);
                tp.setKnownMovement(new Vec3(0.0, 0.0, targetStep));
            }
            if (tm != null) {
                var mover = mount != null ? mount : tm;
                mover.setPos(mover.getX(), mover.getY(), mover.getZ() + targetStep);
                if (mount != null) tm.setPos(mover.getX(), mover.getY() + 0.7, mover.getZ());
                CombatVectors.set(mover, "lastKnownSpeed", new Vec3(0.0, 0.0, targetStep));
                if (mount != null) CombatVectors.set(mount, "lastKnownSpeed", new Vec3(0.0, 0.0, targetStep));
            }
            ItemStack use = a.getUseItem();
            use.onUseTick(level, a, a.getUseItemRemainingTicks());
            CombatVectors.set(a, "useItemRemaining", a.getUseItemRemainingTicks() - 1);
            Map<String, Object> s = new LinkedHashMap<>();
            s.put("t", t);
            s.put("a_delta", CombatVectors.vec(a.getDeltaMovement()));
            s.put("a_damage", a.getMainHandItem().getDamageValue());
            s.put("a_exhaustion", (Float) CombatVectors.get(a.getFoodData(), "exhaustionLevel"));
            if (tp != null) {
                s.put("t_health", tp.getHealth());
                s.put("t_delta", CombatVectors.vec(tp.getDeltaMovement()));
                s.put("t_pending", tp.syncVelocity ? CombatVectors.vec(tp.getDeltaMovement()) : null);
            }
            if (tm != null) {
                s.put("m_health", tm.getHealth());
                s.put("m_delta", CombatVectors.vec(tm.getDeltaMovement()));
                s.put("m_vehicle", tm.getVehicle() != null);
                s.put("m_z", tm.getZ() - CombatVectors.BZ);
            }
            if (tp != null) {
                List<Object> pk = CombatVectors.drain(tp);
                s.put("t_sounds", sounds(pk));
                Object motion = null;
                for (Object pkt : pk) if (pkt instanceof net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket mp && mp.id() == tp.getId()) motion = CombatVectors.vec(mp.movement());
                s.put("t_motion", motion);
                s.put("t_swing", swing(pk, a.getId()));
            }
            states.add(s);
        }
        m.put("states", states);
        out.add(CombatVectors.toJson(m));
        if (tm != null) tm.discard();
        if (mount != null) mount.discard();
        server.getPlayerList().remove(a);
        if (tp != null) server.getPlayerList().remove(tp);
        clear(level);
    }

    static void charges(MinecraftServer server, List<String> out) throws Exception {
        int n = 0;
        double[] speeds = {0.1, 0.2, 0.25, 0.3, 0.35, 0.45, 0.6, 0.8};
        for (String spear : new String[] {"minecraft:wooden_spear", "minecraft:iron_spear", "minecraft:netherite_spear"}) {
            for (double speed : speeds) {
                charge(server, out, "charge/" + spear + "/" + speed, player(spear, 0.0, 0f), speed, 0.0, "player", 40, n++);
            }
        }
        // The target walks toward the charger, and away from it.
        for (double step : new double[] {-0.1, -0.25, 0.1, 0.2}) {
            charge(server, out, "charge_walking/" + step, player("minecraft:iron_spear", 0.0, 0f), 0.3, step, "player", 40, n++);
        }
        for (double speed : new double[] {0.25, 0.4, 0.7}) {
            charge(server, out, "charge_pig/" + speed, player("minecraft:iron_spear", 0.0, 0f), speed, 0.0, "pig", 40, n++);
            charge(server, out, "charge_mounted/" + speed, player("minecraft:iron_spear", 0.0, 0f), speed, 0.0, "mounted_zombie", 40, n++);
        }
        CombatVectors.Side lunge = player("minecraft:iron_spear", 0.0, 0f);
        lunge.ench("minecraft:sharpness", 3);
        charge(server, out, "charge_sharpness", lunge, 0.4, 0.0, "player", 40, n++);
        CombatVectors.Side worn = player("minecraft:iron_spear", 0.0, 0f);
        worn.ench("minecraft:unbreaking", 2);
        worn.mainHandDamage = 200;
        charge(server, out, "charge_worn", worn, 0.45, 0.0, "pig", 40, n++);
    }

    static void run(MinecraftServer server, List<String> out) throws Exception {
        stabs(server, out);
        charges(server, out);
    }
}

// wp44 melee vectors: `Player.attack` against mobs and players with every wrinkle (sweeping against
// both, critical hits, effects, armored and enchanted mobs, riding, water) and the mace (smash
// attack, density, breach, wind burst, the knockback blast). One scenario sets the attacker, a list
// of victims (players or /summoned frozen mobs, the first `target` is hit) and blocks, then attacks
// one or more times (each time after setting the attack strength ticker) and records after each hit:
// the attacker, every victim and the world packets the attacker got (sounds, particles, level
// events, animations, explosions, motions).
class MeleeVectors {
    static final class Victim {
        CombatVectors.Side player;
        String name;
        String type;
        String nbt = "";
        double dx, dy, dz;
        float yaw = 180f;
        String vehicle;
        List<Object[]> effects = new ArrayList<>(); // players: {id, amplifier, duration}

        Victim(CombatVectors.Side p) {
            player = p;
        }

        Victim(String type, double dx, double dy, double dz, String nbt) {
            this.type = type;
            this.dx = dx;
            this.dy = dy;
            this.dz = dz;
            this.nbt = nbt;
        }

        Victim vehicle(String v) {
            vehicle = v;
            return this;
        }

        String command() {
            double x = CombatVectors.BX + dx, y = CombatVectors.BY + dy, z = CombatVectors.BZ + dz;
            String inner = "NoAI:1b,PersistenceRequired:1b" + (nbt.isEmpty() ? "" : "," + nbt);
            if (vehicle == null) return "summon " + type + " " + x + " " + y + " " + z + " {" + inner + ",Rotation:[" + yaw + "f,0f]}";
            return "summon " + vehicle + " " + x + " " + y + " " + z + " {Rotation:[" + yaw + "f,0f],Passengers:[{id:\"" + type + "\"," + inner + "}]}";
        }
    }

    static final class Case {
        final String name;
        final CombatVectors.Side attacker = new CombatVectors.Side();
        final List<Victim> victims = new ArrayList<>();
        int target;
        boolean fallFlying, mounted;
        final List<Object[]> blocks = new ArrayList<>(); // {x, y, z, state}
        final List<Object[]> effects = new ArrayList<>(); // attacker effects {id, amplifier, duration}
        int[] later = new int[0]; // attack strength tickers of the attacks after the first
        String difficulty = "normal";
        boolean pvp = true;
        float pitch;

        Case(String name) {
            this.name = "melee/" + name;
        }

        Case weapon(String item) {
            attacker.mainHand = item;
            return this;
        }

        Case ench(String id, int level) {
            attacker.ench(id, level);
            return this;
        }

        Case mob(String type, double dx, double dy, double dz, String nbt) {
            victims.add(new Victim(type, dx, dy, dz, nbt));
            return this;
        }

        Case mob(String type, double dz, String nbt) {
            return mob(type, 0.0, 0.0, dz, nbt);
        }

        Case player(CombatVectors.Side s) {
            victims.add(new Victim(s));
            return this;
        }

        Case fall(double d) {
            attacker.onGround = false;
            attacker.fallDistance = d;
            return this;
        }

        Case effect(String id, int amp, int dur) {
            effects.add(new Object[] {id, amp, dur});
            return this;
        }

        Case vehicle(int victim, String v) {
            victims.get(victim).vehicle = v;
            return this;
        }

        Case block(int x, int y, int z, String state) {
            blocks.add(new Object[] {x, y, z, state});
            return this;
        }
    }

    // ---------------------------------------------------------------- scenario tables

    static CombatVectors.Side side(double dx, double dz) {
        CombatVectors.Side s = new CombatVectors.Side();
        s.dx = dx;
        s.dz = dz;
        s.yaw = 180f;
        return s;
    }

    static CombatVectors.Side armored(CombatVectors.Side s, String mat) {
        s.armor = new String[] {"minecraft:" + mat + "_boots", "minecraft:" + mat + "_leggings", "minecraft:" + mat + "_chestplate", "minecraft:" + mat + "_helmet"};
        return s;
    }

    /** `equipment:{...}` of a mob: slot name, item, and an optional `components` body. */
    static String equipment(String... slotItemComponents) {
        StringBuilder b = new StringBuilder("equipment:{");
        for (int i = 0; i < slotItemComponents.length; i += 3) {
            if (i > 0) b.append(",");
            b.append(slotItemComponents[i]).append(":{id:\"").append(slotItemComponents[i + 1]).append("\",count:1");
            if (slotItemComponents[i + 2] != null && !slotItemComponents[i + 2].isEmpty()) b.append(",components:{").append(slotItemComponents[i + 2]).append("}");
            b.append("}");
        }
        return b.append("}").toString();
    }

    static String ironSet() {
        return equipment("head", "minecraft:iron_helmet", "", "chest", "minecraft:iron_chestplate", "", "legs", "minecraft:iron_leggings", "", "feet", "minecraft:iron_boots", "");
    }

    static String diamondSet(String components) {
        return equipment("head", "minecraft:diamond_helmet", components, "chest", "minecraft:diamond_chestplate", components,
                "legs", "minecraft:diamond_leggings", components, "feet", "minecraft:diamond_boots", components);
    }

    static String protection(int level) {
        return "\"minecraft:enchantments\":{\"minecraft:protection\":" + level + "}";
    }

    static String join(String... parts) {
        StringBuilder b = new StringBuilder();
        for (String p : parts) {
            if (p == null || p.isEmpty()) continue;
            if (b.length() > 0) b.append(",");
            b.append(p);
        }
        return b.toString();
    }

    static final String[][] MOB_TYPES = {
        {"pig", ""}, {"cow", ""}, {"sheep", ""}, {"chicken", ""}, {"rabbit", ""}, {"horse", ""}, {"wolf", ""}, {"cat", ""}, {"goat", ""}, {"llama", ""},
        {"zombie", ""}, {"husk", ""}, {"drowned", ""}, {"zombie_villager", ""}, {"zombified_piglin", ""}, {"skeleton", ""}, {"stray", ""},
        {"wither_skeleton", ""}, {"spider", ""}, {"silverfish", ""}, {"endermite", ""}, {"creeper", ""}, {"blaze", ""},
        {"slime", "Size:1"}, {"magma_cube", "Size:1"}, {"iron_golem", ""}, {"snow_golem", ""}, {"villager", ""}, {"polar_bear", ""},
        {"ravager", ""}, {"hoglin", ""}, {"piglin", ""}, {"guardian", ""}, {"turtle", ""}, {"phantom", ""}, {"vex", ""}, {"witch", ""},
        {"pillager", ""}, {"vindicator", ""}, {"evoker", ""}, {"ghast", ""}, {"strider", ""}, {"cod", ""},
        {"zoglin", ""}, {"enderman", ""}, {"allay", ""}, {"armadillo", ""}, {"camel", ""}, {"frog", ""}, {"axolotl", ""},
    };

    static void mobTypes(List<Case> out) {
        for (String[] t : MOB_TYPES) {
            String id = "minecraft:" + t[0];
            Case c = new Case("type/" + t[0] + "/fist").mob(id, 2.0, t[1]);
            out.add(c);
            c = new Case("type/" + t[0] + "/sword").weapon("minecraft:iron_sword").mob(id, 2.0, t[1]);
            c.attacker.sprinting = true;
            out.add(c);
        }
        for (String t : new String[] {"zombie", "pig", "skeleton", "iron_golem"}) {
            out.add(new Case("type_baby/" + t).weapon("minecraft:diamond_sword").mob("minecraft:" + t, 2.0, t.equals("pig") ? "Age:-24000" : "IsBaby:1b"));
        }
        out.add(new Case("type_ground/zombie").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "OnGround:1b"));
        out.add(new Case("type_ground/pig_sprint").weapon("minecraft:stone_sword").mob("minecraft:pig", 2.0, "OnGround:1b"));
        out.get(out.size() - 1).attacker.sprinting = true;
        out.add(new Case("type_ground/iron_golem").weapon("minecraft:netherite_sword").mob("minecraft:iron_golem", 2.0, "OnGround:1b"));
        out.add(new Case("type_ground/ravager_kb").weapon("minecraft:diamond_sword").ench("minecraft:knockback", 2).mob("minecraft:ravager", 2.0, "OnGround:1b"));
    }

    static void weapons(List<Case> out) {
        String[] weapons = {null, "minecraft:stick", "minecraft:wooden_sword", "minecraft:stone_sword", "minecraft:iron_sword", "minecraft:diamond_sword",
                "minecraft:netherite_sword", "minecraft:golden_sword", "minecraft:wooden_axe", "minecraft:iron_axe", "minecraft:diamond_axe", "minecraft:netherite_axe",
                "minecraft:iron_shovel", "minecraft:diamond_pickaxe", "minecraft:iron_hoe", "minecraft:trident", "minecraft:shears", "minecraft:copper_sword", "minecraft:copper_axe"};
        for (String w : weapons) {
            for (int ticker : new int[] {100, 13, 6, 0}) {
                String n = (w == null ? "fist" : w.substring(10)) + "/" + ticker;
                Case c = new Case("weapon/zombie/" + n).weapon(w).mob("minecraft:zombie", 2.0, "");
                c.attacker.ticker = ticker;
                out.add(c);
            }
        }
        for (String w : new String[] {"minecraft:diamond_sword", "minecraft:iron_axe", null}) {
            Case c = new Case("weapon/worn/" + (w == null ? "fist" : w.substring(10))).weapon(w).mob("minecraft:pig", 2.0, "");
            c.attacker.mainHandDamage = w == null ? 0 : (w.contains("sword") ? 1560 : 249);
            out.add(c);
        }
        Case c = new Case("weapon/breaks").weapon("minecraft:wooden_sword").mob("minecraft:pig", 2.0, "");
        c.attacker.mainHandDamage = 58;
        out.add(c);
    }

    static void enchants(List<Case> out) {
        for (int l : new int[] {1, 3, 5}) {
            out.add(new Case("ench/sharpness_" + l + "/zombie").weapon("minecraft:diamond_sword").ench("minecraft:sharpness", l).mob("minecraft:zombie", 2.0, ""));
            out.add(new Case("ench/sharpness_" + l + "/axe_pig_partial").weapon("minecraft:iron_axe").ench("minecraft:sharpness", l).mob("minecraft:pig", 2.0, ""));
            out.get(out.size() - 1).attacker.ticker = 8;
            for (String t : new String[] {"zombie", "skeleton", "drowned", "pig", "phantom", "wither_skeleton", "zombified_piglin"}) {
                out.add(new Case("ench/smite_" + l + "/" + t).weapon("minecraft:iron_sword").ench("minecraft:smite", l).mob("minecraft:" + t, 2.0, ""));
            }
            for (String t : new String[] {"spider", "silverfish", "endermite", "pig", "zombie"}) {
                out.add(new Case("ench/bane_" + l + "/" + t).weapon("minecraft:iron_sword").ench("minecraft:bane_of_arthropods", l).mob("minecraft:" + t, 2.0, ""));
            }
        }
        for (int l : new int[] {1, 3}) {
            for (String t : new String[] {"guardian", "drowned", "cod", "zombie", "turtle", "axolotl"}) {
                out.add(new Case("ench/impaling_" + l + "/" + t).weapon("minecraft:trident").ench("minecraft:impaling", l).mob("minecraft:" + t, 2.0, ""));
            }
        }
        out.add(new Case("ench/impaling_in_water").weapon("minecraft:trident").ench("minecraft:impaling", 3).mob("minecraft:zombie", 2.0, "").block(0, 0, 2, "minecraft:water"));
        for (int l : new int[] {1, 2}) {
            for (String t : new String[] {"pig", "zombie", "blaze", "wither_skeleton", "zombified_piglin", "strider", "skeleton", "magma_cube", "iron_golem", "ghast"}) {
                out.add(new Case("ench/fire_" + l + "/" + t).weapon("minecraft:diamond_sword").ench("minecraft:fire_aspect", l).mob("minecraft:" + t, 2.0, t.equals("magma_cube") ? "Size:1" : ""));
            }
            out.add(new Case("ench/fire_" + l + "/zombie_fire_resistance").weapon("minecraft:diamond_sword").ench("minecraft:fire_aspect", l)
                    .mob("minecraft:zombie", 2.0, "active_effects:[{id:\"minecraft:fire_resistance\",amplifier:0b,duration:600}]"));
            out.add(new Case("ench/fire_" + l + "/pig_already_burning").weapon("minecraft:diamond_sword").ench("minecraft:fire_aspect", l).mob("minecraft:pig", 2.0, "Fire:30s"));
            out.add(new Case("ench/fire_" + l + "/pig_in_water").weapon("minecraft:diamond_sword").ench("minecraft:fire_aspect", l).mob("minecraft:pig", 2.0, "").block(0, 0, 2, "minecraft:water"));
        }
        for (int l : new int[] {1, 2, 3}) {
            for (String t : new String[] {"pig", "zombie", "iron_golem", "ravager", "enderman", "warden_none", "hoglin", "polar_bear"}) {
                if (t.equals("warden_none")) continue;
                Case c = new Case("ench/knockback_" + l + "/" + t).weapon("minecraft:stone_sword").ench("minecraft:knockback", l).mob("minecraft:" + t, 2.0, "OnGround:1b");
                out.add(c);
            }
        }
        out.add(new Case("ench/knockback_sprint").weapon("minecraft:stone_sword").ench("minecraft:knockback", 1).mob("minecraft:zombie", -1.0, 0.0, 1.7, "OnGround:1b"));
        out.get(out.size() - 1).attacker.sprinting = true;
        out.get(out.size() - 1).attacker.yaw = 30f;
        for (long seed : new long[] {1, 2, 3, 4, 5, 6}) {
            Case c = new Case("ench/unbreaking_" + seed).weapon("minecraft:diamond_sword").ench("minecraft:unbreaking", 3).mob("minecraft:pig", 2.0, "");
            c.attacker.mainHandDamage = 100;
            out.add(c);
        }
        out.add(new Case("ench/everything").weapon("minecraft:netherite_sword").ench("minecraft:sharpness", 5).ench("minecraft:fire_aspect", 2).ench("minecraft:knockback", 2)
                .ench("minecraft:looting", 3).mob("minecraft:zombie", 2.0, ironSet()));
        out.add(new Case("ench/smite_bane_split").weapon("minecraft:iron_sword").ench("minecraft:smite", 5).mob("minecraft:spider", 2.0, ""));
    }

    static void crits(List<Case> out) {
        for (String t : new String[] {"pig", "zombie", "iron_golem"}) {
            for (String w : new String[] {"minecraft:diamond_sword", "minecraft:iron_axe", null}) {
                String n = t + "/" + (w == null ? "fist" : w.substring(10));
                out.add(new Case("crit/" + n).weapon(w).fall(0.5).mob("minecraft:" + t, 2.0, ""));
                out.add(new Case("crit_sprint/" + n).weapon(w).fall(0.5).mob("minecraft:" + t, 2.0, ""));
                out.get(out.size() - 1).attacker.sprinting = true;
                Case c = new Case("crit_partial/" + n).weapon(w).fall(0.5).mob("minecraft:" + t, 2.0, "");
                c.attacker.ticker = 6;
                out.add(c);
            }
        }
        out.add(new Case("crit/sharpness_fire").weapon("minecraft:diamond_sword").fall(3.0).ench("minecraft:sharpness", 4).ench("minecraft:fire_aspect", 1).mob("minecraft:zombie", 2.0, ""));
        out.add(new Case("crit/in_water").weapon("minecraft:diamond_sword").fall(1.0).mob("minecraft:zombie", 2.0, "").block(0, 0, 0, "minecraft:water"));
        out.add(new Case("crit/on_ladder").weapon("minecraft:diamond_sword").fall(1.0).mob("minecraft:zombie", 2.0, "").block(0, 0, 0, "minecraft:ladder[facing=north]"));
        Case c = new Case("crit/mounted").weapon("minecraft:diamond_sword").fall(1.0).mob("minecraft:zombie", 2.0, "");
        c.mounted = true;
        out.add(c);
        out.add(new Case("crit/blind").weapon("minecraft:diamond_sword").fall(1.0).effect("minecraft:blindness", 0, 200).mob("minecraft:zombie", 2.0, ""));
        out.add(new Case("crit/gliding").weapon("minecraft:diamond_sword").fall(1.0).mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).fallFlying = true;
        out.add(new Case("crit/zero_fall").weapon("minecraft:diamond_sword").fall(0.0).mob("minecraft:zombie", 2.0, ""));
        out.add(new Case("crit/on_ground_with_fall").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).attacker.fallDistance = 2.0;
        out.add(new Case("crit/vs_armor_stand_like_boat").weapon("minecraft:diamond_sword").fall(1.0).mob("minecraft:zombie", 2.0, "").vehicle(0, "minecraft:oak_boat"));
    }

    static void effects(List<Case> out) {
        for (int amp : new int[] {0, 1, 3}) {
            out.add(new Case("effect/strength_" + amp).weapon("minecraft:iron_sword").effect("minecraft:strength", amp, 600).mob("minecraft:zombie", 2.0, ""));
            out.add(new Case("effect/weakness_" + amp).weapon("minecraft:iron_sword").effect("minecraft:weakness", amp, 600).mob("minecraft:zombie", 2.0, ""));
            out.add(new Case("effect/strength_fist_" + amp).effect("minecraft:strength", amp, 600).mob("minecraft:pig", 2.0, ""));
            out.add(new Case("effect/weakness_fist_" + amp).effect("minecraft:weakness", amp, 600).mob("minecraft:pig", 2.0, ""));
        }
        out.add(new Case("effect/haste_ticker").weapon("minecraft:diamond_sword").effect("minecraft:haste", 1, 600).mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).attacker.ticker = 7;
        out.add(new Case("effect/mining_fatigue_ticker").weapon("minecraft:diamond_sword").effect("minecraft:mining_fatigue", 1, 600).mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).attacker.ticker = 11;
        out.add(new Case("effect/strength_crit").weapon("minecraft:diamond_axe").effect("minecraft:strength", 1, 600).fall(1.0).mob("minecraft:zombie", 2.0, ""));
        out.add(new Case("effect/weakness_sharpness").weapon("minecraft:diamond_sword").ench("minecraft:sharpness", 5).effect("minecraft:weakness", 1, 600).mob("minecraft:zombie", 2.0, ""));
        // Effects on the mob.
        out.add(new Case("target_effect/resistance_1").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "active_effects:[{id:\"minecraft:resistance\",amplifier:1b,duration:600}]"));
        out.add(new Case("target_effect/resistance_4").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "active_effects:[{id:\"minecraft:resistance\",amplifier:4b,duration:600}]"));
        out.add(new Case("target_effect/resistance_armor").weapon("minecraft:netherite_sword").mob("minecraft:zombie", 2.0, join(ironSet(), "active_effects:[{id:\"minecraft:resistance\",amplifier:0b,duration:600}]")));
        out.add(new Case("target_effect/absorption").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "AbsorptionAmount:4f"));
        out.add(new Case("target_effect/absorption_partial").weapon("minecraft:iron_sword").mob("minecraft:zombie", 2.0, join(ironSet(), "AbsorptionAmount:2.5f")));
        out.add(new Case("target_effect/slowness").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "active_effects:[{id:\"minecraft:slowness\",amplifier:2b,duration:600}]"));
        out.add(new Case("target_effect/regeneration").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "active_effects:[{id:\"minecraft:regeneration\",amplifier:2b,duration:600}]"));
        out.add(new Case("target_effect/weakness_bane").weapon("minecraft:iron_sword").ench("minecraft:bane_of_arthropods", 3).mob("minecraft:spider", 2.0, "active_effects:[{id:\"minecraft:slowness\",amplifier:1b,duration:30}]"));
        out.add(new Case("target_effect/bane_long").weapon("minecraft:iron_sword").ench("minecraft:bane_of_arthropods", 5).mob("minecraft:spider", 2.0, "active_effects:[{id:\"minecraft:slowness\",amplifier:6b,duration:900}]"));
    }

    static void armorOnMobs(List<Case> out) {
        for (String mat : new String[] {"leather", "golden", "chainmail", "iron", "diamond", "netherite", "turtle"}) {
            String set = mat.equals("turtle") ? equipment("head", "minecraft:turtle_helmet", "") : equipment("head", "minecraft:" + mat + "_helmet", "", "chest", "minecraft:" + mat + "_chestplate", "",
                    "legs", "minecraft:" + mat + "_leggings", "", "feet", "minecraft:" + mat + "_boots", "");
            for (String t : new String[] {"zombie", "skeleton"}) {
                out.add(new Case("armor/" + mat + "/" + t).weapon("minecraft:diamond_sword").mob("minecraft:" + t, 2.0, set));
            }
        }
        out.add(new Case("armor/diamond_axe_netherite").weapon("minecraft:netherite_axe").mob("minecraft:zombie", 2.0, diamondSet("")));
        out.add(new Case("armor/protection_4").weapon("minecraft:netherite_sword").mob("minecraft:zombie", 2.0, diamondSet(protection(4))));
        out.add(new Case("armor/protection_10").weapon("minecraft:diamond_axe").mob("minecraft:zombie", 2.0, diamondSet(protection(10))));
        out.add(new Case("armor/mixed").weapon("minecraft:netherite_sword").fall(1.0).mob("minecraft:zombie", 2.0,
                equipment("head", "minecraft:turtle_helmet", "", "chest", "minecraft:diamond_chestplate", protection(3), "legs", "minecraft:iron_leggings", "")));
        out.add(new Case("armor/worn_out").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0,
                equipment("chest", "minecraft:iron_chestplate", "\"minecraft:damage\":240", "head", "minecraft:leather_helmet", "\"minecraft:damage\":54")));
        out.add(new Case("armor/fire_protection_fire_aspect").weapon("minecraft:golden_sword").ench("minecraft:fire_aspect", 1).mob("minecraft:zombie", 2.0,
                equipment("chest", "minecraft:iron_chestplate", "\"minecraft:enchantments\":{\"minecraft:fire_protection\":4}")));
        out.add(new Case("armor/holding_sword").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, equipment("mainhand", "minecraft:iron_sword", "")));
        out.add(new Case("armor/pumpkin_head").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, equipment("head", "minecraft:carved_pumpkin", "")));
        out.add(new Case("armor/horse_armor").weapon("minecraft:diamond_sword").mob("minecraft:horse", 2.0, "equipment:{body:{id:\"minecraft:diamond_horse_armor\",count:1}}"));
        out.add(new Case("armor/thorns_chest").weapon("minecraft:iron_sword").mob("minecraft:zombie", 2.0, equipment("chest", "minecraft:iron_chestplate", "\"minecraft:enchantments\":{\"minecraft:thorns\":3}")));
        out.add(new Case("armor/thorns_helmet_1").weapon("minecraft:iron_sword").mob("minecraft:zombie", 2.0, equipment("head", "minecraft:iron_helmet", "\"minecraft:enchantments\":{\"minecraft:thorns\":1}")));
        out.add(new Case("armor/unbreaking_armor").weapon("minecraft:iron_sword").mob("minecraft:zombie", 2.0, ironSet().replace("count:1}", "count:1,components:{\"minecraft:enchantments\":{\"minecraft:unbreaking\":3}}}")));
    }

    static void states(List<Case> out) {
        // The hurt cooldown: a second hit while the mob is still invulnerable (damage <= last hurt) and
        // one that adds the difference.
        out.add(cases2("cooldown/same_weapon", "minecraft:diamond_sword", "minecraft:diamond_sword", new int[] {100}, "minecraft:pig"));
        out.add(cases2("cooldown/stronger_second", "minecraft:wooden_sword", "minecraft:diamond_sword", new int[] {100}, "minecraft:pig"));
        out.add(cases2("cooldown/weaker_second", "minecraft:diamond_sword", "minecraft:wooden_sword", new int[] {100}, "minecraft:pig"));
        out.add(cases2("cooldown/partial_second", "minecraft:diamond_sword", "minecraft:diamond_sword", new int[] {8}, "minecraft:pig"));
        out.add(cases2("cooldown/three_hits", "minecraft:iron_sword", "minecraft:iron_sword", new int[] {100, 100}, "minecraft:zombie"));
        // Health: killing blow, exact kill, fractional health.
        for (float h : new float[] {1.0f, 3.0f, 7.0f, 7.5f, 0.5f, 100.0f}) {
            out.add(new Case("health/" + h).weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "Health:" + h + "f"));
            out.add(new Case("health_pig/" + h).weapon("minecraft:iron_sword").mob("minecraft:pig", 2.0, "Health:" + h + "f"));
        }
        out.add(new Case("health/kill_with_fire").weapon("minecraft:diamond_sword").ench("minecraft:fire_aspect", 2).mob("minecraft:pig", 2.0, "Health:2f"));
        out.add(new Case("health/kill_baby").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "IsBaby:1b,Health:3f"));
        out.add(new Case("health/kill_slime").weapon("minecraft:diamond_sword").mob("minecraft:slime", 2.0, "Size:2,Health:3f"));
        // Directions: the attacker faces other ways, the mob is higher or lower, near or far.
        for (float yaw : new float[] {0f, 45f, 90f, -90f, 135f, -37.5f}) {
            Case c = new Case("dir/yaw_" + yaw).weapon("minecraft:diamond_sword").ench("minecraft:knockback", 1);
            // The mob stands where the attacker looks (x = -sin(yaw) * 2, z = cos(yaw) * 2).
            c.mob("minecraft:zombie", -Math.sin(Math.toRadians(yaw)) * 2.0, 0.0, Math.cos(Math.toRadians(yaw)) * 2.0, "OnGround:1b");
            c.attacker.yaw = yaw;
            out.add(c);
        }
        for (double dy : new double[] {-1.0, 0.5, 1.2}) {
            out.add(new Case("dir/dy_" + dy).weapon("minecraft:iron_sword").mob("minecraft:zombie", 0.0, dy, 2.0, "OnGround:1b"));
        }
        for (double dz : new double[] {0.6, 1.0, 3.0, 3.5}) {
            out.add(new Case("dir/dz_" + dz).weapon("minecraft:iron_sword").mob("minecraft:pig", dz, ""));
        }
        out.add(new Case("dir/same_spot").weapon("minecraft:iron_sword").mob("minecraft:pig", 0.0, 0.0, 0.0, ""));
        // Water, boat, riding, difficulty.
        out.add(new Case("world/zombie_in_water").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "").block(0, 0, 2, "minecraft:water").block(0, 1, 2, "minecraft:water"));
        out.add(new Case("world/attacker_in_water").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "").block(0, 0, 0, "minecraft:water").block(0, 1, 0, "minecraft:water"));
        out.add(new Case("world/zombie_on_boat").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "").vehicle(0, "minecraft:oak_boat"));
        out.add(new Case("world/pig_on_boat_kb").weapon("minecraft:diamond_sword").ench("minecraft:knockback", 2).mob("minecraft:pig", 2.0, "").vehicle(0, "minecraft:oak_boat"));
        out.add(new Case("world/skeleton_in_cobweb").weapon("minecraft:diamond_sword").mob("minecraft:skeleton", 2.0, "").block(0, 0, 2, "minecraft:cobweb"));
        out.add(new Case("world/hard").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).difficulty = "hard";
        out.add(new Case("world/peaceful_pig").weapon("minecraft:diamond_sword").mob("minecraft:pig", 2.0, ""));
        out.get(out.size() - 1).difficulty = "peaceful";
        out.add(new Case("world/easy").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).difficulty = "easy";
        Case c = new Case("world/attacker_creative").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "");
        c.attacker.gameMode = "creative";
        out.add(c);
        c = new Case("world/attacker_adventure").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "");
        c.attacker.gameMode = "adventure";
        out.add(c);
        out.add(new Case("world/invulnerable_mob").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 2.0, "Invulnerable:1b"));
        out.add(new Case("world/pitch_up").weapon("minecraft:diamond_sword").mob("minecraft:zombie", 0.0, 1.0, 2.0, ""));
        out.get(out.size() - 1).pitch = -25f;
        out.add(new Case("world/no_gravity_bat").weapon("minecraft:diamond_sword").mob("minecraft:bat", 2.0, ""));
        out.add(new Case("world/item_frame").weapon("minecraft:diamond_sword").mob("minecraft:minecart", 2.0, ""));
    }

    static Case cases2(String name, String first, String second, int[] later, String mob) {
        // The weapon changes between hits: modelled with one weapon and a second attack through the
        // same item (the second hit's strength is what differs); `second` is documented in the name.
        Case c = new Case(name).weapon(first).mob(mob, 2.0, "");
        c.later = later;
        return c;
    }

    // Sweeping: bystanders (mobs and players) around the target at different distances.
    static void sweeps(List<Case> out) {
        String sword = "minecraft:diamond_sword";
        out.add(new Case("sweep/mob_target_mob_bystander").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:zombie", 1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/mob_target_player_bystander").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").player(side(1.0, 2.2)));
        out.add(new Case("sweep/player_target_mob_bystander").weapon(sword).player(side(0.0, 2.0)).mob("minecraft:zombie", 1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/player_target_player_bystander").weapon(sword).player(side(0.0, 2.0)).player(side(1.0, 2.2)));
        for (int level = 0; level <= 3; level++) {
            Case c = new Case("sweep/edge_" + level + "/mobs").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "").mob("minecraft:cow", -1.0, 0.0, 2.0, "");
            if (level > 0) c.ench("minecraft:sweeping_edge", level);
            out.add(c);
            c = new Case("sweep/edge_" + level + "/mixed").weapon("minecraft:netherite_sword").mob("minecraft:zombie", 0.0, 0.0, 2.0, "").player(side(1.0, 2.2)).mob("minecraft:skeleton", -1.0, 0.0, 2.1, "");
            if (level > 0) c.ench("minecraft:sweeping_edge", level);
            out.add(c);
        }
        // Four bystanders, some armored, one out of reach.
        Case c = new Case("sweep/four").weapon(sword).ench("minecraft:sweeping_edge", 2).mob("minecraft:zombie", 0.0, 0.0, 2.0, "")
                .mob("minecraft:zombie", 0.8, 0.0, 2.0, ironSet()).player(armored(side(-0.9, 1.9), "iron")).mob("minecraft:pig", 0.5, 0.0, 2.9, "").mob("minecraft:pig", 4.0, 0.0, 2.0, "");
        out.add(c);
        c = new Case("sweep/far_from_attacker").weapon(sword).ench("minecraft:sweeping_edge", 3).mob("minecraft:zombie", 0.0, 0.0, 2.9, "").mob("minecraft:zombie", 0.0, 0.0, 3.8, "").mob("minecraft:zombie", 0.0, 0.0, 2.0, "");
        out.add(c);
        // The box: x within 1 of the target's box, y within 0.25, z within 1.
        for (double dx : new double[] {0.5, 0.9, 1.2, 1.5, 1.7, -1.4}) {
            out.add(new Case("sweep/box_x_" + dx).weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", dx, 0.0, 2.0, ""));
        }
        for (double dy : new double[] {0.1, 0.3, 1.0, 1.9, 2.1, -0.5, -1.7, -1.9}) {
            out.add(new Case("sweep/box_y_" + dy).weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 0.3, dy, 2.0, ""));
        }
        // Conditions: partial strength, sprint, crit, in the air, moving, an axe, a trident.
        c = new Case("sweep/partial").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "");
        c.attacker.ticker = 10;
        out.add(c);
        c = new Case("sweep/ticker_just_full").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "");
        c.attacker.ticker = 12;
        out.add(c);
        c = new Case("sweep/sprint").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "");
        c.attacker.sprinting = true;
        out.add(c);
        out.add(new Case("sweep/crit").weapon(sword).fall(1.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, ""));
        c = new Case("sweep/in_air_no_fall").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "");
        c.attacker.onGround = false;
        out.add(c);
        for (double v : new double[] {0.1, 0.2, 0.26, 0.3}) {
            c = new Case("sweep/moving_" + v).weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "");
            c.attacker.kmz = v;
            out.add(c);
        }
        out.add(new Case("sweep/axe").weapon("minecraft:diamond_axe").mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/trident").weapon("minecraft:trident").mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/fist").mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/copper_sword").weapon("minecraft:copper_sword").ench("minecraft:sweeping_edge", 1).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, ""));
        c = new Case("sweep/strength").weapon(sword).effect("minecraft:strength", 2, 600).ench("minecraft:sweeping_edge", 3).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "");
        out.add(c);
        // Enchanted sweeps: sharpness, smite and bane count per bystander; fire aspect and knockback.
        out.add(new Case("sweep/sharpness").weapon(sword).ench("minecraft:sweeping_edge", 3).ench("minecraft:sharpness", 5).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/smite").weapon(sword).ench("minecraft:sweeping_edge", 3).ench("minecraft:smite", 5).mob("minecraft:pig", 0.0, 0.0, 2.0, "").mob("minecraft:zombie", 1.0, 0.0, 2.2, "").mob("minecraft:skeleton", -1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/bane").weapon(sword).ench("minecraft:sweeping_edge", 2).ench("minecraft:bane_of_arthropods", 5).mob("minecraft:spider", 0.0, 0.0, 2.0, "").mob("minecraft:zombie", 1.0, 0.0, 2.2, "").mob("minecraft:silverfish", -1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/fire").weapon(sword).ench("minecraft:sweeping_edge", 1).ench("minecraft:fire_aspect", 2).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "").mob("minecraft:blaze", -1.0, 0.0, 2.2, "").player(side(0.3, 2.9)));
        out.add(new Case("sweep/knockback").weapon(sword).ench("minecraft:knockback", 2).ench("minecraft:sweeping_edge", 1).mob("minecraft:zombie", 0.0, 0.0, 2.0, "OnGround:1b").mob("minecraft:pig", 1.0, 0.0, 2.2, "OnGround:1b").mob("minecraft:iron_golem", -1.0, 0.0, 2.2, "OnGround:1b"));
        out.add(new Case("sweep/armored_bystanders").weapon("minecraft:netherite_sword").ench("minecraft:sweeping_edge", 3).mob("minecraft:zombie", 0.0, 0.0, 2.0, "")
                .mob("minecraft:zombie", 1.0, 0.0, 2.2, diamondSet(protection(4))).mob("minecraft:skeleton", -1.0, 0.0, 2.2, ironSet()));
        out.add(new Case("sweep/low_health_bystanders").weapon(sword).ench("minecraft:sweeping_edge", 3).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "Health:1f").mob("minecraft:cow", -1.0, 0.0, 2.2, "Health:2f"));
        out.add(new Case("sweep/baby_bystander").weapon(sword).ench("minecraft:sweeping_edge", 3).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:zombie", 0.4, 0.0, 2.2, "IsBaby:1b"));
        out.add(new Case("sweep/dead_target").weapon(sword).ench("minecraft:sweeping_edge", 3).mob("minecraft:zombie", 0.0, 0.0, 2.0, "Health:2f").mob("minecraft:pig", 1.0, 0.0, 2.2, ""));
        out.add(new Case("sweep/boat_bystander").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, "").vehicle(1, "minecraft:oak_boat"));
        out.add(new Case("sweep/second_mob_cooldown").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.0, 0.0, 2.2, ""));
        out.get(out.size() - 1).later = new int[] {100};
        // Player targets with the sweep (armor, creative bystander, pvp off, spectator).
        out.add(new Case("sweep/players_armor").weapon(sword).ench("minecraft:sweeping_edge", 3).player(armored(side(0.0, 2.0), "iron")).player(armored(side(1.0, 2.2), "diamond")).player(side(-1.0, 2.2)));
        CombatVectors.Side creative = side(1.0, 2.2);
        creative.gameMode = "creative";
        out.add(new Case("sweep/creative_bystander").weapon(sword).player(side(0.0, 2.0)).player(creative));
        CombatVectors.Side spectator = side(1.0, 2.2);
        spectator.gameMode = "spectator";
        out.add(new Case("sweep/spectator_bystander").weapon(sword).player(side(0.0, 2.0)).player(spectator).mob("minecraft:pig", -1.0, 0.0, 2.2, ""));
        c = new Case("sweep/pvp_off").weapon(sword).player(side(0.0, 2.0)).player(side(1.0, 2.2)).mob("minecraft:pig", -1.0, 0.0, 2.2, "");
        c.pvp = false;
        out.add(c);
        out.add(new Case("sweep/mob_target_pvp_off_player").weapon(sword).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").player(side(1.0, 2.2)));
        out.get(out.size() - 1).pvp = false;
        out.add(new Case("sweep/player_thorns").weapon(sword).player(side(0.0, 2.0)).player(armored(side(1.0, 2.2), "iron")));
        ((Victim) out.get(out.size() - 1).victims.get(1)).player.armorEnch(2, "minecraft:thorns", 3);
        out.add(new Case("sweep/all_knockback_players").weapon(sword).ench("minecraft:knockback", 1).player(side(0.0, 2.0)).player(side(1.0, 2.2)).player(side(-1.0, 2.2)));
    }

    static final String[] MACE_FALLS = {"0.0", "1.0", "1.5", "1.6", "3.0", "3.5", "5.0", "5.1", "8.0", "10.0", "25.0", "40.0"};

    static void maces(List<Case> out) {
        String mace = "minecraft:mace";
        // Falls onto a pig, a zombie and a player; in the air and on the ground.
        for (String f : MACE_FALLS) {
            double fall = Double.parseDouble(f);
            for (String t : new String[] {"pig", "zombie", "iron_golem"}) {
                out.add(new Case("mace/fall_" + f + "/" + t).weapon(mace).fall(fall).mob("minecraft:" + t, 2.0, "OnGround:1b"));
            }
            out.add(new Case("mace/fall_" + f + "/player").weapon(mace).fall(fall).player(side(0.0, 2.0)));
            Case c = new Case("mace/ground_" + f + "/pig").weapon(mace).mob("minecraft:pig", 2.0, "");
            c.attacker.fallDistance = fall;
            out.add(c);
            c = new Case("mace/ground_" + f + "/player").weapon(mace).player(side(0.0, 2.0));
            c.attacker.fallDistance = fall;
            out.add(c);
        }
        // Armor variants against the smash (and breach).
        for (String mat : new String[] {"leather", "iron", "diamond", "netherite"}) {
            for (int breach : new int[] {0, 1, 4}) {
                Case c = new Case("mace/armor_" + mat + "/breach_" + breach + "/player").weapon(mace).fall(2.5).player(armored(side(0.0, 2.0), mat));
                if (breach > 0) c.ench("minecraft:breach", breach);
                out.add(c);
                c = new Case("mace/armor_" + mat + "/breach_" + breach + "/zombie").weapon(mace).fall(2.5).mob("minecraft:zombie", 2.0,
                        equipment("head", "minecraft:" + mat + "_helmet", "", "chest", "minecraft:" + mat + "_chestplate", "", "legs", "minecraft:" + mat + "_leggings", "", "feet", "minecraft:" + mat + "_boots", ""));
                if (breach > 0) c.ench("minecraft:breach", breach);
                out.add(c);
            }
        }
        out.add(new Case("mace/protection_4").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, diamondSet(protection(4))));
        out.add(new Case("mace/blast_protection").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, equipment("chest", "minecraft:diamond_chestplate", "\"minecraft:enchantments\":{\"minecraft:blast_protection\":4}")));
        out.add(new Case("mace/feather_falling").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, equipment("feet", "minecraft:diamond_boots", "\"minecraft:enchantments\":{\"minecraft:feather_falling\":4}")));
        // Density.
        for (int d = 1; d <= 5; d++) {
            for (String f : new String[] {"2.0", "5.0", "10.0", "40.0"}) {
                out.add(new Case("mace/density_" + d + "/fall_" + f).weapon(mace).ench("minecraft:density", d).fall(Double.parseDouble(f)).mob("minecraft:zombie", 2.0, ""));
            }
        }
        // Sharpness, smite, fire aspect, knockback with the mace.
        out.add(new Case("mace/smite").weapon(mace).ench("minecraft:smite", 5).fall(4.0).mob("minecraft:zombie", 2.0, ""));
        out.add(new Case("mace/bane").weapon(mace).ench("minecraft:bane_of_arthropods", 5).fall(4.0).mob("minecraft:spider", 2.0, ""));
        out.add(new Case("mace/fire").weapon(mace).ench("minecraft:fire_aspect", 2).fall(4.0).mob("minecraft:zombie", 2.0, ""));
        out.add(new Case("mace/knockback").weapon(mace).ench("minecraft:knockback", 2).fall(4.0).mob("minecraft:zombie", 2.0, "OnGround:1b"));
        out.add(new Case("mace/unbreaking").weapon(mace).ench("minecraft:unbreaking", 3).fall(4.0).mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).attacker.mainHandDamage = 100;
        out.add(new Case("mace/worn").weapon(mace).fall(4.0).mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).attacker.mainHandDamage = 400;
        out.add(new Case("mace/breaks").weapon(mace).fall(4.0).mob("minecraft:zombie", 2.0, ""));
        out.get(out.size() - 1).attacker.mainHandDamage = 499;
        // Conditions that cannot smash: gliding, a weak swing, sprinting (smashes anyway), creative.
        Case c = new Case("mace/gliding").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, "");
        c.fallFlying = true;
        out.add(c);
        c = new Case("mace/partial").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, "");
        c.attacker.ticker = 20;
        out.add(c);
        c = new Case("mace/partial_pig").weapon(mace).fall(6.0).mob("minecraft:pig", 2.0, "");
        c.attacker.ticker = 40;
        out.add(c);
        c = new Case("mace/sprint").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, "");
        c.attacker.sprinting = true;
        out.add(c);
        c = new Case("mace/creative").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, "");
        c.attacker.gameMode = "creative";
        out.add(c);
        c = new Case("mace/mounted").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, "");
        c.mounted = true;
        out.add(c);
        out.add(new Case("mace/in_water").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, "").block(0, 0, 0, "minecraft:water"));
        out.add(new Case("mace/strength").weapon(mace).fall(6.0).effect("minecraft:strength", 1, 600).mob("minecraft:zombie", 2.0, ""));
        out.add(new Case("mace/weakness").weapon(mace).fall(6.0).effect("minecraft:weakness", 1, 600).mob("minecraft:zombie", 2.0, ""));
        out.add(new Case("mace/kill").weapon(mace).fall(10.0).mob("minecraft:zombie", 2.0, "Health:5f"));
        out.add(new Case("mace/kill_player").weapon(mace).fall(10.0).player(side(0.0, 2.0)));
        out.get(out.size() - 1).victims.get(0).player.health = 5f;
        // The knockback blast: bystanders at distances (the target is at 2).
        for (double d : new double[] {0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.4, 3.6, 4.5}) {
            out.add(new Case("mace/blast_pig_at_" + d).weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "OnGround:1b").mob("minecraft:pig", 0.0, 0.0, -d + 0.0, "OnGround:1b"));
            out.add(new Case("mace/blast_side_" + d).weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "OnGround:1b").mob("minecraft:pig", d, 0.0, 0.0, "OnGround:1b"));
        }
        out.add(new Case("mace/blast_heavy").weapon(mace).fall(8.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "OnGround:1b").mob("minecraft:pig", 1.5, 0.0, 0.5, "OnGround:1b").mob("minecraft:cow", -1.5, 0.0, 0.5, "OnGround:1b")
                .mob("minecraft:iron_golem", 0.0, 0.0, -1.5, "OnGround:1b").player(side(2.0, 1.0)));
        out.add(new Case("mace/blast_light").weapon(mace).fall(2.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "OnGround:1b").mob("minecraft:pig", 1.5, 0.0, 0.5, "OnGround:1b").player(side(-2.0, 1.0)));
        out.add(new Case("mace/blast_player_target_mobs").weapon(mace).fall(6.0).player(side(0.0, 2.0)).mob("minecraft:pig", 1.5, 0.0, 0.5, "OnGround:1b").mob("minecraft:zombie", -1.5, 0.0, 1.0, ""));
        out.add(new Case("mace/blast_players").weapon(mace).fall(6.0).player(side(0.0, 2.0)).player(side(1.5, 0.5)).player(armored(side(-1.5, 1.0), "diamond")));
        CombatVectors.Side flying = side(1.5, 0.5);
        flying.gameMode = "creative";
        out.add(new Case("mace/blast_creative_player").weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").player(flying));
        CombatVectors.Side sp = side(1.5, 0.5);
        sp.gameMode = "spectator";
        out.add(new Case("mace/blast_spectator").weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").player(sp));
        out.add(new Case("mace/blast_knockback_resistance").weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:ravager", 1.5, 0.0, 0.5, "OnGround:1b"));
        out.add(new Case("mace/blast_in_air_mobs").weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:pig", 1.5, 0.0, 0.5, ""));
        out.add(new Case("mace/blast_baby").weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:zombie", 1.5, 0.0, 0.5, "IsBaby:1b,OnGround:1b"));
        out.add(new Case("mace/blast_tamed_wolf").weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "").mob("minecraft:wolf", 1.5, 0.0, 0.5, "OnGround:1b"));
        out.add(new Case("mace/blast_dead_target").weapon(mace).fall(6.0).mob("minecraft:zombie", 0.0, 0.0, 2.0, "Health:3f").mob("minecraft:pig", 1.5, 0.0, 0.5, "OnGround:1b"));
        // Wind burst.
        for (int w = 1; w <= 3; w++) {
            for (String f : new String[] {"1.0", "1.6", "4.0", "10.0", "40.0"}) {
                out.add(new Case("mace/wind_" + w + "/fall_" + f).weapon(mace).ench("minecraft:wind_burst", w).fall(Double.parseDouble(f)).mob("minecraft:iron_golem", 2.0, "OnGround:1b"));
            }
            out.add(new Case("mace/wind_" + w + "/bystanders").weapon(mace).ench("minecraft:wind_burst", w).fall(6.0).mob("minecraft:iron_golem", 0.0, 0.0, 2.0, "OnGround:1b").mob("minecraft:pig", 1.5, 0.0, 0.5, "OnGround:1b")
                    .mob("minecraft:cow", -2.5, 0.0, 0.0, "OnGround:1b").mob("minecraft:iron_golem", 0.0, 0.0, -1.5, "OnGround:1b").player(side(2.0, 1.0)));
            c = new Case("mace/wind_" + w + "/ground").weapon(mace).ench("minecraft:wind_burst", w).mob("minecraft:iron_golem", 2.0, "OnGround:1b");
            c.attacker.fallDistance = 6.0;
            out.add(c);
            c = new Case("mace/wind_" + w + "/glide").weapon(mace).ench("minecraft:wind_burst", w).fall(6.0).mob("minecraft:zombie", 2.0, "");
            c.fallFlying = true;
            out.add(c);
        }
        out.add(new Case("mace/wind_density_breach").weapon(mace).ench("minecraft:wind_burst", 2).ench("minecraft:density", 3).fall(12.0).mob("minecraft:iron_golem", 2.0, "OnGround:1b"));
        out.add(new Case("mace/wind_target_player").weapon(mace).ench("minecraft:wind_burst", 2).fall(6.0).player(side(0.0, 2.0)));
        out.add(new Case("mace/wind_partial").weapon(mace).ench("minecraft:wind_burst", 2).fall(6.0).mob("minecraft:iron_golem", 2.0, ""));
        out.get(out.size() - 1).attacker.ticker = 20;
        out.add(new Case("mace/wind_water_block").weapon(mace).ench("minecraft:wind_burst", 2).fall(6.0).mob("minecraft:iron_golem", 2.0, "").block(0, -1, 0, "minecraft:stone").block(0, 0, 2, "minecraft:water"));
        // Two hits: the fall distance is gone after a smash.
        c = new Case("mace/second_hit").weapon(mace).fall(6.0).mob("minecraft:zombie", 2.0, "");
        c.later = new int[] {100};
        out.add(c);
        c = new Case("mace/second_hit_pig_far").weapon(mace).fall(6.0).mob("minecraft:pig", 2.0, "").mob("minecraft:cow", 1.0, 0.0, 1.5, "OnGround:1b");
        c.later = new int[] {100, 100};
        out.add(c);
    }

    static List<Case> cases() {
        List<Case> out = new ArrayList<>();
        mobTypes(out);
        weapons(out);
        enchants(out);
        crits(out);
        effects(out);
        armorOnMobs(out);
        states(out);
        sweeps(out);
        maces(out);
        return out;
    }

    // ---------------------------------------------------------------- recording

    static Object effectsOf(net.minecraft.world.entity.LivingEntity e) {
        List<Object> l = new ArrayList<>();
        for (var inst : e.getActiveEffects()) {
            l.add(List.of(inst.getEffect().unwrapKey().orElseThrow().identifier().toString(), inst.getAmplifier(), inst.getDuration()));
        }
        return l;
    }

    static Map<String, Object> mobState(net.minecraft.world.entity.LivingEntity e) throws Exception {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("kind", "mob");
        m.put("type", BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
        m.put("health", e.getHealth());
        m.put("alive", e.isAlive());
        m.put("velocity", CombatVectors.vec(e.getDeltaMovement()));
        m.put("fire_ticks", e.getRemainingFireTicks());
        m.put("hurt_time", e.hurtTime);
        m.put("damage_cooldown", e.damageCooldownTime);
        m.put("last_hurt", (Float) CombatVectors.get(e, "lastHurt"));
        m.put("absorption", e.getAbsorptionAmount());
        EquipmentSlot[] slots = {EquipmentSlot.FEET, EquipmentSlot.LEGS, EquipmentSlot.CHEST, EquipmentSlot.HEAD, EquipmentSlot.MAINHAND, EquipmentSlot.OFFHAND, EquipmentSlot.BODY};
        List<Object> eq = new ArrayList<>();
        for (EquipmentSlot s : slots) {
            ItemStack st = e.getItemBySlot(s);
            eq.add(st.isEmpty() ? null : st.getDamageValue());
        }
        m.put("equipment_damage", eq);
        m.put("effects", effectsOf(e));
        m.put("on_ground", e.onGround());
        m.put("vehicle", e.getVehicle() == null ? null : BuiltInRegistries.ENTITY_TYPE.getKey(e.getVehicle().getType()).toString());
        m.put("pos", new double[] {e.getX() - CombatVectors.BX, e.getY() - CombatVectors.BY, e.getZ() - CombatVectors.BZ});
        return m;
    }

    static String name(Map<Integer, String> names, int id) {
        String n = names.get(id);
        return n != null ? n : "other";
    }

    static Map<String, Object> packets(List<Object> pk, Map<Integer, String> names) {
        List<Object> sounds = new ArrayList<>(), particles = new ArrayList<>(), events = new ArrayList<>(), animates = new ArrayList<>(),
                explodes = new ArrayList<>(), motions = new ArrayList<>(), entityEvents = new ArrayList<>();
        for (Object p : pk) {
            if (p instanceof net.minecraft.network.protocol.game.ClientboundSoundPacket s) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("sound", s.getSound().value().location().toString());
                m.put("source", s.getSource().getName());
                m.put("volume", s.getVolume());
                m.put("pitch", s.getPitch());
                m.put("pos", new double[] {s.getX() - CombatVectors.BX, s.getY() - CombatVectors.BY, s.getZ() - CombatVectors.BZ});
                sounds.add(m);
            } else if (p instanceof net.minecraft.network.protocol.game.ClientboundLevelParticlesPacket s) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("particle", BuiltInRegistries.PARTICLE_TYPE.getKey(s.particle().getType()).toString());
                m.put("count", s.count());
                m.put("pos", new double[] {s.x() - CombatVectors.BX, s.y() - CombatVectors.BY, s.z() - CombatVectors.BZ});
                m.put("offset", new double[] {s.xDist(), s.yDist(), s.zDist()});
                m.put("speed", new double[] {s.xMaxSpeed(), s.yMaxSpeed(), s.zMaxSpeed()});
                m.put("override_limiter", s.overrideLimiter());
                m.put("always_show", s.alwaysShow());
                particles.add(m);
            } else if (p instanceof net.minecraft.network.protocol.game.ClientboundLevelEventPacket s) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("type", s.getType());
                m.put("pos", new int[] {s.getPos().getX(), s.getPos().getY() - (int) CombatVectors.BY, s.getPos().getZ()});
                m.put("data", s.getData());
                m.put("global", s.isGlobalEvent());
                events.add(m);
            } else if (p instanceof net.minecraft.network.protocol.game.ClientboundAnimatePacket s) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("who", name(names, s.getId()));
                m.put("action", s.getAction());
                animates.add(m);
            } else if (p instanceof net.minecraft.network.protocol.game.ClientboundExplodePacket s) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("center", new double[] {s.center().x - CombatVectors.BX, s.center().y - CombatVectors.BY, s.center().z - CombatVectors.BZ});
                m.put("radius", s.radius());
                m.put("block_count", s.blockCount());
                m.put("knockback", s.playerKnockback().map(CombatVectors::vec).orElse(null));
                m.put("particle", BuiltInRegistries.PARTICLE_TYPE.getKey(s.explosionParticle().getType()).toString());
                m.put("sound", s.explosionSound().value().location().toString());
                m.put("play_sound", s.playSound());
                explodes.add(m);
            } else if (p instanceof ClientboundSetEntityMotionPacket s) {
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("who", name(names, s.id()));
                m.put("motion", CombatVectors.vec(s.movement()));
                motions.add(m);
            } else if (p instanceof net.minecraft.network.protocol.game.ClientboundEntityEventPacket s) {
                Map<String, Object> m = new LinkedHashMap<>();
                try {
                    m.put("who", name(names, (Integer) CombatVectors.get(s, "entityId")));
                } catch (Exception e) {
                    m.put("who", "?");
                }
                m.put("event", (int) s.getEventId());
                entityEvents.add(m);
            }
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("sounds", sounds);
        m.put("particles", particles);
        m.put("level_events", events);
        m.put("animates", animates);
        m.put("explodes", explodes);
        m.put("motions", motions);
        m.put("entity_events", entityEvents);
        return m;
    }

    static net.minecraft.core.Holder<net.minecraft.world.effect.MobEffect> effect(String id) {
        return BuiltInRegistries.MOB_EFFECT.get(Identifier.parse(id)).orElseThrow();
    }

    static List<Object> effectJson(List<Object[]> effects) {
        List<Object> l = new ArrayList<>();
        for (Object[] e : effects) l.add(List.of(e[0], e[1], e[2]));
        return l;
    }

    static String blockCommand(Object[] b) {
        return "setblock " + b[0] + " " + ((int) CombatVectors.BY + (Integer) b[1]) + " " + b[2] + " " + b[3];
    }

    static void run(MinecraftServer server, List<String> out, String filter) throws Exception {
        List<Case> all = cases();
        int n = 0;
        for (Case c : all) {
            if (filter != null && !filter.equals("melee") && !c.name.contains(filter)) continue;
            try {
                out.add(runCase(server, c, n++));
            } catch (Throwable t) {
                t.printStackTrace();
                out.add("{\"name\":\"" + c.name + "\",\"error\":\"" + t.toString().replace('"', '\'').replace('\\', '/') + "\"}");
            }
        }
    }

    static String runCase(MinecraftServer server, Case c, int n) throws Exception {
        ServerLevel level = server.overworld();
        SpearVectors.clear(level);
        SpearVectors.gameTime(level, 5000);
        var cmd = server.createCommandSourceStack();
        server.getCommands().performPrefixedCommand(cmd, "gamerule minecraft:pvp " + c.pvp);
        server.getCommands().performPrefixedCommand(cmd, "difficulty " + c.difficulty);
        for (Object[] b : c.blocks) server.getCommands().performPrefixedCommand(cmd, blockCommand(b));
        String attackerName = "Att" + n;
        ServerPlayer a = CombatVectors.mockPlayer(server, attackerName);
        CombatVectors.setup(server, a, c.attacker);
        a.setXRot(c.pitch);
        List<ServerPlayer> vp = new ArrayList<>();
        List<net.minecraft.world.entity.LivingEntity> made = new ArrayList<>();
        List<net.minecraft.world.entity.Entity> extras = new ArrayList<>();
        java.util.Set<Integer> seen = new java.util.HashSet<>();
        for (int i = 0; i < c.victims.size(); i++) {
            Victim v = c.victims.get(i);
            if (v.player != null) {
                v.name = "Vic" + n + "x" + i;
                ServerPlayer p = CombatVectors.mockPlayer(server, v.name);
                CombatVectors.setup(server, p, v.player);
                for (Object[] e : v.effects) p.addEffect(new net.minecraft.world.effect.MobEffectInstance(effect((String) e[0]), (Integer) e[2], (Integer) e[1]));
                vp.add(p);
                made.add(null);
            } else {
                server.getCommands().performPrefixedCommand(cmd, v.command());
                net.minecraft.world.entity.Entity found = null;
                var fresh = new ArrayList<>(level.getEntities((net.minecraft.world.entity.Entity) null, new net.minecraft.world.phys.AABB(-10, 90, -10, 10, 120, 20), e -> !(e instanceof ServerPlayer)));
                fresh.sort(java.util.Comparator.comparingInt(net.minecraft.world.entity.Entity::getId));
                String want = v.type;
                for (var e : fresh) {
                    if (seen.contains(e.getId())) continue;
                    seen.add(e.getId());
                    if (found == null && BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString().equals(want)) found = e;
                    else extras.add(e);
                }
                if (found == null) throw new IllegalStateException("summon failed: " + v.command());
                made.add(found instanceof net.minecraft.world.entity.LivingEntity le ? le : null);
                vp.add(null);
                CombatVectors.set(found, "random", net.minecraft.util.RandomSource.create(n * 31L + i));
            }
        }
        for (Object[] e : c.effects) a.addEffect(new net.minecraft.world.effect.MobEffectInstance(effect((String) e[0]), (Integer) e[2], (Integer) e[1]));
        if (c.fallFlying) a.startFallFlying();
        // The fluids the attacker stands in (what its ticks would have found out).
        CombatVectors.call(a, "updateFluidInteraction");
        if (c.mounted) {
            var pig = SpearVectors.mob(level, "minecraft:pig", 0.0, 0.0, 0.0, 0f);
            a.setPos(CombatVectors.BX + c.attacker.dx, CombatVectors.BY + c.attacker.dy, CombatVectors.BZ + c.attacker.dz);
            a.startRiding(pig, true, false);
            extras.add(pig);
        }
        net.minecraft.world.entity.Entity target = vp.get(c.target) != null ? vp.get(c.target) : (made.get(c.target) != null ? made.get(c.target) : null);
        if (target == null) {
            // A non-living victim (armor stand is living; end crystal, minecart are not): find by type.
            var fresh = new ArrayList<>(level.getEntities((net.minecraft.world.entity.Entity) null, new net.minecraft.world.phys.AABB(-10, 90, -10, 10, 120, 20), e -> !(e instanceof ServerPlayer)));
            fresh.sort(java.util.Comparator.comparingInt(net.minecraft.world.entity.Entity::getId));
            target = fresh.isEmpty() ? null : fresh.get(0);
        }
        Map<Integer, String> names = new java.util.HashMap<>();
        names.put(a.getId(), "attacker");
        for (int i = 0; i < c.victims.size(); i++) {
            if (vp.get(i) != null) names.put(vp.get(i).getId(), "victim" + i);
            else if (made.get(i) != null) names.put(made.get(i).getId(), "victim" + i);
        }
        CombatVectors.drain(a);
        for (ServerPlayer p : vp) if (p != null) CombatVectors.drain(p);
        long seed = c.name.hashCode();
        level.getRandom().setSeed(seed);
        a.getRandom().setSeed(seed + 1);
        for (int i = 0; i < vp.size(); i++) if (vp.get(i) != null) vp.get(i).getRandom().setSeed(seed + 2 + i);

        // What the attacker's state came to after the fluids and the riding.
        Map<String, Object> atAttack = new LinkedHashMap<>();
        atAttack.put("pos", new double[] {a.getX() - CombatVectors.BX, a.getY() - CombatVectors.BY, a.getZ() - CombatVectors.BZ});
        atAttack.put("fall_distance", a.fallDistance);
        atAttack.put("on_ground", a.onGround());
        List<Object> steps = new ArrayList<>();
        for (int step = 0; step <= c.later.length; step++) {
            if (step > 0) CombatVectors.set(a, "attackStrengthTicker", c.later[step - 1]);
            a.attack(target);
            Map<String, Object> s = new LinkedHashMap<>();
            List<Object> apk = CombatVectors.drain(a);
            Map<String, Object> ao = CombatVectors.outcome(a, apk);
            ao.put("pending_motion", a.syncVelocity ? CombatVectors.vec(a.getDeltaMovement()) : null);
            ao.put("packets", packets(apk, names));
            ao.put("fall_distance", a.fallDistance);
            ao.put("on_ground", a.onGround());
            s.put("attacker", ao);
            List<Object> vs = new ArrayList<>();
            for (int i = 0; i < c.victims.size(); i++) {
                if (vp.get(i) != null) {
                    ServerPlayer p = vp.get(i);
                    List<Object> pk = CombatVectors.drain(p);
                    Map<String, Object> o = CombatVectors.outcome(p, pk);
                    if (i != c.target) o.put("pending_motion", p.syncVelocity ? CombatVectors.vec(p.getDeltaMovement()) : null);
                    o.put("kind", "player");
                    o.put("packets", packets(pk, names));
                    vs.add(o);
                } else if (made.get(i) != null) {
                    vs.add(mobState(made.get(i)));
                } else {
                    Map<String, Object> o = new LinkedHashMap<>();
                    o.put("kind", "other");
                    vs.add(o);
                }
            }
            s.put("victims", vs);
            steps.add(s);
        }
        Map<String, Object> line = new LinkedHashMap<>();
        line.put("name", c.name);
        line.put("level_seed", seed);
        line.put("difficulty", c.difficulty);
        line.put("pvp", c.pvp);
        line.put("attacker_name", attackerName);
        line.put("attacker", c.attacker.json());
        line.put("attacker_effects", effectJson(c.effects));
        line.put("attacker_pitch", c.pitch);
        line.put("attacker_at_attack", atAttack);
        line.put("fall_flying", c.fallFlying);
        line.put("mounted", c.mounted);
        line.put("blocks", c.blocks.stream().map(b -> blockCommand(b)).toList());
        line.put("later", c.later);
        List<Object> victims = new ArrayList<>();
        int vi = 0;
        for (Victim v : c.victims) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("seed", n * 31L + vi++);
            if (v.player != null) {
                m.put("kind", "player");
                m.put("name", v.name);
                m.put("side", v.player.json());
                m.put("effects", effectJson(v.effects));
            } else {
                m.put("kind", "mob");
                m.put("type", v.type);
                m.put("pos", new double[] {v.dx, v.dy, v.dz});
                m.put("yaw", v.yaw);
                m.put("nbt", v.nbt);
                m.put("vehicle", v.vehicle);
            }
            victims.add(m);
        }
        line.put("victims", victims);
        line.put("target", c.target);
        line.put("steps", steps);
        for (ServerPlayer p : vp) if (p != null) server.getPlayerList().remove(p);
        server.getPlayerList().remove(a);
        for (var e : made) if (e != null) e.discard();
        for (var e : extras) e.discard();
        SpearVectors.clear(level);
        return CombatVectors.toJson(line);
    }
}
