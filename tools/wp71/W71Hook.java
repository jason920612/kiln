package w71;

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.ai.Brain;
import net.minecraft.world.level.Level;

/**
 * wp71: per-tick statistics of the farm's villagers on the real server, in the same terms as crates/kiln-sim/tests/farm69.rs (FARM_STATS):
 * per (zombie in the pen, activity class) the villager-ticks, the brain tokens (activities, running behaviours, memories) and how often each behaviour started,
 * the position grid of the pen, the speed and the distance to the zombie. Written to $W71_OUT (key TAB count) every 20000 ticks and at exit.
 */
public final class W71Hook {
    static final int X0 = Integer.getInteger("w71.x0", 4000), Z0 = Integer.getInteger("w71.z0", 4000), PEN_DZ = 40;
    static final Map<String, long[]> C = new TreeMap<>();
    static final Map<Integer, double[]> PREV = new HashMap<>();
    static final Map<Integer, Set<String>> PREV_RUN = new HashMap<>();
    static final Set<Integer> GOLEMS = new HashSet<>();
    static Field memories;
    static long lastGt = -1, lastDump = 0;
    static boolean hooked;

    static void add(String k) {
        C.computeIfAbsent(k, x -> new long[1])[0]++;
    }

    public static void afterTick(Object o) {
        try {
            ServerLevel level = (ServerLevel) o;
            if (!level.dimension().equals(Level.OVERWORLD)) return;
            long gt = level.getGameTime();
            if (gt == lastGt) return;
            lastGt = gt;
            if (!hooked) {
                hooked = true;
                memories = Brain.class.getDeclaredField("memories");
                memories.setAccessible(true);
                Runtime.getRuntime().addShutdownHook(new Thread(W71Hook::dump));
            }
            List<Entity> villagers = new ArrayList<>(), zombies = new ArrayList<>();
            for (Entity e : level.getAllEntities()) {
                if (e.isRemoved()) continue;
                if (e instanceof net.minecraft.world.entity.npc.villager.Villager && e.entityTags().contains("wp66")) villagers.add(e);
                else if (e instanceof net.minecraft.world.entity.monster.zombie.Zombie && e.entityTags().contains("wp66z")) zombies.add(e);
                else if (e instanceof net.minecraft.world.entity.animal.golem.IronGolem && GOLEMS.add(e.getId())) add("golem");
            }
            Map<Integer, List<Entity>> zombiesByPen = new HashMap<>();
            for (Entity z : zombies) zombiesByPen.computeIfAbsent(pen(z.getZ()), k -> new ArrayList<>()).add(z);
            Map<Integer, Boolean> panicByPen = new HashMap<>();
            for (Entity v : villagers) {
                LivingEntity lv = (LivingEntity) v;
                Brain<?> brain = lv.getBrain();
                List<String> acts = new ArrayList<>();
                for (var a : brain.getActiveActivities()) acts.add(a.getName());
                String cls = acts.contains("panic") ? "P" : acts.contains("rest") ? "R" : acts.contains("idle") ? "I" : "O";
                int pen = pen(v.getZ());
                List<Entity> zs = zombiesByPen.getOrDefault(pen, List.of());
                String ph = zs.isEmpty() ? "N" : "Z";
                panicByPen.merge(pen, cls.equals("P"), Boolean::logicalOr);
                add("ticks|" + ph + "|" + cls);
                Set<String> run = new HashSet<>();
                for (var a : acts) add("tok|" + ph + "|" + cls + "|act:" + a);
                for (var b : brain.getRunningBehaviors()) {
                    String n = b.getClass().getSimpleName();
                    if (n.isEmpty()) continue;
                    run.add(n);
                    add("tok|" + ph + "|" + cls + "|run:" + n);
                }
                Set<String> before = PREV_RUN.getOrDefault(v.getId(), Set.of());
                for (String n : run) if (!before.contains(n)) add("start|" + ph + "|" + cls + "|" + n);
                PREV_RUN.put(v.getId(), run);
                for (var en : ((Map<?, ?>) memories.get(brain)).entrySet()) {
                    Object slot = en.getValue();
                    if ((Boolean) slot.getClass().getMethod("hasValue").invoke(slot)) add("tok|" + ph + "|" + cls + "|m:" + en.getKey());
                }
                double x = v.getX(), z = v.getZ();
                int dx = Math.min(12, Math.max(0, (int) Math.floor(x - X0) + 1)), dz = Math.min(10, Math.max(0, (int) Math.floor(z - Z0 - pen * PEN_DZ) + 1));
                add("grid|" + ph + "|" + cls + "|" + dx + "|" + dz);
                double[] p = PREV.get(v.getId());
                if (p != null) add("speed|" + ph + "|" + cls + "|" + Math.min(50, (int) (Math.hypot(x - p[0], z - p[1]) / 0.01)));
                PREV.put(v.getId(), new double[] {x, z});
                if (!zs.isEmpty()) {
                    double best = 1e9;
                    for (Entity zb : zs) best = Math.min(best, Math.sqrt(v.distanceToSqr(zb)));
                    add("zd|" + cls + "|" + Math.min(48, (int) (best / 0.5)));
                }
            }
            if (gt % 100 == 0) {
                for (var en : zombiesByPen.entrySet()) {
                    add("opp|all");
                    if (panicByPen.getOrDefault(en.getKey(), false)) add("opp|any");
                }
            }
            if (gt - lastDump >= 20000) {
                lastDump = gt;
                dump();
            }
        } catch (Throwable t) {
            t.printStackTrace();
        }
    }

    static int pen(double z) {
        return Math.floorDiv((int) Math.floor(z - Z0) + 20, PEN_DZ);
    }

    static synchronized void dump() {
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(System.getenv().getOrDefault("W71_OUT", "w71agent.out"))))) {
            for (var en : C.entrySet()) w.println(en.getKey() + "\t" + en.getValue()[0]);
        } catch (Exception e) {
            e.printStackTrace();
        }
    }
}
