// Item stack facts and test vectors from vanilla 26.3's own codecs, with the vanilla datapack
// loaded the way the dedicated server loads it (so data-driven registries, tags and default item
// components are all bound).
//
// usage: java -cp <server jar + libraries> tools/ItemVectors.java defaults <out.json>
//        java -cp <server jar + libraries> tools/ItemVectors.java corpus <out-dir> [vector files...]
// (tools/item_vectors.py builds the classpath)
//
// defaults: every data component type (registry order) and every item's default components,
//           each value encoded with the component's network codec (hex); read by `cargo xtask codegen`.
// corpus:   <out-dir>/corpus.jsonl, one item stack per line, encoded with ItemStack.OPTIONAL_STREAM_CODEC,
//           OPTIONAL_UNTRUSTED_STREAM_CODEC and ItemStack.CODEC (network NBT), plus per patch entry its
//           network encoding, persistent NBT and container_click hash (HashedPatchMap). Stacks: every
//           item as-is and with its defaults removed, every distinct default component value moved
//           onto another item, the hand-written stacks in the vector files (command syntax, one per
//           line, optional leading count), and seeded random combinations and nestings of all of those.

import com.mojang.brigadier.StringReader;
import com.mojang.serialization.DynamicOps;
import com.mojang.serialization.Lifecycle;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import it.unimi.dsi.fastutil.objects.Reference2ObjectMap;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.Set;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.item.ItemParser;
import net.minecraft.core.Holder;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.component.TypedDataComponent;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.Tag;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.resources.RegistryOps;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.util.HashOps;
import net.minecraft.util.Util;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.BundleContents;
import net.minecraft.world.item.component.ChargedProjectiles;
import net.minecraft.world.item.component.ItemContainerContents;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.levelgen.presets.WorldPresets;

