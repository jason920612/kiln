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

        Map<String, Object> json() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("main_hand", mainHand);
            m.put("main_hand_damage", mainHandDamage);
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
        boolean pvp = true;
        String difficulty = "normal";

        Scenario(String name) {
            this.name = name;
            target.dz = 2.0;
            target.yaw = 180f;
        }
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
        return out;
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
        server.halt(false);
        System.exit(0);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=25594",
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

        attacker.attack(target);

        Map<String, Object> result = new LinkedHashMap<>();
        result.put("attacker", outcome(attacker, target));
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
        if (o instanceof Boolean || o instanceof Integer) return o.toString();
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
