// wp77: villager trading against a vanilla 26.3 server, bit for bit:
//  - "gossip": random sequences of GossipContainer.add / decay / transferFrom (seeded random), the whole state and the reputations after each step;
//  - "offer": MerchantOffer prices (getCostA / getCostB) after demand updates, use resets and special price changes;
//  - "special": Villager.updateSpecialPrices(player) for a gossip reputation and Hero of the Village levels, on a villager with random offers;
//  - "restock": a villager with used-up offers and a script of time steps, uses and shouldRestock()/restock() calls: restock counters, uses, demand.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/VillagerVectors.java <out.jsonl>

import com.mojang.authlib.GameProfile;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Optional;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.util.RandomSource;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.ai.gossip.GossipContainer;
import net.minecraft.world.entity.ai.gossip.GossipType;
import net.minecraft.world.entity.npc.villager.Villager;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.trading.ItemCost;
import net.minecraft.world.item.trading.MerchantOffer;
import net.minecraft.world.item.trading.MerchantOffers;
import net.minecraft.world.level.GameType;

public class VillagerVectors {
    static final UUID[] PEOPLE = {new UUID(1, 1), new UUID(2, 2), new UUID(3, 3)};
    static final GossipType[] TYPES = GossipType.values();

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
        }, "VillagerVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        server.submit(() -> {
            ServerLevel l = server.overworld();
            for (int cx = -1; cx <= 1; cx++)
                for (int cz = -1; cz <= 1; cz++) {
                    l.setChunkForced(cx, cz, true);
                    l.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(3000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            try {
                ServerLevel level = server.overworld();
                level.tickRateManager().setFrozen(true);
                ServerPlayer player = mockPlayer(server, "Trader");
                player.setGameMode(GameType.SURVIVAL);
                Random r = new Random(7701);
                for (int i = 0; i < 60; i++) lines.add(gossip(r, i));
                for (int i = 0; i < 200; i++) lines.add(offer(r, i));
                for (int i = 0; i < 80; i++) lines.add(special(server, level, player, r, i));
                for (int i = 0; i < 60; i++) lines.add(restock(server, level, r, i));
            } catch (Throwable t) {
                t.printStackTrace();
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("VillagerVectors: wrote " + lines.size() + " cases to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static ServerPlayer mockPlayer(MinecraftServer server, String name) {
        CommonListenerCookie cookie = CommonListenerCookie.createInitial(new GameProfile(UUID.nameUUIDFromBytes(name.getBytes()), name), false);
        ServerPlayer p = new ServerPlayer(server, server.overworld(), cookie.gameProfile(), cookie.clientInformation());
        Connection connection = new Connection(PacketFlow.SERVERBOUND);
        new EmbeddedChannel(connection);
        server.getPlayerList().placeNewPlayer(connection, p, cookie);
        return p;
    }

    static String gossipState(GossipContainer g) {
        StringBuilder sb = new StringBuilder("[");
        for (int p = 0; p < PEOPLE.length; p++) {
            var m = g.getGossipEntries().get(PEOPLE[p]);
            for (GossipType t : TYPES) {
                int v = m == null ? 0 : m.getInt(t);
                sb.append(p > 0 || t != TYPES[0] ? "," : "").append(v);
            }
        }
        sb.append("],[");
        for (int p = 0; p < PEOPLE.length; p++) sb.append(p > 0 ? "," : "").append(g.getReputation(PEOPLE[p], x -> true));
        return sb.append("]").toString();
    }

    static String gossip(Random r, int index) {
        GossipContainer g = new GossipContainer(), other = new GossipContainer();
        StringBuilder ops = new StringBuilder();
        for (int k = 0; k < 25; k++) {
            int kind = r.nextInt(10);
            String op;
            if (kind < 6) {
                int who = r.nextInt(3), type = r.nextInt(5), amount = r.nextInt(4) == 0 ? 1 + r.nextInt(3) : 1 + r.nextInt(40);
                g.add(PEOPLE[who], TYPES[type], amount);
                op = String.format("[\"add\",%d,%d,%d]", who, type, amount);
            } else if (kind < 7) {
                g.decay();
                op = "[\"decay\"]";
            } else if (kind < 8) {
                int who = r.nextInt(3), type = r.nextInt(5), amount = 1 + r.nextInt(60);
                other.add(PEOPLE[who], TYPES[type], amount);
                op = String.format("[\"other\",%d,%d,%d]", who, type, amount);
            } else {
                long seed = r.nextLong();
                int max = 1 + r.nextInt(10);
                // The order of `unpack()` is the one of a HashMap of UUIDs and of enum-keyed maps (identity hashes), so the vector carries it.
                StringBuilder order = new StringBuilder();
                try {
                    var m = GossipContainer.class.getDeclaredMethod("unpack");
                    m.setAccessible(true);
                    for (Object e : ((java.util.stream.Stream<?>) m.invoke(other)).toList()) {
                        var t = e.getClass().getDeclaredMethod("target");
                        t.setAccessible(true);
                        var ty = e.getClass().getDeclaredMethod("type");
                        ty.setAccessible(true);
                        UUID who2 = (UUID) t.invoke(e);
                        Object type2 = ty.invoke(e);
                        int pi = java.util.Arrays.asList(PEOPLE).indexOf(who2), ti = java.util.Arrays.asList(TYPES).indexOf(type2);
                        order.append(order.length() > 0 ? "," : "").append("[").append(pi).append(",").append(ti).append("]");
                    }
                } catch (Exception ex) {
                    throw new RuntimeException(ex);
                }
                int picked = g.transferFrom(other, new net.minecraft.world.level.levelgen.LegacyRandomSource(seed), max);
                op = String.format("[\"transfer\",\"%d\",%d,%d,[%s]]", seed, max, picked, order);
            }
            ops.append(ops.length() > 0 ? "," : "").append("{\"op\":").append(op).append(",\"state\":[").append(gossipState(g)).append("]}");
        }
        return String.format(Locale.ROOT, "{\"kind\":\"gossip\",\"name\":\"gossip_%d\",\"steps\":[%s]}", index, ops);
    }

    static String describe(MerchantOffer o) {
        return String.format(Locale.ROOT, "[%d,%d,%d,%d,%d,%d]", o.getCostA().getCount(), o.getCostB().isEmpty() ? 0 : o.getCostB().getCount(), o.getUses(), o.getDemand(), o.getSpecialPriceDiff(), o.needsRestock() ? 1 : 0);
    }

    static MerchantOffer randomOffer(Random r) {
        int a = 1 + r.nextInt(r.nextInt(3) == 0 ? 64 : 12);
        var costItem = new ItemCost(r.nextBoolean() ? Items.EMERALD : Items.WHEAT, a);
        Optional<ItemCost> b = r.nextInt(3) == 0 ? Optional.of(new ItemCost(Items.BOOK, 1 + r.nextInt(8))) : Optional.empty();
        float mult = r.nextBoolean() ? 0.05f : 0.2f;
        int max = 1 + r.nextInt(16);
        return new MerchantOffer(costItem, b, new ItemStack(Items.BREAD, 1 + r.nextInt(4)), r.nextInt(max + 1), max, 1 + r.nextInt(30), mult, r.nextInt(5) == 0 ? r.nextInt(120) : r.nextInt(8));
    }

    static String offer(Random r, int index) {
        MerchantOffer o = randomOffer(r);
        String setup = offerSetup(o, "");
        StringBuilder ops = new StringBuilder();
        for (int k = 0; k < 8; k++) {
            int kind = r.nextInt(5);
            String op;
            if (kind == 0) {
                o.updateDemand();
                op = "[\"demand\"]";
            } else if (kind == 1) {
                o.resetUses();
                op = "[\"reset\"]";
            } else if (kind == 2) {
                int n = 1 + r.nextInt(3);
                for (int i = 0; i < n; i++) o.increaseUses();
                op = String.format("[\"use\",%d]", n);
            } else if (kind == 3) {
                int d = -r.nextInt(70);
                o.addToSpecialPriceDiff(d);
                op = String.format("[\"special\",%d]", d);
            } else {
                o.resetSpecialPriceDiff();
                op = "[\"unspecial\"]";
            }
            ops.append(ops.length() > 0 ? "," : "").append("{\"op\":").append(op).append(",\"state\":").append(describe(o)).append("}");
        }
        // The setup, so the other side can make the same offer.
        return String.format(Locale.ROOT, "{\"kind\":\"offer\",\"name\":\"offer_%d\",\"setup\":%s,\"steps\":[%s]}", index, setup, ops);
    }

    static String offerSetup(MerchantOffer o, String start) {
        var a = o.getItemCostA();
        String b = o.getItemCostB().map(c -> "[\"" + c.itemStack().getItem().builtInRegistryHolder().key().identifier() + "\"," + c.count() + "]").orElse("null");
        return String.format(Locale.ROOT, "{\"a\":[\"%s\",%d],\"b\":%s,\"uses\":%d,\"max_uses\":%d,\"xp\":%d,\"mult\":%s,\"demand\":%d,\"special\":%d,\"result\":%d}",
                a.itemStack().getItem().builtInRegistryHolder().key().identifier(), a.count(), b, o.getUses(), o.getMaxUses(), o.getXp(), Float.toString(o.getPriceMultiplier()), o.getDemand(), o.getSpecialPriceDiff(), o.getResult().getCount());
    }

    static MerchantOffers offers(Random r, int n, StringBuilder setup) {
        MerchantOffers list = new MerchantOffers();
        for (int i = 0; i < n; i++) {
            MerchantOffer o = randomOffer(r);
            setup.append(setup.length() > 0 ? "," : "").append(offerSetup(o, ""));
            list.add(o);
        }
        return list;
    }

    static Villager villager(ServerLevel level) {
        Villager v = net.minecraft.world.entity.EntityTypes.VILLAGER.create(level, EntitySpawnReason.COMMAND);
        v.setPos(0.5, 80, 0.5);
        return v;
    }

    static Object field(Object o, String name) throws Exception {
        Field f = o.getClass().getDeclaredField(name);
        f.setAccessible(true);
        return f.get(o);
    }

    static void setField(Object o, String name, Object v) throws Exception {
        Field f = o.getClass().getDeclaredField(name);
        f.setAccessible(true);
        f.set(o, v);
    }

    static String special(MinecraftServer server, ServerLevel level, ServerPlayer player, Random r, int index) throws Exception {
        Villager v = villager(level);
        StringBuilder setup = new StringBuilder();
        v.setOffers(offers(r, 1 + r.nextInt(5), setup));
        // gossip about the player
        StringBuilder gossip = new StringBuilder();
        for (int k = 0; k < 1 + r.nextInt(4); k++) {
            int type = r.nextInt(5), amount = 1 + r.nextInt(40);
            v.getGossips().add(player.getUUID(), TYPES[type], amount);
            gossip.append(gossip.length() > 0 ? "," : "").append("[").append(type).append(",").append(amount).append("]");
        }
        int hero = r.nextInt(3) == 0 ? -1 : r.nextInt(5);
        player.removeAllEffects();
        if (hero >= 0) player.addEffect(new MobEffectInstance(MobEffects.HERO_OF_THE_VILLAGE, 1000, hero));
        Method m = Villager.class.getDeclaredMethod("updateSpecialPrices", net.minecraft.world.entity.player.Player.class);
        m.setAccessible(true);
        m.invoke(v, player);
        StringBuilder after = new StringBuilder();
        for (MerchantOffer o : v.getOffers()) after.append(after.length() > 0 ? "," : "").append(describe(o));
        player.removeAllEffects();
        return String.format(Locale.ROOT, "{\"kind\":\"special\",\"name\":\"special_%d\",\"offers\":[%s],\"gossip\":[%s],\"hero\":%d,\"after\":[%s]}", index, setup, gossip, hero, after);
    }

    static String restock(MinecraftServer server, ServerLevel level, Random r, int index) throws Exception {
        var data = (net.minecraft.world.level.storage.ServerLevelData) level.getLevelData();
        long start = 1000L + r.nextInt(100000);
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), "time set " + (start % 24000));
        data.setGameTime(start);
        Villager v = villager(level);
        StringBuilder setup = new StringBuilder();
        MerchantOffers offers = offers(r, 1 + r.nextInt(4), setup);
        v.setOffers(offers);
        long lastRestock = r.nextInt(3) == 0 ? 0 : start - r.nextInt(30000);
        int today = r.nextInt(4);
        setField(v, "lastRestockGameTime", lastRestock);
        setField(v, "numberOfRestocksToday", today);
        setField(v, "lastRestockCheckDay", (long) (r.nextInt(3) == 0 ? 0 : (start / 24000 - r.nextInt(2))));
        long checkDay = (Long) field(v, "lastRestockCheckDay");
        StringBuilder steps = new StringBuilder();
        long game = start;
        int[] deltas = {0, 1, 20, 100, 2399, 2400, 2401, 5000, 11999, 12000, 12001, 23999, 24000, 24001, 40000};
        for (int k = 0; k < 40; k++) {
            int kind = r.nextInt(10);
            String op;
            if (kind < 4) {
                int d = deltas[r.nextInt(deltas.length)];
                game += d;
                data.setGameTime(game);
                server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), "time add " + d);
                op = String.format("[\"advance\",%d]", d);
            } else if (kind < 6) {
                int i = r.nextInt(offers.size()), n = 1 + r.nextInt(3);
                for (int j = 0; j < n; j++) offers.get(i).increaseUses();
                op = String.format("[\"use\",%d,%d]", i, n);
            } else {
                boolean should = v.shouldRestock(level);
                boolean did = should && r.nextBoolean();
                if (did) v.restock();
                op = String.format("[\"should\",%d,%d]", should ? 1 : 0, did ? 1 : 0);
            }
            StringBuilder st = new StringBuilder();
            for (MerchantOffer o : v.getOffers()) st.append(st.length() > 0 ? "," : "").append(describe(o));
            steps.append(steps.length() > 0 ? "," : "").append("{\"op\":").append(op).append(",\"counters\":[").append(field(v, "numberOfRestocksToday")).append(",")
                    .append(field(v, "lastRestockGameTime")).append(",").append(field(v, "lastRestockCheckDay")).append("],\"offers\":[").append(st).append("]}");
        }
        return String.format(Locale.ROOT, "{\"kind\":\"restock\",\"name\":\"restock_%d\",\"game_time\":%d,\"day_time\":%d,\"last_restock\":%d,\"today\":%d,\"check_day\":%d,\"offers\":[%s],\"steps\":[%s]}",
                index, start, start % 24000, lastRestock, today, checkDay, setup, steps);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + (System.getenv("KILN_HARNESS_PORT") != null ? System.getenv("KILN_HARNESS_PORT") : "25581"),
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:plains\"}",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "spawn-monsters=false",
                "spawn-animals=false",
                "generate-structures=false",
                "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
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
}
