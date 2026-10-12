package wp58;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.blaze3d.platform.InputConstants;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonInfo;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.HitResult;

/**
 * wp58 test driver, loaded into the real 26.3 client with -javaagent: a line protocol on
 * 127.0.0.1:25999 that feeds the game's own input handlers (KeyboardHandler.keyPress,
 * MouseHandler.onButton, charTyped) from the render thread and reports what the client sees.
 * It never touches the operating system's keyboard or mouse, so the game window needs no focus.
 */
public class ClientCtl {
    static final int PORT = 25999;

    public static void premain(String args) {
        Thread t = new Thread(ClientCtl::serve, "wp58-ctl");
        t.setDaemon(true);
        t.start();
    }

    static void serve() {
        try (ServerSocket server = new ServerSocket(PORT, 4, InetAddress.getLoopbackAddress())) {
            while (true) {
                Socket s = server.accept();
                Thread t = new Thread(() -> session(s), "wp58-ctl-session");
                t.setDaemon(true);
                t.start();
            }
        } catch (Exception e) {
            e.printStackTrace();
        }
    }

    static void session(Socket s) {
        try (s;
             BufferedReader in = new BufferedReader(new InputStreamReader(s.getInputStream(), StandardCharsets.UTF_8));
             PrintWriter out = new PrintWriter(new OutputStreamWriter(s.getOutputStream(), StandardCharsets.UTF_8), true)) {
            String line;
            while ((line = in.readLine()) != null) {
                String reply;
                try {
                    reply = handle(line.trim());
                } catch (Throwable e) {
                    reply = "ERR " + e;
                }
                out.println(reply);
            }
        } catch (Exception ignored) {
        }
    }

    interface Task<T> { T get() throws Exception; }

    static <T> T onMain(Task<T> f) throws Exception {
        Minecraft mc = Minecraft.getInstance();
        if (mc == null) {
            throw new IllegalStateException("no client yet");
        }
        CompletableFuture<T> r = new CompletableFuture<>();
        mc.execute(() -> {
            try {
                r.complete(f.get());
            } catch (Throwable e) {
                r.completeExceptionally(e);
            }
        });
        return r.get(10, TimeUnit.SECONDS);
    }

    static int keyCode(String name) throws Exception {
        Field f = InputConstants.class.getField("KEY_" + name.toUpperCase());
        return f.getInt(null);
    }

    static void key(Minecraft mc, int code, boolean down, int mods) {
        mc.keyboardHandler.keyPress(mc.getWindow().handle(), down ? InputConstants.PRESS : InputConstants.RELEASE,
            new KeyEvent(code, code, mods));
    }

    static void setField(Object o, String name, Object v) throws Exception {
        Field f = o.getClass().getDeclaredField(name);
        f.setAccessible(true);
        f.set(o, v);
    }

    static void mouseButton(Minecraft mc, int button, boolean down) throws Exception {
        if (mc.gui.screen() == null && !mc.mouseHandler.isMouseGrabbed()) {
            setField(mc.mouseHandler, "mouseGrabbed", true);
        }
        mc.mouseHandler.onButton(mc.getWindow().handle(), new MouseButtonInfo(button, 0), down ? 1 : 0);
    }

