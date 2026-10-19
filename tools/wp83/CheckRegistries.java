// wp83: what vanilla 26.3 sends a client for a set of data packs, against what Kiln sends.
//
// usage: java -cp <server jar + libraries> tools/wp83/CheckRegistries.java <dir with the packs> <kiln dump> [vanilla dump out]
//
// Loads the registries like the dedicated server does (`WorldLoader.load` with the packs of <dir> enabled on top
// of the vanilla pack), then builds the `Registry Data` entries (`RegistrySynchronization.packRegistries` for a client that knows
// `minecraft:core`) and the `Update Tags` payload. <kiln dump> is `wp83_dump` of Kiln on the same packs (lines
// `entry <registry> <name> <snbt or ->` and `tags <registry> <tag> <id,id,...>`). Checks, per registry, that the entry names
// come in the same order, that the same entries carry data, that the data decodes with the registry's network codec and encodes
// back to what vanilla sent (so Kiln's numbers/booleans/ids/text forms mean the same), and that every tag has the same ids in
// the same order. Prints the differences; exit code 1 if there are any.

import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.Lifecycle;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import net.minecraft.commands.Commands;
import net.minecraft.core.LayeredRegistryAccess;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistrySynchronization;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtOps;
import net.minecraft.nbt.Tag;
import net.minecraft.nbt.TagParser;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.RegistryOps;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.KnownPack;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.tags.TagNetworkSerialization;
import net.minecraft.util.Util;
import net.minecraft.world.level.DataPackConfig;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.flag.FeatureFlags;

public final class CheckRegistries {
    record Entry(String name, Tag data) {}

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Path packs = Path.of(args[0]);
        PackRepository repo = ServerPacksSource.createPackRepository(packs, new net.minecraft.world.level.validation.DirectoryValidator(p -> true));
        repo.reload();
        List<String> selected = new ArrayList<>(List.of("vanilla"));
        for (String id : repo.getAvailableIds()) {
            if (id.startsWith("file/")) selected.add(id);
        }
        repo.setSelected(selected);
        System.out.println("packs: " + repo.getSelectedIds());
        var config = new WorldLoader.PackConfig(repo, new WorldDataConfiguration(new DataPackConfig(selected, List.of()), FeatureFlags.DEFAULT_FLAGS), false, false);
        var init = new WorldLoader.InitConfig(config, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
        LayeredRegistryAccess<RegistryLayer> layers = WorldLoader.<Void, LayeredRegistryAccess<RegistryLayer>>load(init, ctx -> {
            Registry<LevelStem> none = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
            var dims = ctx.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET).getOrThrow(WorldPresets.NORMAL).value().createWorldDimensions().bake(none);
            return new WorldLoader.DataLoadOutput<>(null, dims.dimensionsRegistryAccess());
        }, (manager, res, layered, cookie) -> {
            manager.close();
            return layered;
        }, Util.backgroundExecutor(), Runnable::run).get();

