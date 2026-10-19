// wp87: the mace's smash attack from a real fall, on vanilla 26.3. A mock player (an embedded channel, as CombatVectors makes them) stands high
// above a mob, starts falling with the client's own physics (v = (v - 0.08) * 0.98 each tick, sent as the position packets a client sends, on_ground
// false), and when the mob is in reach attacks it with a mace (ServerboundAttackPacket), with Density, Breach and Wind Burst. One JSON line per
// scenario: the setup (so that Kiln replays the same packets) and what came out: the mob's health and motion, the attacker's motion, fall distance and
// mace damage, the packets the attacker was sent.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/wp87/MaceFall.java <out.jsonl> [name-filter]
// (tools/wp87/mace87.sh sets this up)

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
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundBundlePacket;
import net.minecraft.network.protocol.game.ClientboundExplodePacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket;
import net.minecraft.network.protocol.game.ServerboundAttackPacket;
import net.minecraft.network.protocol.game.ServerboundMovePlayerPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.phys.Vec3;

public class MaceFall {
    static final double X = 0.5, Y0 = 100.0, Z = 0.5;

    record Scenario(String name, double height, Map<String, Integer> enchants, String victim, boolean armored) {}

    static List<Scenario> scenarios() {
        List<Scenario> out = new ArrayList<>();
        double[] heights = {1.7, 2.5, 4.0, 8.0, 16.0, 40.0};
        for (double h : heights) {
            out.add(new Scenario("plain_h" + h, h, Map.of(), "zombie", false));
            out.add(new Scenario("plain_armored_h" + h, h, Map.of(), "zombie", true));
            for (int l = 1; l <= 5; l += 2) out.add(new Scenario("density" + l + "_h" + h, h, Map.of("minecraft:density", l), "zombie", false));
            for (int l = 1; l <= 4; l += 1) out.add(new Scenario("breach" + l + "_armored_h" + h, h, Map.of("minecraft:breach", l), "zombie", true));
            for (int l = 1; l <= 3; l += 1) out.add(new Scenario("windburst" + l + "_h" + h, h, Map.of("minecraft:wind_burst", l), "zombie", false));
            out.add(new Scenario("all_armored_h" + h, h, Map.of("minecraft:density", 3, "minecraft:breach", 2, "minecraft:wind_burst", 2), "zombie", true));
        }
        return out;
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
        }, "MaceFall main");
        main.start();
        MinecraftServer server = awaitServer();
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
        int n = 0;
        for (Scenario s : scenarios()) {
            if (filter != null && !s.name.contains(filter)) continue;
            final int index = n++;
            server.submit(() -> {
                try {
                    lines.add(runOne(server, s, index));
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("{\"name\":\"" + s.name + "\",\"error\":\"" + t + "\"}");
                }
            }).get();
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("MaceFall: wrote " + lines.size() + " scenarios to " + outPath);
        server.halt(false);
        Runtime.getRuntime().halt(0);
    }

    static String runOne(MinecraftServer server, Scenario s, int index) throws Exception {
        var cmd = server.createCommandSourceStack();
        server.getCommands().performPrefixedCommand(cmd, "difficulty normal");
        server.getCommands().performPrefixedCommand(cmd, "gamerule minecraft:pvp true");
        ServerLevel level = server.overworld();
        // The mob: a zombie that does not think, on the ground.
        Mob mob = (Mob) EntityType.ZOMBIE.create(level, EntitySpawnReason.COMMAND);
        mob.setPos(X, Y0, Z);
        mob.setNoAi(true);
        mob.setPersistenceRequired();
        if (s.armored) {
            mob.setItemSlot(EquipmentSlot.HEAD, new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse("minecraft:diamond_helmet"))));
            mob.setItemSlot(EquipmentSlot.CHEST, new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse("minecraft:diamond_chestplate"))));
            mob.setItemSlot(EquipmentSlot.LEGS, new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse("minecraft:diamond_leggings"))));
            mob.setItemSlot(EquipmentSlot.FEET, new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse("minecraft:diamond_boots"))));
        }
        level.addFreshEntity(mob);
        // The attacker.
        ServerPlayer p = mockPlayer(server, "Faller" + index);
        p.setGameMode(GameType.SURVIVAL);
        call(p.connection, "markClientLoaded");
        double top = Y0 + mob.getBbHeight();
        double y = top + s.height;
        p.setPos(X, y, Z);
        p.setYRot(0f);
        p.setYHeadRot(0f);
        p.setXRot(90f);
        p.setDeltaMovement(Vec3.ZERO);
        p.setOnGround(true);
        p.fallDistance = 0.0;
        ItemStack mace = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse("minecraft:mace")));
        for (var e : s.enchants.entrySet()) mace.enchant(enchantment(server, e.getKey()), e.getValue());
        p.getInventory().clearContent();
        p.getInventory().setSelectedSlot(0);
        p.setItemSlot(EquipmentSlot.MAINHAND, mace);
        call(p, "detectEquipmentUpdates");
        p.setHealth(20f);
        p.getFoodData().setFoodLevel(17);
        // The position the connection expects the client to start from, with no teleport pending.
        set(p.connection, "awaitingPositionFromClient", null);
        call(p.connection, "resetPosition");
        server.overworld().getRandom().setSeed(1000 + index);
        p.getRandom().setSeed(2000 + index);
        mob.getRandom().setSeed(3000 + index);
        drain(p);
        // The fall, as the client moves: v = (v - 0.08) * 0.98, the position packet carries the new position, never on the ground.
        List<Double> path = new ArrayList<>();
        double vy = 0.0;
        int attackAt = -1;
        for (int tick = 0; tick < 200; tick++) {
            vy = (vy - 0.08) * 0.98;
            y += vy;
            path.add(y);
            p.connection.handleMovePlayer(new ServerboundMovePlayerPacket.PosRot(X, y, Z, 0f, 90f, false, false));
            set(p, "attackStrengthTicker", 1000);
            if (p.isWithinAttackRange(mace, mob.getBoundingBox(), 3.0)) {
                attackAt = tick;
                break;
            }
            p.connection.tick();
        }
        if (attackAt < 0) throw new IllegalStateException("never in reach");
        double fallBefore = p.fallDistance;
        float healthBefore = mob.getHealth();
        int durabilityBefore = p.getMainHandItem().getDamageValue();
        p.connection.handleAttack(new ServerboundAttackPacket(mob.getId()));
        // The attacker's packets.
        List<String> packets = new ArrayList<>();
        for (Object o : drain(p)) {
            if (o instanceof ClientboundSetEntityMotionPacket m) {
                packets.add("motion " + m.id() + " " + m.movement().x + " " + m.movement().y + " " + m.movement().z);
            } else if (o instanceof ClientboundExplodePacket e) {
                packets.add("explode " + e.center().x + " " + e.center().y + " " + e.center().z + " " + e.radius() + " " + e.playerKnockback().map(v -> v.x + " " + v.y + " " + v.z).orElse("none"));
            } else {
                packets.add(o.getClass().getSimpleName());
            }
        }
        Map<String, Object> line = new LinkedHashMap<>();
        line.put("name", s.name);
        line.put("height", s.height);
        line.put("victim_armored", s.armored);
        Map<String, Object> ench = new LinkedHashMap<>();
        for (var e : s.enchants.entrySet()) ench.put(e.getKey(), e.getValue());
        line.put("enchants", ench);
        line.put("path", path);
        line.put("attack_at", attackAt);
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("fall_distance_before", fallBefore);
        result.put("victim_health_before", healthBefore);
        result.put("victim_health", mob.getHealth());
        result.put("victim_dead", mob.isDeadOrDying());
        result.put("victim_delta", new double[] {mob.getDeltaMovement().x, mob.getDeltaMovement().y, mob.getDeltaMovement().z});
        result.put("attacker_delta", new double[] {p.getDeltaMovement().x, p.getDeltaMovement().y, p.getDeltaMovement().z});
        result.put("attacker_fall_distance", p.fallDistance);
        result.put("mace_damage", p.getMainHandItem().getDamageValue() - durabilityBefore);
        result.put("packets", packets);
        line.put("result", result);
        server.getPlayerList().remove(p);
        mob.discard();
        return toJson(line);
    }

    // ---------------------------------------------------------------- the server and the mock players (as CombatVectors makes them)

    static ServerPlayer mockPlayer(MinecraftServer server, String name) {
        CommonListenerCookie cookie = CommonListenerCookie.createInitial(new GameProfile(UUID.nameUUIDFromBytes(name.getBytes()), name), false);
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

    static net.minecraft.core.Holder<net.minecraft.world.item.enchantment.Enchantment> enchantment(MinecraftServer server, String id) {
        return server.registryAccess().lookupOrThrow(net.minecraft.core.registries.Registries.ENCHANTMENT)
                .getOrThrow(net.minecraft.resources.ResourceKey.create(net.minecraft.core.registries.Registries.ENCHANTMENT, Identifier.parse(id)));
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=25597",
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
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(q -> q.toFile().delete());
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
        if (o instanceof List<?> l) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < l.size(); i++) b.append(i > 0 ? "," : "").append(toJson(l.get(i)));
            return b.append("]").toString();
        }
        if (o instanceof Map<?, ?> m) {
            StringBuilder b = new StringBuilder("{");
            boolean first = true;
            for (var e : m.entrySet()) {
                b.append(first ? "" : ",").append(toJson(e.getKey().toString())).append(":").append(toJson(e.getValue()));
                first = false;
            }
            return b.append("}").toString();
        }
        throw new IllegalArgumentException(o.getClass().toString());
    }
}