    static String handle(String line) throws Exception {
        String[] a = line.split("\\s+");
        Minecraft mc = Minecraft.getInstance();
        switch (a[0]) {
            case "ping":
                return "pong";
            case "state":
                return onMain(() -> state(mc, a.length > 1 ? Integer.parseInt(a[1]) : 24)).toString();
            case "key": { // key NAME down|up|tap [mods]
                int code = keyCode(a[1]);
                int mods = a.length > 3 ? Integer.parseInt(a[3]) : 0;
                if (a[2].equals("tap")) {
                    onMain(() -> { key(mc, code, true, mods); return null; });
                    Thread.sleep(60);
                    onMain(() -> { key(mc, code, false, mods); return null; });
                } else {
                    onMain(() -> { key(mc, code, a[2].equals("down"), mods); return null; });
                }
                return "ok";
            }
            case "mouse": { // mouse left|right|middle down|up|tap
                int button = switch (a[1]) { case "left" -> 1; case "middle" -> 2; default -> 3; };
                if (a[2].equals("tap")) {
                    onMain(() -> { mouseButton(mc, button, true); return null; });
                    Thread.sleep(80);
                    onMain(() -> { mouseButton(mc, button, false); return null; });
                } else {
                    onMain(() -> { mouseButton(mc, button, a[2].equals("down")); return null; });
                }
                return "ok";
            }
            case "look": // look YAW PITCH (absolute, degrees)
                return onMain(() -> {
                    var p = mc.player;
                    p.setYRot(Float.parseFloat(a[1]));
                    p.setXRot(Float.parseFloat(a[2]));
                    p.yRotO = p.getYRot();
                    p.xRotO = p.getXRot();
                    return "ok";
                });
            case "lookat": // lookat X Y Z (eye towards the point)
                return onMain(() -> {
                    var p = mc.player;
                    var e = p.getEyePosition();
                    double dx = Double.parseDouble(a[1]) - e.x, dy = Double.parseDouble(a[2]) - e.y, dz = Double.parseDouble(a[3]) - e.z;
                    double h = Math.sqrt(dx * dx + dz * dz);
                    p.setYRot((float) (Math.toDegrees(Math.atan2(-dx, dz))));
                    p.setXRot((float) (-Math.toDegrees(Math.atan2(dy, h))));
                    p.yRotO = p.getYRot();
                    p.xRotO = p.getXRot();
                    return "ok";
                });
            case "type": { // type TEXT (characters, as typed in chat / signs / anvils)
                String text = line.substring(line.indexOf(' ') + 1);
                onMain(() -> {
                    text.codePoints().forEach(cp -> mc.keyboardHandler.charTyped(mc.getWindow().handle(), new CharacterEvent(cp)));
                    return null;
                });
                return "ok";
            }
            case "say": { // say TEXT: open chat with t (or / for commands), type the rest, enter
                String text = line.substring(line.indexOf(' ') + 1);
                boolean cmd = text.startsWith("/");
                onMain(() -> { key(mc, cmd ? InputConstants.KEY_SLASH : InputConstants.KEY_T, true, 0); return null; });
                Thread.sleep(60);
                onMain(() -> { key(mc, cmd ? InputConstants.KEY_SLASH : InputConstants.KEY_T, false, 0); return null; });
                Thread.sleep(150);
                String rest = cmd ? text.substring(1) : text;
                onMain(() -> {
                    rest.codePoints().forEach(cp -> mc.keyboardHandler.charTyped(mc.getWindow().handle(), new CharacterEvent(cp)));
                    return null;
                });
                Thread.sleep(100);
                onMain(() -> { key(mc, InputConstants.KEY_RETURN, true, 0); return null; });
                Thread.sleep(60);
                onMain(() -> { key(mc, InputConstants.KEY_RETURN, false, 0); return null; });
                return "ok";
            }
            case "hotbar": { // hotbar 1..9
                int code = InputConstants.KEY_1 + Integer.parseInt(a[1]) - 1;
                onMain(() -> { key(mc, code, true, 0); return null; });
                Thread.sleep(40);
                onMain(() -> { key(mc, code, false, 0); return null; });
                return "ok";
            }
            case "releaseall":
                return onMain(() -> { net.minecraft.client.KeyMapping.releaseAll(); return "ok"; });
            case "guiclick": { // guiclick SLOT left|right [shift]: click the slot of the open container screen
                int slot = Integer.parseInt(a[1]);
                int button = a[2].equals("left") ? 1 : 3;
                boolean shift = a.length > 3 && a[3].equals("shift");
                return onMain(() -> {
                    try {
                        var screen = mc.gui.screen();
                        if (!(screen instanceof AbstractContainerScreen<?> c)) {
                            return "ERR no container screen";
                        }
                        Slot s = c.getMenu().slots.get(slot);
                        Field lp = AbstractContainerScreen.class.getDeclaredField("leftPos");
                        Field tp = AbstractContainerScreen.class.getDeclaredField("topPos");
                        lp.setAccessible(true);
                        tp.setAccessible(true);
                        double scale = mc.getWindow().getGuiScale();
                        double x = (lp.getInt(c) + s.x + 8) * scale;
                        double y = (tp.getInt(c) + s.y + 8) * scale;
                        setField(mc.mouseHandler, "xpos", x);
                        setField(mc.mouseHandler, "ypos", y);
                        int mods = shift ? 1 : 0;
                        if (shift) {
                            key(mc, InputConstants.KEY_LSHIFT, true, 1);
                        }
                        mc.mouseHandler.onButton(mc.getWindow().handle(), new MouseButtonInfo(button, mods), 1);
                        mc.mouseHandler.onButton(mc.getWindow().handle(), new MouseButtonInfo(button, mods), 0);
                        if (shift) {
                            key(mc, InputConstants.KEY_LSHIFT, false, 0);
                        }
                        return "ok";
                    } catch (Exception e) {
                        return "ERR " + e;
                    }
                });
            }
            case "mousepos": // mousepos X Y (window pixels)
                return onMain(() -> {
                    try {
                        setField(mc.mouseHandler, "xpos", Double.parseDouble(a[1]));
                        setField(mc.mouseHandler, "ypos", Double.parseDouble(a[2]));
                        return "ok";
                    } catch (Exception e) {
                        return "ERR " + e;
                    }
                });
            case "block": // block X Y Z
                return onMain(() -> mc.level.getBlockState(new BlockPos(Integer.parseInt(a[1]), Integer.parseInt(a[2]), Integer.parseInt(a[3]))).toString());
            case "find": { // find SUBSTRING RADIUS [MAX]: block positions near the player whose id contains SUBSTRING
                String sub = a[1];
                int r = Math.min(Integer.parseInt(a[2]), 64);
                int max = a.length > 3 ? Integer.parseInt(a[3]) : 40;
                return onMain(() -> {
                    JsonArray arr = new JsonArray();
                    var p = mc.player.blockPosition();
                    var pos = new BlockPos.MutableBlockPos();
                    java.util.List<int[]> hits = new java.util.ArrayList<>();
                    for (int dx = -r; dx <= r; dx++) {
                        for (int dy = -r; dy <= r; dy++) {
                            for (int dz = -r; dz <= r; dz++) {
                                pos.set(p.getX() + dx, p.getY() + dy, p.getZ() + dz);
                                var st = mc.level.getBlockState(pos);
                                if (st.isAir()) {
                                    continue;
                                }
                                String id = BuiltInRegistries.BLOCK.getKey(st.getBlock()).getPath();
                                if (id.contains(sub)) {
                                    hits.add(new int[] {pos.getX(), pos.getY(), pos.getZ(), dx * dx + dy * dy + dz * dz});
                                }
                            }
                        }
                    }
                    hits.sort((x, y) -> Integer.compare(x[3], y[3]));
                    for (int i = 0; i < Math.min(max, hits.size()); i++) {
                        JsonArray h = new JsonArray();
                        h.add(hits.get(i)[0]);
                        h.add(hits.get(i)[1]);
                        h.add(hits.get(i)[2]);
                        arr.add(h);
                    }
                    return arr.toString();
                });
            }
            case "findbox": { // findbox SUBSTRING X0 Y0 Z0 X1 Y1 Z1 [MAX]: blocks in a box (loaded chunks only), nearest to the player first
                String sub = a[1];
                int x0 = Integer.parseInt(a[2]), y0 = Integer.parseInt(a[3]), z0 = Integer.parseInt(a[4]);
                int x1 = Integer.parseInt(a[5]), y1 = Integer.parseInt(a[6]), z1 = Integer.parseInt(a[7]);
                int max = a.length > 8 ? Integer.parseInt(a[8]) : 40;
                return onMain(() -> {
                    JsonArray arr = new JsonArray();
                    var pos = new BlockPos.MutableBlockPos();
                    var pp = mc.player.blockPosition();
                    java.util.List<int[]> hits = new java.util.ArrayList<>();
                    for (int cx = x0 >> 4; cx <= x1 >> 4; cx++) {
                        for (int cz = z0 >> 4; cz <= z1 >> 4; cz++) {
                            if (mc.level.getChunk(cx, cz, net.minecraft.world.level.chunk.status.ChunkStatus.FULL, false) == null) {
                                continue;
                            }
                            for (int x = Math.max(x0, cx << 4); x <= Math.min(x1, (cx << 4) + 15); x++) {
                                for (int z = Math.max(z0, cz << 4); z <= Math.min(z1, (cz << 4) + 15); z++) {
                                    for (int y = y0; y <= y1; y++) {
                                        pos.set(x, y, z);
                                        var st = mc.level.getBlockState(pos);
                                        if (st.isAir()) {
                                            continue;
                                        }
                                        if (BuiltInRegistries.BLOCK.getKey(st.getBlock()).getPath().contains(sub)) {
                                            int dx = x - pp.getX(), dy = y - pp.getY(), dz = z - pp.getZ();
                                            hits.add(new int[] {x, y, z, dx * dx + dy * dy + dz * dz});
                                        }
                                    }
                                }
                            }
                        }
                    }
                    hits.sort((p1, p2) -> Integer.compare(p1[3], p2[3]));
                    for (int i = 0; i < Math.min(max, hits.size()); i++) {
                        JsonArray h = new JsonArray();
                        h.add(hits.get(i)[0]);
                        h.add(hits.get(i)[1]);
                        h.add(hits.get(i)[2]);
                        arr.add(h);
                    }
                    return arr.toString();
                });
            }
            default:
                return "ERR unknown command " + a[0];
        }
    }

