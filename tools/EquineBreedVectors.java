// Differential test vectors for horse, donkey, mule and llama breeding (wp63): in a real vanilla 26.3 dedicated server, many pairs of
// tame parents with varied attributes (health, speed, jump strength, a llama's strength and coat, a horse's coat and markings) and a
// pinned random each breed once (`AgeableMob.getBreedOffspring`, what `Animal.spawnChildFromBreeding` calls). One line per breeding:
// the parents as set, the child (kind, attribute bases, strength, coat) and the next `nextLong` of the parent's random, which says
// the random was used the same number of times.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/EquineBreedVectors.java <out.jsonl> [samples per case]

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.AgeableMob;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.animal.equine.AbstractHorse;
import net.minecraft.world.entity.animal.equine.Horse;
import net.minecraft.world.entity.animal.equine.Llama;

public class EquineBreedVectors {
    static MinecraftServer server;

    public static void main(String[] args) throws Exception {
        Path out = Path.of(args[0]).toAbsolutePath();
        int samples = args.length > 1 ? Integer.parseInt(args[1]) : 400;
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n", "server-port=25584", "online-mode=false", "level-name=world", "level-seed=1",
                "level-type=minecraft\\:flat", "generate-structures=false", "max-tick-time=-1", "view-distance=3", "simulation-distance=3", "sync-chunk-writes=false",
                "enable-rcon=false", "enable-query=false", "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "EquineBreedVectors main");
        main.start();
        server = awaitServer();
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            try {
                ServerLevel level = server.overworld();
                level.tickRateManager().setFrozen(true);
                Object[][] cases = {
                    {"horse_horse", EntityType.HORSE, EntityType.HORSE},
                    {"donkey_donkey", EntityType.DONKEY, EntityType.DONKEY},
                    {"horse_donkey", EntityType.HORSE, EntityType.DONKEY},
                    {"donkey_horse", EntityType.DONKEY, EntityType.HORSE},
                    {"llama_llama", EntityType.LLAMA, EntityType.LLAMA},
                    {"llama_trader", EntityType.LLAMA, EntityType.TRADER_LLAMA},
                    {"trader_llama", EntityType.TRADER_LLAMA, EntityType.LLAMA},
                    {"trader_trader", EntityType.TRADER_LLAMA, EntityType.TRADER_LLAMA},
                };
                for (int c = 0; c < cases.length; c++) {
                    for (int i = 0; i < samples; i++) {
                        long seed = 1000003L * (c + 1) + 7919L * i;
                        lines.add(sample((String) cases[c][0], (EntityType<?>) cases[c][1], (EntityType<?>) cases[c][2], seed, i, level));
                    }
                }
            } catch (Throwable t) {
                t.printStackTrace();
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("EquineBreedVectors: wrote " + lines.size() + " lines to " + out);
        server.halt(false);
        System.exit(0);
    }

    /** A small deterministic generator for the parents' attributes (not the entity's random). */
    static final class Gen {
        long s;
        Gen(long seed) { s = seed * 0x9E3779B97F4A7C15L + 12345; }
        double next() { s ^= s << 13; s ^= s >>> 7; s ^= s << 17; return ((s >>> 11) & ((1L << 53) - 1)) / (double) (1L << 53); }
        int nextInt(int bound) { return (int) (next() * bound); }
    }

    static String sample(String name, EntityType<?> ta, EntityType<?> tb, long seed, int index, ServerLevel level) throws Exception {
        Gen g = new Gen(seed);
        AbstractHorse a = (AbstractHorse) ta.create(level, EntitySpawnReason.COMMAND);
        AbstractHorse b = (AbstractHorse) tb.create(level, EntitySpawnReason.COMMAND);
        for (AbstractHorse p : new AbstractHorse[] {a, b}) {
            p.setTamed(true);
            // Mostly inside the attributes' natural ranges, now and then outside (a command can set any value) or at the edges.
            int mode = index % 10;
            double h, j, sp;
            if (mode == 0) { h = 15; j = 0.4000000059604645; sp = 0.44999998807907104 * 0.25; }
            else if (mode == 1) { h = 30; j = 1.0; sp = 1.35 * 0.25; }
            else if (mode == 2) { h = 1 + g.next() * 100; j = g.next() * 2; sp = g.next() * 0.6; }
            else { h = 15 + g.next() * 15; j = 0.4 + g.next() * 0.6; sp = (0.45 + g.next() * 0.9) * 0.25; }
            p.getAttribute(Attributes.MAX_HEALTH).setBaseValue(h);
            p.getAttribute(Attributes.JUMP_STRENGTH).setBaseValue(j);
            p.getAttribute(Attributes.MOVEMENT_SPEED).setBaseValue(sp);
            if (p instanceof Llama l) {
                call(l, "setStrength", 1 + g.nextInt(5));
                Object[] variants = Llama.Variant.values();
                call(l, "setVariant", variants[g.nextInt(variants.length)]);
            }
            if (p instanceof Horse hh) {
                call(hh, "setTypeVariant", g.nextInt(7) | (g.nextInt(5) << 8));
            }
        }
        a.getRandom().setSeed(seed);
        String before = parent(a) + "," + parent(b);
        AgeableMob child = a.getBreedOffspring(level, b);
        long next = a.getRandom().nextLong();
        return String.format(Locale.ROOT, "{\"case\":\"%s\",\"seed\":%d,\"a\":%s,\"b\":%s,\"child\":%s,\"next\":%d}", name, seed, parent(a), parent(b), describe(child), next);
    }

    static String parent(AbstractHorse p) throws Exception {
        return describe(p);
    }

    static String describe(AgeableMob m) throws Exception {
        AbstractHorse h = (AbstractHorse) m;
        StringBuilder b = new StringBuilder("{\"kind\":\"" + net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE.getKey(m.getType()) + "\"");
        b.append(",\"max_health\":").append(Double.doubleToLongBits(h.getAttributeBaseValue(Attributes.MAX_HEALTH)));
        b.append(",\"speed\":").append(Double.doubleToLongBits(h.getAttributeBaseValue(Attributes.MOVEMENT_SPEED)));
        b.append(",\"jump\":").append(Double.doubleToLongBits(h.getAttributeBaseValue(Attributes.JUMP_STRENGTH)));
        if (m instanceof Llama l) {
            b.append(",\"strength\":").append(l.getStrength());
            b.append(",\"variant\":").append(l.getVariant().getId());
        }
        if (m instanceof Horse hh) {
            Method get = Horse.class.getDeclaredMethod("getTypeVariant");
            get.setAccessible(true);
            b.append(",\"type_variant\":").append((int) get.invoke(hh));
        }
        return b.append("}").toString();
    }

    static void call(Object target, String name, Object arg) throws Exception {
        for (Method m : target.getClass().getDeclaredMethods()) {
            if (m.getName().equals(name) && m.getParameterCount() == 1) {
                m.setAccessible(true);
                m.invoke(target, arg);
                return;
            }
        }
        for (Method m : target.getClass().getSuperclass().getDeclaredMethods()) {
            if (m.getName().equals(name) && m.getParameterCount() == 1) {
                m.setAccessible(true);
                m.invoke(target, arg);
                return;
            }
        }
        throw new NoSuchMethodException(name);
    }

    static MinecraftServer awaitServer() throws Exception {
        for (int i = 0; i < 6000; i++) {
            for (Thread t : Thread.getAllStackTraces().keySet()) {
                if (!t.getName().equals("Server thread")) continue;
                java.lang.reflect.Field holderField = Thread.class.getDeclaredField("holder");
                holderField.setAccessible(true);
                Object holder = holderField.get(t);
                java.lang.reflect.Field taskField = holder.getClass().getDeclaredField("task");
                taskField.setAccessible(true);
                Object task = taskField.get(holder);
                for (java.lang.reflect.Field f : task.getClass().getDeclaredFields()) {
                    f.setAccessible(true);
                    if (f.get(task) instanceof java.util.concurrent.atomic.AtomicReference<?> ref && ref.get() instanceof MinecraftServer s) {
                        while (!s.isReady()) Thread.sleep(100);
                        return s;
                    }
                }
            }
            Thread.sleep(100);
        }
        throw new IllegalStateException("server did not start");
    }
}