public class ItemVectors {
    // Captured before Bootstrap redirects System.out into the game's logger.
    static final PrintStream OUT = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);
    static final HexFormat HEX = HexFormat.of();
    static RegistryAccess access;
    static RegistryOps<Tag> nbtOps;
    static DynamicOps<com.google.common.hash.HashCode> hashOps;

    record Loaded(net.minecraft.server.ReloadableServerResources resources, RegistryAccess.Frozen access) {}

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        access = load();
        nbtOps = access.createSerializationContext(NbtOps.INSTANCE);
        hashOps = access.createSerializationContext(HashOps.CRC32C_INSTANCE);
        switch (args[0]) {
            case "defaults" -> defaults(Path.of(args[1]));
            case "corpus" -> corpus(Path.of(args[1]), List.of(args).subList(2, args.length));
            default -> throw new IllegalArgumentException("unknown mode " + args[0]);
        }
    }

    /** Loads the vanilla datapack like a dedicated server does and binds default item components. */
    static RegistryAccess load() throws Exception {
        PackRepository repo = ServerPacksSource.createVanillaTrustedRepository();
        var packs = new WorldLoader.PackConfig(repo, WorldDataConfiguration.DEFAULT, false, true);
        var init = new WorldLoader.InitConfig(packs, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
        Loaded loaded = WorldLoader.<Void, Loaded>load(init, ctx -> {
            Registry<LevelStem> none = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
            var dims = ctx.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET)
                    .getOrThrow(WorldPresets.NORMAL).value().createWorldDimensions().bake(none);
            return new WorldLoader.DataLoadOutput<>(null, dims.dimensionsRegistryAccess());
        }, (manager, resources, layers, cookie) -> {
            manager.close();
            return new Loaded(resources, layers.compositeAccess());
        }, Util.backgroundExecutor(), Runnable::run).get();
        loaded.resources().updateComponentsAndStaticRegistryTags();
        return loaded.access();
    }

    // ---- encoding helpers -------------------------------------------------------------------

    static String hex(ByteBuf buf) {
        return ByteBufUtil.hexDump(buf);
    }

    static <T> String wire(StreamCodec<? super RegistryFriendlyByteBuf, T> codec, T value) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        codec.encode(buf, value);
        return hex(buf);
    }

    static String nbt(Tag tag) {
        var buf = Unpooled.buffer();
        try {
            net.minecraft.nbt.NbtIo.writeAnyTag(tag, new io.netty.buffer.ByteBufOutputStream(buf));
        } catch (java.io.IOException e) {
            throw new RuntimeException(e);
        }
        return hex(buf);
    }

    @SuppressWarnings("unchecked")
    static <T> String componentWire(TypedDataComponent<T> c) {
        return wire((StreamCodec<? super RegistryFriendlyByteBuf, T>) c.type().streamCodec(), c.value());
    }

    static String componentNbt(TypedDataComponent<?> c) {
        if (c.type().isTransient()) return null;
        return c.encodeValue(nbtOps).mapOrElse(ItemVectors::nbt, e -> "!" + e.message());
    }

    static Integer componentHash(TypedDataComponent<?> c) {
        if (c.type().isTransient()) return null;
        return c.encodeValue(hashOps).mapOrElse(h -> h.asInt(), e -> null);
    }

    static String json(String s) {
        StringBuilder b = new StringBuilder("\"");
        for (char ch : s.toCharArray()) {
            switch (ch) {
                case '"' -> b.append("\\\"");
                case '\\' -> b.append("\\\\");
                case '\n' -> b.append("\\n");
                default -> {
                    if (ch < 0x20 || ch > 0x7e) b.append(String.format("\\u%04x", (int) ch));
                    else b.append(ch);
                }
            }
        }
        return b.append('"').toString();
    }

    static int typeId(DataComponentType<?> t) {
        return BuiltInRegistries.DATA_COMPONENT_TYPE.getId(t);
    }

    static String typeName(DataComponentType<?> t) {
        return BuiltInRegistries.DATA_COMPONENT_TYPE.getKey(t).toString();
    }

    // ---- defaults -----------------------------------------------------------------------------

    static void defaults(Path out) throws Exception {
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out, StandardCharsets.UTF_8))) {
            w.println("{\"components\": [");
            int n = BuiltInRegistries.DATA_COMPONENT_TYPE.size();
            for (int id = 0; id < n; id++) {
                DataComponentType<?> t = BuiltInRegistries.DATA_COMPONENT_TYPE.byId(id);
                w.printf("  {\"name\": %s, \"persistent\": %b}%s%n", json(typeName(t)), !t.isTransient(), id + 1 < n ? "," : "");
            }
            w.println("], \"items\": [");
            int items = BuiltInRegistries.ITEM.size();
            for (int id = 0; id < items; id++) {
                Item item = BuiltInRegistries.ITEM.byId(id);
                List<String> comps = new ArrayList<>();
                for (TypedDataComponent<?> c : item.components()) {
                    comps.add("[" + typeId(c.type()) + ", \"" + componentWire(c) + "\"]");
                }
                comps.sort(null);
                w.printf("  {\"name\": %s, \"components\": [%s]}%s%n", json(BuiltInRegistries.ITEM.getKey(item).toString()),
                        String.join(", ", comps), id + 1 < items ? "," : "");
            }
            w.println("]}");
        }
        OUT.println("wrote " + out);
    }

    // ---- corpus -------------------------------------------------------------------------------

    static Field patchMap;

    @SuppressWarnings("unchecked")
    static Reference2ObjectMap<DataComponentType<?>, Object> entries(DataComponentPatch patch) throws Exception {
        if (patchMap == null) {
            patchMap = DataComponentPatch.class.getDeclaredField("map");
            patchMap.setAccessible(true);
        }
        return (Reference2ObjectMap<DataComponentType<?>, Object>) patchMap.get(patch);
    }

    static int records;

    static boolean isRemoved(Object value) {
        return value.getClass().getName().equals("net.minecraft.core.component.Removed");
    }

    static void emit(PrintWriter w, String desc, ItemStack stack) throws Exception {
        StringBuilder b = new StringBuilder();
        b.append("{\"d\": ").append(json(desc));
        b.append(", \"w\": \"").append(wire(ItemStack.OPTIONAL_STREAM_CODEC, stack)).append('"');
        b.append(", \"u\": \"").append(wire(ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC, stack)).append('"');
        String nbt = stack.isEmpty() ? null : ItemStack.CODEC.encodeStart(nbtOps, stack).mapOrElse(ItemVectors::nbt, e -> "!" + e.message());
        b.append(", \"n\": ").append(nbt == null ? "null" : json(nbt));
        b.append(", \"c\": [");
        List<String> removed = new ArrayList<>();
        boolean first = true;
        if (!stack.isEmpty()) {
            for (var e : entries(stack.getComponentsPatch()).reference2ObjectEntrySet()) {
                DataComponentType<?> t = e.getKey();
                if (isRemoved(e.getValue())) {
                    removed.add(String.valueOf(typeId(t)));
                    continue;
                }
                var c = TypedDataComponent.createUnchecked(t, e.getValue());
                String cn = componentNbt(c);
                Integer h = componentHash(c);
                b.append(first ? "" : ", ").append('[').append(typeId(t)).append(", \"").append(componentWire(c)).append("\", ")
                        .append(cn == null ? "null" : json(cn)).append(", ").append(h == null ? "null" : h.toString()).append(']');
                first = false;
            }
        }
        b.append("], \"r\": [").append(String.join(", ", removed)).append(']');
        // What a client would send for this stack in container_click (absent when a transient
        // component makes vanilla's hash generator fail).
        try {
            net.minecraft.network.HashedPatchMap.HashGenerator hashes = c -> c.encodeValue(hashOps).getOrThrow().asInt();
            String hashed = wire(net.minecraft.network.HashedStack.STREAM_CODEC, net.minecraft.network.HashedStack.create(stack, hashes));
            b.append(", \"h\": \"").append(hashed).append('"');
        } catch (RuntimeException e) {
            // no hashed form
        }
        w.println(b.append('}'));
        records++;
    }

    static ItemStack stack(Holder<Item> item, int count, DataComponentPatch patch) {
        return new ItemStack(item, count, patch);
    }

    static void corpus(Path outDir, List<String> vectorFiles) throws Exception {
        Files.createDirectories(outDir);
        Path out = outDir.resolve("corpus.jsonl");
        var items = BuiltInRegistries.ITEM;
        Random rng = new Random(0x4b494c4eL);
        // Distinct default component values across all items, in first-seen order.
        Set<TypedDataComponent<?>> pool = new LinkedHashSet<>();
        List<ItemStack> interesting = new ArrayList<>();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out, StandardCharsets.UTF_8))) {
            emit(w, "empty", ItemStack.EMPTY);
            for (Item item : items) {
                if (item == net.minecraft.world.item.Items.AIR) continue;
                Holder<Item> h = items.wrapAsHolder(item);
                String name = items.getKey(item).toString();
                emit(w, name, new ItemStack(h, 1));
                DataComponentPatch.Builder removeAll = DataComponentPatch.builder();
                for (TypedDataComponent<?> c : item.components()) {
                    pool.add(c);
                    removeAll.remove(c.type());
                }
                emit(w, name + " without defaults", stack(h, 1 + rng.nextInt(99), removeAll.build()));
            }
            // Every default value on an item that does not have it.
            List<Holder<Item>> hosts = List.of(items.wrapAsHolder(net.minecraft.world.item.Items.STICK),
                    items.wrapAsHolder(net.minecraft.world.item.Items.APPLE));
            for (TypedDataComponent<?> c : pool) {
                for (Holder<Item> host : hosts) {
                    if (!java.util.Objects.equals(c.value(), host.value().components().get(c.type()))) {
                        ItemStack s = stack(host, 1, DataComponentPatch.builder().set(c).build());
                        emit(w, "default " + typeName(c.type()), s);
                        interesting.add(s);
                        break;
                    }
                }
            }
            // Hand-written stacks.
            ItemParser parser = new ItemParser(access);
            for (String file : vectorFiles) {
                int lineNo = 0;
                for (String line : Files.readAllLines(Path.of(file), StandardCharsets.UTF_8)) {
                    lineNo++;
                    String text = line.strip();
                    if (text.isEmpty() || text.startsWith("#")) continue;
                    int count = 1;
                    int sp = text.indexOf(' ');
                    if (sp > 0 && text.substring(0, sp).chars().allMatch(Character::isDigit)) {
                        count = Integer.parseInt(text.substring(0, sp));
                        text = text.substring(sp + 1).strip();
                    }
                    String where = Path.of(file).getFileName() + ":" + lineNo;
                    try {
                        var input = parser.parse(new StringReader(text));
                        ItemStack s = stack(input.item(), count, input.components());
                        emit(w, where, s);
                        interesting.add(s);
                        for (var e : entries(input.components()).reference2ObjectEntrySet()) {
                            if (!isRemoved(e.getValue())) {
                                pool.add(TypedDataComponent.createUnchecked(e.getKey(), e.getValue()));
                            }
                        }
                    } catch (Exception e) {
                        OUT.println("!! " + where + ": " + e.getMessage());
                    }
                }
            }
            // Random combinations of known values, with removals.
            List<TypedDataComponent<?>> values = new ArrayList<>(pool);
            List<Item> all = items.stream().filter(i -> i != net.minecraft.world.item.Items.AIR).toList();
            for (int i = 0; i < 3000; i++) {
                Item item = all.get(rng.nextInt(all.size()));
                DataComponentPatch.Builder patch = DataComponentPatch.builder();
                int adds = 1 + rng.nextInt(6);
                for (int k = 0; k < adds; k++) patch.set(values.get(rng.nextInt(values.size())));
                List<TypedDataComponent<?>> defaults = new ArrayList<>();
                item.components().forEach(defaults::add);
                int removes = defaults.isEmpty() ? 0 : rng.nextInt(3);
                for (int k = 0; k < removes; k++) patch.remove(defaults.get(rng.nextInt(defaults.size())).type());
                ItemStack s = stack(items.wrapAsHolder(item), 1 + rng.nextInt(99), patch.build());
                emit(w, "random " + i, s);
                if (i % 10 == 0) interesting.add(s);
            }
            // Nested stacks in containers, bundles and crossbows.
            for (int i = 0; i < 300; i++) {
                List<ItemStack> inner = new ArrayList<>();
                int n = 1 + rng.nextInt(5);
                for (int k = 0; k < n; k++) inner.add(interesting.get(rng.nextInt(interesting.size())).copy());
                DataComponentPatch.Builder patch = DataComponentPatch.builder();
                Holder<Item> host;
                switch (i % 3) {
                    case 0 -> {
                        host = items.wrapAsHolder(net.minecraft.world.item.Items.SHULKER_BOX);
                        patch.set(DataComponents.CONTAINER, ItemContainerContents.fromItems(inner));
                    }
                    case 1 -> {
                        host = items.wrapAsHolder(net.minecraft.world.item.Items.BUNDLE);
                        patch.set(DataComponents.BUNDLE_CONTENTS, new BundleContents(inner.stream().map(ItemStack::copy)
                                .map(net.minecraft.world.item.ItemStackTemplate::fromNonEmptyStack).toList()));
                    }
                    default -> {
                        host = items.wrapAsHolder(net.minecraft.world.item.Items.CROSSBOW);
                        patch.set(DataComponents.CHARGED_PROJECTILES, ChargedProjectiles.ofNonEmpty(inner));
                    }
                }
                ItemStack s = stack(host, 1, patch.build());
                emit(w, "nested " + i, s);
                interesting.add(s);
            }
        }
        OUT.println("wrote " + records + " stacks (" + pool.size() + " distinct component values) to " + out);
    }
}