    static String id(ItemStack s) {
        return BuiltInRegistries.ITEM.getKey(s.getItem()).toString();
    }

    static JsonObject state(Minecraft mc, int entityRange) {
        JsonObject o = new JsonObject();
        o.addProperty("screen", mc.gui.screen() == null ? (String) null : mc.gui.screen().getClass().getSimpleName());
        o.addProperty("overlay", mc.gui.overlay() == null ? (String) null : mc.gui.overlay().getClass().getSimpleName());
        o.addProperty("grabbed", mc.mouseHandler.isMouseGrabbed());
        o.addProperty("guiScale", mc.getWindow().getGuiScale());
        if (mc.gui.screen() != null) {
            o.addProperty("title", mc.gui.screen().getTitle().getString());
        }
        var p = mc.player;
        if (p == null || mc.level == null) {
            o.addProperty("inGame", false);
            return o;
        }
        o.addProperty("inGame", true);
        o.addProperty("x", p.getX());
        o.addProperty("y", p.getY());
        o.addProperty("z", p.getZ());
        o.addProperty("yaw", p.getYRot());
        o.addProperty("pitch", p.getXRot());
        o.addProperty("health", p.getHealth());
        o.addProperty("food", p.getFoodData().getFoodLevel());
        o.addProperty("xpLevel", p.experienceLevel);
        o.addProperty("onGround", p.onGround());
        o.addProperty("sleeping", p.isSleeping());
        o.addProperty("fallFlying", p.isFallFlying());
        o.addProperty("vehicle", p.getVehicle() == null ? (String) null : BuiltInRegistries.ENTITY_TYPE.getKey(p.getVehicle().getType()).toString());
        o.addProperty("dimension", mc.level.dimension().toString());
        o.addProperty("gameTime", mc.level.getGameTime());
        o.addProperty("clock", mc.level.getOverworldClockTime());
        o.addProperty("selected", p.getInventory().getSelectedSlot());
        JsonArray inv = new JsonArray();
        for (int i = 0; i < 41; i++) {
            ItemStack s = p.getInventory().getItem(i);
            if (!s.isEmpty()) {
                JsonObject e = new JsonObject();
                e.addProperty("slot", i);
                e.addProperty("id", id(s));
                e.addProperty("n", s.getCount());
                inv.add(e);
            }
        }
        o.add("inv", inv);
        HitResult h = mc.hitResult;
        if (h instanceof BlockHitResult b && h.getType() == HitResult.Type.BLOCK) {
            JsonObject e = new JsonObject();
            BlockPos bp = b.getBlockPos();
            e.addProperty("type", "block");
            e.addProperty("x", bp.getX());
            e.addProperty("y", bp.getY());
            e.addProperty("z", bp.getZ());
            e.addProperty("face", b.getDirection().getName());
            e.addProperty("id", BuiltInRegistries.BLOCK.getKey(mc.level.getBlockState(bp).getBlock()).toString());
            o.add("hit", e);
        } else if (h instanceof EntityHitResult en) {
            JsonObject e = new JsonObject();
            e.addProperty("type", "entity");
            e.addProperty("id", BuiltInRegistries.ENTITY_TYPE.getKey(en.getEntity().getType()).toString());
            e.addProperty("eid", en.getEntity().getId());
            o.add("hit", e);
        }
        JsonArray ents = new JsonArray();
        double r2 = (double) entityRange * entityRange;
        for (Entity en : mc.level.entitiesForRendering()) {
            if (en == p || en.distanceToSqr(p) > r2 || ents.size() >= 60) {
                continue;
            }
            JsonObject e = new JsonObject();
            e.addProperty("t", BuiltInRegistries.ENTITY_TYPE.getKey(en.getType()).getPath());
            e.addProperty("id", en.getId());
            e.addProperty("x", en.getX());
            e.addProperty("y", en.getY());
            e.addProperty("z", en.getZ());
            ents.add(e);
        }
        o.add("entities", ents);
        if (mc.gui.screen() instanceof AbstractContainerScreen<?> c) {
            JsonArray slots = new JsonArray();
            for (Slot s : c.getMenu().slots) {
                JsonObject e = new JsonObject();
                e.addProperty("i", s.index);
                e.addProperty("id", s.hasItem() ? id(s.getItem()) : "");
                e.addProperty("n", s.hasItem() ? s.getItem().getCount() : 0);
                slots.add(e);
            }
            o.add("slots", slots);
            o.addProperty("carried", id(c.getMenu().getCarried()) + " x" + c.getMenu().getCarried().getCount());
            o.addProperty("menu", c.getMenu().getClass().getSimpleName());
            try {
                Field lp = AbstractContainerScreen.class.getDeclaredField("leftPos");
                Field tp = AbstractContainerScreen.class.getDeclaredField("topPos");
                lp.setAccessible(true);
                tp.setAccessible(true);
                o.addProperty("leftPos", lp.getInt(c));
                o.addProperty("topPos", tp.getInt(c));
            } catch (Exception ignored) {
            }
        }
        return o;
    }
}
