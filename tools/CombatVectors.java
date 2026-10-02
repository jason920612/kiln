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

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + System.getenv().getOrDefault("KILN_HARNESS_PORT", "25594"),
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
        for (Object pkt : drain(p)) {
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