        // Vanilla's Registry Data for a client that knows minecraft:core.
        var ops = layers.compositeAccess().createSerializationContext(NbtOps.INSTANCE);
        Map<String, List<Entry>> vanilla = new LinkedHashMap<>();
        RegistrySynchronization.packRegistries(ops, layers.getAccessFrom(RegistryLayer.WORLD), Set.of(KnownPack.vanilla("core")), (key, entries) -> {
            List<Entry> list = new ArrayList<>();
            for (var e : entries) list.add(new Entry(e.id().toString(), e.data().orElse(null)));
            vanilla.put(key.identifier().toString(), list);
        });
        // Vanilla's tags.
        Map<String, Map<String, List<Integer>>> vanillaTags = new TreeMap<>();
        TagNetworkSerialization.serializeTagsToNetwork(layers).forEach((key, payload) -> {
            Map<String, List<Integer>> m = new TreeMap<>();
            payload.tags().forEach((tag, ids) -> m.put(tag.toString(), new ArrayList<>(ids)));
            vanillaTags.put(key.identifier().toString(), m);
        });
        if (args.length > 2) {
            try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[2])))) {
                vanilla.forEach((reg, list) -> list.forEach(e -> w.println("entry " + reg + " " + e.name() + " " + (e.data() == null ? "-" : e.data()))));
                vanillaTags.forEach((reg, tags) -> tags.forEach((tag, ids) -> w.println("tags " + reg + " " + tag + " " + ids.toString().replaceAll("[\\[\\] ]", ""))));
            }
        }

        // Kiln's dump.
        Map<String, List<Entry>> kiln = new LinkedHashMap<>();
        Map<String, Map<String, List<Integer>>> kilnTags = new TreeMap<>();
        for (String line : Files.readAllLines(Path.of(args[1]))) {
            String[] p = line.split(" ", 4);
            if (p[0].equals("entry")) {
                Tag data = p[3].equals("-") ? null : TagParser.parseCompoundFully(p[3]);
                kiln.computeIfAbsent(p[1], k -> new ArrayList<>()).add(new Entry(p[2], data));
            } else if (p[0].equals("tags")) {
                String[] t = line.split(" ", 4);
                List<Integer> ids = new ArrayList<>();
                if (t.length > 3 && !t[3].isEmpty()) for (String s : t[3].split(",")) ids.add(Integer.parseInt(s));
                kilnTags.computeIfAbsent(t[1], k -> new TreeMap<>()).put(t[2], ids);
            }
        }

        int problems = 0;
        var lookup = layers.compositeAccess();
        var decodeOps = RegistryOps.create(NbtOps.INSTANCE, lookup);
        Map<String, RegistryDataLoader.RegistryData<?>> codecs = new HashMap<>();
        for (var d : RegistryDataLoader.SYNCHRONIZED_REGISTRIES) codecs.put(d.key().identifier().toString(), d);
        int compared = 0;
        for (var reg : vanilla.entrySet()) {
            List<Entry> v = reg.getValue();
            List<Entry> k = kiln.get(reg.getKey());
            if (k == null) {
                System.out.println("MISSING registry in Kiln: " + reg.getKey());
                problems++;
                continue;
            }
            List<String> vn = v.stream().map(Entry::name).toList();
            List<String> kn = k.stream().map(Entry::name).toList();
            if (!vn.equals(kn)) {
                System.out.println("ORDER/NAMES differ in " + reg.getKey() + ": vanilla " + vn.size() + " kiln " + kn.size());
                for (int i = 0; i < Math.min(vn.size(), kn.size()); i++) {
                    if (!vn.get(i).equals(kn.get(i))) {
                        System.out.println("  first difference at " + i + ": vanilla " + vn.get(i) + " kiln " + kn.get(i));
                        break;
                    }
                }
                problems++;
                continue;
            }
            for (int i = 0; i < v.size(); i++) {
                Tag vd = v.get(i).data(), kd = k.get(i).data();
                if ((vd == null) != (kd == null)) {
                    System.out.println("DATA PRESENCE differs for " + reg.getKey() + " " + vn.get(i) + ": vanilla " + (vd != null) + " kiln " + (kd != null));
                    problems++;
                    continue;
                }
                if (vd == null) continue;
                compared++;
                @SuppressWarnings("unchecked")
                Codec<Object> codec = (Codec<Object>) codecs.get(reg.getKey()).elementCodec();
                DataResult<Object> decoded = codec.parse(decodeOps, kd);
                if (decoded.error().isPresent()) {
                    System.out.println("DECODE FAILS for " + reg.getKey() + " " + vn.get(i) + ": " + decoded.error().get().message() + "\n  kiln: " + kd);
                    problems++;
                    continue;
                }
                Tag again = codec.encodeStart(layers.compositeAccess().createSerializationContext(NbtOps.INSTANCE), decoded.result().get()).getOrThrow();
                if (!again.equals(vd)) {
                    System.out.println("DATA differs for " + reg.getKey() + " " + vn.get(i) + "\n  vanilla: " + vd + "\n  kiln->vanilla codec: " + again + "\n  kiln: " + kd);
                    problems++;
                }
            }
        }
        for (String reg : kiln.keySet()) {
            if (!vanilla.containsKey(reg)) {
                System.out.println("EXTRA registry in Kiln: " + reg);
                problems++;
            }
        }
        int tagsCompared = 0;
        for (var reg : vanillaTags.entrySet()) {
            Map<String, List<Integer>> kt = kilnTags.getOrDefault(reg.getKey(), Map.of());
            for (var tag : reg.getValue().entrySet()) {
                tagsCompared++;
                List<Integer> other = kt.get(tag.getKey());
                if (other == null) {
                    System.out.println("TAG missing in Kiln: " + reg.getKey() + " " + tag.getKey());
                    problems++;
                } else if (!other.equals(tag.getValue())) {
                    System.out.println("TAG differs: " + reg.getKey() + " " + tag.getKey() + "\n  vanilla " + tag.getValue() + "\n  kiln    " + other);
                    problems++;
                }
            }
            for (String t : kt.keySet()) {
                if (!reg.getValue().containsKey(t)) {
                    System.out.println("TAG extra in Kiln: " + reg.getKey() + " " + t);
                    problems++;
                }
            }
        }
        for (String reg : kilnTags.keySet()) {
            if (!vanillaTags.containsKey(reg) && !kilnTags.get(reg).isEmpty()) {
                System.out.println("TAG registry extra in Kiln: " + reg);
                problems++;
            }
        }
        System.out.println("registries " + vanilla.size() + ", entries with data compared " + compared + ", tags compared " + tagsCompared + ", problems " + problems);
        System.exit(problems == 0 ? 0 : 1);
    }
}
