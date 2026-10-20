// wp87: the elastic pull of a lead on vanilla 26.3 (Leashable.tickLeash) for entities that hang on four points (horses, camels, sniffers, boats) held by a happy ghast, and for ordinary leads.
// Scenarios: a horse, donkey, camel, sniffer, boat, llama and cow led by a happy ghast, a cow and a ghast, at several offsets and yaws, both without AI;
// per scenario twelve calls of Leashable.tickLeash, after each of which the led entity's delta, yaw and the lead's angular momentum are written.
//
// usage (cwd = a scratch server directory): tools/wp87/leash87.sh

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

public class LeashQuad {
    static final double X = 0.5, Y0 = -63.0, Z = 0.5;

    record Scenario(String name, String leashed, String holder, double dx, double dy, double dz, float leashedYaw, float holderYaw) {}

    static List<Scenario> scenarios() {
        List<Scenario> out = new ArrayList<>();
        String[] leashed = {"horse", "donkey", "camel", "sniffer", "oak_boat", "llama", "cow"};
        String[] holders = {"happy_ghast", "cow", "ghast"};
        double[][] offsets = {{9.5, 2.0, 3.0}, {-8.0, 0.0, 7.25}, {2.5, 3.5, -10.0}, {13.0, 5.0, 0.0}};
        float[] yaws = {0f, 37f, 150f, 271f};
        int i = 0;
        for (String l : leashed) for (String h : holders) for (double[] o : offsets) {
            out.add(new Scenario(l + "_by_" + h + "_" + i, l, h, o[0], o[1], o[2], yaws[i % 4], yaws[(i + 1) % 4] + 11f));
            i++;
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
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "LeashQuad main");
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
        for (Scenario s : scenarios()) {
            server.submit(() -> {
                try {
                    runOne(server, s, lines);
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("error " + s.name + " " + t);
                }
            }).get();
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("LeashQuad: wrote " + lines.size() + " lines to " + outPath);
        server.halt(false);
        Runtime.getRuntime().halt(0);
    }

    static net.minecraft.world.entity.Entity make(ServerLevel level, String type) {
        var types = net.minecraft.world.entity.EntityTypes.class;
        try {
            var f = types.getField(type.toUpperCase());
            var t = (net.minecraft.world.entity.EntityType<?>) f.get(null);
            return t.create(level, net.minecraft.world.entity.EntitySpawnReason.COMMAND);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(type, e);
        }
    }

    static void runOne(MinecraftServer server, Scenario s, List<String> lines) throws Exception {
        ServerLevel level = server.overworld();
        var holder = make(level, s.holder);
        var led = make(level, s.leashed);
        if (holder instanceof Mob m) m.setNoAi(true);
        if (led instanceof Mob m) m.setNoAi(true);
        holder.setPos(X + s.dx, Y0 + s.dy, Z + s.dz);
        holder.setYRot(s.holderYaw);
        led.setPos(X, Y0, Z);
        led.setYRot(s.leashedYaw);
        led.setOnGround(true);
        level.addFreshEntity(holder);
        level.addFreshEntity(led);
        ((net.minecraft.world.entity.Leashable) led).setLeashedTo(holder, false);
        for (int step = 0; step < 12; step++) {
            net.minecraft.world.entity.Leashable.tickLeash(level, (net.minecraft.world.entity.Entity & net.minecraft.world.entity.Leashable) led);
            var data = ((net.minecraft.world.entity.Leashable) led).getLeashData();
            Vec3 d = led.getDeltaMovement();
            lines.add(s.name + " " + step + " " + d.x + " " + d.y + " " + d.z + " " + led.getYRot() + " " + (data == null ? "none" : Double.toString(data.angularMomentum)));
            if (data == null || ((net.minecraft.world.entity.Leashable) led).getLeashHolder() == null) break;
        }
        holder.discard();
        led.discard();
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
