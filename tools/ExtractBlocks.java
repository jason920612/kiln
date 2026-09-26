// Dumps per-block-state facts the data generator does not report (light, collision boxes,
// hardness, block entity type, ...) by querying the vanilla 26.3 game API. Heightmap
// membership is tag-driven in 26.3 (blocks_motion_in_heightmap*) and comes from the
// generated tags instead.
// Run through `cargo xtask extract`; output: one JSON object per state id, in id order.
//
// usage: java -cp <server jar + libraries> tools/ExtractBlocks.java <out.json>

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonParser;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.tags.TagKey;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;

public class ExtractBlocks {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var getter = EmptyBlockGetter.INSTANCE;
        var pos = BlockPos.ZERO;
        int n = Block.BLOCK_STATE_REGISTRY.size();
        // The server computes per-state caches (solidity, shapes, light) during startup;
        // without this, cached answers such as legacy solidity are defaults.
        for (int id = 0; id < n; id++) {
            Block.BLOCK_STATE_REGISTRY.byId(id).initCache();
        }
        List<String> rows = new ArrayList<>(n);
        for (int id = 0; id < n; id++) {
            BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
            int faces = 0;
            for (Direction d : Direction.values()) {
                if (Block.isFaceFull(s.getFaceOcclusionShape(d), d)) {
                    faces |= 1 << d.get3DDataValue();
                }
            }
            StringBuilder boxes = new StringBuilder();
            for (AABB b : s.getCollisionShape(getter, pos).toAabbs()) {
                if (boxes.length() > 0) boxes.append(',');
                boxes.append(String.format(Locale.ROOT, "[%s,%s,%s,%s,%s,%s]",
                        num(b.minX), num(b.minY), num(b.minZ), num(b.maxX), num(b.maxY), num(b.maxZ)));
            }
            String beType = "null";
            for (BlockEntityType<?> t : BuiltInRegistries.BLOCK_ENTITY_TYPE) {
                if (t.isValid(s)) {
                    beType = "\"" + BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(t) + "\"";
                    break;
                }
            }
            rows.add(String.format(Locale.ROOT,
                    "{\"id\":%d,\"emission\":%d,\"dampening\":%d,\"sky_down\":%b,\"shape_occludes\":%b,"
                            + "\"full_faces\":%d,\"can_occlude\":%b,\"solid_render\":%b,\"air\":%b,\"liquid\":%b,"
                            + "\"replaceable\":%b,\"random_ticks\":%b,\"block_entity\":%b,\"hardness\":%s,"
                            + "\"correct_tool\":%b,\"full_collision\":%b,\"block_entity_type\":%s,\"collision\":[%s]",
                    id, s.getLightEmission(), s.getLightDampening(), s.propagatesSkylightDown(),
                    s.useShapeForLightOcclusion(), faces, s.canOcclude(), s.isSolidRender(), s.isAir(),
                    s.liquid(), s.canBeReplaced(), s.isRandomlyTicking(), s.hasBlockEntity(),
                    num(s.getDestroySpeed(getter, pos)), s.requiresCorrectToolForDrops(),
                    s.isCollisionShapeFullBlock(getter, pos), beType, boxes));
        }
        // Tag-dependent answers below need the block tags, which only a running server loads;
        // bind them from the data generator output (the extractor runs in the work directory).
        bindBlockTags(Path.of("generated/data/minecraft/tags/block"));
        Map<Block, Integer> keepGroups = keepGroups();
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            out.println("[");
            for (int id = 0; id < n; id++) {
                Block block = Block.BLOCK_STATE_REGISTRY.byId(id).getBlock();
                out.printf(Locale.ROOT, "%s,\"keep_block_entity_group\":%d}%s%n", rows.get(id),
                        keepGroups.getOrDefault(block, 0), id + 1 < n ? "," : "");
            }
            out.println("]");
        }
    }

    private static void bindBlockTags(Path dir) throws Exception {
        Map<String, JsonArray> raw = new HashMap<>();
        try (var files = Files.walk(dir)) {
            for (Path p : (Iterable<Path>) files.filter(f -> f.toString().endsWith(".json"))::iterator) {
                String rel = dir.relativize(p).toString().replace('\\', '/');
                String name = "minecraft:" + rel.substring(0, rel.length() - ".json".length());
                raw.put(name, JsonParser.parseString(Files.readString(p)).getAsJsonObject().getAsJsonArray("values"));
            }
        }
        Map<TagKey<Block>, List<Holder<Block>>> tags = new HashMap<>();
        for (String name : raw.keySet()) {
            Set<Block> members = new LinkedHashSet<>();
            resolveTag(name, raw, members);
            List<Holder<Block>> holders = new ArrayList<>();
            for (Block b : members) holders.add(b.builtInRegistryHolder());
            tags.put(TagKey.create(Registries.BLOCK, Identifier.parse(name)), holders);
        }
        BuiltInRegistries.BLOCK.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK, tags)).apply();
    }

    private static void resolveTag(String name, Map<String, JsonArray> raw, Set<Block> out) {
        for (JsonElement v : raw.get(name)) {
            boolean required = !v.isJsonObject() || !v.getAsJsonObject().has("required")
                    || v.getAsJsonObject().get("required").getAsBoolean();
            String id = v.isJsonObject() ? v.getAsJsonObject().get("id").getAsString() : v.getAsString();
            if (id.startsWith("#")) {
                if (raw.containsKey(id.substring(1))) resolveTag(id.substring(1), raw, out);
                else if (required) throw new IllegalStateException("unknown tag " + id);
            } else {
                var block = BuiltInRegistries.BLOCK.getOptional(Identifier.parse(id));
                if (block.isPresent()) out.add(block.get());
                else if (required) throw new IllegalStateException("unknown block " + id);
            }
        }
    }

    // When a block is replaced by a different block, the chunk keeps the old block entity only
    // if the new block's shouldChangedStateKeepBlockEntity(old state) says so (copper chests
    // and statues across oxidation stages). Blocks that keep each other's block entities get
    // the same group number (1..); 0 means never. Fails if the relation is not an equivalence.
    private static Map<Block, Integer> keepGroups() {
        List<Block> withEntity = new ArrayList<>();
        for (Block b : BuiltInRegistries.BLOCK) {
            if (b.defaultBlockState().hasBlockEntity()) withEntity.add(b);
        }
        Map<Block, Integer> groups = new HashMap<>();
        int next = 1;
        for (Block b : withEntity) {
            List<Block> keeps = new ArrayList<>();
            for (Block old : withEntity) {
                if (old != b && b.defaultBlockState().shouldChangedStateKeepBlockEntity(old.defaultBlockState())) {
                    keeps.add(old);
                }
            }
            if (keeps.isEmpty()) continue;
            Integer g = null;
            for (Block old : keeps) {
                if (groups.containsKey(old)) g = groups.get(old);
            }
            if (g == null) g = next++;
            groups.put(b, g);
            for (Block old : keeps) {
                if (groups.getOrDefault(old, g) != (int) g) throw new IllegalStateException("keep relation of " + b);
                groups.put(old, g);
            }
        }
        for (Block b : groups.keySet()) {
            for (Block old : groups.keySet()) {
                boolean same = groups.get(b).equals(groups.get(old));
                if (b != old && same != b.defaultBlockState().shouldChangedStateKeepBlockEntity(old.defaultBlockState())) {
                    throw new IllegalStateException("keep relation is not an equivalence: " + b + " / " + old);
                }
            }
        }
        return groups;
    }

    private static String num(double v) {
        return v == Math.rint(v) ? String.valueOf((long) v) : String.valueOf(v);
    }
}
