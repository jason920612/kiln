// Decodes packet bodies with vanilla's StreamCodecs and prints every decoded field, and prints
// the entity tables (types, data fields, serializers) from the running game for comparison with
// Kiln's generated ones. Run through tools/entity_vectors.py.
//
// usage: java -cp <server jar + libraries> tools/VanillaDump.java packets <dir>   (reads <dir>/manifest.txt)
//        java -cp <server jar + libraries> tools/VanillaDump.java entities <class list file>

import com.google.common.collect.Multimap;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.lang.reflect.ParameterizedType;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.PublicKey;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.List;
import java.util.Optional;
import java.util.OptionalInt;
import java.util.Set;
import java.util.UUID;
import net.minecraft.core.Holder;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.Component;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.network.syncher.EntityDataSerializer;
import net.minecraft.network.syncher.EntityDataSerializers;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.entity.EntityReference;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.state.BlockState;

public class VanillaDump {
    // Captured before Bootstrap redirects System.out into the game's logger.
    static final PrintStream OUT = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);
    static RegistryAccess access;

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        access = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        // Default item components are only bound once data-driven registries load; item stacks
        // on the wire are (count, item id, component patch), so empty prototypes suffice here.
        BuiltInRegistries.ITEM.listElements().filter(h -> !h.areComponentsBound())
                .forEach(h -> h.bindComponents(DataComponentMap.EMPTY));
        if (args[0].equals("packets")) {
            packets(Path.of(args[1]));
        } else {
            entities(Files.readAllLines(Path.of(args[1])));
        }
    }

    static void packets(Path dir) throws Exception {
        for (String line : Files.readAllLines(dir.resolve("manifest.txt"))) {
            String[] parts = line.split(" ");
            String name = parts[0];
            OUT.println("== " + name);
            try {
                @SuppressWarnings("unchecked")
                StreamCodec<ByteBuf, Object> codec =
                        (StreamCodec<ByteBuf, Object>) Class.forName(parts[1]).getField("STREAM_CODEC").get(null);
                byte[] data = Files.readAllBytes(dir.resolve(name + ".bin"));
                ByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(data), access);
                Object packet = codec.decode(buf);
                if (buf.readableBytes() != 0) {
                    OUT.println("!! " + buf.readableBytes() + " trailing bytes");
                }
                ByteBuf again = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
                codec.encode(again, packet);
                byte[] reencoded = ByteBufUtil.getBytes(again);
                int at = Arrays.mismatch(reencoded, data);
                OUT.println("## reencode " + (at < 0 ? "same" : "differs at byte " + at + ": vanilla writes "
                        + HexFormat.of().formatHex(reencoded, at, Math.min(reencoded.length, at + 24))));
                List<String> out = new ArrayList<>();
                dump("", packet, out, 0);
                out.forEach(OUT::println);
            } catch (Throwable t) {
                OUT.println("!! " + t);
            }
        }
    }

    static String join(String path, String name) {
        return path.isEmpty() ? name : path + "." + name;
    }

    static void dump(String path, Object o, List<String> out, int depth) throws Exception {
        if (depth > 16) {
            out.add(path + "=<too deep>");
        } else if (o == null) {
            out.add(path + "=null");
        } else if (o instanceof Optional<?> opt) {
            if (opt.isPresent()) dump(path, opt.get(), out, depth + 1);
            else out.add(path + "=empty");
        } else if (o instanceof OptionalInt opt) {
            out.add(path + "=" + (opt.isPresent() ? opt.getAsInt() : "empty"));
        } else if (o instanceof Number || o instanceof Boolean || o instanceof CharSequence || o instanceof UUID) {
            out.add(path + "=" + o);
        } else if (o instanceof Enum<?> e) {
            out.add(path + "=" + e.name());
        } else if (o instanceof Set<?> set) {
            out.add(path + "=" + set.stream().map(e -> e instanceof Enum<?> en ? en.name() : String.valueOf(e)).sorted().toList());
        } else if (o instanceof List<?> list) {
            if (list.isEmpty()) out.add(path + "=[]");
            for (int i = 0; i < list.size(); i++) dump(path + "[" + i + "]", list.get(i), out, depth + 1);
        } else if (o instanceof Multimap<?, ?> map) {
            List<?> values = new ArrayList<>(map.values());
            dump(path, values, out, depth + 1);
        } else if (o instanceof byte[] bytes) {
            out.add(path + "=" + HexFormat.of().formatHex(bytes));
        } else if (o instanceof Instant t) {
            out.add(path + "=" + t.toEpochMilli());
        } else if (o instanceof PublicKey k) {
            out.add(path + "=" + HexFormat.of().formatHex(k.getEncoded()));
        } else if (o instanceof Component c) {
            out.add(path + "=" + c.getString());
            if (c.getStyle().getColor() != null) out.add(path + ".color=" + c.getStyle().getColor().serialize());
        } else if (o instanceof EntityDataSerializer<?> s) {
            out.add(path + "=" + EntityDataSerializers.getSerializedId(s));
        } else if (o instanceof Holder<?> h) {
            out.add(path + "=" + h.unwrapKey().map(k -> k.identifier().toString()).orElse("<direct>"));
        } else if (o instanceof ResourceKey<?> k) {
            out.add(path + "=" + k.identifier());
        } else if (o instanceof EntityType<?> t) {
            out.add(path + "=" + BuiltInRegistries.ENTITY_TYPE.getKey(t));
        } else if (o instanceof EntityReference<?> r) {
            out.add(path + "=" + r.getUUID());
        } else if (o instanceof BlockState s) {
            out.add(path + "=" + s);
        } else if (o instanceof ParticleOptions p) {
            out.add(path + "=" + BuiltInRegistries.PARTICLE_TYPE.getKey(p.getType()));
        } else if (o instanceof ItemStack s) {
            out.add(path + "=" + (s.isEmpty() ? "empty" : s.getCount() + " " + BuiltInRegistries.ITEM.getKey(s.getItem())));
        } else if (o.getClass().getName().startsWith("java.")) {
            out.add(path + "=" + o);
        } else if (o.getClass().isRecord()) {
            for (var c : o.getClass().getRecordComponents()) {
                var accessor = c.getAccessor();
                accessor.setAccessible(true);
                dump(join(path, c.getName()), accessor.invoke(o), out, depth + 1);
            }
        } else {
            for (Class<?> c = o.getClass(); c != null && c != Object.class; c = c.getSuperclass()) {
                for (Field f : c.getDeclaredFields()) {
                    if (Modifier.isStatic(f.getModifiers())) continue;
                    f.setAccessible(true);
                    dump(join(path, f.getName()), f.get(o), out, depth + 1);
                }
            }
        }
    }

    static void entities(List<String> classes) throws Exception {
        for (Field f : EntityDataSerializers.class.getDeclaredFields()) {
            if (Modifier.isStatic(f.getModifiers()) && EntityDataSerializer.class.isAssignableFrom(f.getType())) {
                f.setAccessible(true);
                var s = (EntityDataSerializer<?>) f.get(null);
                OUT.println("serializer " + f.getName() + " " + EntityDataSerializers.getSerializedId(s));
            }
        }
        for (EntityType<?> t : BuiltInRegistries.ENTITY_TYPE) {
            var d = t.getDimensions();
            OUT.println("type " + BuiltInRegistries.ENTITY_TYPE.getKey(t) + " " + BuiltInRegistries.ENTITY_TYPE.getId(t)
                    + " " + d.width() + " " + d.height() + " " + d.eyeHeight() + " " + t.clientTrackingRange()
                    + " " + t.updateInterval() + " " + t.trackDeltas());
        }
        for (Field f : EntityTypes.class.getFields()) {
            if (f.getType() == EntityType.class && f.getGenericType() instanceof ParameterizedType p) {
                var key = BuiltInRegistries.ENTITY_TYPE.getKey((EntityType<?>) f.get(null));
                OUT.println("typeclass " + key + " " + ((Class<?>) p.getActualTypeArguments()[0]).getName());
            }
        }
        for (String name : classes) {
            Class<?> c = Class.forName(name.trim(), true, VanillaDump.class.getClassLoader());
            for (Field f : c.getDeclaredFields()) {
                if (Modifier.isStatic(f.getModifiers()) && f.getType() == EntityDataAccessor.class) {
                    f.setAccessible(true);
                    var a = (EntityDataAccessor<?>) f.get(null);
                    OUT.println("field " + c.getName() + " " + f.getName() + " " + a.id() + " "
                            + EntityDataSerializers.getSerializedId(a.serializer()));
                }
            }
        }
    }
}
