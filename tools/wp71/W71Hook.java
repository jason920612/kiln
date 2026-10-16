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
    static final Map<Integer, String> PREV_WALK = new HashMap<>();
    static final Map<Integer, Boolean> PREV_CANT = new HashMap<>();
    static final Map<Integer, Object[]> PENDING = new HashMap<>();
    static final Map<Integer, String> TRACE_PREV = new HashMap<>();
    static long zombieSince = -1, traceFrom = -1;
    static final boolean TRACE = Boolean.getBoolean("w71.trace");
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
                // The funnel of walk targets: each new one (by value) with its distance, and what came of it in the next ticks.
                boolean sinkStarted = run.contains("MoveToTargetSink") && !before.contains("MoveToTargetSink");
                boolean cant = brain.hasMemoryValue(net.minecraft.world.entity.ai.memory.MemoryModuleType.CANT_REACH_WALK_TARGET_SINCE);
                boolean wasCant = PREV_CANT.getOrDefault(v.getId(), false);
                PREV_CANT.put(v.getId(), cant);
                var wopt = brain.getMemory(net.minecraft.world.entity.ai.memory.MemoryModuleType.WALK_TARGET);
                String key = "";
                String sig = null;
                double[] at = null;
                if (wopt.isPresent()) {
                    var w = wopt.get();
                    var tp = w.getTarget().currentPosition();
                    boolean ent = w.getTarget() instanceof net.minecraft.world.entity.ai.behavior.EntityTracker;
                    at = new double[] {tp.x, tp.y, tp.z};
                    key = String.format("%b|%.2f|%.2f|%.2f|%s|%d", ent, tp.x, tp.y, tp.z, Float.toString(w.getSpeedModifier()), w.getCloseEnoughDist());
                    sig = (ent ? "E" : "B") + "s" + String.format("%.2f", w.getSpeedModifier()) + "c" + w.getCloseEnoughDist();
                }
                String prevKey = PREV_WALK.put(v.getId(), key);
                if (prevKey == null) prevKey = "";
                if (sig != null && !key.equals(prevKey)) {
                    add("wt|" + ph + "|" + cls + "|" + sig);
                    add("wtd|" + ph + "|" + cls + "|" + sig + "|" + Math.min(15, (int) Math.hypot(at[0] - x0v(v), at[2] - z0v(v))));
                    if (run.contains("MoveToTargetSink")) {
                        add("wtR|" + ph + "|" + cls + "|" + sig);
                        PENDING.remove(v.getId());
                    } else {
                        PENDING.put(v.getId(), new Object[] {sig, 4});
                    }
                } else {
                    Object[] pend = PENDING.remove(v.getId());
                    if (pend != null) {
                        String psig = (String) pend[0];
                        int ttl = (Integer) pend[1];
                        if (key.isEmpty()) add("wtX|" + ph + "|" + cls + "|" + psig);
                        else if (sinkStarted) add("wtS|" + ph + "|" + cls + "|" + psig);
                        else if (cant && !wasCant) add("wtC|" + ph + "|" + cls + "|" + psig);
                        else if (ttl > 1) PENDING.put(v.getId(), new Object[] {psig, ttl - 1});
                        else add("wtN|" + ph + "|" + cls + "|" + psig);
                    }
                }
                if (TRACE && pen == 0 && traceFrom >= 0 && gt >= traceFrom && gt < traceFrom + 1500) {
                    StringBuilder sb = new StringBuilder();
                    for (var a : acts) sb.append(a).append(' ');
                    for (String n : new java.util.TreeSet<>(run)) sb.append("run:").append(n).append(' ');
                    java.util.TreeSet<String> ms = new java.util.TreeSet<>();
                    for (var en : ((Map<?, ?>) memories.get(brain)).entrySet()) {
                        Object slot = en.getValue();
                        if ((Boolean) slot.getClass().getMethod("hasValue").invoke(slot)) ms.add(en.getKey().toString().replace("minecraft:", ""));
                    }
                    for (String m : ms) sb.append(m).append(' ');
                    String desc = sb.toString() + " | " + String.format("%.2f %.2f", v.getX(), v.getZ()) + (wopt.isPresent() ? " W" + sig + "@" + String.format("%.1f,%.1f", at[0], at[2]) : "");
                    String descNoPos = sb.toString() + (wopt.isPresent() ? " W" + sig : "");
                    if (!descNoPos.equals(TRACE_PREV.put(v.getId(), descNoPos))) System.out.println("TRACE " + (gt - traceFrom) + " v" + v.getId() + " " + desc);
                }
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
            if (TRACE) {
                if (zombiesByPen.containsKey(0)) { if (zombieSince < 0) zombieSince = gt; if (traceFrom < 0 && gt - zombieSince >= 2000) traceFrom = gt; }
                else zombieSince = -1;
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

    static double x0v(Entity v) {
        return v.getX();
    }

    static double z0v(Entity v) {
        return v.getZ();
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
